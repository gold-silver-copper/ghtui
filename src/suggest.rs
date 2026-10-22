//! Committing someone's suggestion, as the website's "Commit suggestion"
//! and "Add suggestion to batch" do: with git, in the clone the diff is
//! in (yours or the cache's), the suggested lines go in place of the ones
//! they're on, in a commit on GitHub's tip of the head branch with
//! GitHub's message, which is pushed by its ID; then the threads are
//! resolved, and the diff follows GitHub until it shows the new head.

use std::collections::BTreeMap;

use crate::api::model::{NodeId, PrDetail, PrRef, RepoId, Side as ApiSide};
use crate::data::DataKey;
use crate::diff::anchor::{LinePos, Side};
use crate::diff_screen::{self, DiffOf, Pane};
use crate::forms::Answer;
use crate::git::branch::{self, refused};
use crate::git::{Oid, commit};
use crate::keymap::Action;
use crate::nav::{Doable, row};
use crate::route::Route;
use crate::state::{Cmds, State};
use crate::ui::annotations::AnnotationKey;
use crate::ui::annotations::ThreadRowKind;
use crate::ui::diff_doc::{Pos, Row};
use crate::ui::text::short_sha;
use crate::work::{Cx, Failure, Job};

/// What committing suggestions adds: on a pull request's diff, the
/// suggestion under the cursor committed, or added to a batch.
pub const FEATURE: crate::features::Feature = crate::features::Feature {
    owns: &[Action::CommitSuggestion, Action::BatchSuggestion],
    act,
    offers,
    follow,
    ..crate::features::Feature::NONE
};

/// A suggestion to commit: the thread it's in, whose it is, the lines of
/// the head's file it's on, and what it puts there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub thread: NodeId,
    /// You may resolve its thread once it's in.
    pub can_resolve: bool,
    pub author: String,
    pub path: String,
    /// The first and last lines it replaces.
    pub start: u32,
    pub end: u32,
    /// Whole lines, each with its newline; empty to delete them.
    pub lines: String,
}

impl Suggestion {
    fn overlaps(&self, other: &Suggestion) -> bool {
        self.path == other.path && self.start <= other.end && other.start <= self.end
    }

    fn lines_label(&self) -> String {
        if self.start == self.end {
            format!("{}:{}", self.path, self.start)
        } else {
            format!("{}:{}–{}", self.path, self.start, self.end)
        }
    }
}

/// What committing suggestions keeps: those waiting in a batch, on one
/// pull request's head, and the commit the diff waits for GitHub to show.
#[derive(Debug, Default)]
pub struct Suggestions {
    batch: Option<Batch>,
    /// The pull request, and the head the diff is to load once GitHub
    /// shows it.
    reload: Option<(PrRef, String)>,
}

#[derive(Debug)]
struct Batch {
    pr: PrRef,
    /// The head their lines are on.
    head: String,
    items: Vec<Suggestion>,
}

impl Suggestions {
    /// The batch on `pr`'s `head`, if it has anything in it.
    fn batch(&self, pr: &PrRef, head: &str) -> Option<&[Suggestion]> {
        (self.batch.as_ref())
            .filter(|b| b.pr == *pr && b.head == head && !b.items.is_empty())
            .map(|b| b.items.as_slice())
    }
}

/// The text of the one ```` ```suggestion ```` block in a comment: the
/// lines it suggests.
pub fn suggested(body: &str) -> Result<String, &'static str> {
    use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};
    let body = body.replace("\r\n", "\n");
    let (mut blocks, mut inside) = (Vec::new(), false);
    for event in Parser::new(&body) {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info)))
                if info.split([',', ' ']).next() == Some("suggestion") =>
            {
                inside = true;
                blocks.push(String::new());
            }
            Event::Text(text) if inside => {
                if let Some(block) = blocks.last_mut() {
                    block.push_str(&text);
                }
            }
            Event::End(TagEnd::CodeBlock) => inside = false,
            _ => {}
        }
    }
    match &blocks[..] {
        [] => Err("It suggests no change"),
        [one] => Ok(one.clone()),
        _ => Err("It has more than one suggestion: GitHub commits only a comment's only one"),
    }
}

/// `text` with each suggestion's lines in place of the ones it's on.
pub fn apply(text: &str, suggestions: &[&Suggestion]) -> Result<String, String> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut sorted = suggestions.to_vec();
    sorted.sort_by_key(|s| std::cmp::Reverse(s.start));
    let mut out: Vec<String> = lines.iter().map(|l| (*l).to_owned()).collect();
    for s in sorted {
        let (first, last) = (s.start as usize, s.end as usize);
        if first == 0 || last < first || last > out.len() {
            return Err(format!("{} has no lines {first}–{last}", s.path));
        }
        let mut new = s.lines.clone();
        // The file's last line keeps having no newline.
        let ends_bare = last == out.len() && out.last().is_some_and(|l| !l.ends_with('\n'));
        if ends_bare && new.ends_with('\n') {
            new.pop();
        }
        let tail = out.split_off(last);
        out.truncate(first - 1);
        out.push(new);
        out.extend(tail);
    }
    Ok(out.concat())
}

/// Why there's no suggestion to commit under the cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Lack {
    /// None is there.
    None(String),
    /// One is, but it can't be committed.
    Cant(String),
}

impl Lack {
    fn why(self) -> String {
        match self {
            Lack::None(why) | Lack::Cant(why) => why,
        }
    }
}

/// The suggestion under the diff's cursor, with its pull request and
/// head, or why there's none to commit.
fn here(state: &State) -> Result<(PrRef, String, Suggestion), Lack> {
    let none = |why: &str| Lack::None(why.to_owned());
    let cant = |why: &str| Lack::Cant(why.to_owned());
    let (screen, diff) = (state.screen().diff(), state.diff());
    let (Some(screen), Some(diff)) = (screen, diff) else {
        return Err(none("Suggestions are committed from Files changed"));
    };
    let Some(pr) = screen.of.pr().cloned() else {
        return Err(none("Only a pull request's suggestions are committed"));
    };
    let doc = &diff.doc;
    let under = (screen.focus == Pane::Diff)
        .then(|| doc.annotation_under(screen.cursor, screen.half))
        .flatten();
    let Some((at, ann)) = under.and_then(|i| Some((i, doc.annotations().get(i as usize)?))) else {
        return Err(none("Move to a suggestion"));
    };
    // The comment under the cursor, else the first that suggests.
    let comment = comment_under(state, at).and_then(|i| ann.comments.get(i));
    let comment = comment
        .filter(|c| suggested(&c.body).is_ok())
        .or_else(|| ann.comments.iter().find(|c| suggested(&c.body).is_ok()));
    let Some(comment) = comment else {
        let first = ann.comments.first().map(|c| suggested(&c.body));
        return Err(match first {
            Some(Err(why)) if why.contains("more than one") => cant(why),
            _ => none("Move to a suggestion"),
        });
    };
    let AnnotationKey::Thread(id) = &ann.key else {
        return Err(cant(
            "A draft's suggestion is yours: submit your review first",
        ));
    };
    let Some(thread) = diff.inputs().thread_list().iter().find(|t| t.id == *id) else {
        return Err(none("Move to a suggestion"));
    };
    let lines = suggested(&comment.body).map_err(cant)?;
    if comment.pending {
        return Err(cant(
            "It's in a pending review: it can be committed once that's submitted",
        ));
    }
    if thread.file_level {
        return Err(cant("It's on the whole file, not on lines"));
    }
    let (Some(end), false) = (thread.line, thread.outdated) else {
        return Err(cant("It's outdated: the lines it's on changed since"));
    };
    if thread.side == ApiSide::Left || thread.start_side == Some(ApiSide::Left) {
        return Err(cant("It's on deleted lines"));
    }
    if diff.range().is_some() {
        let all = crate::review::all_changes(state);
        return Err(Lack::Cant(format!(
            "It's committed on the whole pull request: {all}"
        )));
    }
    let Some(d) = state.pr(&pr) else {
        return Err(Lack::Cant(format!("{pr} hasn't loaded yet")));
    };
    let head = diff.head().map(|h| h.to_string());
    if head.as_deref() != Some(d.head_oid.as_str()) {
        let refresh = state.first_key(Action::Refresh);
        return Err(Lack::Cant(format!(
            "A push came in since the diff loaded: {refresh} loads it"
        )));
    }
    may_push(&pr, d).map_err(Lack::Cant)?;
    let start = thread.start_line.unwrap_or(end).min(end);
    // Nothing to commit if the lines already say it.
    let path = thread.path.clone();
    let file = doc.files().iter().find(|f| f.meta.path() == path);
    if let Some(text) = file.and_then(|f| f.text()) {
        let now: String = (start..=end)
            .map(|line| {
                let line = LinePos {
                    side: Side::Right,
                    line,
                };
                format!("{}\n", text.line(line))
            })
            .collect();
        if now == lines {
            return Err(cant("It's in already: the lines say what it suggests"));
        }
    }
    let suggestion = Suggestion {
        thread: thread.id.clone(),
        can_resolve: thread.can_resolve,
        author: comment.author.clone(),
        path,
        start,
        end,
        lines,
    };
    Ok((pr, d.head_oid.clone(), suggestion))
}

/// Whether you may push to `pr`'s branch, and why not.
pub(crate) fn may_push(pr: &PrRef, d: &PrDetail) -> Result<(), String> {
    if !d.state.is_open() {
        return Err(format!("{pr} isn't open"));
    }
    if d.head_gone {
        return Err(format!("Its branch, {}, is gone", d.head_ref));
    }
    let fork = d.head_repo.as_deref().filter(|r| *r != pr.repo.to_string());
    match fork {
        _ if d.may.push_head => Ok(()),
        Some(fork) if d.may.merge => Err(format!(
            "Its branch is {fork}'s, whose author doesn't let maintainers push to it"
        )),
        _ => Err(format!(
            "Committing pushes to its branch, {}, which you can't push to",
            d.head_ref
        )),
    }
}

/// Which of the thread's comments the cursor is on, by its place.
fn comment_under(state: &State, ann: u32) -> Option<usize> {
    let screen = state.screen().diff()?;
    let doc = &state.diff()?.doc;
    let file = doc.files().get(screen.cursor.file)?;
    for row in (0..=screen.cursor.row).rev() {
        let pos = Pos {
            row,
            ..screen.cursor
        };
        let Some(Row::Thread(t)) = doc.row(pos) else { return None };
        let r = file.thread_row(t).filter(|r| r.ann == ann)?;
        match r.kind {
            ThreadRowKind::Head { comment, .. } | ThreadRowKind::Reactions { comment } => {
                return Some(comment);
            }
            _ => {}
        }
    }
    None
}

/// The batch's suggestions committed, else the one under the cursor; a
/// suggestion added to the batch, or taken out.
fn act(state: &mut State, action: Action) -> Cmds {
    let here = here(state);
    let batch = state
        .screen()
        .diff()
        .and_then(|s| s.of.pr())
        .and_then(|pr| {
            let d = state.pr(pr)?;
            let items = state.suggestions.batch(pr, &d.head_oid)?;
            Some((pr.clone(), d.head_oid.clone(), items.to_vec()))
        });
    match action {
        Action::CommitSuggestion => {
            let (pr, head, items) = match (batch, here) {
                (Some(batch), _) => batch,
                (None, Ok((pr, head, s))) => (pr, head, vec![s]),
                (None, Err(lack)) => return state.just_say(lack.why()),
            };
            ask(state, pr, head, items)
        }
        Action::BatchSuggestion => {
            let (pr, head, s) = or_return!(here, |lack| state.just_say(lack.why()));
            let batch = &mut state.suggestions.batch;
            let fresh = batch.as_ref().is_none_or(|b| b.pr != pr || b.head != head);
            if fresh {
                *batch = Some(Batch {
                    pr,
                    head,
                    items: Vec::new(),
                });
            }
            let Some(b) = batch.as_mut() else { return Cmds::new() };
            if let Some(at) = b.items.iter().position(|i| i.thread == s.thread) {
                b.items.remove(at);
                let n = b.items.len();
                return state.just_say(format!("Taken out of the batch ({n} in it)"));
            }
            if let Some(other) = b.items.iter().find(|i| i.overlaps(&s)) {
                let lines = other.lines_label();
                return state.just_say(format!(
                    "It's on lines a suggestion in the batch changes ({lines}): commit them apart"
                ));
            }
            b.items.push(s);
            let n = b.items.len();
            let key = state.first_key(Action::CommitSuggestion);
            state.just_say(format!(
                "Added to the batch ({n} in it): {key} commits {}",
                if n == 1 { "it" } else { "them" }
            ))
        }
        _ => Cmds::new(),
    }
}

/// The commit's message, as GitHub writes one: its first line, and each
/// suggestion's author but you as a co-author (their trailers come once
/// GitHub names them).
fn headline(items: &[Suggestion]) -> String {
    match items {
        [one] => format!("Update {}", one.path),
        _ => "Apply suggestions from code review".to_owned(),
    }
}

/// Asks before committing `items` to `pr`'s branch.
fn ask(state: &mut State, pr: PrRef, head: String, items: Vec<Suggestion>) -> Cmds {
    let Some(d) = state.pr(&pr) else {
        return state.just_say(format!("{pr} hasn't loaded yet"));
    };
    if let Err(why) = may_push(&pr, d) {
        return state.just_say(why);
    }
    let head_repo = (d.head_repo.as_deref())
        .and_then(RepoId::parse)
        .unwrap_or_else(|| pr.repo.clone());
    let branch = d.head_ref.clone();
    let n = items.len();
    let title = match n {
        1 => format!("Commit the suggestion to {branch}?"),
        n => format!("Commit {n} suggestions to {branch}?"),
    };
    let mut facts = vec![
        (format!("“{}”", headline(&items)), false),
        (
            format!("On GitHub's latest {head_repo}:{branch}, pushed there"),
            false,
        ),
        (
            (items.iter().map(Suggestion::lines_label))
                .collect::<Vec<_>>()
                .join(", "),
            false,
        ),
    ];
    let unresolvable = items.iter().filter(|s| !s.can_resolve).count();
    facts.push(match unresolvable {
        0 => (format!("Then {} resolved", noun_the(n)), false),
        _ if n == 1 => (
            "You can't resolve its thread: it stays open".to_owned(),
            false,
        ),
        u => (
            format!(
                "You can't resolve {u} of their {n} threads: {}",
                if u == 1 {
                    "it stays open"
                } else {
                    "they stay open"
                }
            ),
            false,
        ),
    });
    let label = match n {
        1 => "Commit suggestion".to_owned(),
        n => format!("Commit {n} suggestions"),
    };
    let job = Commit {
        pr: pr.clone(),
        head_repo,
        branch,
        seen: head,
        suggestions: items,
        viewer: state.viewer.clone(),
    };
    let choices = vec![(label, Answer::CommitSuggestions(Box::new(job)))];
    crate::forms::ask(state, Route::pr(pr), title, facts, choices)
}

/// "its thread is" or "their threads are".
fn noun_the(n: usize) -> &'static str {
    if n == 1 {
        "its thread is"
    } else {
        "their threads are"
    }
}

/// The question was answered: the suggestions are committed.
pub(crate) fn send(state: &mut State, job: Commit) -> Cmds {
    crate::forms::sending(state);
    crate::work::start(job)
}

/// Commits suggestions on GitHub's tip of a pull request's branch and
/// pushes it, then resolves their threads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub pr: PrRef,
    /// Where its head branch is, and the branch.
    pub head_repo: RepoId,
    pub branch: String,
    /// The head the suggestions' lines were read on: a file changed on
    /// GitHub since isn't committed to.
    pub seen: String,
    pub suggestions: Vec<Suggestion>,
    /// Your login: the commit's author when git has no name for you, and
    /// no co-author.
    pub viewer: Option<String>,
}

/// What committing did: the commit, and the threads GitHub didn't
/// resolve, with why.
#[derive(Debug)]
pub struct Committed {
    pub oid: String,
    pub unresolved: Vec<(NodeId, String)>,
}

/// Where GitHub's tip of the branch is fetched to, to commit on.
const TIP: &str = "refs/ghtui/suggest/tip";

impl Commit {
    /// The commit's whole message: GitHub's first line, and a co-author
    /// trailer for each author but you.
    async fn message(&self, cx: &Cx) -> String {
        let mut message = headline(&self.suggestions);
        let mut authors: Vec<&str> = Vec::new();
        for s in &self.suggestions {
            let theirs = Some(s.author.as_str()) != self.viewer.as_deref();
            if theirs && !authors.contains(&s.author.as_str()) {
                authors.push(&s.author);
            }
        }
        if !authors.is_empty() {
            message.push('\n');
        }
        for login in authors {
            let author = cx.gh.co_author(login).await;
            message.push_str(&format!("\nCo-authored-by: {author}"));
        }
        message
    }
}

impl Job for Commit {
    type Out = Committed;

    async fn run(&self, cx: &Cx) -> Result<Committed, Failure> {
        let base = (&*self.pr.repo.owner, &*self.pr.repo.name);
        let repo = cx.git.repo(base.0, base.1, &|_| {}).await?;
        let head = (&*self.head_repo.owner, &*self.head_repo.name);
        let remote = branch::remote_for(&repo, base, head).await;
        repo.ensure(&Oid::parse(&self.seen)?, &|_| {}).await?;
        let message = self.message(cx).await;
        let mut by_path: BTreeMap<&str, Vec<&Suggestion>> = BTreeMap::new();
        for s in &self.suggestions {
            by_path.entry(&s.path).or_default().push(s);
        }
        let mut made = None;
        // Once more if the branch moves on between fetching and pushing.
        for attempt in 0..2 {
            let tip = branch::tip(&repo, &remote, &self.branch, TIP).await;
            let tip = tip.map_err(|e| {
                Failure::msg(format!("Couldn't fetch {}: {}", self.branch, refused(&e)))
            })?;
            let mut additions = Vec::new();
            for (path, items) in &by_path {
                let now = commit::blob(&repo, &tip, path).await;
                if now != commit::blob(&repo, &self.seen, path).await {
                    return Err(Failure::msg(format!(
                        "{path} changed on {} since the suggestion was made: it isn't committed",
                        self.branch
                    )));
                }
                let text = repo.file_text(&self.seen, path).await?;
                let text = apply(&text, items).map_err(Failure::msg)?;
                additions.push(((*path).to_owned(), text));
            }
            let change = commit::Change {
                additions: &additions,
                deletions: &[],
                message: &message,
                login: self.viewer.as_deref(),
            };
            let oid = commit::make(&repo, &tip, &change).await?;
            match branch::push(&repo, &remote, &oid, &self.branch).await {
                Ok(()) => {
                    made = Some(oid);
                    break;
                }
                Err(err) if attempt == 0 && commit::moved_on(&err) => {}
                Err(err) => {
                    return Err(Failure::msg(format!(
                        "GitHub refused the push: {}",
                        refused(&err)
                    )));
                }
            }
        }
        let Some(oid) = made else {
            return Err(Failure::msg(format!(
                "{} kept changing while committing: try again",
                self.branch
            )));
        };
        let mut unresolved = Vec::new();
        let mut threads: Vec<&NodeId> = Vec::new();
        for s in self.suggestions.iter().filter(|s| s.can_resolve) {
            if threads.contains(&&s.thread) {
                continue;
            }
            threads.push(&s.thread);
            if let Err(err) = cx.gh.set_resolved(&s.thread, true).await {
                unresolved.push((s.thread.clone(), Failure::from(err).to_string()));
            }
        }
        Ok(Committed { oid, unresolved })
    }

    /// Committed: out of the batch, the threads resolved, and the diff
    /// loads the new head once GitHub shows it.
    fn done(self, state: &mut State, result: Result<Committed, Failure>) -> Cmds {
        let route = Route::pr(self.pr.clone());
        let done = or_return!(result, |why| {
            crate::forms::failed(state, &route, format!("Not committed: {why}"))
        });
        crate::forms::close_sender(state, &route);
        if let Some(b) = &mut state.suggestions.batch {
            b.items
                .retain(|i| !self.suggestions.iter().any(|s| s.thread == i.thread));
        }
        let of = DiffOf::Pr(self.pr.clone());
        for s in self.suggestions.iter().filter(|s| s.can_resolve) {
            if !done.unresolved.iter().any(|(id, _)| *id == s.thread) {
                diff_screen::edit(state, &of, |i| i.set_resolved(&s.thread, true));
            }
        }
        let n = self.suggestions.len();
        let what = if n == 1 {
            "the suggestion".to_owned()
        } else {
            format!("{n} suggestions")
        };
        let mut text = format!(
            "Committed {what} to {} as {}",
            self.branch,
            short_sha(&done.oid)
        );
        if let Some((_, why)) = done.unresolved.first() {
            text.push_str(&format!("; GitHub didn't resolve its thread: {why}"));
        }
        state.info(text);
        state.suggestions.reload = Some((self.pr.clone(), done.oid));
        crate::live::soon(state);
        state.ensure(DataKey::Pr(self.pr), true)
    }
}

/// Once GitHub shows the commit as the pull request's head, its diff
/// loads it.
fn follow(state: &mut State) -> Cmds {
    let Some((pr, oid)) = &state.suggestions.reload else { return Cmds::new() };
    if state.pr(pr).is_none_or(|d| d.head_oid != *oid) {
        return Cmds::new();
    }
    let pr = pr.clone();
    state.suggestions.reload = None;
    if state.diffs.remove(&DiffOf::Pr(pr.clone())).is_some() {
        state.info(format!("Files changed shows {pr} at its new head"));
    }
    Cmds::new()
}

/// The menu's rows, where the cursor is on a suggestion or a batch waits.
fn offers(state: &State, out: &mut Vec<Doable>) {
    let Some(pr) = state.screen().diff().and_then(|s| s.of.pr()) else { return };
    let batch = state
        .pr(pr)
        .and_then(|d| state.suggestions.batch(pr, &d.head_oid));
    let here = here(state);
    let suggests = !matches!(here, Err(Lack::None(_)));
    if !suggests && batch.is_none() {
        return;
    }
    let commit = match batch {
        Some([_]) => "Commit the suggestion in the batch".to_owned(),
        Some(items) => format!("Commit the {} suggestions in the batch", items.len()),
        None => "Commit the suggestion".to_owned(),
    };
    let why = here.as_ref().err().map(|lack| lack.clone().why());
    let commit_why = batch.is_none().then(|| why.clone()).flatten();
    out.push(row(
        "Review",
        Action::CommitSuggestion,
        commit,
        "",
        commit_why,
    ));
    if suggests {
        let thread = here.as_ref().ok().map(|(_, _, s)| &s.thread);
        let batched = batch.is_some_and(|b| b.iter().any(|s| Some(&s.thread) == thread));
        let label = if batched {
            "Take the suggestion out of the batch"
        } else {
            "Add the suggestion to the batch"
        };
        out.push(row("Review", Action::BatchSuggestion, label, "", why));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(start: u32, end: u32, lines: &str) -> Suggestion {
        Suggestion {
            thread: NodeId::new(format!("T{start}")),
            can_resolve: true,
            author: "alice".into(),
            path: "a.rs".into(),
            start,
            end,
            lines: lines.into(),
        }
    }

    #[test]
    fn a_comments_suggestion_is_its_one_block() {
        let body = "Better:\r\n```suggestion\r\nlet x = 2;\r\nlet y = 3;\r\n```\r\nThanks";
        assert_eq!(suggested(body).unwrap(), "let x = 2;\nlet y = 3;\n");
        assert_eq!(suggested("```suggestion\n```").unwrap(), "");
        suggested("```rust\nx\n```").unwrap_err();
        let two = "```suggestion\na\n```\n\n```suggestion\nb\n```";
        assert!(suggested(two).unwrap_err().contains("more than one"));
    }

    #[test]
    fn suggestions_replace_their_lines() {
        let text = "1\n2\n3\n4";
        let (two, last, gone) = (at(2, 2, "two\nmore\n"), at(4, 4, "four\n"), at(3, 3, ""));
        assert_eq!(
            apply(text, &[&two, &last, &gone]).unwrap(),
            "1\ntwo\nmore\nfour"
        );
        apply(text, &[&at(4, 5, "x\n")]).unwrap_err();
        assert!(at(2, 3, "").overlaps(&at(3, 4, "")));
        assert!(!at(2, 3, "").overlaps(&at(4, 4, "")));
    }

    /// A suggestion committed on GitHub's tip of the branch, which moved
    /// on meanwhile, with GitHub's message; the clone's branch, index and
    /// working tree stay as they were.
    #[tokio::test]
    async fn a_suggestion_is_committed_on_the_tip_and_pushed() {
        use crate::git::fixture::{cache_clone, commit as commit_files, origin};
        use crate::git::set_up as git;
        let github = tempfile::tempdir().unwrap();
        let dir = github.path();
        origin(dir).await;
        commit_files(dir, &[("a.rs", "let x = 1;\n")], "First").await;
        git(dir, &["switch", "-q", "-c", "topic"]).await;
        let seen = commit_files(dir, &[("a.rs", "let x = 1;\nlet y = 1;\n")], "Y").await;
        commit_files(dir, &[("b.rs", "b\n")], "Moved on").await;
        git(dir, &["switch", "-q", "main"]).await;
        let (_cache, repo) = cache_clone(dir).await;
        let tip = branch::tip(&repo, "origin", "topic", TIP).await.unwrap();
        repo.ensure(&Oid::parse(&seen).unwrap(), &|_| {})
            .await
            .unwrap();
        let text = repo.file_text(&seen, "a.rs").await.unwrap();
        let s = at(2, 2, "let y = 2;\n");
        let new = apply(&text, &[&s]).unwrap();
        let additions = [("a.rs".to_owned(), new)];
        let message = format!(
            "{}\n\nCo-authored-by: alice <1+alice@users.noreply.github.com>",
            headline(&[s])
        );
        let change = commit::Change {
            additions: &additions,
            deletions: &[],
            message: &message,
            login: Some("me"),
        };
        let oid = commit::make(&repo, &tip, &change).await.unwrap();
        branch::push(&repo, "origin", &oid, "topic").await.unwrap();
        assert_eq!(
            git(dir, &["show", "topic:a.rs"]).await,
            "let x = 1;\nlet y = 2;\n"
        );
        assert_eq!(git(dir, &["show", "topic:b.rs"]).await, "b\n");
        let said = git(dir, &["log", "-1", "--format=%B", "topic"]).await;
        assert!(
            said.starts_with("Update a.rs\n\nCo-authored-by: alice"),
            "{said}"
        );
        // Pushed past: a commit on the old tip is refused, not forced.
        let stale = commit::make(&repo, &seen, &change).await.unwrap();
        let refused = branch::push(&repo, "origin", &stale, "topic")
            .await
            .unwrap_err();
        assert!(commit::moved_on(&refused), "{refused}");
    }
}
