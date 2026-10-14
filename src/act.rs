//! Changing GitHub from a page: merging, closing and reopening, marking a
//! draft ready, updating a branch, approving, re-running and cancelling
//! runs, marking notifications read or done. What an action does where
//! you are is decided once, by [`plan`]: the key, the menu and the command
//! palette all ask it, so they can't disagree, and it asks GitHub's own
//! word on what you may do. Every change goes to GitHub as a [`Change`]
//! ([`Changing`]) and its answer is handled by [`on_changed`]; the page
//! then follows GitHub until it shows the change ([`Shows`]).

use crate::api::ApiError;
use crate::api::browse::{CheckOutcome, IssueState, Job, PrState, WorkflowRun};
use crate::api::change::{Change, CloseReason, MergeMethod};
use crate::api::model::{
    ChecksState, MergeState, PrDetail, PrRef, Readiness, RepoId, ReviewDecision,
};
use crate::api::notifications::Inbox;
use crate::api::project::ProjectRef;
use crate::route::{Route, discussions, issues, projects, pulls, runs, search, security, settings};
use crossterm::event::{KeyCode, KeyEvent};

use crate::data::{DataKey, Picked};
use crate::follow::{Expectation, Shown};
use crate::keymap::Action;
use crate::nav;
use crate::state::{Api, Cmd, Overlay, Screen, State};
use crate::work::{self, Cx, Failure};

mod inbox;
mod merge;

/// The actions that change GitHub, as the menu lists them.
pub const ACTIONS: [Action; 10] = [
    Action::MarkRead,
    Action::MarkDone,
    Action::MarkAllRead,
    Action::MarkAllDone,
    Action::Merge,
    Action::Approve,
    Action::ReadyForReview,
    Action::UpdateBranch,
    Action::Rerun,
    Action::Close,
];

/// A change to confirm: what it is, what's worth knowing first, and the
/// ways to make it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirm {
    /// What asked for it: planned again as GitHub's answers arrive, so
    /// the facts stay true and a change that no longer applies closes.
    pub action: Action,
    /// What it changes, kept: on a list, the selection may move on.
    pub about: Subject,
    pub title: String,
    /// Each with whether it's a problem.
    pub facts: Vec<(String, bool)>,
    pub choices: Vec<(String, Change)>,
    pub selected: usize,
    pub sending: bool,
    pub error: Option<String>,
}

impl Confirm {
    /// What's happening while GitHub answers.
    pub fn busy(&self) -> Option<String> {
        let (_, change) = self.choices.get(self.selected).filter(|_| self.sending)?;
        Some(format!("{}…", doing(change)))
    }
}

/// What sent a change: GitHub's answer goes back there, and only there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum By {
    /// A confirmation, about this.
    Confirm(Subject),
    /// An action that asks nothing first, about this.
    Act(Subject),
    /// The composer, posting a comment.
    Compose,
    /// The composer, replying to one of this pull request's threads.
    Reply(PrRef),
    Star,
    /// An edit to what's on GitHub (from the composer or a picker), about
    /// this.
    Edit(Subject),
}

/// What a page shows once GitHub has a change made on it: GitHub can
/// take a while (a cancel, a minute or more), so what it's about is
/// fetched again until it shows it ([`crate::follow`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shows {
    pub change: Change,
    /// What it changed: the page's own subject, or a row's on a list.
    pub about: Subject,
}

/// An action asked for on something not loaded yet (a list's row): it
/// runs once that's in, if you're still on the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub action: Action,
    pub subject: Subject,
    pub route: Option<Route>,
}

/// What an action does here.
#[derive(Debug)]
enum Plan {
    /// Ask first: what, what's worth knowing (each with whether it's a
    /// problem), and the ways to make it.
    Ask(String, Vec<(String, bool)>, Vec<(String, Change)>),
    Do(Change),
    Approve(PrRef),
    /// It's about something that has to load first.
    Load(DataKey),
    /// Choose why to dismiss a security alert.
    Dismiss(crate::security::Dismissing),
}

/// Runs one of [`ACTIONS`] on what's on screen, or on the selected row
/// of a list.
#[must_use]
pub fn act(state: &mut State, action: Action) -> Vec<Cmd> {
    let Some(subject) = subject(state, action) else {
        state.info(elsewhere(action));
        return Vec::new();
    };
    // A row's may be an old copy (from the cache, before a change); a
    // page keeps its own fresh.
    let row = state.route().is_some_and(|r| Subject::of(r).is_none());
    let mut cmds = if row {
        state.ensure(subject.need(), false)
    } else {
        Vec::new()
    };
    cmds.extend(act_on(state, subject, action));
    cmds
}

#[must_use]
fn act_on(state: &mut State, subject: Subject, action: Action) -> Vec<Cmd> {
    match plan_for(state, &subject, action) {
        Err(why) => {
            state.info(why);
            Vec::new()
        }
        Ok(Plan::Ask(title, facts, choices)) => {
            let confirm = Confirm {
                action,
                about: subject,
                title,
                facts,
                choices,
                selected: 0,
                sending: false,
                error: None,
            };
            state.overlay = Some(Overlay::Confirm(Box::new(confirm)));
            Vec::new()
        }
        Ok(Plan::Do(change)) => {
            state.info(format!("{}…", doing(&change)));
            vec![send(change, By::Act(subject))]
        }
        Ok(Plan::Approve(pr)) => crate::review::open_approve(state, pr),
        Ok(Plan::Dismiss(d)) => crate::security::open_dismiss(state, d),
        Ok(Plan::Load(need)) => {
            state.info(format!("Loading {}…", subject.name()));
            let route = state.route().cloned();
            state.pending = Some(Pending {
                action,
                subject,
                route,
            });
            state.ensure(need, false)
        }
    }
}

/// What an action changes: a pull request, an issue, a run, a job, a
/// notification's thread or what the inbox shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subject {
    Pr(PrRef),
    Issue(RepoId, u64),
    Run(RepoId, u64, Option<u64>),
    /// A job, with its run when its page has it.
    Job(RepoId, Option<u64>, u64),
    /// A thread, by its ID, in the inbox `query` lists.
    Thread {
        id: String,
        query: String,
    },
    /// What the inbox `query` lists shows.
    Inbox {
        query: String,
    },
    /// A project's items, at a view.
    Project(ProjectRef, Option<u64>),
    /// A security alert.
    Alert(crate::api::security::AlertRef),
    /// A discussion and its comments.
    Discussion(crate::api::browse::DiscussionsOf, u64),
    /// A page of a repository's Settings tab ([`crate::settings`]).
    Settings(RepoId, crate::api::settings::SettingsPage),
}

impl Subject {
    /// The subject of a page about one thing.
    fn of(route: &Route) -> Option<Self> {
        Some(match route {
            Route::Pulls(pulls::Pr { pr, .. }) => Subject::Pr(pr.clone()),
            Route::Issues(issues::Issue { repo, number }) => Subject::Issue(repo.clone(), *number),
            Route::Runs(runs::WorkflowRun { repo, run, attempt }) => {
                Subject::Run(repo.clone(), *run, *attempt)
            }
            Route::Runs(runs::Job { repo, run, job, .. }) => Subject::Job(repo.clone(), *run, *job),
            Route::Security(security::Alert { repo, kind, number }) => {
                Subject::Alert(crate::api::security::AlertRef {
                    repo: repo.clone(),
                    kind: *kind,
                    number: *number,
                })
            }
            Route::Discussions(discussions::Discussion { of, number }) => {
                Subject::Discussion(of.clone(), *number)
            }
            _ => return None,
        })
    }

    /// Its page.
    fn route(&self) -> Route {
        match self {
            Subject::Pr(pr) => Route::pr(pr.clone()),
            Subject::Issue(repo, number) => Route::Issues(issues::Issue {
                repo: repo.clone(),
                number: *number,
            }),
            Subject::Run(repo, run, attempt) => Route::Runs(runs::WorkflowRun {
                repo: repo.clone(),
                run: *run,
                attempt: *attempt,
            }),
            Subject::Job(repo, run, job) => Route::Runs(runs::Job {
                repo: repo.clone(),
                run: *run,
                job: *job,
                step: None,
                query: String::new(),
            }),
            Subject::Thread { query, .. } | Subject::Inbox { query } => {
                Route::Search(search::Notifications {
                    query: query.clone(),
                })
            }
            Subject::Project(project, view) => Route::Projects(projects::Project {
                project: project.clone(),
                view: *view,
            }),
            Subject::Alert(a) => Route::Security(security::Alert {
                repo: a.repo.clone(),
                kind: a.kind,
                number: a.number,
            }),
            Subject::Discussion(of, number) => Route::Discussions(discussions::Discussion {
                of: of.clone(),
                number: *number,
            }),
            Subject::Settings(repo, page) => Route::Settings(settings::Page {
                repo: repo.clone(),
                page: *page,
            }),
        }
    }

    /// What planning for it reads.
    fn need(&self) -> DataKey {
        match self {
            Subject::Pr(pr) => DataKey::Pr(pr.clone()),
            Subject::Issue(repo, number) => DataKey::Issue(repo.clone(), *number),
            Subject::Run(repo, run, attempt) => DataKey::Run(repo.clone(), *run, *attempt),
            Subject::Job(repo, _, job) => DataKey::Job(repo.clone(), *job),
            Subject::Thread { query, .. } | Subject::Inbox { query } => {
                DataKey::Notifications(query.clone())
            }
            Subject::Project(project, view) => DataKey::Project(project.clone(), *view),
            Subject::Alert(a) => DataKey::Alert(a.clone()),
            Subject::Discussion(of, number) => DataKey::Discussion(of.clone(), *number),
            Subject::Settings(repo, page) => DataKey::Settings(repo.clone(), *page),
        }
    }

    /// Whether `action` is for it: the one place that says so.
    fn fits(&self, action: Action) -> bool {
        use Action as A;
        match self {
            Subject::Pr(_) => matches!(
                action,
                A::Merge | A::Approve | A::ReadyForReview | A::UpdateBranch | A::Close
            ),
            Subject::Issue(..) | Subject::Alert(_) | Subject::Discussion(..) => action == A::Close,
            Subject::Run(..) | Subject::Job(..) => matches!(action, A::Rerun | A::Close),
            Subject::Thread { .. } => matches!(action, A::MarkRead | A::MarkDone),
            Subject::Inbox { .. } => matches!(action, A::MarkAllRead | A::MarkAllDone),
            // Moving and setting go through `crate::projects`, writing in
            // a discussion through `crate::discuss`, settings through
            // `crate::settings`.
            Subject::Project(..) | Subject::Settings(..) => false,
        }
    }

    fn name(&self) -> String {
        match self {
            Subject::Pr(pr) => pr.to_string(),
            Subject::Issue(repo, number) => format!("{repo}#{number}"),
            Subject::Run(..) => "the run".to_owned(),
            Subject::Job(..) => "the job".to_owned(),
            Subject::Thread { .. } => "the notification".to_owned(),
            Subject::Inbox { .. } => "your notifications".to_owned(),
            Subject::Project(..) => "the project".to_owned(),
            Subject::Alert(_) => "the alert".to_owned(),
            Subject::Discussion(..) => "the discussion".to_owned(),
            Subject::Settings(repo, _) => format!("{repo}'s settings"),
        }
    }
}

/// What an action would change here: the page's own subject (a pull
/// request, its files, an issue, a run, a job), or on a list, the
/// selected row's (in the inbox, its thread is what's marked read).
pub(crate) fn subject(state: &State, action: Action) -> Option<Subject> {
    if matches!(action, Action::MarkRead | Action::MarkDone) {
        return thread(state);
    }
    if matches!(action, Action::MarkAllRead | Action::MarkAllDone) {
        let query = state.route().and_then(|r| match r {
            Route::Search(search::Notifications { query }) => Some(query.clone()),
            _ => None,
        });
        return query.map(|query| Subject::Inbox { query });
    }
    match state.screen() {
        Screen::Diff(d) => d.of.pr().map(|pr| Subject::Pr(pr.clone())),
        Screen::Page(p) => Subject::of(&p.route).or_else(|| {
            let url = p.selected_link()?.url()?;
            match crate::route::Dest::from_url(url).target {
                crate::route::Target::Page(route) => Subject::of(&route),
                crate::route::Target::Files(of) => of.pr().cloned().map(Subject::Pr),
                crate::route::Target::External(_) => None,
            }
        }),
    }
}

/// The selected notification's thread: the inbox's rows are its
/// notifications, in order.
fn thread(state: &State) -> Option<Subject> {
    let Screen::Page(p) = state.screen() else {
        return None;
    };
    let Route::Search(search::Notifications { query }) = &p.route else {
        return None;
    };
    let key = DataKey::Notifications(query.clone());
    let inbox = state.picked::<Inbox>(&key)?;
    let n = inbox.list.items.get(p.selected?)?;
    let url = p.selected_link().and_then(crate::ui::page::Link::url);
    (url == Some(n.url.as_str())).then(|| Subject::Thread {
        id: n.id.clone(),
        query: query.clone(),
    })
}

/// Where `action` works, for when it doesn't here.
fn elsewhere(action: Action) -> &'static str {
    match action {
        Action::MarkRead | Action::MarkDone => "That works on a notification in your inbox",
        Action::MarkAllRead | Action::MarkAllDone => "That works in your inbox",
        Action::Close => {
            "Closing works on an issue, a pull request, a discussion, a security alert or a running workflow run (or its row)"
        }
        Action::Rerun => "Re-running works on a workflow run or a job",
        _ => "That works on a pull request (or its row in a list)",
    }
}

fn plan_for(state: &State, subject: &Subject, action: Action) -> Result<Plan, String> {
    let plan = plan_here(state, subject, action)?;
    // GitHub hasn't shown the last change yet: what's on screen is old.
    if let Some(waiting) = awaiting_here(state) {
        return Err(format!(
            "GitHub hasn't shown the last change yet ({}): wait a moment",
            doing(&waiting.change).to_lowercase()
        ));
    }
    Ok(plan)
}

/// `remote`'s data; `None` until it's loaded; why it couldn't load.
fn loaded<'a, T>(
    remote: Option<&'a crate::state::Remote<T>>,
    name: &str,
) -> Result<Option<&'a T>, String> {
    match remote {
        Some(r) if r.data.is_some() => Ok(r.data.as_ref()),
        Some(r) if !r.loading && r.error.is_some() => Err(format!(
            "Couldn't load {name}: {}",
            r.error.as_deref().unwrap_or_default()
        )),
        _ => Ok(None),
    }
}

fn plan_here(state: &State, subject: &Subject, action: Action) -> Result<Plan, String> {
    if !subject.fits(action) {
        return Err(elsewhere(action).to_owned());
    }
    let name = subject.name();
    match subject {
        Subject::Pr(pr) => {
            let loaded = loaded(state.pr_remote(pr), &name)?;
            let Some(detail) = loaded.and_then(PrDetail::pick) else {
                return Ok(Plan::Load(subject.need()));
            };
            pr_plan(state, pr, detail, action)
        }
        Subject::Issue(repo, number) => {
            let key = DataKey::Issue(repo.clone(), *number);
            let Some(data) = loaded(state.remote(&key), &name)? else {
                return Ok(Plan::Load(subject.need()));
            };
            // A number that's a pull request is one.
            let crate::data::Data::Issue(Some(issue)) = data else {
                return Err(elsewhere(action).to_owned());
            };
            let issue_id = issue.id.clone();
            Ok(match issue.state {
                IssueState::Open if !issue.can_close => {
                    return Err(
                        "GitHub doesn't let you close it: that takes its author or triage access"
                            .into(),
                    );
                }
                IssueState::Open => Plan::Ask(
                    format!("Close {name}?"),
                    Vec::new(),
                    vec![
                        (
                            "Close as completed".into(),
                            Change::CloseIssue {
                                issue: issue_id.clone(),
                                reason: CloseReason::Completed,
                            },
                        ),
                        (
                            "Close as not planned".into(),
                            Change::CloseIssue {
                                issue: issue_id.clone(),
                                reason: CloseReason::NotPlanned,
                            },
                        ),
                        // Which one it duplicates is chosen next.
                        (
                            "Duplicate…".into(),
                            Change::Manage(crate::api::manage::Manage::CloseAsDuplicate {
                                issue: issue_id,
                                of: None,
                            }),
                        ),
                    ],
                ),
                _ if !issue.can_reopen => {
                    return Err("GitHub doesn't let you reopen it".into());
                }
                IssueState::Closed | IssueState::NotPlanned => Plan::Ask(
                    format!("Reopen {name}?"),
                    Vec::new(),
                    vec![("Reopen".into(), Change::ReopenIssue { issue: issue_id })],
                ),
            })
        }
        Subject::Run(repo, run, attempt) => {
            let key = DataKey::Run(repo.clone(), *run, *attempt);
            if loaded(state.remote(&key), &name)?.is_none() {
                return Ok(Plan::Load(subject.need()));
            }
            let run = state
                .picked::<WorkflowRun>(&key)
                .ok_or("GitHub sent something else for the run")?;
            let about = RunAbout {
                id: run.id,
                name: format!("{} #{}", run.name, run.number),
                outcome: run.outcome,
            };
            run_plan(state, repo, &about, None, action)
        }
        Subject::Job(repo, _, job) => {
            let key = DataKey::Job(repo.clone(), *job);
            if loaded(state.remote(&key), &name)?.is_none() {
                return Ok(Plan::Load(subject.need()));
            }
            let job = state
                .picked::<Job>(&key)
                .ok_or("GitHub sent something else for the job")?;
            // A job can end while its run goes on: it's the run that's
            // cancelled or re-run.
            let about = RunAbout {
                id: job.run_id,
                name: format!("{} #{}", job.run.name, job.run.number),
                outcome: job.run.outcome,
            };
            run_plan(state, repo, &about, Some(job), action)
        }
        Subject::Thread { id, query } => {
            let key = DataKey::Notifications(query.clone());
            if loaded(state.remote(&key), &name)?.is_none() {
                return Ok(Plan::Load(subject.need()));
            }
            let n = state
                .picked::<Inbox>(&key)
                .and_then(|inbox| inbox.list.items.iter().find(|n| n.id == *id))
                .ok_or("It's no longer in your inbox")?;
            match action {
                Action::MarkRead if !n.unread => Err("It's read already".into()),
                Action::MarkRead => Ok(Plan::Do(Change::MarkRead { thread: id.clone() })),
                _ => Ok(Plan::Do(Change::MarkDone {
                    thread: crate::api::notifications::ThreadAt::of(n),
                })),
            }
        }
        Subject::Inbox { query } => {
            let key = DataKey::Notifications(query.clone());
            let Some(data) = loaded(state.remote(&key), &name)? else {
                return Ok(Plan::Load(subject.need()));
            };
            let crate::data::Data::Notifications(list) = data else {
                return Err("GitHub sent something else for your notifications".into());
            };
            let fetched = (state.remote(&key)).and_then(|r| r.cached_at.or(r.asked_at));
            inbox::plan(query, list, fetched, action)
        }
        Subject::Alert(a) => Ok(match crate::security::plan(state, a)? {
            crate::security::Plan::Load(need) => Plan::Load(need),
            crate::security::Plan::Ask(title, facts, choices) => Plan::Ask(title, facts, choices),
            crate::security::Plan::Dismiss(d) => Plan::Dismiss(d),
        }),
        Subject::Discussion(of, number) => {
            match crate::manage::discussion_close(state, of, *number)? {
                Some((title, facts, choices)) => Ok(Plan::Ask(title, facts, choices)),
                None => Ok(Plan::Load(subject.need())),
            }
        }
        Subject::Project(..) | Subject::Settings(..) => Err(elsewhere(action).to_owned()),
    }
}

fn pr_plan(state: &State, pr: &PrRef, d: &PrDetail, action: Action) -> Result<Plan, String> {
    let open = d.state.is_open();
    let key = |a: Action| state.first_key(a);
    let not_open = || match d.state {
        PrState::Merged => "It's merged".to_owned(),
        _ => format!("It's closed: {} reopens it", key(Action::Close)),
    };
    let readiness = d.readiness();
    match action {
        Action::Merge if readiness == Readiness::Draft => Err(format!(
            "A draft can't be merged: {} marks it ready for review",
            key(Action::ReadyForReview)
        )),
        Action::Merge | Action::UpdateBranch | Action::Approve if !open => Err(not_open()),
        Action::Merge => merge::plan(state, pr, d),
        Action::UpdateBranch if readiness == Readiness::Conflicts => Err(format!(
            "It conflicts with {}: resolve that on GitHub",
            d.base_ref
        )),
        Action::Close => {
            let (title, label, reopen) = match d.state {
                PrState::Merged => {
                    return Err("It's merged: a merged pull request can't be reopened".into());
                }
                PrState::Open | PrState::Draft if !d.may.close => {
                    return Err(
                        "GitHub doesn't let you close it: that takes its author or triage access"
                            .into(),
                    );
                }
                PrState::Open | PrState::Draft => {
                    (format!("Close {pr}?"), "Close pull request", false)
                }
                PrState::Closed if !d.may.reopen => {
                    return Err("GitHub doesn't let you reopen it".into());
                }
                PrState::Closed => (format!("Reopen {pr}?"), "Reopen", true),
            };
            let facts = if reopen {
                Vec::new()
            } else {
                let again = format!("Its branch stays; {} reopens it", key(Action::Close));
                vec![(again, false)]
            };
            let change = Change::SetPrOpen {
                pr: d.id.clone(),
                open: reopen,
            };
            Ok(Plan::Ask(title, facts, vec![(label.into(), change)]))
        }
        Action::ReadyForReview if d.state == PrState::Draft && !d.may.edit => {
            Err("Marking it ready takes its author or write access".into())
        }
        Action::ReadyForReview if d.state == PrState::Draft => {
            Ok(Plan::Do(Change::ReadyForReview { pr: d.id.clone() }))
        }
        Action::ReadyForReview if open && !d.may.edit => {
            Err("Converting it to a draft takes its author or write access".into())
        }
        Action::ReadyForReview if open => Ok(Plan::Ask(
            format!("Convert {pr} to a draft?"),
            vec![(
                format!(
                    "It can't be merged until it's marked ready again ({})",
                    key(Action::ReadyForReview)
                ),
                false,
            )],
            vec![(
                "Convert to draft".into(),
                Change::Manage(crate::api::manage::Manage::ConvertToDraft { pr: d.id.clone() }),
            )],
        )),
        Action::ReadyForReview => Err(not_open()),
        Action::UpdateBranch if !d.may.push_head => Err(format!(
            "Updating it pushes to its branch, {}, which you can't push to",
            d.head_ref
        )),
        // GitHub only says it's behind when the repository requires it up
        // to date; the comparison says so either way.
        Action::UpdateBranch if d.behind_by == Some(0) && d.merge_state != MergeState::Behind => {
            Err(format!("It's up to date with {}", d.base_ref))
        }
        Action::UpdateBranch => {
            let update = |rebase| Change::UpdateBranch {
                pr: d.id.clone(),
                rebase,
                head: d.head_oid.clone(),
            };
            let mut choices = vec![(format!("Merge {} into it", d.base_ref), update(false))];
            if d.merge_methods.contains(&MergeMethod::Rebase) {
                choices.push((format!("Rebase it on {}", d.base_ref), update(true)));
            }
            let facts = vec![(
                format!(
                    "Its branch, {}, is {}; updating pushes to it",
                    d.head_ref,
                    behind(d)
                ),
                false,
            )];
            Ok(Plan::Ask(format!("Update {pr}'s branch?"), facts, choices))
        }
        Action::Approve if state.viewer.as_deref() == Some(d.author.as_str()) => {
            Err("You can't approve your own pull request".into())
        }
        Action::Approve => Ok(Plan::Approve(pr.clone())),
        _ => Err(elsewhere(action).to_owned()),
    }
}

/// "3 commits behind main".
fn behind(d: &PrDetail) -> String {
    match d.behind_by {
        Some(1) => format!("1 commit behind {}", d.base_ref),
        Some(n) => format!("{n} commits behind {}", d.base_ref),
        None => format!("behind {}", d.base_ref),
    }
}

/// What's worth knowing before merging (what stops it is refused before).
fn merge_facts(state: &State, d: &PrDetail) -> Vec<(String, bool)> {
    let mut facts = vec![(format!("Into {} from {}", d.base_ref, d.head_ref), false)];
    match d.checks {
        Some(ChecksState::Passing) => facts.push(("Checks passed".into(), false)),
        Some(ChecksState::Failing) => facts.push(("Some checks failed".into(), true)),
        Some(ChecksState::Pending) => facts.push(("Checks are still running".into(), true)),
        None => facts.push(("No checks".into(), false)),
    }
    match d.review {
        Some(ReviewDecision::Approved) => facts.push(("Approved".into(), false)),
        Some(ReviewDecision::ChangesRequested) => {
            facts.push(("Changes were requested".into(), true));
        }
        Some(ReviewDecision::ReviewRequired) => facts.push(("A review is required".into(), true)),
        None => {}
    }
    facts.push(match d.readiness() {
        Readiness::Checking => (
            "GitHub is still working out whether it can merge".into(),
            true,
        ),
        Readiness::Blocked if d.may.merge_as_admin => (
            "A required review or check is missing: you'd merge as an administrator".into(),
            true,
        ),
        Readiness::Blocked => ("A required review or check is missing".into(), true),
        _ if d.merge_state == MergeState::Unstable => (
            "No failed check is required, so GitHub will merge it".into(),
            false,
        ),
        _ => ("Ready to merge".into(), false),
    });
    if d.behind_by.is_some_and(|n| n > 0) {
        let update = state.first_key(Action::UpdateBranch);
        facts.push((
            format!("Its branch is {} ({update} updates it)", behind(d)),
            false,
        ));
    }
    facts
}

/// The run an action on a run's page or a job's is about.
struct RunAbout {
    id: u64,
    /// "CI #12".
    name: String,
    outcome: CheckOutcome,
}

/// Cancelling (`Close`) or re-running a run, from its page (`job: None`)
/// or one of its jobs'.
fn run_plan(
    state: &State,
    repo: &RepoId,
    run: &RunAbout,
    job: Option<&Job>,
    action: Action,
) -> Result<Plan, String> {
    if state.overview(repo).is_some_and(|o| !o.can_write) {
        return Err(format!(
            "Re-running and cancelling take write access to {repo}"
        ));
    }
    let running = run.outcome == CheckOutcome::Pending;
    let name = &run.name;
    match action {
        Action::Close if running => {
            let change = Change::CancelRun {
                repo: repo.clone(),
                run: run.id,
            };
            Ok(Plan::Ask(
                format!("Cancel {name}?"),
                vec![("Jobs still running stop where they are".into(), false)],
                vec![("Cancel the run".into(), change)],
            ))
        }
        Action::Close => Err("The run has finished: there's nothing to cancel".into()),
        _ if running => Err(format!(
            "The run is still going: {} cancels it",
            state.first_key(Action::Close)
        )),
        _ => {
            let rerun = |failed_only| Change::Rerun {
                repo: repo.clone(),
                run: run.id,
                failed_only,
            };
            let mut choices = Vec::new();
            if let Some(job) = job {
                let change = Change::RerunJob {
                    repo: repo.clone(),
                    job: job.id,
                };
                choices.push(("Re-run this job".to_owned(), change));
            }
            if matches!(run.outcome, CheckOutcome::Failure | CheckOutcome::Cancelled) {
                choices.push(("Re-run failed jobs".to_owned(), rerun(true)));
            }
            choices.push(("Re-run all jobs".to_owned(), rerun(false)));
            Ok(Plan::Ask(format!("Re-run {name}?"), Vec::new(), choices))
        }
    }
}

/// Keys in the confirmation: choose, do it, or cancel.
#[must_use]
pub fn on_confirm_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    let Some(Overlay::Confirm(confirm)) = &mut state.overlay else {
        return Vec::new();
    };
    let n = confirm.choices.len().max(1);
    match key.code {
        KeyCode::Esc => state.overlay = None,
        _ if confirm.sending => {}
        KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
            confirm.selected = (confirm.selected + 1) % n;
        }
        KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
            confirm.selected = (confirm.selected + n - 1) % n;
        }
        KeyCode::Enter => {
            if let Some((_, change)) = confirm.choices.get(confirm.selected) {
                let (change, about) = (change.clone(), confirm.about.clone());
                // A revert or a transfer answers with what it made; a
                // duplicate is chosen first.
                if let Some(cmds) = crate::manage::confirmed(state, &change, &about) {
                    return cmds;
                }
                if let Some(Overlay::Confirm(confirm)) = &mut state.overlay {
                    confirm.sending = true;
                    confirm.error = None;
                }
                return vec![send(change, By::Confirm(about))];
            }
        }
        _ => {}
    }
    Vec::new()
}

/// A change sent to GitHub, and what sent it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changing {
    pub change: Change,
    pub by: By,
}

/// Sends `change` to GitHub; its answer goes back to `by`.
pub fn send(change: Change, by: By) -> Cmd {
    Cmd::Work(Changing { change, by }.into())
}

impl work::Job for Changing {
    type Out = ();

    async fn run(&self, cx: &Cx) -> Result<(), Failure> {
        Ok(cx.gh.change(&self.change).await?)
    }

    fn done(self, state: &mut State, result: Result<(), Failure>) -> Vec<Cmd> {
        on_changed(state, &self.change, self.by, result)
    }
}

/// GitHub's answer to a change, back to what sent it. Whatever was shown
/// is fetched again (a write marks everything stale), so the page ends up
/// as GitHub has it.
#[must_use]
fn on_changed(state: &mut State, change: &Change, by: By, result: Result<(), Failure>) -> Vec<Cmd> {
    // The dialog or the composer that sent it, if it's still waiting.
    let sender = match (&by, &state.overlay) {
        (By::Confirm(_), Some(Overlay::Confirm(c))) => {
            c.sending
                && c.choices
                    .get(c.selected)
                    .is_some_and(|(_, sent)| sent == change)
        }
        (By::Compose | By::Reply(_) | By::Edit(_), Some(Overlay::Compose(c))) => c.sending,
        _ => false,
    };
    match result {
        Ok(()) => {
            if sender {
                state.overlay = None;
            }
            state.info(done(change));
            // The unread count follows, as the inbox does.
            let mut cmds = Vec::from_iter(notifies(change).then_some(Cmd::Api(Api::FetchUnread)));
            if let Change::Edit(_) = change {
                cmds.extend(crate::edit::after(state));
            }
            cmds.extend(crate::manage::after(state, change));
            cmds.extend(crate::settings::after(state, change));
            match by {
                By::Confirm(about) | By::Act(about) | By::Edit(about) => {
                    if let Some(route) = state.route().cloned() {
                        let change = change.clone();
                        crate::follow::expect(state, route, Shows { change, about });
                    }
                }
                // It shows in its thread.
                By::Reply(pr) => return vec![Cmd::Api(Api::FetchThreads(pr))],
                By::Compose | By::Star => {}
            }
            cmds.extend(state.load_visible(false));
            cmds
        }
        Err(err) => {
            tracing::warn!(?change, %err, "a change to GitHub failed");
            // GitHub couldn't be reached: it waits in the queue, if it can.
            if let Some(cmds) = crate::offline::queue_change(state, change, &by, &err, sender) {
                return cmds;
            }
            if let Change::Star { repo, starred, .. } = change {
                nav::set_starred(state, repo, !starred);
            }
            let text = failure(change, &err);
            // A form's text comes back to edit.
            if sender && crate::settings::refused(state, change, &text) {
                return Vec::new();
            }
            match &mut state.overlay {
                Some(Overlay::Confirm(confirm)) if sender => {
                    confirm.sending = false;
                    confirm.error = Some(text);
                }
                Some(Overlay::Compose(compose)) if sender => {
                    compose.sending = false;
                    compose.error = Some(text);
                }
                _ => state.error(text),
            }
            Vec::new()
        }
    }
}

/// The change the page on screen waits to see, if any.
pub fn awaiting_here(state: &State) -> Option<&Shows> {
    crate::follow::here(state)
}

impl Expectation for Shows {
    /// What the page is about, and its edits or project fields where the
    /// change shows there.
    fn needs(&self, _: &State, _: &Route) -> Vec<DataKey> {
        let edit =
            matches!(self.change, Change::Edit(_)) || crate::manage::polls_activity(&self.change);
        let more = edit.then(|| crate::edit::polls(&self.about)).flatten();
        let fields = crate::projects::sidebar::polls(&self.change, &self.about);
        std::iter::once(self.about.need())
            .chain(more.or(fields))
            .collect()
    }

    fn waiting(&self) -> &'static str {
        match &self.change {
            Change::CancelRun { .. } => "Cancel requested: waiting for GitHub to stop the run",
            Change::Rerun { .. } | Change::RerunJob { .. } => {
                "Waiting for GitHub to start the re-run"
            }
            Change::Merge { .. } => "Waiting for GitHub to show the merge",
            Change::EnableAutoMerge { .. } => "Waiting for GitHub to show auto-merge",
            Change::Enqueue { .. } => "Waiting for GitHub to show it in the merge queue",
            Change::UpdateBranch { .. } => "Waiting for GitHub to update the branch",
            _ => "Waiting for GitHub to show the change",
        }
    }

    /// On another page (a re-run) only when it was made on its own page,
    /// not a list's row.
    fn shown(&self, state: &State, route: &Route) -> Shown {
        match shown(state, &self.change, &self.about) {
            Shown::There(..) if Subject::of(route).as_ref() != Some(&self.about) => Shown::Yes,
            shown => shown,
        }
    }

    /// The rest of the page (a pull request's commits, or the list the
    /// row was on) follows too.
    fn then(&self, state: &mut State, route: &Route) -> Vec<Cmd> {
        let mut cmds = state.ensure_route(route, true);
        // GitHub may count the unread later than it marks them.
        if notifies(&self.change) {
            cmds.push(Cmd::Api(Api::FetchUnread));
        }
        cmds
    }
}

/// `change` shown on `about`'s page, to follow there.
fn there(change: &Change, about: Subject) -> Shown {
    let change = change.clone();
    Shown::There(about.route(), Box::new(Shows { change, about }))
}

/// Whether the page shows a change GitHub accepted: on another page, the
/// run's latest attempt, the job re-run.
fn shown(state: &State, change: &Change, about: &Subject) -> Shown {
    let yes = |b: bool| if b { Shown::Yes } else { Shown::No };
    let pr = |pr: &PrRef| state.pr(pr);
    match (change, about) {
        (Change::Manage(m), about) => yes(crate::manage::shows(state, m, about)),
        (Change::Settings(c), Subject::Settings(repo, page)) => {
            yes(crate::settings::shows(state, c, repo, *page))
        }
        (_, Subject::Discussion(of, number)) => {
            yes(crate::discuss::shows(state, change, of, *number))
        }
        (Change::Edit(edit), about) => yes(crate::edit::shown(state, edit, about)),
        (Change::SetProjectField { .. }, Subject::Issue(..) | Subject::Pr(_)) => {
            yes(crate::projects::sidebar::shows(state, change, about))
        }
        (_, Subject::Pr(r)) => {
            let Some(d) = pr(r) else {
                return Shown::No;
            };
            let merged = d.state == PrState::Merged;
            yes(match change {
                Change::Merge { .. } => merged,
                // Auto-merge ends once it merges, or in a queue, once
                // it's queued.
                Change::EnableAutoMerge { .. } => {
                    d.auto_merge.is_some() || d.queued.is_some() || merged
                }
                Change::DisableAutoMerge { .. } => d.auto_merge.is_none(),
                Change::Enqueue { .. } => d.queued.is_some() || merged,
                Change::Dequeue { .. } => d.queued.is_none(),
                Change::SetPrOpen { open, .. } => d.state.is_open() == *open,
                Change::ReadyForReview { .. } => d.state != PrState::Draft,
                Change::UpdateBranch { head, .. } => d.head_oid != *head,
                _ => true,
            })
        }
        (Change::CloseIssue { .. } | Change::ReopenIssue { .. }, Subject::Issue(repo, number)) => {
            let key = DataKey::Issue(repo.clone(), *number);
            let issue = state.picked::<Option<Box<crate::api::browse::IssueDetail>>>(&key);
            let Some(Some(issue)) = issue else {
                return Shown::No;
            };
            let open = issue.state == IssueState::Open;
            yes(open == matches!(change, Change::ReopenIssue { .. }))
        }
        (Change::Rerun { .. }, Subject::Run(repo, run, Some(_))) => {
            there(change, Subject::Run(repo.clone(), *run, None))
        }
        (Change::Rerun { .. } | Change::CancelRun { .. }, Subject::Run(repo, run, attempt)) => {
            let key = DataKey::Run(repo.clone(), *run, *attempt);
            let Some(r) = state.picked::<WorkflowRun>(&key) else {
                return Shown::No;
            };
            let running = r.outcome == CheckOutcome::Pending;
            yes(running == matches!(change, Change::Rerun { .. }))
        }
        (
            Change::Rerun { .. } | Change::RerunJob { .. } | Change::CancelRun { .. },
            Subject::Job(repo, run, job),
        ) => {
            let Some(j) = state.picked::<Job>(&DataKey::Job(repo.clone(), *job)) else {
                return Shown::No;
            };
            let latest = j.attempt == j.run.attempt;
            let running = j.run.outcome == CheckOutcome::Pending;
            match (change, j.run.rerun) {
                (Change::CancelRun { .. }, _) => yes(!running),
                (_, Some(rerun)) => there(change, Subject::Job(repo.clone(), *run, rerun)),
                // Following it here, or its re-run left this job out.
                (Change::Rerun { failed_only, .. }, None) => yes((latest
                    && running
                    && j.attempt > 1)
                    || (*failed_only
                        && !latest
                        && !matches!(j.outcome, CheckOutcome::Failure | CheckOutcome::Cancelled))),
                _ => yes(latest && running && j.attempt > 1),
            }
        }
        (Change::MarkRead { .. } | Change::MarkDone { .. }, Subject::Thread { query, .. }) => {
            let key = DataKey::Notifications(query.clone());
            let Some(inbox) = state.picked::<Inbox>(&key) else {
                return Shown::No;
            };
            // Read, it stays, read (or leaves a list of the unread); done,
            // it leaves.
            let row = |id: &str| inbox.list.items.iter().find(|n| n.id == id);
            yes(match change {
                Change::MarkDone { thread } => row(&thread.id).is_none(),
                Change::MarkRead { thread } => row(thread).is_none_or(|n| !n.unread),
                _ => true,
            })
        }
        (Change::MarkAllRead { .. } | Change::MarkEach { .. }, Subject::Inbox { query }) => {
            let key = DataKey::Notifications(query.clone());
            let shows = state.picked::<Inbox>(&key);
            yes(shows.is_some_and(|i| inbox::shows(i, change)))
        }
        (Change::SetProjectField { .. } | Change::Project(_), Subject::Project(project, view)) => {
            yes(crate::projects::shows(state, change, project, *view))
        }
        (Change::Alert(c), _) => yes(crate::security::shows(state, c)),
        _ => Shown::Yes,
    }
}

/// After GitHub's data arrives: runs an action that waited for what it's
/// about, and plans an open confirmation again, so it says what's true
/// now, or closes, saying why, if the change no longer applies.
#[must_use]
pub fn follow(state: &mut State) -> Vec<Cmd> {
    let mut cmds = Vec::new();
    // An action asked for before what it's about loaded.
    if let Some(p) = state.pending.clone() {
        if state.route() != p.route.as_ref() {
            state.pending = None;
        } else if !matches!(plan_for(state, &p.subject, p.action), Ok(Plan::Load(_))) {
            state.pending = None;
            cmds.extend(act_on(state, p.subject, p.action));
        }
    }
    let Some(Overlay::Confirm(confirm)) = &state.overlay else {
        return cmds;
    };
    // Another module's confirmation (deleting a comment) is its own.
    if confirm.sending || !ACTIONS.contains(&confirm.action) {
        return cmds;
    }
    let planned = plan_for(state, &confirm.about, confirm.action);
    match (planned, &mut state.overlay) {
        (Err(why), _) => {
            state.overlay = None;
            state.info(why);
        }
        (Ok(Plan::Ask(title, facts, choices)), Some(Overlay::Confirm(confirm))) => {
            confirm.selected = confirm.selected.min(choices.len().saturating_sub(1));
            (confirm.title, confirm.facts, confirm.choices) = (title, facts, choices);
        }
        _ => {}
    }
    cmds
}

/// Whether a change is to your notifications.
fn notifies(change: &Change) -> bool {
    matches!(
        change,
        Change::MarkRead { .. }
            | Change::MarkDone { .. }
            | Change::MarkAllRead { .. }
            | Change::MarkEach { .. }
    )
}

/// What a change is doing while GitHub answers: "Merging".
fn doing(change: &Change) -> &'static str {
    match change {
        Change::Comment { .. } | Change::CommitComment { .. } | Change::Reply { .. } => "Posting",
        Change::Star { starred: true, .. } => "Starring",
        Change::Star { .. } => "Unstarring",
        Change::Merge { .. } => "Merging",
        Change::EnableAutoMerge { .. } => "Enabling auto-merge",
        Change::DisableAutoMerge { .. } => "Disabling auto-merge",
        Change::Enqueue { .. } => "Adding it to the merge queue",
        Change::Dequeue { .. } => "Removing it from the merge queue",
        Change::SetPrOpen { open: true, .. } | Change::ReopenIssue { .. } => "Reopening",
        Change::SetPrOpen { .. } | Change::CloseIssue { .. } => "Closing",
        Change::ReadyForReview { .. } => "Marking it ready for review",
        Change::UpdateBranch { .. } => "Updating the branch",
        Change::Rerun { .. } | Change::RerunJob { .. } => "Asking GitHub to re-run it",
        Change::CancelRun { .. } => "Cancelling the run",
        Change::MarkRead { .. } => "Marking it read",
        Change::MarkDone { .. } => "Marking it done",
        Change::MarkAllRead { .. } | Change::MarkEach { done: false, .. } => "Marking them read",
        Change::MarkEach { .. } => "Marking them done",
        Change::Edit(edit) => crate::edit::doing(edit),
        Change::Discussion(d) => crate::discuss::doing(d),
        Change::SetProjectField { value: None, .. } => "Clearing the field",
        Change::SetProjectField { .. } => "Setting the field",
        Change::Alert(c) => crate::security::doing(c),
        Change::Project(c) => crate::projects::doing(c),
        Change::Manage(m) => crate::manage::doing(m),
        Change::Settings(c) => crate::settings::doing(c),
    }
}

/// What a change did, once GitHub said so.
fn done(change: &Change) -> String {
    match change {
        Change::Comment { .. } | Change::CommitComment { .. } => "Comment posted".into(),
        Change::Reply { .. } => "Reply posted".into(),
        Change::Star {
            repo,
            starred: true,
            ..
        } => format!("Starred {repo}"),
        Change::Star { repo, .. } => format!("Unstarred {repo}"),
        Change::Merge { .. } => "Merged".into(),
        Change::EnableAutoMerge { .. } => "Auto-merge enabled: it merges when ready".into(),
        Change::DisableAutoMerge { .. } => "Auto-merge disabled".into(),
        Change::Enqueue { .. } => "Added to the merge queue".into(),
        Change::Dequeue { .. } => "Removed from the merge queue".into(),
        Change::SetPrOpen { open: true, .. } | Change::ReopenIssue { .. } => "Reopened".into(),
        Change::SetPrOpen { .. } => "Closed".into(),
        Change::CloseIssue {
            reason: CloseReason::NotPlanned,
            ..
        } => "Closed as not planned".into(),
        Change::CloseIssue { .. } => "Closed as completed".into(),
        Change::ReadyForReview { .. } => "Ready for review".into(),
        Change::UpdateBranch { rebase: true, .. } => "Branch rebased".into(),
        Change::UpdateBranch { .. } => "Branch updated".into(),
        Change::Rerun {
            failed_only: true, ..
        } => "Re-running the failed jobs".into(),
        Change::Rerun { .. } => "Re-running the run".into(),
        Change::RerunJob { .. } => "Re-running the job".into(),
        Change::CancelRun { .. } => "Cancelling the run".into(),
        Change::MarkRead { .. } | Change::MarkAllRead { .. } => "Marked read".into(),
        Change::MarkDone { .. } => "Marked done".into(),
        Change::MarkEach { threads, done } => {
            let state = if *done { "done" } else { "read" };
            match threads.len() {
                1 => format!("Marked 1 notification {state}"),
                n => format!("Marked {n} notifications {state}"),
            }
        }
        Change::Edit(edit) => crate::edit::done(edit).into(),
        Change::Discussion(d) => crate::discuss::done(d).into(),
        Change::SetProjectField {
            value: None, said, ..
        } => format!("Cleared {said}"),
        Change::SetProjectField { said, .. } => format!("Set {said}"),
        Change::Alert(c) => crate::security::done(c),
        Change::Project(c) => crate::projects::done(c).into(),
        Change::Manage(m) => crate::manage::done(m),
        Change::Settings(c) => crate::settings::done(c),
    }
}

/// The permission a fine-grained token needs for `change`.
pub fn needs(change: &Change) -> &'static str {
    match change {
        Change::Comment { .. } | Change::Edit(_) => "Issues or Pull requests: write",
        Change::CommitComment { .. } => "Contents: write",
        Change::Star { .. } => "Starring: write",
        Change::Merge { .. }
        | Change::UpdateBranch { .. }
        | Change::EnableAutoMerge { .. }
        | Change::Enqueue { .. } => "Contents: write and Pull requests: write",
        Change::SetPrOpen { .. }
        | Change::ReadyForReview { .. }
        | Change::Reply { .. }
        | Change::DisableAutoMerge { .. }
        | Change::Dequeue { .. } => "Pull requests: write",
        Change::CloseIssue { .. } | Change::ReopenIssue { .. } => "Issues: write",
        Change::Rerun { .. } | Change::RerunJob { .. } | Change::CancelRun { .. } => {
            "Actions: write"
        }
        Change::MarkRead { .. }
        | Change::MarkDone { .. }
        | Change::MarkAllRead { .. }
        | Change::MarkEach { .. } => "Notifications (classic tokens only)",
        Change::SetProjectField { .. } | Change::Project(_) => "Projects: write",
        Change::Alert(c) => crate::security::needs(c),
        Change::Discussion(_) => "Discussions: write",
        Change::Manage(m) => crate::manage::needs(m),
        Change::Settings(c) => crate::settings::needs(c),
    }
}

/// Why a change failed, in GitHub's words, and the permission it needs
/// when GitHub says the token lacks one.
fn failure(change: &Change, err: &Failure) -> String {
    let what = match change {
        Change::Comment { .. } | Change::CommitComment { .. } => "the comment",
        Change::Reply { .. } => "the reply",
        Change::Star { starred: true, .. } => "the star",
        Change::Star { .. } => "removing the star",
        Change::Merge { .. } => "the merge",
        Change::EnableAutoMerge { .. } => "enabling auto-merge",
        Change::DisableAutoMerge { .. } => "disabling auto-merge",
        Change::Enqueue { .. } => "adding it to the merge queue",
        Change::Dequeue { .. } => "removing it from the merge queue",
        Change::SetPrOpen { open: true, .. } | Change::ReopenIssue { .. } => "reopening it",
        Change::SetPrOpen { .. } | Change::CloseIssue { .. } => "closing it",
        Change::ReadyForReview { .. } => "marking it ready for review",
        Change::UpdateBranch { .. } => "updating the branch",
        Change::Rerun { .. } | Change::RerunJob { .. } => "the re-run",
        Change::CancelRun { .. } => "cancelling the run",
        Change::MarkRead { .. } => "marking it read",
        Change::MarkDone { .. } => "marking it done",
        Change::MarkAllRead { .. } | Change::MarkEach { done: false, .. } => "marking them read",
        Change::MarkEach { .. } => "marking them done",
        Change::Edit(edit) => crate::edit::what(edit),
        Change::Discussion(d) => crate::discuss::what(d),
        Change::SetProjectField { .. } => "setting the field",
        Change::Alert(c) => crate::security::what(c),
        Change::Project(c) => crate::projects::what(c),
        Change::Manage(m) => crate::manage::what(m),
        Change::Settings(c) => crate::settings::what(c),
    };
    let reason = err.to_string();
    // GitHub refuses with 403 for other reasons too ("already running"),
    // so only its words say it's a permission.
    let denied = matches!(
        err.api(),
        Some(ApiError::Http { .. } | ApiError::GraphQl(_))
    ) && {
        let reason = reason.to_lowercase();
        ["not accessible", "permission", "must have", "forbidden"]
            .iter()
            .any(|s| reason.contains(s))
    };
    let mut text = format!("GitHub refused {what}: {reason}");
    match change {
        // Fine-grained tokens can't read notifications at all.
        _ if denied && matches!(change, Change::SetProjectField { .. } | Change::Project(_)) => text.push_str(
            ". Your token may lack the permission: a classic token needs the project scope; a fine-grained one, Projects: write",
        ),
        _ if denied && matches!(change, Change::Alert(_)) => text.push_str(&format!(
            ". Your token may lack the permission: a fine-grained token needs {}; a classic one, the security_events or repo scope",
            needs(change)
        )),
        _ if denied && notifies(change) => text.push_str(
            ". Your token may lack the permission: notifications take a classic token with the notifications or repo scope",
        ),
        _ if denied => text.push_str(&format!(
            ". Your token may lack the permission: a fine-grained token needs {}; a classic one, the repo scope",
            needs(change)
        )),
        _ => {}
    }
    text
}

/// The changes the menu offers for what's on screen: each action, its
/// label, and why it can't be made now, if it can't.
pub fn doables(state: &State) -> Vec<(Action, String, Option<String>)> {
    (ACTIONS.into_iter())
        .filter_map(|action| {
            let subject = subject(state, action).filter(|s| s.fits(action))?;
            let label = match (action, &subject) {
                (Action::Close, Subject::Run(..) | Subject::Job(..)) => "Cancel the run",
                (Action::Close, Subject::Alert(_)) => "Dismiss or reopen the alert",
                (Action::Close, Subject::Discussion(..)) => "Close or reopen the discussion",
                (Action::ReadyForReview, Subject::Pr(pr)) => match state.pr(pr) {
                    Some(d) if d.state.is_open() && d.state != PrState::Draft => "Convert to draft",
                    _ => "Mark the draft ready for review",
                },
                (Action::Close, _) => "Close or reopen",
                (Action::Merge, Subject::Pr(pr)) => {
                    state.pr(pr).map_or(action.description(), merge::label)
                }
                (action, _) => action.description(),
            };
            let why = plan_for(state, &subject, action).err();
            Some((action, label.to_owned(), why))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A refusal names a missing permission only when GitHub's words say
    /// that's what it is: it also answers 403 for a run already going.
    #[test]
    fn only_a_permission_refusal_names_the_permission() {
        let rerun = Change::RerunJob {
            repo: RepoId::new("o", "r"),
            job: 9,
        };
        let refused = |message: &str| {
            Failure::from(ApiError::Http {
                status: 403,
                message: message.into(),
            })
        };
        let busy = failure(
            &rerun,
            &refused("The workflow run containing this job is already running"),
        );
        assert_eq!(
            busy,
            "GitHub refused the re-run: The workflow run containing this job is already running"
        );
        let denied = failure(
            &rerun,
            &refused("Resource not accessible by personal access token"),
        );
        assert!(denied.contains("Actions: write"), "{denied}");
    }
}
