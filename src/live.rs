//! Watching what's on screen. While a pull request's Files changed, its
//! Review tab or its conversation is on screen, ghtui asks GitHub now
//! and then whether anything changed there (review comments, a push,
//! check results); other pages have their own watchers ([`page`]): a
//! running job, the Actions tab, a run, an issue, a discussion. Every
//! watcher asks with the ETag of the answer before, so an unchanged
//! answer costs nothing, and keeps the same [`Pace`]: it asks less often
//! while nothing changes, and seldom while the terminal is out of focus.
//! What came in since you started looking is marked, and the status bar
//! says what it was. The first answer after opening a page is only "what
//! was there" when it shows nothing the page doesn't: one newer than what
//! the page loaded is a change.

use crate::route::{Route, pulls};
use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use crate::api::browse::PrActivity;
use crate::api::model::{NodeId, PrRef, ReviewThread};
use crate::api::{ApiError, Polled};
use crate::data::DataKey;
use crate::diff_screen::{DiffOf, LastReview};
use crate::keymap::Action;
use crate::state::{Api, Cmd, Cmds, Screen, State, Timer};
use crate::ui::pages::PrTab;
use crate::ui::text::{noun, short_sha};

/// The first wait between asks; it doubles while nothing changes, up to
/// the most.
pub const FIRST_MS: u64 = 10_000;
const MOST_MS: u64 = 160_000;
/// The wait while the terminal is out of focus.
const AWAY_MS: u64 = 300_000;

pub mod page;

/// When a watcher asks next, and what it asked last: shared by every
/// watcher.
#[derive(Debug)]
pub struct Pace<P> {
    /// Which watch a timer is for: a new one ignores an old one's.
    id: u64,
    /// The last answers' ETags.
    etags: HashMap<P, String>,
    /// How long until the next ask.
    wait: u64,
    /// An ask is scheduled, or on its way.
    busy: bool,
}

impl<P: Clone + Eq + Hash> Pace<P> {
    fn new() -> Self {
        Self {
            id: crate::fresh(),
            etags: HashMap::new(),
            wait: FIRST_MS,
            busy: false,
        }
    }

    /// Whether `id` is this watch's timer.
    fn is(&self, id: u64) -> bool {
        self.id == id
    }

    /// The timer for the next ask, unless one is scheduled or on its way.
    fn due(&mut self, focused: bool) -> Option<Cmd> {
        if self.busy {
            return None;
        }
        self.busy = true;
        let wait = if focused {
            self.wait
        } else {
            self.wait.max(AWAY_MS)
        };
        Some(Cmd::Timer(Timer::Watch(self.id), wait))
    }

    /// Back in focus: the next ask is soon. A timer already running is
    /// the old wait's, and ignored.
    fn refocus(&mut self) {
        (self.id, self.busy, self.wait) = (crate::fresh(), false, FIRST_MS);
    }

    /// `probe`, asked with the ETag of its last answer.
    fn ask(&self, probe: P) -> Ask<P> {
        Ask {
            etag: self.etags.get(&probe).cloned(),
            probe,
        }
    }

    /// Notes an answer's ETag; true if it's the first answer to `probe`.
    fn answered(&mut self, probe: &P, etag: String) -> bool {
        let first = !self.etags.contains_key(probe);
        if !etag.is_empty() {
            self.etags.insert(probe.clone(), etag);
        }
        first
    }

    /// The answers are in: soon again after a change, else later, up to
    /// `most`.
    fn settle(&mut self, changed: bool, most: u64) {
        self.busy = false;
        self.wait = if changed {
            FIRST_MS
        } else {
            self.wait.saturating_mul(2).min(most)
        };
    }

    /// Nothing was asked this time (what to ask isn't loaded yet).
    fn idle(&mut self) {
        self.busy = false;
    }
}

/// What's asked about a pull request: a REST answer whose ETag says
/// whether it changed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Probe {
    /// The pull request itself: its head (a push), and when it was last
    /// updated (a comment or review in its conversation).
    Pull,
    /// Its review comments, the last updated first.
    ReviewComments,
    /// The check runs on a head.
    CheckRuns(String),
    /// The commit statuses on a head.
    Statuses(String),
}

impl Probe {
    /// Where it's asked.
    pub fn path(&self, pr: &PrRef) -> String {
        let (repo, n) = (&pr.repo, pr.number);
        match self {
            Probe::Pull => format!("/repos/{repo}/pulls/{n}"),
            Probe::ReviewComments => {
                format!("/repos/{repo}/pulls/{n}/comments?sort=updated&direction=desc&per_page=1")
            }
            Probe::CheckRuns(head) => {
                format!("/repos/{repo}/commits/{head}/check-runs?per_page=100")
            }
            Probe::Statuses(head) => format!("/repos/{repo}/commits/{head}/status"),
        }
    }
}

/// A probe, asked with the ETag of its last answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask<P = Probe> {
    pub probe: P,
    pub etag: Option<String>,
}

/// The pull request watched, and what's been seen of it.
#[derive(Debug)]
pub struct Watch {
    pub pr: PrRef,
    /// When it asks next, and the last answers' ETags.
    pace: Pace<Probe>,
    /// The head GitHub last showed.
    head: Option<String>,
    /// Review threads and their comments, by id, there when you started
    /// looking or since; `None` until they're first in.
    seen_threads: Option<HashSet<NodeId>>,
    /// The conversation's comments, by GitHub's number, likewise.
    seen_comments: Option<HashSet<u64>>,
    /// Its reviews, by GitHub's number, likewise.
    seen_reviews: Option<HashSet<u64>>,
    /// Review threads and comments that came in since you started
    /// looking (not yours).
    pub new_threads: HashSet<NodeId>,
    /// The conversation's comments that did.
    pub new_comments: HashSet<u64>,
    /// The reviews that did.
    pub new_reviews: HashSet<u64>,
}

impl Watch {
    /// Watching `pr` from now: what's loaded of it is what was there.
    fn new(pr: PrRef, state: &State) -> Self {
        let head = state.pr(&pr);
        let threads = state.diffs.get(&DiffOf::Pr(pr.clone()));
        let threads = threads.and_then(|d| d.inputs().threads.data.as_deref());
        let activity = state.picked::<PrActivity>(&DataKey::PrActivity(pr.clone()));
        // The conversation's threads, when the diff's aren't here.
        let threads = threads.or(activity.map(|a| a.threads.as_slice()));
        Self {
            pace: Pace::new(),
            head: head.map(|d| d.head_oid.clone()),
            seen_threads: threads.map(thread_ids),
            seen_comments: activity.map(|a| comment_ids(a).map(|(id, _)| id).collect()),
            seen_reviews: activity.map(|a| review_ids(a).map(|(id, _)| id).collect()),
            new_threads: HashSet::new(),
            new_comments: HashSet::new(),
            new_reviews: HashSet::new(),
            pr,
        }
    }
}

/// The ids of review threads and their comments.
fn thread_ids(threads: &[ReviewThread]) -> HashSet<NodeId> {
    let comments = threads
        .iter()
        .flat_map(|t| t.comments.iter().map(|c| &c.id));
    threads
        .iter()
        .map(|t| &t.id)
        .chain(comments)
        .cloned()
        .collect()
}

/// The conversation's comments' numbers, with their authors.
fn comment_ids(activity: &PrActivity) -> impl Iterator<Item = (u64, &str)> {
    (activity.comments()).filter_map(|c| Some((c.id?, c.author.as_str())))
}

/// The conversation's submitted reviews' numbers, with their authors.
fn review_ids(activity: &PrActivity) -> impl Iterator<Item = (u64, &str)> {
    (activity.reviews())
        .filter(|r| r.state != crate::api::model::ReviewState::Pending)
        .filter_map(|r| Some((r.id?, r.author.as_str())))
}

/// The pull request whose review is on screen: its Files changed, its
/// Review tab or its conversation. Nothing else is watched.
pub fn watched(state: &State) -> Option<&PrRef> {
    match state.screen() {
        Screen::Diff(screen) => screen.of.pr(),
        Screen::Page(p) => match &p.route {
            Route::Pulls(pulls::Pr {
                pr,
                tab: PrTab::Conversation | PrTab::Review | PrTab::Reviewed(_),
            }) => Some(pr),
            _ => None,
        },
    }
}

/// Watches what's on screen, and schedules the next ask about it. Leaving
/// a pull request forgets what was new on it: coming back is looking anew.
pub fn schedule(state: &mut State) -> Cmds {
    let here = watched(state).cloned();
    if state.watch.as_ref().map(|w| &w.pr) != here.as_ref() {
        if let Some(old) = state.watch.take() {
            forget(state, &old.pr);
        }
        state.watch = here.map(|pr| Watch::new(pr, state));
    }
    let mut cmds = page::schedule(state);
    let focused = state.focused;
    if let Some(w) = state.watch.as_mut() {
        cmds.extend(w.pace.due(focused));
    }
    cmds
}

/// The terminal came into focus (`true`) or left it. Back in focus, the
/// next ask is soon.
pub fn on_focus(state: &mut State, focused: bool) {
    let was = std::mem::replace(&mut state.focused, focused);
    if focused && !was {
        if let Some(w) = &mut state.watch {
            w.pace.refocus();
        }
        page::refocus(state);
    }
}

/// Time to ask about the watched pull request (or page), if the timer is
/// still its.
pub fn on_timer(state: &mut State, id: u64) -> Cmds {
    let Some(w) = state.watch.as_mut().filter(|w| w.pace.is(id)) else {
        return page::on_timer(state, id);
    };
    let mut probes = vec![Probe::Pull, Probe::ReviewComments];
    if let Some(head) = &w.head {
        probes.extend([
            Probe::CheckRuns(head.clone()),
            Probe::Statuses(head.clone()),
        ]);
    }
    let asks = probes.into_iter().map(|probe| w.pace.ask(probe)).collect();
    Cmd::Api(Api::Poll {
        pr: w.pr.clone(),
        asks,
    })
    .into()
}

/// A pull request's REST answer, as much as is read of it: serde skips
/// the rest without building it.
#[derive(serde::Deserialize)]
struct PullHead {
    head: Option<Sha>,
    updated_at: Option<String>,
}

#[derive(serde::Deserialize)]
struct Sha {
    sha: String,
}

/// A review comment, as much as is read of it.
#[derive(serde::Deserialize)]
struct NodeIdOnly {
    node_id: String,
}

/// The newest review comment a review comments answer names.
fn newest_comment(body: &str) -> Option<NodeId> {
    let comments: Vec<NodeIdOnly> = serde_json::from_str(body).ok()?;
    Some(NodeId::new(&comments.first()?.node_id))
}

/// GitHub's answers to an ask: what changed is fetched again, and a push
/// is said, with the way to see what it changed since your review.
pub fn on_polled(
    state: &mut State,
    pr: &PrRef,
    answers: Vec<(Probe, Result<Polled, ApiError>)>,
) -> Cmds {
    // The conversation is fetched again where it's shown: on a page; and
    // every review, where they're listed.
    let lists_reviews = matches!(
        state.route(),
        Some(Route::Pulls(pulls::Pr {
            tab: PrTab::Review | PrTab::Reviewed(_),
            ..
        }))
    );
    let talk: Vec<DataKey> = (state.route().is_some())
        .then(|| DataKey::PrActivity(pr.clone()))
        .into_iter()
        .chain(lists_reviews.then(|| DataKey::Reviews(pr.clone())))
        .collect();
    // The review comments the diff shows, to tell whether the first
    // answer is newer than they are.
    let shown = (state.diffs.get(&DiffOf::Pr(pr.clone())))
        .and_then(|d| d.inputs().threads.data.as_deref().map(thread_ids));
    // When the pull request shown was last updated, likewise (a review).
    let updated = state.pr(pr).map(|d| d.updated_at.clone());
    let Some(w) = state.watch.as_mut().filter(|w| w.pr == *pr) else {
        return Cmds::new();
    };
    let (mut changed, mut threads, mut pushed) = (false, false, None);
    let mut needs: Vec<DataKey> = Vec::new();
    for (probe, answer) in answers {
        let (etag, body) = match answer {
            Ok(Polled::Changed { etag, body }) => (etag, body),
            Ok(Polled::Same) => continue,
            Err(err) => {
                tracing::warn!(%pr, ?probe, %err, "asking whether the pull request changed");
                continue;
            }
        };
        let pull = (probe == Probe::Pull)
            .then(|| serde_json::from_str::<PullHead>(&body).ok())
            .flatten();
        if let Some(head) = pull.as_ref().and_then(|p| p.head.as_ref()) {
            if w.head.as_ref().is_some_and(|h| *h != head.sha) {
                pushed = Some(head.sha.clone());
            }
            w.head = Some(head.sha.clone());
        }
        // The first answer is only what's there, unless a comment came in
        // after the diff's threads were fetched.
        let first = w.pace.answered(&probe, etag);
        let unseen = match probe {
            Probe::ReviewComments => (shown.as_ref())
                .is_some_and(|ids| newest_comment(&body).is_some_and(|id| !ids.contains(&id))),
            // Updated after what's shown was fetched: a review, a comment.
            Probe::Pull => (pull.as_ref().and_then(|p| p.updated_at.as_deref()))
                .zip(updated.as_deref())
                .is_some_and(|(now, shown)| now > shown),
            Probe::CheckRuns(_) | Probe::Statuses(_) => false,
        };
        if first && pushed.is_none() && !unseen {
            continue;
        }
        changed = true;
        let fetch = match probe {
            Probe::Pull => std::iter::once(DataKey::Pr(pr.clone()))
                .chain(talk.iter().cloned())
                .collect(),
            Probe::ReviewComments => {
                threads = true;
                talk.clone()
            }
            Probe::CheckRuns(_) | Probe::Statuses(_) => {
                vec![DataKey::Pr(pr.clone()), DataKey::PrChecks(pr.clone())]
            }
        };
        for need in fetch {
            if !needs.contains(&need) {
                needs.push(need);
            }
        }
    }
    w.pace.settle(changed, MOST_MS);
    let mut cmds: Cmds = needs
        .into_iter()
        .flat_map(|need| state.ensure(need, true))
        .collect();
    if threads && state.diffs.contains_key(&DiffOf::Pr(pr.clone())) {
        cmds.push(Cmd::Api(Api::FetchThreads(pr.clone())));
    }
    if let Some(head) = pushed {
        say_pushed(state, pr, &head);
    }
    cmds
}

/// Says a push came in, and how to see it: since your last review, if
/// you've reviewed.
fn say_pushed(state: &mut State, pr: &PrRef, head: &str) {
    let reviewed = state.diffs.get(&DiffOf::Pr(pr.clone())).is_some_and(|d| {
        let i = d.inputs();
        matches!(i.last_review, LastReview::At(_)) || i.review.last_reviewed_head.is_some()
    });
    let how = if reviewed {
        let since = state.first_key(Action::ToggleSinceReview);
        format!("{since} shows what changed since your review")
    } else {
        let refresh = state.first_key(Action::Refresh);
        format!("{refresh} shows it")
    };
    state.info(format!("New push to {pr}: {} · {how}", short_sha(head)));
}

/// Whether GitHub has shown a head for the watched pull request other
/// than the one its diff shows: a push since the diff was made.
pub fn pushed_past(state: &State, pr: &PrRef, shown: Option<&str>) -> bool {
    let head = fresh(state, pr).and_then(|w| w.head.as_deref());
    matches!((head, shown), (Some(head), Some(shown)) if head != shown)
}

/// Notes review threads that are in: the first time, they're what was
/// there; after, what wasn't (and isn't yours) is new. How many threads
/// and replies came.
fn note_threads(w: &mut Watch, threads: &[ReviewThread], viewer: Option<&str>) -> (u64, u64) {
    let mine = |author: &str| viewer == Some(author);
    let (mut new_threads, mut replies) = (0u64, 0u64);
    let ids = thread_ids(threads);
    let Some(seen) = &mut w.seen_threads else {
        w.seen_threads = Some(ids);
        return (0, 0);
    };
    for t in threads {
        let came: Vec<&NodeId> = (t.comments.iter())
            .filter(|c| !seen.contains(&c.id) && !mine(&c.author))
            .map(|c| &c.id)
            .collect();
        if came.is_empty() {
            continue;
        }
        w.new_threads.extend(came.iter().map(|id| (*id).clone()));
        if seen.contains(&t.id) {
            replies += came.len() as u64;
        } else {
            w.new_threads.insert(t.id.clone());
            new_threads += 1;
        }
    }
    seen.extend(ids);
    (new_threads, replies)
}

/// "2 threads, 1 reply", for the status bar.
fn threads_said(new_threads: u64, replies: u64) -> Vec<String> {
    let mut what = Vec::new();
    if new_threads > 0 {
        what.push(format!("{new_threads} {}", noun(new_threads, "thread")));
    }
    if replies > 0 {
        what.push(format!("{replies} {}", noun(replies, "reply")));
    }
    what
}

/// The pull request's review threads are in: the first time, they're
/// what was there; after, what wasn't is new, and said. The diff marks
/// them.
pub fn threads_loaded(state: &mut State, pr: &PrRef) {
    let State {
        watch,
        diffs,
        viewer,
        notices,
        ..
    } = state;
    let Some(w) = watch.as_mut().filter(|w| w.pr == *pr) else {
        return;
    };
    let Some(diff) = diffs.get_mut(&DiffOf::Pr(pr.clone())) else {
        return;
    };
    let Some(threads) = diff.inputs().threads.data.as_ref() else {
        return;
    };
    let (new_threads, replies) = note_threads(w, threads, viewer.as_deref());
    if diff.inputs().fresh != w.new_threads {
        let fresh = w.new_threads.clone();
        diff.edit(|i| i.fresh = fresh);
    }
    let what = threads_said(new_threads, replies);
    if !what.is_empty() {
        notices.info(format!("New on {pr}: {}", what.join(", ")));
    }
}

/// The conversation is in: as with threads, its comments, reviews and
/// threads that weren't there before are new, and said (a review by who
/// and what they decided).
pub fn activity_loaded(state: &mut State, pr: &PrRef) {
    let State {
        watch,
        data,
        viewer,
        notices,
        shown_gen,
        ..
    } = state;
    let activity = (data.get(&DataKey::PrActivity(pr.clone())))
        .and_then(|r| r.data.as_ref())
        .and_then(<PrActivity as crate::data::Picked>::pick);
    let Some(activity) = activity else {
        return;
    };
    let viewer = viewer.as_deref();
    let Some(w) = watch.as_mut().filter(|w| w.pr == *pr) else {
        return;
    };
    let mut what = Vec::new();
    let mine = |author: &&str| viewer == Some(*author);
    // Comments.
    let came = note(&mut w.seen_comments, comment_ids(activity), mine);
    w.new_comments.extend(came.iter().map(|(id, _)| *id));
    let new = came.len() as u64;
    if new > 0 {
        what.push(format!("{new} {}", noun(new, "comment")));
    }
    // Reviews, each named.
    for (id, author) in note(&mut w.seen_reviews, review_ids(activity), mine) {
        w.new_reviews.insert(id);
        let r = activity.reviews().find(|r| r.id == Some(id));
        let said = r.map_or("reviewed", |r| match r.state {
            crate::api::model::ReviewState::Approved => "approved",
            crate::api::model::ReviewState::ChangesRequested => "requested changes",
            _ => "reviewed",
        });
        what.push(format!("{author} {said}"));
    }
    // Threads, which the diff's own fetch may have noted already.
    let (new_threads, replies) = note_threads(w, &activity.threads, viewer);
    what.extend(threads_said(new_threads, replies));
    if !what.is_empty() {
        *shown_gen += 1;
        notices.info(format!("New on {pr}: {}", what.join(", ")));
    }
}

/// Notes what's in, by GitHub's number: the first time, it's what was
/// there; after, what wasn't is new. What came that isn't `mine`.
fn note<T>(
    seen: &mut Option<HashSet<u64>>,
    items: impl IntoIterator<Item = (u64, T)>,
    mine: impl Fn(&T) -> bool,
) -> Vec<(u64, T)> {
    let Some(known) = seen.as_mut() else {
        *seen = Some(items.into_iter().map(|(id, _)| id).collect());
        return Vec::new();
    };
    (items.into_iter())
        .filter(|(id, t)| known.insert(*id) && !mine(t))
        .collect()
}

/// Forgets what was new on `pr`: you've left it.
fn forget(state: &mut State, pr: &PrRef) {
    if let Some(diff) = state.diffs.get_mut(&DiffOf::Pr(pr.clone()))
        && !diff.inputs().fresh.is_empty()
    {
        diff.edit(|i| i.fresh.clear());
    }
}

/// The marks of what's new on the pull request on screen: review threads
/// and comments, and the conversation's comments.
pub fn fresh<'a>(state: &'a State, pr: &PrRef) -> Option<&'a Watch> {
    state.watch.as_ref().filter(|w| w.pr == *pr)
}
