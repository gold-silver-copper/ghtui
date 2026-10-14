//! Writes waiting for GitHub. A write GitHub couldn't be reached for
//! (one that never left, [`ApiError::Offline`]) is queued, shown on its
//! page as queued, and sent, in order, once GitHub is back.
//!
//! Each write is sent once at most. Before it's sent, the queue writes it
//! down as being sent ([`Status::Sending`]), durably; GitHub's answer then
//! takes it out (taken), or marks it refused with GitHub's reason (its
//! text kept, to copy or send again). A write whose fate isn't known (the
//! connection dropped after it left, or ghtui stopped while sending it)
//! is never sent again on its own, unless sending it twice changes
//! nothing (a label, a reaction, closing): it's marked unsure, for you to
//! check and send again or throw away. The queue is kept in the cache's
//! database, apart for each host and account, so it outlives a restart.

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::api::change::{Change, CloseReason};
use crate::api::discussion::Discuss;
use crate::api::edit::Edit;
use crate::api::model::{PrRef, ReviewEvent};
use crate::api::{ApiError, GitHub};
use crate::store::{DraftComment, Store};

/// A write waiting for GitHub, or one GitHub didn't take.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Its place in the queue: the order it's sent in (0 until queued).
    pub id: u64,
    /// When it was written (Unix seconds).
    pub at: u64,
    pub write: Write,
    /// The page it shows on, as queued (its link).
    pub page: String,
    /// That page's title.
    pub title: String,
    pub status: Status,
}

/// Where a queued write is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    /// Waiting to be sent.
    Queued,
    /// On its way: written down before it's sent.
    Sending,
    /// GitHub refused it, and said why.
    Refused(String),
    /// It may or may not have reached GitHub, and why that isn't known.
    Unsure(String),
}

/// What's queued: a change that can wait for GitHub ([`Write::of`]), or
/// a review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Write {
    Change(Change),
    /// A review, with its comments.
    Review(Review),
}

/// A review to submit, as it was when it was written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Review {
    pub pr: PrRef,
    /// The head its comments were written against.
    pub head: String,
    pub drafts: Vec<DraftComment>,
    pub event: ReviewEvent,
    pub body: String,
    /// The comments in your pending review on GitHub you'd seen.
    pub seen: u64,
}

impl Write {
    /// `change`, if it can wait for GitHub: merging and the like can't,
    /// as what they'd do depends on GitHub's state when they're sent.
    pub fn of(change: &Change) -> Option<Self> {
        let waits = matches!(
            change,
            Change::Comment { .. }
                | Change::CommitComment { .. }
                | Change::Reply { .. }
                | Change::Star { .. }
                | Change::SetPrOpen { .. }
                | Change::CloseIssue { .. }
                | Change::ReopenIssue { .. }
                | Change::Edit(_)
                | Change::Discussion(_)
        );
        waits.then(|| Write::Change(change.clone()))
    }

    /// Whether sending it twice does what sending it once does: then a
    /// send whose fate isn't known is simply sent again.
    pub fn idempotent(&self) -> bool {
        match self {
            Write::Review(_)
            | Write::Change(Change::Comment { .. } | Change::CommitComment { .. })
            | Write::Change(Change::Reply { .. }) => false,
            Write::Change(Change::Discussion(d)) => !matches!(d, Discuss::Comment { .. }),
            Write::Change(_) => true,
        }
    }

    /// What it is, as people say it ("A comment", "Closing the issue").
    pub fn what(&self) -> String {
        let change = match self {
            Write::Change(change) => change,
            Write::Review(r) => {
                let event = match r.event {
                    ReviewEvent::Approve => "An approval",
                    ReviewEvent::RequestChanges => "A review asking for changes",
                    ReviewEvent::Comment => "A review",
                };
                return match r.drafts.len() {
                    0 => event.to_owned(),
                    1 => format!("{event}, with a comment"),
                    n => format!("{event}, with {n} comments"),
                };
            }
        };
        match change {
            Change::Comment { .. } | Change::CommitComment { .. } => "A comment".into(),
            Change::Reply { .. } => "A reply".into(),
            Change::Star { starred: true, .. } => "Starring the repository".into(),
            Change::Star { .. } => "Unstarring the repository".into(),
            Change::SetPrOpen { open: true, .. } => "Reopening the pull request".into(),
            Change::SetPrOpen { .. } => "Closing the pull request".into(),
            Change::CloseIssue { reason, .. } => match reason {
                CloseReason::Completed => "Closing the issue as completed".into(),
                CloseReason::NotPlanned => "Closing the issue as not planned".into(),
                CloseReason::Duplicate => "Closing the issue as a duplicate".into(),
            },
            Change::ReopenIssue { .. } => "Reopening the issue".into(),
            Change::Edit(edit) => match edit {
                Edit::Body { .. } => "An edit".into(),
                Edit::Delete { .. } => "Deleting a comment".into(),
                Edit::Title { title, .. } => format!("A new title: {title}"),
                Edit::React { content, on, .. } => format!(
                    "{} {}",
                    if *on { "Reacting" } else { "Taking back" },
                    content.emoji()
                ),
                Edit::Labels { add, remove, .. } => format!("Labels{}", changes(add, remove)),
                Edit::Assignees { add, remove, .. } => {
                    format!("Assignees{}", changes(add, remove))
                }
                Edit::Reviewers { .. } => "Asking for reviews".into(),
                Edit::Milestone { milestone, .. } => match milestone {
                    Some(m) => format!("The milestone {}", m.name),
                    None => "No milestone".into(),
                },
            },
            Change::Discussion(d) => match d {
                Discuss::Comment {
                    reply_to: Some(_), ..
                } => "A reply".into(),
                Discuss::Comment { .. } => "A comment".into(),
                Discuss::Edit { .. } => "An edit".into(),
                Discuss::Delete { .. } => "Deleting a comment".into(),
                Discuss::Answer { on: true, .. } => "Marking the answer".into(),
                Discuss::Answer { .. } => "Unmarking the answer".into(),
            },
            // Never queued.
            _ => "A change".into(),
        }
    }

    /// The text written, to show and to copy.
    pub fn text(&self) -> Option<String> {
        match self {
            Write::Change(
                Change::Comment { body, .. }
                | Change::CommitComment { body, .. }
                | Change::Reply { body, .. }
                | Change::Edit(Edit::Body { body, .. })
                | Change::Discussion(Discuss::Comment { body, .. } | Discuss::Edit { body, .. }),
            ) => Some(body.clone()),
            Write::Review(r) => {
                let comments = r.drafts.iter().map(|d| match d.line {
                    Some(line) => format!("{}:{line}\n{}", d.path, d.body),
                    None => format!("{}\n{}", d.path, d.body),
                });
                let all: Vec<String> = std::iter::once(r.body.clone())
                    .filter(|b| !b.trim().is_empty())
                    .chain(comments)
                    .collect();
                (!all.is_empty()).then(|| all.join("\n\n"))
            }
            Write::Change(_) => None,
        }
    }
}

/// ` +bug −wontfix`.
fn changes(add: &[crate::api::edit::Named], remove: &[crate::api::edit::Named]) -> String {
    let add = add.iter().map(|n| format!(" +{}", n.name));
    let remove = remove.iter().map(|n| format!(" −{}", n.name));
    add.chain(remove).collect()
}

/// How sending a write went.
#[derive(Debug)]
pub enum Sent {
    /// GitHub took it.
    Taken,
    /// GitHub can't be reached: it never left.
    Unsent,
    /// GitHub refused it, and why.
    Refused(String),
    /// It may have reached GitHub, and why that isn't known.
    Unknown(String),
    /// A review GitHub took some of: the rest, refused, and why.
    Partly(Review, String),
}

impl Sent {
    /// How a change's answer went.
    pub fn of(result: Result<(), ApiError>) -> Self {
        match result {
            Ok(()) => Sent::Taken,
            Err(ApiError::Offline(_)) => Sent::Unsent,
            Err(err @ ApiError::Network(_)) => Sent::Unknown(err.to_string()),
            Err(ApiError::GraphQl(messages)) => Sent::Refused(messages.join("; ")),
            Err(err) => Sent::Refused(err.to_string()),
        }
    }
}

/// What a queue tells the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Told {
    /// The queue as it is now.
    All(Vec<Entry>),
    /// This one reached GitHub.
    Taken(Box<Entry>),
}

/// The queue, as kept: in memory, and in the store before anything is
/// sent.
pub struct Queue {
    store: Store,
    entries: Vec<Entry>,
}

impl Queue {
    /// The queue the store keeps. A write that was being sent when ghtui
    /// stopped may have reached GitHub: one that's safe to send again is
    /// queued again, any other is unsure.
    pub fn load(store: Store) -> Self {
        let mut entries: Vec<Entry> = store
            .queue_all::<Entry>()
            .into_iter()
            .map(|(id, entry)| Entry { id, ..entry })
            .collect();
        for entry in &mut entries {
            if entry.status == Status::Sending {
                entry.status = if entry.write.idempotent() {
                    Status::Queued
                } else {
                    Status::Unsure("ghtui stopped while sending it".into())
                };
                store.queue_put(entry.id, entry);
            }
        }
        Self { store, entries }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Queues `entry` last (or, given its old place, there again), and
    /// keeps it; false when it couldn't be kept.
    pub fn add(&mut self, mut entry: Entry) -> bool {
        if entry.id == 0 || self.entries.iter().any(|e| e.id == entry.id) {
            entry.id = self.entries.iter().map(|e| e.id).max().unwrap_or(0) + 1;
        }
        if entry.status == Status::Sending {
            entry.status = Status::Queued;
        }
        let kept = self.store.queue_put(entry.id, &entry);
        let at = self.entries.partition_point(|e| e.id < entry.id);
        self.entries.insert(at, entry);
        kept
    }

    /// Queues a refused or unsure write again.
    pub fn retry(&mut self, id: u64) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id)
            && entry.status != Status::Sending
        {
            entry.status = Status::Queued;
            self.store.queue_put(id, entry);
        }
    }

    /// Throws a write away (not one being sent).
    pub fn discard(&mut self, id: u64) {
        let sending = self
            .entries
            .iter()
            .any(|e| e.id == id && e.status == Status::Sending);
        if !sending {
            self.entries.retain(|e| e.id != id);
            self.store.queue_remove(id);
        }
    }

    /// The next write to send, marked as being sent first, durably. None
    /// when nothing's queued, or when that couldn't be written down (then
    /// it isn't sent: after a restart it might be sent again). Without a
    /// store at all (another ghtui has it), the queue lives and dies with
    /// this ghtui, and is sent all the same.
    pub fn take_next(&mut self) -> Option<Entry> {
        let entry = self
            .entries
            .iter_mut()
            .find(|e| e.status == Status::Queued)?;
        entry.status = Status::Sending;
        if !self.store.queue_put(entry.id, entry) && self.store.is_enabled() {
            entry.status = Status::Queued;
            return None;
        }
        Some(entry.clone())
    }

    /// Settles write `id` with how sending it went; whether to go on
    /// sending (not when GitHub can't be reached).
    pub fn settle(&mut self, id: u64, sent: Sent) -> bool {
        let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) else {
            return true;
        };
        let (status, go_on) = match sent {
            Sent::Taken => {
                self.store.queue_remove(id);
                self.entries.retain(|e| e.id != id);
                return true;
            }
            Sent::Unsent => (Status::Queued, false),
            Sent::Unknown(_) if entry.write.idempotent() => (Status::Queued, false),
            Sent::Unknown(why) => (Status::Unsure(why), true),
            Sent::Refused(why) => (Status::Refused(why), true),
            Sent::Partly(rest, why) => {
                entry.write = Write::Review(rest);
                (Status::Refused(why), true)
            }
        };
        entry.status = status;
        self.store.queue_put(id, entry);
        go_on
    }

    /// Sends what's queued, in order, one at a time, with `send`; stops
    /// when GitHub can't be reached. Tells `told` each change.
    pub async fn flush<F, Fut>(&mut self, mut send: F, told: &mut impl FnMut(Told))
    where
        F: FnMut(Write) -> Fut,
        Fut: Future<Output = Sent>,
    {
        while let Some(entry) = self.take_next() {
            told(Told::All(self.entries.clone()));
            let sent = send(entry.write.clone()).await;
            let taken = matches!(sent, Sent::Taken);
            let go_on = self.settle(entry.id, sent);
            if taken {
                told(Told::Taken(Box::new(entry)));
            }
            told(Told::All(self.entries.clone()));
            if !go_on {
                break;
            }
        }
    }
}

/// What the queue is asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// Queue a write (with its old place, to put back one thrown away).
    Add(Box<Entry>),
    /// Send what's queued.
    Flush,
    /// Queue a refused or unsure write again, and send.
    Retry(u64),
    Discard(u64),
}

/// Runs the queue: one task, so writes are sent one at a time, in order.
/// It sends what's queued from the start (what was left last time).
pub fn start(
    gh: GitHub,
    tx: mpsc::UnboundedSender<crate::state::Msg>,
) -> mpsc::UnboundedSender<Op> {
    let (ops, mut rx) = mpsc::unbounded_channel::<Op>();
    let store = gh.store().clone();
    tokio::spawn(async move {
        let tell = |told: Told| {
            let _ = tx.send(crate::state::Msg::Offline(crate::offline::Msg::Queue(told)));
        };
        let mut tell = tell;
        let mut queue = match tokio::task::spawn_blocking(move || Queue::load(store)).await {
            Ok(queue) => queue,
            Err(err) => {
                tracing::error!(%err, "couldn't read the queued writes");
                return;
            }
        };
        tell(Told::All(queue.entries().to_vec()));
        let mut flush = true;
        loop {
            if flush {
                queue.flush(|write| send(&gh, write), &mut tell).await;
            }
            let Some(op) = rx.recv().await else { return };
            flush = match op {
                Op::Add(entry) => {
                    if !queue.add(*entry) {
                        let text = "The write is queued, but couldn't be kept on disk: it's lost if ghtui stops";
                        let notice = crate::ui::bars::Notice::Error(text.into());
                        let _ = tx.send(crate::state::Msg::Notice(notice));
                    }
                    true
                }
                Op::Flush => true,
                Op::Retry(id) => {
                    queue.retry(id);
                    true
                }
                Op::Discard(id) => {
                    queue.discard(id);
                    false
                }
            };
            tell(Told::All(queue.entries().to_vec()));
        }
    });
    ops
}

/// Sends one queued write.
async fn send(gh: &GitHub, write: Write) -> Sent {
    match write {
        Write::Review(review) => crate::review::send_review(gh, review).await,
        Write::Change(change) => Sent::of(gh.change(&change).await),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::model::NodeId;
    use std::cell::RefCell;

    fn comment(body: &str) -> Entry {
        Entry {
            id: 0,
            at: 1,
            write: Write::Change(Change::Comment {
                subject: NodeId::new("I_1"),
                body: body.into(),
            }),
            page: "https://github.com/o/r/issues/1".into(),
            title: "o/r#1".into(),
            status: Status::Queued,
        }
    }

    fn label() -> Entry {
        let edit = Edit::Labels {
            subject: NodeId::new("I_1"),
            add: Vec::new(),
            remove: Vec::new(),
        };
        Entry {
            write: Write::Change(Change::Edit(edit)),
            ..comment("")
        }
    }

    fn store(dir: &std::path::Path) -> Store {
        Store::open(&dir.join("cache.redb")).for_account("github.com", "aaaa")
    }

    fn body(write: &Write) -> String {
        write.text().unwrap_or_default()
    }

    /// Sends with `answers` in turn, saying what was sent.
    async fn flush(queue: &mut Queue, answers: Vec<Sent>) -> Vec<String> {
        let sent = RefCell::new(Vec::new());
        let answers = RefCell::new(answers.into_iter());
        queue
            .flush(
                |write| {
                    sent.borrow_mut().push(body(&write));
                    let answer = answers.borrow_mut().next().unwrap_or(Sent::Taken);
                    async move { answer }
                },
                &mut |_| {},
            )
            .await;
        sent.into_inner()
    }

    #[tokio::test]
    async fn writes_go_in_order_once_and_stop_while_github_is_away() {
        let dir = tempfile::tempdir().unwrap();
        let mut queue = Queue::load(store(dir.path()));
        for text in ["one", "two", "three"] {
            assert!(queue.add(comment(text)));
        }
        // GitHub goes away at the second: it and the third wait.
        let sent = flush(&mut queue, vec![Sent::Taken, Sent::Unsent]).await;
        assert_eq!(sent, ["one", "two"]);
        let left: Vec<String> = queue.entries().iter().map(|e| body(&e.write)).collect();
        assert_eq!(left, ["two", "three"]);
        assert!(queue.entries().iter().all(|e| e.status == Status::Queued));
        // Back: each is sent once, in order; nothing's sent again.
        assert_eq!(flush(&mut queue, Vec::new()).await, ["two", "three"]);
        assert!(queue.entries().is_empty());
        assert!(flush(&mut queue, Vec::new()).await.is_empty());
    }

    #[tokio::test]
    async fn a_refused_write_keeps_its_text_and_the_rest_go_on() {
        let dir = tempfile::tempdir().unwrap();
        let mut queue = Queue::load(store(dir.path()));
        queue.add(comment("locked meanwhile"));
        queue.add(comment("elsewhere"));
        let refused = Sent::Refused("the conversation is locked".into());
        let sent = flush(&mut queue, vec![refused]).await;
        assert_eq!(sent, ["locked meanwhile", "elsewhere"]);
        let [entry]: [Entry; 1] = queue.entries().to_vec().try_into().unwrap();
        assert_eq!(
            entry.status,
            Status::Refused("the conversation is locked".into())
        );
        assert_eq!(body(&entry.write), "locked meanwhile");
        // Not sent again until asked; then it is.
        assert!(flush(&mut queue, Vec::new()).await.is_empty());
        queue.retry(entry.id);
        assert_eq!(flush(&mut queue, Vec::new()).await, ["locked meanwhile"]);
    }

    #[tokio::test]
    async fn a_comment_whose_fate_is_unknown_is_never_sent_again_on_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let mut queue = Queue::load(store(dir.path()));
        queue.add(comment("maybe posted"));
        queue.add(label());
        let dropped = || Sent::Unknown("the connection dropped".into());
        // The comment may be on GitHub: unsure. The labels, the same
        // either way: queued again, and the queue waits.
        let sent = flush(&mut queue, vec![dropped(), dropped()]).await;
        assert_eq!(sent.len(), 2);
        let statuses: Vec<&Status> = queue.entries().iter().map(|e| &e.status).collect();
        assert_eq!(
            statuses,
            [
                &Status::Unsure("the connection dropped".into()),
                &Status::Queued
            ]
        );
        assert_eq!(flush(&mut queue, Vec::new()).await, [""]);
    }

    #[tokio::test]
    async fn after_a_restart_the_queue_is_as_it_was_and_a_send_cut_short_is_unsure() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut queue = Queue::load(store(dir.path()));
            queue.add(comment("cut short"));
            queue.add(label());
            queue.add(comment("waiting"));
            // ghtui stops while the first is on its way.
            assert_eq!(
                queue.take_next().map(|e| body(&e.write)).unwrap(),
                "cut short"
            );
        }
        let mut queue = Queue::load(store(dir.path()));
        let statuses: Vec<(String, Status)> = queue
            .entries()
            .iter()
            .map(|e| (body(&e.write), e.status.clone()))
            .collect();
        assert_eq!(
            statuses,
            [
                (
                    "cut short".into(),
                    Status::Unsure("ghtui stopped while sending it".into())
                ),
                (String::new(), Status::Queued),
                ("waiting".into(), Status::Queued),
            ]
        );
        assert_eq!(flush(&mut queue, Vec::new()).await, ["", "waiting"]);
        // Another account's queue is its own.
        let other = queue.store.clone().for_account("github.com", "bbbb");
        assert!(other.is_enabled());
        assert!(Queue::load(other).entries().is_empty());
    }

    #[test]
    fn only_writes_that_can_wait_are_queued() {
        let merge = Change::Merge {
            pr: NodeId::new("PR_1"),
            method: crate::api::change::MergeMethod::Squash,
            head: "abc".into(),
        };
        assert_eq!(Write::of(&merge), None);
        let comment = Change::Comment {
            subject: NodeId::new("I_1"),
            body: "hi".into(),
        };
        let write = Write::of(&comment).unwrap();
        assert_eq!(write, Write::Change(comment));
        assert!(!write.idempotent());
    }
}
