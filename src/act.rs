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

use crate::data::{DataKey, Picked};
use crate::follow::{Expectation, Shown};
use crate::forms::{About, Answer, Ask};
use crate::keymap::Action;
use crate::nav;
use crate::state::{Api, Cmd, Cmds, Overlay, Screen, State};
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

/// Why an issue, a pull request or a discussion can't be closed, or
/// reopened, when GitHub says so.
pub(crate) const CANT_CLOSE: &str =
    "GitHub doesn't let you close it: that takes its author or triage access";
pub(crate) const CANT_REOPEN: &str = "GitHub doesn't let you reopen it";

/// A question before an action's change ([`Ask`]): what it is, what's
/// worth knowing first, and the ways to make it.
pub fn confirm(
    action: Action,
    about: Subject,
    title: String,
    facts: Vec<(String, bool)>,
    choices: Vec<(String, Change)>,
) -> Ask {
    Ask::new(About::Act(action, about), title, facts, answers(choices))
}

/// Asking first about the one way to make a change.
pub fn ask_one(
    action: Action,
    about: Subject,
    title: String,
    facts: Vec<(String, bool)>,
    label: &str,
    change: Change,
) -> Step {
    let choices = vec![(label.to_owned(), change)];
    Step::Ask(Box::new(confirm(action, about, title, facts, choices)))
}

/// Ways to make a change, as a question's answers.
fn answers(choices: Vec<(String, Change)>) -> Vec<(String, Answer)> {
    (choices.into_iter())
        .map(|(label, change)| (label, Answer::Change(change)))
        .collect()
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

/// An action asked for on something not loaded yet (a list's row, a
/// pull request to check out): it runs once that's in, if you're still
/// on the page.
#[derive(Debug, Clone)]
pub struct Pending {
    action: Action,
    subject: Subject,
    route: Option<Route>,
    then: fn(&mut State, Subject, Action) -> Cmds,
}

/// What an action does here.
#[derive(Debug)]
pub(crate) enum Plan {
    /// Ask first: what, what's worth knowing (each with whether it's a
    /// problem), and the ways to make it.
    Ask(String, Vec<(String, bool)>, Vec<(String, Change)>),
    Do(Change),
    Approve(PrRef),
    /// It's about something that has to load first.
    Load,
    /// Choose why to dismiss a security alert.
    Dismiss(crate::security::Dismissing),
}

/// What an action on what's on screen does, once its feature has planned
/// it, or a picker's row: what [`take`] goes ahead with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Say what's being done, and send it.
    Send(Change, Subject),
    /// Ask first.
    Ask(Box<Ask>),
    /// Choose in a picker.
    Pick(crate::picker::Kind),
    /// Write it in the composer, starting with this text.
    Compose(crate::compose::Target, String),
    /// Say what's being done, and start the job.
    Start(String, work::Work),
    /// Set a form's row to this ([`crate::forms::fill`]).
    Fill(Route, crate::ui::form::Row, String),
}

/// Goes ahead with a planned step, or says why it can't.
pub fn take(state: &mut State, planned: Result<Step, String>) -> Cmds {
    match planned {
        Err(why) => state.just_say(why),
        Ok(Step::Send(change, about)) => {
            state.info(format!("{}…", doing(&change)));
            send(change, By::Act(about)).into()
        }
        Ok(Step::Ask(ask)) => {
            state.overlay = Some(Overlay::Ask(ask));
            Cmds::new()
        }
        Ok(Step::Pick(kind)) => state.open_picker(kind),
        Ok(Step::Compose(target, text)) => state.compose(target, &text),
        Ok(Step::Start(doing, job)) => {
            state.info(doing);
            work::start(job)
        }
        Ok(Step::Fill(route, row, value)) => crate::forms::fill(state, &route, &row, value),
    }
}

/// Why what's on screen can't be acted on yet: GitHub hasn't shown the
/// last change, so it's old.
pub fn settled(state: &State) -> Result<(), String> {
    match awaiting_here(state) {
        Some(waiting) => Err(format!(
            "GitHub hasn't shown the last change yet ({}): wait a moment",
            doing(&waiting.change).to_lowercase()
        )),
        None => Ok(()),
    }
}

/// Runs one of [`ACTIONS`] on what's on screen, or on the selected row
/// of a list.
pub fn act(state: &mut State, action: Action) -> Cmds {
    let Some(subject) = subject(state, action) else {
        return state.just_say(elsewhere(action));
    };
    // A row's may be an old copy (from the cache, before a change); a
    // page keeps its own fresh.
    let row = state.route().is_some_and(|r| Subject::of(r).is_none());
    let mut cmds = if row {
        state.ensure(subject.need(), false)
    } else {
        Cmds::new()
    };
    cmds.extend(act_on(state, subject, action));
    cmds
}

fn act_on(state: &mut State, subject: Subject, action: Action) -> Cmds {
    let step = match plan_for(state, &subject, action) {
        Err(why) => Err(why),
        Ok(Plan::Ask(title, facts, choices)) => Ok(Step::Ask(Box::new(confirm(
            action, subject, title, facts, choices,
        )))),
        Ok(Plan::Do(change)) => Ok(Step::Send(change, subject)),
        Ok(Plan::Dismiss(d)) => Ok(Step::Pick(crate::picker::Kind::Alert(d))),
        Ok(Plan::Approve(pr)) => return crate::review::open_approve(state, pr),
        Ok(Plan::Load) => return wait(state, action, subject, act_on),
    };
    take(state, step)
}

/// Loads what `action` is about, then does it (`then`), if you're still
/// on the page: with what's loaded, or saying why it couldn't be.
pub fn wait(
    state: &mut State,
    action: Action,
    subject: Subject,
    then: fn(&mut State, Subject, Action) -> Cmds,
) -> Cmds {
    state.info(format!("Loading {}…", subject.name()));
    let need = subject.need();
    state.pending = Some(Pending {
        action,
        subject,
        route: state.route().cloned(),
        then,
    });
    state.ensure(need, false)
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
    /// What's on screen: a page about one thing, or a pull request's
    /// files.
    pub(crate) fn on_screen(state: &State) -> Option<Self> {
        match state.screen() {
            Screen::Diff(d) => d.of.pr().cloned().map(Subject::Pr),
            Screen::Page(p) => Subject::of(&p.route),
        }
    }

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
            Subject::Issue(repo, number) => Route::issue(repo.clone(), *number),
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
    subject_or(state, action, || here(state))
}

/// [`subject`], with what's on screen or its row (`here`) asked only for
/// the actions it's for.
fn subject_or(
    state: &State,
    action: Action,
    here: impl FnOnce() -> Option<Subject>,
) -> Option<Subject> {
    match action {
        Action::MarkRead | Action::MarkDone => thread(state),
        Action::MarkAllRead | Action::MarkAllDone => match state.route()? {
            Route::Search(search::Notifications { query }) => Some(Subject::Inbox {
                query: query.clone(),
            }),
            _ => None,
        },
        _ => here(),
    }
}

/// The page's own subject, or the selected row's.
fn here(state: &State) -> Option<Subject> {
    Subject::on_screen(state).or_else(|| {
        let p = state.screen().page()?;
        let url = p.selected_link()?.url()?;
        match crate::route::Dest::from_url(url).target {
            crate::route::Target::Page(route) => Subject::of(&route),
            crate::route::Target::Files(of) => of.pr().cloned().map(Subject::Pr),
            crate::route::Target::External(_) => None,
        }
    })
}

/// The issue `number` in `repo`, loaded.
pub(crate) fn issue<'a>(
    state: &'a State,
    repo: &RepoId,
    number: u64,
) -> Option<&'a crate::api::browse::IssueDetail> {
    let key = DataKey::Issue(repo.clone(), number);
    state
        .picked::<Option<Box<crate::api::browse::IssueDetail>>>(&key)?
        .as_deref()
}

/// The selected notification's thread: the inbox's rows are its
/// notifications, in order.
fn thread(state: &State) -> Option<Subject> {
    let p = state.screen().page()?;
    let Route::Search(search::Notifications { query }) = &p.route else { return None };
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
    settled(state)?;
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
            let Some(detail) = loaded.and_then(PrDetail::pick) else { return Ok(Plan::Load) };
            pr_plan(state, pr, detail, action)
        }
        Subject::Issue(repo, number) => {
            let key = DataKey::Issue(repo.clone(), *number);
            let Some(data) = loaded(state.remote(&key), &name)? else { return Ok(Plan::Load) };
            // A number that's a pull request is one.
            let crate::data::Data::Issue(Some(issue)) = data else {
                return Err(elsewhere(action).to_owned());
            };
            let id = || issue.id.clone();
            let close = |reason| Change::CloseIssue {
                issue: id(),
                reason,
            };
            Ok(match issue.state {
                IssueState::Open if !issue.can_close => return Err(CANT_CLOSE.into()),
                IssueState::Open => Plan::Ask(
                    format!("Close {name}?"),
                    Vec::new(),
                    vec![
                        ("Close as completed".into(), close(CloseReason::Completed)),
                        (
                            "Close as not planned".into(),
                            close(CloseReason::NotPlanned),
                        ),
                        // Which one it duplicates is chosen next.
                        (
                            "Duplicate…".into(),
                            Change::Manage(crate::api::manage::Manage::CloseAsDuplicate {
                                issue: id(),
                                of: None,
                            }),
                        ),
                    ],
                ),
                _ if !issue.can_reopen => return Err(CANT_REOPEN.into()),
                IssueState::Closed | IssueState::NotPlanned => Plan::Ask(
                    format!("Reopen {name}?"),
                    Vec::new(),
                    vec![("Reopen".into(), Change::ReopenIssue { issue: id() })],
                ),
            })
        }
        Subject::Run(repo, run, attempt) => {
            let key = DataKey::Run(repo.clone(), *run, *attempt);
            if loaded(state.remote(&key), &name)?.is_none() {
                return Ok(Plan::Load);
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
                return Ok(Plan::Load);
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
                return Ok(Plan::Load);
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
            let Some(data) = loaded(state.remote(&key), &name)? else { return Ok(Plan::Load) };
            let crate::data::Data::Notifications(list) = data else {
                return Err("GitHub sent something else for your notifications".into());
            };
            let fetched = (state.remote(&key)).and_then(|r| r.cached_at.or(r.asked_at));
            inbox::plan(query, list, fetched, action)
        }
        Subject::Alert(a) => crate::security::plan(state, a),
        Subject::Discussion(of, number) => {
            loaded(state.remote(&subject.need()), &name)?;
            crate::manage::discussion_close(state, of, *number)
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
                PrState::Open | PrState::Draft if !d.may.close => return Err(CANT_CLOSE.into()),
                PrState::Open | PrState::Draft => {
                    (format!("Close {pr}?"), "Close pull request", false)
                }
                PrState::Closed if !d.may.reopen => return Err(CANT_REOPEN.into()),
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
    facts.push(match d.checks {
        Some(ChecksState::Passing) => ("Checks passed".into(), false),
        Some(ChecksState::Failing) => ("Some checks failed".into(), true),
        Some(ChecksState::Pending) => ("Checks are still running".into(), true),
        None => ("No checks".into(), false),
    });
    facts.extend(d.review.map(|review| match review {
        ReviewDecision::Approved => ("Approved".into(), false),
        ReviewDecision::ChangesRequested => ("Changes were requested".into(), true),
        ReviewDecision::ReviewRequired => ("A review is required".into(), true),
    }));
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

/// The change chosen in a question about what an action changes.
pub fn confirmed(state: &mut State, change: Change, about: Subject) -> Cmds {
    // A revert or a transfer answers with what it made; a duplicate is
    // chosen first.
    if let Some(cmds) = crate::manage::confirmed(state, &change, &about) {
        return cmds;
    }
    crate::forms::sending(state);
    send(change, By::Confirm(about)).into()
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

    fn done(self, state: &mut State, result: Result<(), Failure>) -> Cmds {
        on_changed(state, &self.change, self.by, result)
    }
}

/// GitHub's answer to a change, back to what sent it. Whatever was shown
/// is fetched again (a write marks everything stale), so the page ends up
/// as GitHub has it.
fn on_changed(state: &mut State, change: &Change, by: By, result: Result<(), Failure>) -> Cmds {
    // The dialog or the composer that sent it, if it's still waiting.
    let sender = match (&by, &state.overlay) {
        (By::Confirm(_), Some(Overlay::Ask(c))) => {
            c.sent.sending
                && (c.choices.get(c.selected))
                    .is_some_and(|(_, sent)| matches!(sent, Answer::Change(c) if c == change))
        }
        (By::Compose | By::Reply(_) | By::Edit(_), Some(Overlay::Compose(c))) => c.sent.sending,
        _ => false,
    };
    match result {
        Ok(()) => {
            if sender {
                state.overlay = None;
            }
            state.info(done(change));
            // The unread count follows, as the inbox does.
            let mut cmds = Cmds::new();
            if notifies(change) {
                cmds.extend(state.ensure(DataKey::Unread, true));
            }
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
                By::Reply(pr) => return Cmd::Api(Api::FetchThreads(pr)).into(),
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
            // What was written comes back to edit.
            let mut cmds = Cmds::new();
            if sender
                && let Some(Overlay::Ask(confirm)) = &mut state.overlay
                && let Some((target, written)) = confirm.back.take()
            {
                cmds = state.compose(target, &written);
            }
            if sender {
                cmds.extend(crate::forms::say(state, text));
            } else {
                state.error(text);
            }
            cmds
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
    fn then(&self, state: &mut State, route: &Route) -> Cmds {
        let mut cmds = state.ensure_route(route, true);
        // GitHub may count the unread later than it marks them.
        if notifies(&self.change) {
            cmds.extend(state.ensure(DataKey::Unread, true));
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
            let Some(d) = pr(r) else { return Shown::No };
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
            let Some(issue) = issue(state, repo, *number) else { return Shown::No };
            let open = issue.state == IssueState::Open;
            yes(open == matches!(change, Change::ReopenIssue { .. }))
        }
        (Change::Rerun { .. }, Subject::Run(repo, run, Some(_))) => {
            there(change, Subject::Run(repo.clone(), *run, None))
        }
        (Change::Rerun { .. } | Change::CancelRun { .. }, Subject::Run(repo, run, attempt)) => {
            let key = DataKey::Run(repo.clone(), *run, *attempt);
            let Some(r) = state.picked::<WorkflowRun>(&key) else { return Shown::No };
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
            let Some(inbox) = state.picked::<Inbox>(&key) else { return Shown::No };
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
pub fn follow(state: &mut State) -> Cmds {
    let mut cmds = Cmds::new();
    // An action asked for before what it's about loaded.
    if let Some(p) = state.pending.clone() {
        if state.route() != p.route.as_ref() {
            state.pending = None;
        } else if !matches!(loaded(state.remote(&p.subject.need()), ""), Ok(None)) {
            state.pending = None;
            cmds.extend((p.then)(state, p.subject, p.action));
        }
    }
    let Some(Overlay::Ask(ask)) = &state.overlay else { return cmds };
    // Another module's question (deleting a comment, a form's) is its own.
    let About::Act(action, about) = &ask.about else { return cmds };
    if ask.sent.sending || !ACTIONS.contains(action) {
        return cmds;
    }
    let planned = plan_for(state, about, *action);
    match (planned, &mut state.overlay) {
        (Err(why), _) => {
            state.overlay = None;
            state.info(why);
        }
        (Ok(Plan::Ask(title, facts, choices)), Some(Overlay::Ask(ask))) => {
            ask.selected = ask.selected.min(choices.len().saturating_sub(1));
            (ask.title, ask.facts, ask.choices) = (title, facts, answers(choices));
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
pub(crate) fn doing(change: &Change) -> std::borrow::Cow<'static, str> {
    match change {
        Change::SetProjectField { said, value, .. } => {
            let doing = if value.is_some() {
                "Setting"
            } else {
                "Clearing"
            };
            format!("{doing} {said}").into()
        }
        _ => said(change).0.into(),
    }
}

/// What a change is doing while GitHub answers ("Merging"), and what it
/// is, for "GitHub refused …" ("the merge").
fn said(change: &Change) -> (&'static str, &'static str) {
    match change {
        Change::Comment { .. } | Change::CommitComment { .. } => ("Posting", "the comment"),
        Change::Reply { .. } => ("Posting", "the reply"),
        Change::Star { starred: true, .. } => ("Starring", "the star"),
        Change::Star { .. } => ("Unstarring", "removing the star"),
        Change::Merge { .. } => ("Merging", "the merge"),
        Change::EnableAutoMerge { .. } => ("Enabling auto-merge", "enabling auto-merge"),
        Change::DisableAutoMerge { .. } => ("Disabling auto-merge", "disabling auto-merge"),
        Change::Enqueue { .. } => (
            "Adding it to the merge queue",
            "adding it to the merge queue",
        ),
        Change::Dequeue { .. } => (
            "Removing it from the merge queue",
            "removing it from the merge queue",
        ),
        Change::SetPrOpen { open: true, .. } | Change::ReopenIssue { .. } => {
            ("Reopening", "reopening it")
        }
        Change::SetPrOpen { .. } | Change::CloseIssue { .. } => ("Closing", "closing it"),
        Change::ReadyForReview { .. } => {
            ("Marking it ready for review", "marking it ready for review")
        }
        Change::UpdateBranch { .. } => ("Updating the branch", "updating the branch"),
        Change::Rerun { .. } | Change::RerunJob { .. } => {
            ("Asking GitHub to re-run it", "the re-run")
        }
        Change::CancelRun { .. } => ("Cancelling the run", "cancelling the run"),
        Change::MarkRead { .. } => ("Marking it read", "marking it read"),
        Change::MarkDone { .. } => ("Marking it done", "marking it done"),
        Change::MarkAllRead { .. } | Change::MarkEach { done: false, .. } => {
            ("Marking them read", "marking them read")
        }
        Change::MarkEach { .. } => ("Marking them done", "marking them done"),
        // Its progress names the field ([`doing`]).
        Change::SetProjectField { .. } => ("Setting the field", "setting the field"),
        Change::Edit(edit) => crate::edit::said(edit),
        Change::Discussion(d) => (crate::discuss::doing(d), crate::discuss::what(d)),
        Change::Alert(c) => (crate::security::doing(c), crate::security::what(c)),
        Change::Project(c) => (crate::projects::doing(c), crate::projects::what(c)),
        Change::Manage(m) => crate::manage::said(m),
        Change::Settings(c) => crate::settings::said(c),
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
    let what = said(change).1;
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

/// What issue and pull request actions add: their own actions, wherever
/// what they're about is.
pub const FEATURE: crate::features::Feature = crate::features::Feature {
    owns: &ACTIONS,
    act,
    offers,
    follow,
    ..crate::features::Feature::NONE
};

/// The changes the menu offers for what's on screen, and why one can't be
/// made now, if it can't.
fn offers(state: &State, out: &mut Vec<crate::nav::Doable>) {
    // What's on screen, or its row, found once for all of them.
    let here = here(state);
    let rows = (ACTIONS.into_iter()).filter_map(|action| {
        let subject = subject_or(state, action, || here.clone()).filter(|s| s.fits(action))?;
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
        let short = match action {
            Action::Merge => "merge",
            Action::Approve => "approve",
            Action::Rerun => "re-run",
            Action::MarkRead => "read",
            Action::MarkDone => "done",
            _ => "",
        };
        let why = plan_for(state, &subject, action).err();
        Some(crate::nav::row("Change", action, label, short, why))
    });
    out.extend(rows);
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
