//! Reading offline. There's no separate offline mode: a page shows what
//! the cache has first, and GitHub refreshes it, so when GitHub can't be
//! reached the page is simply the cache's, marked offline with its age,
//! and code comes from the clones. What's offline here:
//! - whether GitHub can be reached ([`crate::api::reach`]): the header
//!   says so, once, and requests fail at once instead of loading;
//! - writes that waited for it ([`queue`]): shown on their pages as
//!   queued, sent in order once it's back, a refused one kept;
//! - repositories kept to read offline ([`keep`]), fetched ahead.
//!
//! The Offline page (`:offline`, or the header's link) lists both.

pub mod keep;
mod page;
pub mod queue;

use crate::act::By;
use crate::api::ApiError;
use crate::api::change::Change;
use crate::api::model::PrRef;
use crate::keymap::Action;
use crate::route::{Dest, Route};
use crate::state::{Cmd as Do, Overlay, Screen, State, Timer};
use crate::ui::page::Link;

pub use page::{build, queued_here};

/// What the background says.
#[derive(Debug)]
pub enum Msg {
    /// GitHub became unreachable (since when), or reachable again.
    Reach(Option<u64>),
    /// A probe was answered (or wasn't).
    Probed(bool),
    Queue(queue::Told),
    /// The kept repositories, and how big the cache is.
    Kept(Vec<keep::Kept>, Option<u64>),
}

/// What's asked of the background.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cmd {
    /// Ask whether GitHub's there.
    Probe,
    Queue(queue::Op),
    Keep(keep::Op),
}

/// What the page knows of all that.
#[derive(Debug, Default)]
pub struct Offline {
    /// Since when GitHub has been unreachable, if it is.
    pub since: Option<u64>,
    /// The queued writes, as the queue said last.
    pub queue: Vec<queue::Entry>,
    /// The kept repositories, as the keeper said last.
    pub kept: Vec<keep::Kept>,
    /// How big the cache is.
    pub cache: Option<u64>,
    pub limits: keep::Limits,
    /// How long until the next probe (ms), while one's scheduled.
    probe: Option<u64>,
    /// The write thrown away last, to bring back.
    discarded: Option<queue::Entry>,
}

/// How soon GitHub is asked again after it went away, and the most.
const PROBE_FIRST: u64 = 2_000;
const PROBE_MOST: u64 = 30_000;

impl Offline {
    /// The writes still waiting (queued, or on their way).
    pub fn waiting(&self) -> usize {
        (self.queue.iter())
            .filter(|e| matches!(e.status, queue::Status::Queued | queue::Status::Sending))
            .count()
    }

    /// The writes GitHub didn't take, or that may not have reached it.
    pub fn unsent(&self) -> usize {
        self.queue.len() - self.waiting()
    }

    pub fn keeps(&self, repo: &crate::api::model::RepoId) -> Option<&keep::Kept> {
        self.kept.iter().find(|k| k.repo == *repo)
    }
}

#[must_use]
pub fn on_msg(state: &mut State, msg: Msg) -> Vec<Do> {
    match msg {
        Msg::Reach(Some(since)) => {
            state.offline.since = Some(since);
            return schedule(state);
        }
        Msg::Reach(None) => {
            if state.offline.since.take().is_none() {
                return Vec::new();
            }
            state.info("GitHub is back");
            let mut cmds = vec![
                Do::Offline(Cmd::Queue(queue::Op::Flush)),
                Do::Offline(Cmd::Keep(keep::Op::Now)),
            ];
            cmds.extend(state.load_visible(false));
            return cmds;
        }
        Msg::Probed(true) => state.offline.probe = None,
        Msg::Probed(false) => {
            let off = &mut state.offline;
            let wait = off.probe.map_or(PROBE_FIRST, |w| (w * 2).min(PROBE_MOST));
            off.probe = Some(wait);
            if off.since.is_some() {
                return vec![Do::Timer(Timer::Probe, wait)];
            }
        }
        Msg::Queue(queue::Told::All(entries)) => state.offline.queue = entries,
        Msg::Queue(queue::Told::Taken(entry)) => {
            state.info(format!(
                "Sent to GitHub: {} on {}",
                entry.write.what(),
                entry.title
            ));
            // What it changed shows (it's a write: everything's stale).
            return state.load_visible(false);
        }
        Msg::Kept(kept, cache) => (state.offline.kept, state.offline.cache) = (kept, cache),
    }
    Vec::new()
}

/// The first probe after GitHub went away.
fn schedule(state: &mut State) -> Vec<Do> {
    if state.offline.probe.is_some() {
        return Vec::new();
    }
    state.offline.probe = Some(PROBE_FIRST);
    vec![Do::Timer(Timer::Probe, PROBE_FIRST)]
}

/// It's time to ask whether GitHub's back.
#[must_use]
pub fn on_timer(state: &mut State) -> Vec<Do> {
    if state.offline.since.is_none() {
        state.offline.probe = None;
        return Vec::new();
    }
    vec![Do::Offline(Cmd::Probe)]
}

/// Queues a change GitHub couldn't be reached for, if it can wait:
/// what sent it closes, and the page shows it queued. `None` when it
/// can't wait (or it reached GitHub after all).
#[must_use]
pub fn queue_change(
    state: &mut State,
    change: &Change,
    by: &By,
    err: &ApiError,
    sender: bool,
) -> Option<Vec<Do>> {
    let ApiError::Offline(_) = err else {
        return None;
    };
    let write = queue::Write::of(change)?;
    let route = match by {
        By::Reply(pr) => Some(Route::pr(pr.clone())),
        _ => here(state),
    };
    // What sent it is done with it: it's in the queue now.
    if sender
        && matches!(
            state.overlay,
            Some(Overlay::Compose(_) | Overlay::Confirm(_))
        )
    {
        state.overlay = None;
    }
    state.info(format!(
        "GitHub can't be reached: {} is queued, to send when it's back",
        write.what().to_lowercase()
    ));
    Some(vec![add(state, write, route)])
}

/// A review GitHub couldn't be reached for: queued, its comments with it.
#[must_use]
pub fn queue_review(state: &mut State, review: &queue::Review) -> Do {
    let route = Route::pr(review.pr.clone());
    state.info("GitHub can't be reached: your review is queued, to send when it's back");
    add(state, queue::Write::Review(review.clone()), Some(route))
}

fn add(state: &State, write: queue::Write, route: Option<Route>) -> Do {
    let (page, title) = route.map_or_else(
        || (String::new(), "GitHub".to_owned()),
        |r| (r.url(), r.title()),
    );
    let entry = queue::Entry {
        id: 0,
        at: (state.clock)(),
        write,
        page,
        title,
        status: queue::Status::Queued,
    };
    Do::Offline(Cmd::Queue(queue::Op::Add(Box::new(entry))))
}

/// The page a write is made on: the page on screen, or a pull request's
/// conversation for its diff.
fn here(state: &State) -> Option<Route> {
    match state.screen() {
        Screen::Page(p) => Some(p.route.clone()),
        Screen::Diff(d) => match &d.of {
            crate::diff_screen::DiffOf::Pr(pr) => Some(Route::pr(PrRef::clone(pr))),
            _ => None,
        },
    }
}

/// What the header says: GitHub can't be reached, and what waits for it;
/// or what it didn't take.
/// Short, most important first: on a narrow header, what doesn't fit is
/// left out from the last.
pub fn header(state: &State) -> Vec<String> {
    let off = &state.offline;
    let waiting = off.waiting();
    let mut said = Vec::new();
    if off.since.is_some() {
        said.push("⚠ Offline".to_owned());
    }
    match (off.since, waiting, off.unsent()) {
        (Some(_), 0, _) | (None, 0, 0) => {}
        (Some(_), n, _) => said.push(format!("{n} queued")),
        (None, 0, n) => said.push(format!("⚠ {n} not sent")),
        (None, n, _) => said.push(format!("Sending {n}")),
    }
    said
}

/// The banner over a page showing the cache's copy because GitHub can't
/// be reached.
pub fn banner(state: &State, when: &str) -> Option<String> {
    state
        .offline
        .since
        .map(|_| format!("Offline: showing what was loaded {when}."))
}

/// The queued write a row is, if it's one.
fn selected(state: &State) -> Option<&queue::Entry> {
    let Screen::Page(p) = state.screen() else {
        return None;
    };
    let Some(Link::Queued(id)) = p.selected_link() else {
        return None;
    };
    state.offline.queue.iter().find(|e| e.id == *id)
}

/// Whether `action` is offline's here.
pub fn claims(state: &State, action: Action) -> bool {
    match action {
        Action::Offline | Action::KeepOffline => true,
        Action::Refresh => state.offline.since.is_some(),
        // Where the write was thrown away: elsewhere, it's a draft's or a
        // Home section's.
        Action::UndoDelete => state.offline.discarded.as_ref().is_some_and(|e| {
            let here = state.route();
            here == Some(&Route::Offline) || here.is_some_and(|r| r.url() == e.page)
        }),
        Action::Rerun | Action::Delete | Action::Copy => selected(state).is_some(),
        _ => false,
    }
}

#[must_use]
pub fn act(state: &mut State, action: Action) -> Vec<Do> {
    match action {
        Action::Offline if state.route() == Some(&Route::Offline) => {
            state.info("This is the Offline page");
            Vec::new()
        }
        Action::Offline => state.push(Route::Offline),
        Action::KeepOffline => keep(state),
        // Asked now, besides fetching again.
        Action::Refresh => {
            let mut cmds = state.load_visible(true);
            cmds.push(Do::Offline(Cmd::Probe));
            state.info("Asking whether GitHub's back…");
            cmds
        }
        Action::UndoDelete => match state.offline.discarded.take() {
            Some(entry) => {
                state.info(format!("Brought back: {}", entry.write.what()));
                vec![Do::Offline(Cmd::Queue(queue::Op::Add(Box::new(entry))))]
            }
            None => Vec::new(),
        },
        _ => {
            let Some(entry) = selected(state).cloned() else {
                return Vec::new();
            };
            on_entry(state, action, entry)
        }
    }
}

/// `action` on a queued write's row.
fn on_entry(state: &mut State, action: Action, entry: queue::Entry) -> Vec<Do> {
    let what = entry.write.what();
    match action {
        Action::Copy => match entry.write.text() {
            Some(text) => {
                state.info(format!("Copied the text of {}", what.to_lowercase()));
                vec![Do::Copy(text)]
            }
            None => {
                state.info(format!("{what} has no text to copy"));
                Vec::new()
            }
        },
        Action::Rerun => match entry.status {
            queue::Status::Refused(_) | queue::Status::Unsure(_) => {
                state.info(format!("Sending again: {what}"));
                vec![Do::Offline(Cmd::Queue(queue::Op::Retry(entry.id)))]
            }
            queue::Status::Queued | queue::Status::Sending => {
                state.info(format!("{what} is queued already"));
                Vec::new()
            }
        },
        Action::Delete if entry.status == queue::Status::Sending => {
            state.info(format!("{what} is on its way to GitHub"));
            Vec::new()
        }
        Action::Delete => {
            let undo = state.first_key(Action::UndoDelete);
            state.info(format!("Threw away: {what} · {undo} brings it back"));
            let id = entry.id;
            state.offline.discarded = Some(entry);
            vec![Do::Offline(Cmd::Queue(queue::Op::Discard(id)))]
        }
        _ => Vec::new(),
    }
}

/// Keeps the repository on screen to read offline, or stops.
fn keep(state: &mut State) -> Vec<Do> {
    let Some(repo) = state.route().and_then(Route::repo).cloned() else {
        state.info("Keep offline works on a repository's pages");
        return Vec::new();
    };
    if state.offline.keeps(&repo).is_some() {
        state.offline.kept.retain(|k| k.repo != repo);
        state.info(format!(
            "No longer keeping {repo} offline (what the cache has stays until it's replaced)"
        ));
    } else {
        state.info(format!("Keeping {repo} to read offline: fetching it ahead"));
    }
    vec![Do::Offline(Cmd::Keep(keep::Op::Toggle(repo)))]
}

/// Following a queued write's row: its page (from the Offline page).
#[must_use]
pub fn follow(state: &mut State, id: u64) -> Vec<Do> {
    let Some(entry) = state.offline.queue.iter().find(|e| e.id == id) else {
        return Vec::new();
    };
    let here = state.route().map(Route::url);
    if entry.page.is_empty() || here.as_deref() == Some(entry.page.as_str()) {
        let (retry, copy) = (
            state.first_key(Action::Rerun),
            state.first_key(Action::Copy),
        );
        state.info(format!(
            "{} · {retry} sends a refused one again · {copy} copies its text",
            entry.write.what()
        ));
        return Vec::new();
    }
    let page = entry.page.clone();
    state.go(Dest::from_url(&page))
}

/// The Space menu's offline rows.
pub fn doables(state: &State, out: &mut Vec<crate::nav::Doable>) {
    use crate::nav::Doable;
    let row = |action: Action, label: String| Doable {
        section: "Offline",
        action,
        label,
        short: "",
        unavailable: None,
    };
    if let Some(entry) = selected(state) {
        let what = entry.write.what().to_lowercase();
        let mut again = row(Action::Rerun, format!("Send {what} again"));
        if matches!(entry.status, queue::Status::Queued | queue::Status::Sending) {
            again.unavailable = Some("it's queued".into());
        }
        out.push(again);
        out.push(row(Action::Delete, format!("Throw away {what}")));
        let mut copy = row(Action::Copy, format!("Copy the text of {what}"));
        if entry.write.text().is_none() {
            copy.unavailable = Some("it has no text".into());
        }
        out.push(copy);
    }
    if let Some(repo) = state.route().and_then(Route::repo) {
        let label = match state.offline.keeps(repo) {
            Some(_) => format!("Stop keeping {repo} offline"),
            None => format!("Keep {repo} to read offline"),
        };
        out.push(row(Action::KeepOffline, label));
    }
    // First of the menu's Go rows, which follow.
    if state.route() != Some(&Route::Offline) {
        out.push(Doable {
            section: "Go",
            ..row(
                Action::Offline,
                "Offline: what's queued for GitHub, and what's kept".into(),
            )
        });
    }
}
