//! Watching a pull request while you review it. While its Files changed,
//! its Review tab or its conversation is on screen, ghtui asks GitHub now
//! and then whether anything changed there (review comments, a push,
//! check results), with the ETag of the answer before, so an unchanged
//! answer costs nothing. It asks less often while nothing changes, and
//! seldom while the terminal is out of focus. What came in since you
//! started looking is marked, and the status bar says what it was.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::api::browse::PrActivity;
use crate::api::model::{NodeId, PrRef, ReviewThread};
use crate::api::{ApiError, Polled};
use crate::browse::{DataKey, Need};
use crate::diff_screen::{DiffOf, LastReview};
use crate::keymap::Action;
use crate::route::Route;
use crate::state::{Api, Cmd, Screen, State, Timer};
use crate::ui::pages::PrTab;
use crate::ui::text::{noun, short_sha};

/// The first wait between asks; it doubles while nothing changes, up to
/// the most.
pub const FIRST_MS: u64 = 10_000;
const MOST_MS: u64 = 160_000;
/// The wait while the terminal is out of focus.
const AWAY_MS: u64 = 300_000;

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
        let repo = format!("/repos/{}/{}", pr.repo.owner, pr.repo.name);
        let n = pr.number;
        match self {
            Probe::Pull => format!("{repo}/pulls/{n}"),
            Probe::ReviewComments => {
                format!("{repo}/pulls/{n}/comments?sort=updated&direction=desc&per_page=1")
            }
            Probe::CheckRuns(head) => format!("{repo}/commits/{head}/check-runs?per_page=100"),
            Probe::Statuses(head) => format!("{repo}/commits/{head}/status"),
        }
    }
}

/// A probe, asked with the ETag of its last answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    pub probe: Probe,
    pub etag: Option<String>,
}

/// The pull request watched, and what's been seen of it.
#[derive(Debug)]
pub struct Watch {
    pub pr: PrRef,
    /// Which watch a timer is for: a new one ignores an old one's.
    id: u64,
    /// The last answers' ETags.
    etags: HashMap<Probe, String>,
    /// How long until the next ask.
    wait: u64,
    /// An ask is scheduled, or on its way.
    busy: bool,
    /// The head GitHub last showed.
    head: Option<String>,
    /// Review threads and their comments, by id, there when you started
    /// looking or since; `None` until they're first in.
    seen_threads: Option<HashSet<NodeId>>,
    /// The conversation's comments, by GitHub's number, likewise.
    seen_comments: Option<HashSet<u64>>,
    /// Review threads and comments that came in since you started
    /// looking (not yours).
    pub new_threads: HashSet<NodeId>,
    /// The conversation's comments that did.
    pub new_comments: HashSet<u64>,
}

/// An id no watch has had.
fn next_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl Watch {
    /// Watching `pr` from now: what's loaded of it is what was there.
    fn new(pr: PrRef, state: &State) -> Self {
        let head = state.prs.get(&pr).and_then(|r| r.data.as_ref());
        let threads = state.diffs.get(&DiffOf::Pr(pr.clone()));
        let threads = threads.and_then(|d| d.inputs().threads.data.as_deref());
        let activity = state.picked::<PrActivity>(&DataKey::PrActivity(pr.clone()));
        Self {
            id: next_id(),
            etags: HashMap::new(),
            wait: FIRST_MS,
            busy: false,
            head: head.map(|d| d.head_oid.clone()),
            seen_threads: threads.map(thread_ids),
            seen_comments: activity.map(|a| comment_ids(a).map(|(id, _)| id).collect()),
            new_threads: HashSet::new(),
            new_comments: HashSet::new(),
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
    (activity.comments.iter()).filter_map(|c| Some((c.id?, c.author.as_str())))
}

/// The pull request whose review is on screen: its Files changed, its
/// Review tab or its conversation. Nothing else is watched.
pub fn watched(state: &State) -> Option<&PrRef> {
    match state.screen() {
        Screen::Diff(screen) => screen.of.pr(),
        Screen::Page(p) => match &p.route {
            Route::Pr {
                pr,
                tab: PrTab::Conversation | PrTab::Review,
            } => Some(pr),
            _ => None,
        },
    }
}

/// Watches what's on screen, and schedules the next ask about it. Leaving
/// a pull request forgets what was new on it: coming back is looking anew.
#[must_use]
pub fn schedule(state: &mut State) -> Vec<Cmd> {
    let here = watched(state).cloned();
    if state.watch.as_ref().map(|w| &w.pr) != here.as_ref() {
        if let Some(old) = state.watch.take() {
            forget(state, &old.pr);
        }
        state.watch = here.map(|pr| Watch::new(pr, state));
    }
    let focused = state.focused;
    let Some(w) = state.watch.as_mut().filter(|w| !w.busy) else {
        return Vec::new();
    };
    w.busy = true;
    let wait = if focused { w.wait } else { w.wait.max(AWAY_MS) };
    vec![Cmd::Timer(Timer::Watch(w.id), wait)]
}

/// The terminal came into focus (`true`) or left it. Back in focus, the
/// next ask is soon.
pub fn on_focus(state: &mut State, focused: bool) {
    let was = std::mem::replace(&mut state.focused, focused);
    if focused
        && !was
        && let Some(w) = &mut state.watch
    {
        // A timer already running is the old wait's: it's ignored.
        (w.id, w.busy, w.wait) = (next_id(), false, FIRST_MS);
    }
}

/// Time to ask about the watched pull request, if the timer is still its.
#[must_use]
pub fn on_timer(state: &mut State, id: u64) -> Vec<Cmd> {
    let Some(w) = state.watch.as_mut().filter(|w| w.id == id) else {
        return Vec::new();
    };
    let mut probes = vec![Probe::Pull, Probe::ReviewComments];
    if let Some(head) = &w.head {
        probes.extend([
            Probe::CheckRuns(head.clone()),
            Probe::Statuses(head.clone()),
        ]);
    }
    let asks = probes
        .into_iter()
        .map(|probe| Ask {
            etag: w.etags.get(&probe).cloned(),
            probe,
        })
        .collect();
    vec![Cmd::Api(Api::Poll {
        pr: w.pr.clone(),
        asks,
    })]
}

/// The head a pull request's REST answer names.
fn head_of(body: &str) -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    Some(json.get("head")?.get("sha")?.as_str()?.to_owned())
}

/// The newest review comment a review comments answer names.
fn newest_comment(body: &str) -> Option<NodeId> {
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    Some(NodeId::new(json.get(0)?.get("node_id")?.as_str()?))
}

/// GitHub's answers to an ask: what changed is fetched again, and a push
/// is said, with the way to see what it changed since your review.
#[must_use]
pub fn on_polled(
    state: &mut State,
    pr: &PrRef,
    answers: Vec<(Probe, Result<Polled, ApiError>)>,
) -> Vec<Cmd> {
    // The conversation is fetched again where it's shown: on a page.
    let talk = matches!(state.screen(), Screen::Page(_))
        .then(|| Need::Data(DataKey::PrActivity(pr.clone())));
    // The review comments the diff shows, to tell whether the first
    // answer is newer than they are.
    let shown = (state.diffs.get(&DiffOf::Pr(pr.clone())))
        .and_then(|d| d.inputs().threads.data.as_deref().map(thread_ids));
    let Some(w) = state.watch.as_mut().filter(|w| w.pr == *pr) else {
        return Vec::new();
    };
    w.busy = false;
    let (mut changed, mut threads, mut pushed) = (false, false, None);
    let mut needs: Vec<Need> = Vec::new();
    for (probe, answer) in answers {
        let (etag, body) = match answer {
            Ok(Polled::Changed { etag, body }) => (etag, body),
            Ok(Polled::Same) => continue,
            Err(err) => {
                tracing::warn!(%pr, ?probe, %err, "asking whether the pull request changed");
                continue;
            }
        };
        if probe == Probe::Pull
            && let Some(head) = head_of(&body)
        {
            if w.head.as_ref().is_some_and(|h| *h != head) {
                pushed = Some(head.clone());
            }
            w.head = Some(head);
        }
        // The first answer is only what's there, unless a comment came in
        // after the diff's threads were fetched.
        let first = !w.etags.contains_key(&probe);
        if !etag.is_empty() {
            w.etags.insert(probe.clone(), etag);
        }
        let unseen = probe == Probe::ReviewComments
            && (shown.as_ref())
                .is_some_and(|ids| newest_comment(&body).is_some_and(|id| !ids.contains(&id)));
        if first && pushed.is_none() && !unseen {
            continue;
        }
        changed = true;
        let fetch = match probe {
            Probe::Pull => vec![Some(Need::Pr(pr.clone())), talk.clone()],
            Probe::ReviewComments => {
                threads = true;
                vec![talk.clone()]
            }
            Probe::CheckRuns(_) | Probe::Statuses(_) => vec![Some(Need::Pr(pr.clone()))],
        };
        for need in fetch.into_iter().flatten() {
            if !needs.contains(&need) {
                needs.push(need);
            }
        }
    }
    w.wait = if changed {
        FIRST_MS
    } else {
        w.wait.saturating_mul(2).min(MOST_MS)
    };
    let mut cmds: Vec<Cmd> = needs
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
    let known = state.watch.as_ref().filter(|w| w.pr == *pr);
    let head = known.and_then(|w| w.head.as_deref());
    matches!((head, shown), (Some(head), Some(shown)) if head != shown)
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
    let mine = |author: &str| viewer.as_deref() == Some(author);
    let (mut new_threads, mut replies) = (0u64, 0u64);
    let ids = thread_ids(threads);
    match &mut w.seen_threads {
        None => w.seen_threads = Some(ids),
        Some(seen) => {
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
        }
    }
    if diff.inputs().fresh != w.new_threads {
        let fresh = w.new_threads.clone();
        diff.edit(|i| i.fresh = fresh);
    }
    let mut what = Vec::new();
    if new_threads > 0 {
        what.push(format!("{new_threads} {}", noun(new_threads, "thread")));
    }
    if replies > 0 {
        what.push(format!("{replies} {}", noun(replies, "reply")));
    }
    if !what.is_empty() {
        notices.info(format!("New on {pr}: {}", what.join(", ")));
    }
}

/// The conversation's comments are in: as with threads, what wasn't
/// there before is new, and said.
pub fn activity_loaded(state: &mut State, pr: &PrRef) {
    let Some(activity) = state.picked::<PrActivity>(&DataKey::PrActivity(pr.clone())) else {
        return;
    };
    let viewer = state.viewer.as_deref();
    let comments: Vec<(u64, bool)> = comment_ids(activity)
        .map(|(id, author)| (id, viewer == Some(author)))
        .collect();
    let Some(w) = state.watch.as_mut().filter(|w| w.pr == *pr) else {
        return;
    };
    let Some(seen) = &mut w.seen_comments else {
        w.seen_comments = Some(comments.iter().map(|(id, _)| *id).collect());
        return;
    };
    let mut new = 0;
    for (id, mine) in comments {
        if seen.insert(id) && !mine {
            w.new_comments.insert(id);
            new += 1;
        }
    }
    if new > 0 {
        state.data_gen += 1;
        state.info(format!("New on {pr}: {new} {}", noun(new, "comment")));
    }
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
