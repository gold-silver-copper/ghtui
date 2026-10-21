//! Watching pages other than a pull request's review, with the same
//! [`Pace`] and the same rule for the first answer:
//! - a running job: its steps and outcome as they change, then its log
//!   once it ends (GitHub writes a job's log only then), its page
//!   following the end the way `tail -f` does unless you've moved away;
//! - the Actions tab (and a workflow's runs): new runs, and running ones;
//! - a run: its outcome and its jobs';
//! - an issue: new comments, edits, its state, labels and assignees;
//! - a discussion: new comments and replies, edits, its answer.
//!
//! What another author wrote since you started looking is marked New and
//! said in the status bar; what you wrote isn't.

use crate::route::{Route, discussions, issues, runs};
use std::collections::HashSet;

use crate::api::browse::{
    CheckOutcome, DiscussionDetail, DiscussionsOf, IssueDetail, Job, Results, RunSummary,
    WorkflowRun, rest_actions,
};
use crate::api::model::RepoId;
use crate::api::{ApiError, Polled};
use crate::data::DataKey;
use crate::discuss::same_text;
use crate::state::{Api, Cmd, Cmds, Screen, State};
use crate::ui::text::noun;

use super::{Ask, MOST_MS, Pace};

/// The longest wait while what's on screen is running: it changes soon.
const RUNNING_MS: u64 = 30_000;

/// A page watched.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Watched {
    /// A job, while it (or its run) runs.
    Job {
        repo: RepoId,
        job: u64,
    },
    /// A run, or one attempt of it.
    Run {
        repo: RepoId,
        run: u64,
        attempt: Option<u64>,
    },
    /// Runs, filtered as the Actions tab filters them.
    Runs {
        repo: RepoId,
        query: String,
    },
    Issue {
        repo: RepoId,
        number: u64,
    },
    Discussion {
        of: DiscussionsOf,
        number: u64,
    },
}

/// What's asked about a page.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Probe {
    /// A job, its steps and how they went.
    Job(RepoId, u64),
    /// A run (an attempt of it).
    Run(RepoId, u64, Option<u64>),
    /// Its jobs.
    RunJobs(RepoId, u64, Option<u64>),
    /// The first page of runs a filter lists; finding where takes looking
    /// up a workflow named by its name ([`crate::api::GitHub::runs_path`]).
    Runs(RepoId, String),
    /// An issue: its title, text, state, labels, assignees, and how many
    /// comments it has.
    Issue(RepoId, u64),
    /// Its comments created or edited since a time (ISO 8601).
    IssueComments(RepoId, u64, String),
    /// A discussion: GraphQL has no ETags, so its answer is its own tag
    /// ([`crate::api::GitHub::poll_discussion`]).
    Discussion(RepoId, u64),
}

impl Probe {
    /// Where a REST probe asks; `None` for one asked another way.
    pub fn path(&self) -> Option<String> {
        let repo = |r: &RepoId| format!("/repos/{}/{}", r.owner, r.name);
        let run = |r: &RepoId, run: u64, attempt: Option<u64>| {
            let base = format!("{}/actions/runs/{run}", repo(r));
            match attempt {
                Some(n) => format!("{base}/attempts/{n}"),
                None => base,
            }
        };
        Some(match self {
            Probe::Job(r, job) => format!("{}/actions/jobs/{job}", repo(r)),
            Probe::Run(r, id, attempt) => run(r, *id, *attempt),
            Probe::RunJobs(r, id, attempt) => {
                format!("{}/jobs?per_page=100", run(r, *id, *attempt))
            }
            Probe::Issue(r, n) => format!("{}/issues/{n}", repo(r)),
            Probe::IssueComments(r, n, since) => {
                format!("{}/issues/{n}/comments?since={since}&per_page=100", repo(r))
            }
            Probe::Runs(..) | Probe::Discussion(..) => return None,
        })
    }
}

/// The page watched, and what's been seen of it.
#[derive(Debug)]
pub struct PageWatch {
    pub what: Watched,
    pace: Pace<Probe>,
    /// Comments (by GitHub's number) or runs there when you started
    /// looking, or since; `None` until they're first in.
    seen: Option<HashSet<u64>>,
    /// Comments that came in since (not yours).
    pub new: HashSet<u64>,
    /// The job was running when you started looking: its page follows its
    /// end.
    pub follows: bool,
    /// How the job ended has been said.
    said: bool,
    /// When you started looking, a minute early: an issue's comments
    /// edited since are asked for.
    since: String,
}

/// The page on screen, if it's one that's watched. A job is watched only
/// while it runs, or its run does.
fn watched(state: &State) -> Option<Watched> {
    let Screen::Page(p) = state.screen() else {
        return None;
    };
    Some(match &p.route {
        Route::Runs(runs::Job { repo, job, .. }) => {
            let j = state.picked::<Job>(&DataKey::Job(repo.clone(), *job))?;
            if !running(j) {
                return None;
            }
            Watched::Job {
                repo: repo.clone(),
                job: *job,
            }
        }
        Route::Runs(runs::WorkflowRun { repo, run, attempt }) => Watched::Run {
            repo: repo.clone(),
            run: *run,
            attempt: *attempt,
        },
        Route::Runs(runs::Actions { repo, query }) => Watched::Runs {
            repo: repo.clone(),
            query: query.clone(),
        },
        Route::Runs(runs::Workflow { repo, file }) => Watched::Runs {
            repo: repo.clone(),
            query: format!("workflow:{file}"),
        },
        // Not one that's a pull request (it redirects).
        Route::Issues(issues::Issue { repo, number }) => {
            let key = DataKey::Issue(repo.clone(), *number);
            state.picked::<Option<Box<IssueDetail>>>(&key)?.as_ref()?;
            Watched::Issue {
                repo: repo.clone(),
                number: *number,
            }
        }
        Route::Discussions(discussions::Discussion { of, number }) => Watched::Discussion {
            of: of.clone(),
            number: *number,
        },
        _ => return None,
    })
}

/// A job still going, or in a run still going (which can still re-run it).
fn running(job: &Job) -> bool {
    job.outcome == CheckOutcome::Pending || job.run.outcome == CheckOutcome::Pending
}

/// The data a watched page shows, which a change fetches again.
fn key(what: &Watched) -> DataKey {
    match what {
        Watched::Job { repo, job } => DataKey::Job(repo.clone(), *job),
        Watched::Run { repo, run, attempt } => DataKey::Run(repo.clone(), *run, *attempt),
        Watched::Runs { repo, query } => DataKey::Runs(repo.clone(), query.clone()),
        Watched::Issue { repo, number } => DataKey::Issue(repo.clone(), *number),
        Watched::Discussion { of, number } => DataKey::Discussion(of.clone(), *number),
    }
}

/// Whether the live refresh leaves `key` to a watcher: what the watchers
/// fetch again, they fetch when GitHub says it changed.
pub fn watches(key: &DataKey) -> bool {
    matches!(
        key,
        DataKey::Job(..) | DataKey::JobLog(..) | DataKey::Run(..) | DataKey::Runs(..)
    )
}

/// What's seen of a watched page now: its comments, or its runs, as
/// fetched fresh (a cached copy may be old: what's new waits for GitHub's).
fn seen_now(state: &State, what: &Watched) -> Option<HashSet<u64>> {
    let k = key(what);
    let remote = state.remote(&k)?;
    if remote.cached_at.is_some() || remote.loading && remote.data.is_none() {
        return None;
    }
    Some(match what {
        Watched::Issue { .. } => {
            let issue = state.picked::<Option<Box<IssueDetail>>>(&k)?.as_ref()?;
            issue_comments(issue).map(|(id, _)| id).collect()
        }
        Watched::Discussion { .. } => discussion_comments(state.picked::<DiscussionDetail>(&k)?)
            .map(|(id, _)| id)
            .collect(),
        Watched::Runs { .. } => (state.picked::<Results<RunSummary>>(&k)?.items.iter())
            .map(|r| r.id)
            .collect(),
        Watched::Job { .. } | Watched::Run { .. } => return None,
    })
}

/// An issue's comments' numbers, with their authors.
fn issue_comments(issue: &IssueDetail) -> impl Iterator<Item = (u64, &str)> {
    issue
        .comments()
        .filter_map(|c| Some((c.id?, c.author.as_str())))
}

/// A discussion's comments' and replies' numbers, with their authors.
fn discussion_comments(d: &DiscussionDetail) -> impl Iterator<Item = (u64, &str)> {
    d.comments.iter().flat_map(|c| {
        std::iter::once(&c.comment)
            .chain(c.replies.iter())
            .filter_map(|c| Some((c.id?, c.author.as_str())))
    })
}

impl PageWatch {
    fn new(what: Watched, state: &State) -> Self {
        let follows = matches!(&what, Watched::Job { .. });
        let now = (state.clock)();
        Self {
            seen: seen_now(state, &what),
            pace: Pace::new(),
            new: HashSet::new(),
            follows,
            said: false,
            since: crate::ui::time::iso8601(now.saturating_sub(60)),
            what,
        }
    }
}

/// Watches the page on screen, and schedules the next ask about it.
/// Leaving a page forgets what was new on it.
pub fn schedule(state: &mut State) -> Cmds {
    let here = watched(state);
    if state.page_watch.as_ref().map(|w| &w.what) != here.as_ref() {
        // A job that ended stays followed until you leave it.
        let ended = matches!(
            (&state.page_watch, &here, state.screen()),
            (Some(PageWatch { what: Watched::Job { job, .. }, .. }), None, Screen::Page(p))
                if matches!(&p.route, Route::Runs(runs::Job { job: on, .. }) if on == job)
        );
        if !ended {
            state.page_watch = here.map(|what| PageWatch::new(what, state));
            // A followed job's page shows where it's at from now.
            if state.page_watch.as_ref().is_some_and(|w| w.follows) {
                state.shown_gen += 1;
                state.sync_page();
            }
        }
    }
    let focused = state.focused;
    let live = matches!(state.page_watch.as_ref(), Some(w) if watched_now(state, &w.what));
    match state.page_watch.as_mut() {
        Some(w) if live => w.pace.due(focused).into_iter().collect(),
        _ => Cmds::new(),
    }
}

/// Whether the watch is still asking: a job that ended isn't.
fn watched_now(state: &State, what: &Watched) -> bool {
    watched(state).as_ref() == Some(what)
}

/// Back in focus: the next ask is soon.
pub fn refocus(state: &mut State) {
    if let Some(w) = &mut state.page_watch {
        w.pace.refocus();
    }
}

/// Time to ask about the watched page, if the timer is still its.
pub fn on_timer(state: &mut State, id: u64) -> Cmds {
    let Some(what) = (state.page_watch.as_ref())
        .filter(|w| w.pace.is(id))
        .map(|w| w.what.clone())
    else {
        return Cmds::new();
    };
    // A discussion is asked about in the repository it's in.
    let discussion_repo = match &what {
        Watched::Discussion { of, number } => (state
            .picked::<DiscussionDetail>(&DataKey::Discussion(of.clone(), *number)))
        .map(|d| d.repo.clone()),
        _ => None,
    };
    let run_going = match &what {
        Watched::Job { repo, job } => state
            .picked::<Job>(&DataKey::Job(repo.clone(), *job))
            .filter(|j| j.outcome != CheckOutcome::Pending)
            .map(|j| j.run_id),
        _ => None,
    };
    let Some(w) = state.page_watch.as_mut() else {
        return Cmds::new();
    };
    let probes = match &what {
        // A job that ended in a run still going: the run's end shows on it.
        Watched::Job { repo, job } => match run_going {
            Some(run) => vec![
                Probe::Job(repo.clone(), *job),
                Probe::Run(repo.clone(), run, None),
            ],
            None => vec![Probe::Job(repo.clone(), *job)],
        },
        Watched::Run { repo, run, attempt } => vec![
            Probe::Run(repo.clone(), *run, *attempt),
            Probe::RunJobs(repo.clone(), *run, *attempt),
        ],
        Watched::Runs { repo, query } => vec![Probe::Runs(repo.clone(), query.clone())],
        Watched::Issue { repo, number } => vec![
            Probe::Issue(repo.clone(), *number),
            Probe::IssueComments(repo.clone(), *number, w.since.clone()),
        ],
        Watched::Discussion { number, .. } => match discussion_repo {
            Some(repo) => vec![Probe::Discussion(repo, *number)],
            // Not loaded yet: asked about next time.
            None => {
                w.pace.idle();
                return Cmds::new();
            }
        },
    };
    let asks: Vec<Ask<Probe>> = probes.into_iter().map(|p| w.pace.ask(p)).collect();
    Cmd::Api(Api::Watch { what, asks }).into()
}

/// Asks GitHub one probe's question, with the ETag of its last answer.
pub async fn ask(gh: &crate::api::GitHub, ask: &Ask<Probe>) -> Result<Polled, ApiError> {
    let etag = ask.etag.as_deref();
    match &ask.probe {
        Probe::Runs(repo, query) => {
            let (path, _) = gh.runs_path(repo, query, 1).await?;
            gh.poll(&path, etag).await
        }
        Probe::Discussion(repo, number) => gh.poll_discussion(repo, *number, etag).await,
        probe => {
            let path = probe.path().unwrap_or_default();
            gh.poll(&path, etag).await
        }
    }
}

/// GitHub's answers about a watched page: what changed is fetched again.
pub fn on_watched(
    state: &mut State,
    what: &Watched,
    answers: Vec<(Probe, Result<Polled, ApiError>)>,
) -> Cmds {
    let Some(w) = state.page_watch.as_mut().filter(|w| w.what == *what) else {
        return Cmds::new();
    };
    let mut fresh = Vec::new();
    for (probe, answer) in answers {
        match answer {
            Ok(Polled::Changed { etag, body }) => {
                let first = w.pace.answered(&probe, etag);
                fresh.push((probe, body, first));
            }
            Ok(Polled::Same) => {}
            Err(err) => tracing::warn!(?what, ?probe, %err, "asking whether the page changed"),
        }
    }
    // The first answer is only what was there, unless it shows what the
    // page doesn't: something that came in after the page was fetched.
    let changed: Vec<&(Probe, String, bool)> = (fresh.iter())
        .filter(|(probe, body, first)| !first || differs(state, probe, body))
        .collect();
    // A job that ended has its log now.
    let ended = changed.iter().any(|(probe, body, _)| {
        matches!(probe, Probe::Job(..))
            && serde_json::from_str::<rest_actions::Job>(body)
                .is_ok_and(|j| j.outcome() != CheckOutcome::Pending)
    });
    let k = key(what);
    let busy = state.remote(&k).and_then(|r| r.data.as_ref());
    let running = busy.is_some_and(crate::data::Data::running);
    let extended = state
        .data
        .get(&k)
        .is_some_and(crate::state::Remote::extended);
    let any = !changed.is_empty();
    if let Some(w) = state.page_watch.as_mut() {
        w.pace
            .settle(any, if running { RUNNING_MS } else { MOST_MS });
    }
    if !any {
        return Cmds::new();
    }
    // More pages loaded would go: say so instead.
    if extended {
        let refresh = state.first_key(crate::keymap::Action::Refresh);
        return state.just_say(format!("New on the list: {refresh} shows it"));
    }
    let mut cmds = state.ensure(k, true);
    if ended && let Watched::Job { repo, job } = what {
        cmds.extend(state.ensure(DataKey::JobLog(repo.clone(), *job), true));
    }
    cmds
}

/// Whether a first answer shows something the page doesn't: missing data
/// or an unreadable answer counts as showing something.
fn differs(state: &State, probe: &Probe, body: &str) -> bool {
    differs_or_none(state, probe, body).unwrap_or(true)
}

fn differs_or_none(state: &State, probe: &Probe, body: &str) -> Option<bool> {
    Some(match probe {
        Probe::Job(repo, job) => {
            let shown = state.picked::<Job>(&DataKey::Job(repo.clone(), *job))?;
            let wire: rest_actions::Job = serde_json::from_str(body).ok()?;
            let steps = wire
                .steps
                .iter()
                .map(|s| CheckOutcome::of(s.status, s.conclusion));
            wire.outcome() != shown.outcome || !steps.eq(shown.steps.iter().map(|s| s.outcome))
        }
        Probe::Run(repo, run, attempt) => {
            let shown = state.picked::<WorkflowRun>(&DataKey::Run(repo.clone(), *run, *attempt))?;
            let wire: rest_actions::Run = serde_json::from_str(body).ok()?;
            CheckOutcome::of(wire.status, wire.conclusion) != shown.outcome
                || wire.updated_at != shown.updated_at
        }
        Probe::RunJobs(repo, run, attempt) => {
            let shown = state.picked::<WorkflowRun>(&DataKey::Run(repo.clone(), *run, *attempt))?;
            let wire: rest_actions::Jobs = serde_json::from_str(body).ok()?;
            wire.jobs.iter().any(|j| {
                let outcome = j.outcome();
                !shown
                    .jobs
                    .iter()
                    .any(|s| s.id == j.id && s.outcome == outcome)
            })
        }
        Probe::Runs(repo, query) => {
            let key = DataKey::Runs(repo.clone(), query.clone());
            let shown = state.picked::<Results<RunSummary>>(&key)?;
            let wire: rest_actions::Runs = serde_json::from_str(body).ok()?;
            wire.workflow_runs.iter().any(|r| {
                let outcome = CheckOutcome::of(r.status, r.conclusion);
                !shown
                    .items
                    .iter()
                    .any(|s| s.id == r.id && s.outcome == outcome)
            })
        }
        Probe::Issue(repo, number) => {
            let key = DataKey::Issue(repo.clone(), *number);
            let shown = state.picked::<Option<Box<IssueDetail>>>(&key)?.as_ref()?;
            let wire: wire::Issue = serde_json::from_str(body).ok()?;
            wire.differs(shown)
        }
        Probe::IssueComments(repo, number, _) => {
            let key = DataKey::Issue(repo.clone(), *number);
            let shown = state.picked::<Option<Box<IssueDetail>>>(&key)?.as_ref()?;
            let wire: Vec<wire::Comment> = serde_json::from_str(body).ok()?;
            wire.iter().any(|c| {
                let body = c.body.as_deref().unwrap_or_default();
                !shown
                    .comments()
                    .any(|s| s.id == Some(c.id) && same_text(&s.body, body))
            })
        }
        Probe::Discussion(..) => {
            let shown = discussion_here(state)?;
            let wire: wire::Discussion = serde_json::from_str(body).ok()?;
            wire.title != shown.title
                || !same_text(&shown.body, &wire.body)
                || wire.comments.total_count != shown.comments.total()
        }
    })
}

/// The discussion on screen, as loaded.
fn discussion_here(state: &State) -> Option<&DiscussionDetail> {
    match state.route()? {
        Route::Discussions(discussions::Discussion { of, number }) => {
            state.picked::<DiscussionDetail>(&DataKey::Discussion(of.clone(), *number))
        }
        _ => None,
    }
}

/// Fresh data came in for `key`: on a watched page, what wasn't there
/// before is new, and said; a followed job that ended says how.
pub fn loaded(state: &mut State, key: &DataKey) {
    let Some(what) = state.page_watch.as_ref().map(|w| w.what.clone()) else {
        return;
    };
    if self::key(&what) != *key {
        return;
    }
    let viewer = state.viewer.clone();
    let mine = |author: &str| viewer.as_deref() == Some(author);
    let (came, word, place): (Vec<(u64, bool)>, &str, String) = match &what {
        Watched::Issue { repo, number } => {
            let Some(Some(issue)) = state.picked::<Option<Box<IssueDetail>>>(key) else {
                return;
            };
            let came = issue_comments(issue).map(|(id, a)| (id, mine(a))).collect();
            (came, "comment", format!("{repo}#{number}"))
        }
        Watched::Discussion { number, .. } => {
            let Some(d) = state.picked::<DiscussionDetail>(key) else {
                return;
            };
            let came = discussion_comments(d)
                .map(|(id, a)| (id, mine(a)))
                .collect();
            (
                came,
                "comment",
                format!("{}'s discussion #{number}", d.repo),
            )
        }
        Watched::Runs { .. } => {
            let Some(runs) = state.picked::<Results<RunSummary>>(key) else {
                return;
            };
            let came = (runs.items.iter())
                .map(|r| (r.id, r.actor.as_deref().is_some_and(mine)))
                .collect();
            (came, "run", "the Actions tab".to_owned())
        }
        Watched::Job { .. } => {
            job_loaded(state, key);
            return;
        }
        Watched::Run { .. } => return,
    };
    let Some(w) = state.page_watch.as_mut() else {
        return;
    };
    let came = super::note(&mut w.seen, came, |mine| *mine);
    // Runs are said, not marked: the list's order shows them.
    if word != "run" {
        w.new.extend(came.iter().map(|(id, _)| *id));
    }
    let new = came.len() as u64;
    if new > 0 {
        state.shown_gen += 1;
        state.info(format!("New on {place}: {new} {}", noun(new, word)));
    }
}

/// A followed job's steps came in: once it ends, say how.
fn job_loaded(state: &mut State, key: &DataKey) {
    let Some(job) = state.picked::<Job>(key) else {
        return;
    };
    if job.outcome == CheckOutcome::Pending {
        return;
    }
    let how = match job.outcome {
        CheckOutcome::Success => "succeeded",
        CheckOutcome::Failure => "failed",
        CheckOutcome::Cancelled => "was cancelled",
        CheckOutcome::Skipped => "was skipped",
        CheckOutcome::Neutral | CheckOutcome::Pending => "ended",
    };
    let text = format!("{} {how}", job.name);
    if let Some(w) = state.page_watch.as_mut().filter(|w| w.follows && !w.said) {
        w.said = true;
        state.info(text);
    }
}

/// The comments marked New on a watched issue or discussion, by GitHub's
/// number.
pub fn fresh<'a>(state: &'a State, route: &Route) -> Option<&'a HashSet<u64>> {
    let w = state.page_watch.as_ref()?;
    let here = match (&w.what, route) {
        (Watched::Issue { repo, number }, Route::Issues(issues::Issue { repo: r, number: n })) => {
            repo == r && number == n
        }
        (
            Watched::Discussion { of, number },
            Route::Discussions(discussions::Discussion { of: o, number: n }),
        ) => of == o && number == n,
        _ => false,
    };
    here.then_some(&w.new)
}

/// Whether job `job`'s page follows its end: it was running when you
/// started looking, and still is or just ended.
pub fn follows(state: &State, repo: &RepoId, job: u64) -> bool {
    state.page_watch.as_ref().is_some_and(
        |w| matches!(&w.what, Watched::Job { repo: r, job: j } if r == repo && *j == job),
    )
}

/// GitHub's answers, as the probes read them.
mod wire {
    use serde::Deserialize;

    use crate::api::browse::IssueDetail;
    use crate::api::model::IssueState;

    #[derive(Deserialize)]
    pub struct Name {
        pub name: String,
    }

    #[derive(Deserialize)]
    pub struct Login {
        pub login: String,
    }

    /// An issue, as REST has it.
    #[derive(Deserialize)]
    pub struct Issue {
        pub title: String,
        pub body: Option<String>,
        pub state: String,
        pub state_reason: Option<String>,
        pub comments: u64,
        #[serde(default)]
        pub labels: Vec<Name>,
        #[serde(default)]
        pub assignees: Vec<Login>,
    }

    impl Issue {
        /// Whether it shows something `shown` doesn't.
        pub fn differs(&self, shown: &IssueDetail) -> bool {
            let state = match (self.state.as_str(), self.state_reason.as_deref()) {
                ("open", _) => IssueState::Open,
                (_, Some("not_planned" | "duplicate")) => IssueState::NotPlanned,
                _ => IssueState::Closed,
            };
            let mut labels: Vec<&str> = self.labels.iter().map(|l| l.name.as_str()).collect();
            let mut shown_labels: Vec<&str> =
                shown.labels.iter().map(|l| l.name.as_str()).collect();
            labels.sort_unstable();
            shown_labels.sort_unstable();
            let mut people: Vec<&str> = self.assignees.iter().map(|a| a.login.as_str()).collect();
            let mut shown_people: Vec<&str> = shown.assignees.iter().map(String::as_str).collect();
            people.sort_unstable();
            shown_people.sort_unstable();
            self.title != shown.title
                || !crate::discuss::same_text(&shown.body, self.body.as_deref().unwrap_or_default())
                || state != shown.state
                || self.comments != shown.comment_count
                || labels != shown_labels
                || people != shown_people
        }
    }

    /// An issue's comment, as REST has it.
    #[derive(Deserialize)]
    pub struct Comment {
        pub id: u64,
        pub body: Option<String>,
    }

    /// A discussion, as [`crate::api::raw::DISCUSSION_PROBE`] has it.
    #[derive(Deserialize)]
    pub struct Discussion {
        pub title: String,
        pub body: String,
        pub comments: Count,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Count {
        pub total_count: u64,
    }
}
