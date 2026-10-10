//! Changing GitHub from a page: merging, closing and reopening, marking a
//! draft ready, updating a branch, approving, re-running and cancelling
//! runs. What an action does where you are is decided once, by [`plan`]:
//! the key, the menu and the command palette all ask it, so they can't
//! disagree, and it asks GitHub's own word on what you may do. Every
//! change goes to GitHub as a [`Change`] and comes back as
//! [`Msg::Changed`], handled by [`on_changed`]; the page then follows
//! GitHub ([`follow`]) until it shows the change.

use crossterm::event::{KeyCode, KeyEvent};
use ghtui_api::ApiError;
use ghtui_api::browse::{
    CheckOutcome, IssueDetail, IssueState, IssueSummary, Job, SearchResults, WorkflowRun,
};
use ghtui_api::change::{Change, CloseReason, MergeMethod};
use ghtui_api::model::{
    ChecksState, MergeState, PrDetail, PrRef, Readiness, RepoId, ReviewDecision,
};

use crate::browse::{Data, DataKey, Need};
use crate::keymap::Action;
use crate::nav;
use crate::route::Route;
use crate::state::{Api, Cmd, Overlay, Screen, State};

/// The actions that change GitHub, as the menu lists them.
pub const ACTIONS: [Action; 6] = [
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
}

/// A change GitHub accepted that the page it was made on doesn't show
/// yet: GitHub can take a while (a cancel, a minute or more). While that
/// page is on screen, what it's about is fetched again until it shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Awaiting {
    pub change: Change,
    /// The page it was made on.
    pub route: Route,
    /// What it changed: the page's own subject, or a row's on a list.
    pub about: Subject,
    polls: u32,
}

/// An action asked for on something not loaded yet (a list's row): it
/// runs once that's in, if you're still on the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub action: Action,
    pub subject: Subject,
    pub route: Option<Route>,
}

/// Rows of a list changed from it. A list that filters by state (open
/// pull requests) loses a row you close as soon as it's fetched again, and
/// the next row slides under the cursor; so the rows you changed stay where
/// they were, showing what their own pages say now, until you refresh or
/// leave the list. Home's sections and searches are lists alike.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kept {
    /// The list.
    pub route: Route,
    subjects: Vec<Subject>,
}

/// How many times a page is fetched again for a change before giving up:
/// a few minutes, at the live refresh's pace.
const POLLS: u32 = 40;

/// What an action does here.
#[derive(Debug)]
enum Plan {
    /// Ask first: what, what's worth knowing (each with whether it's a
    /// problem), and the ways to make it.
    Ask(String, Vec<(String, bool)>, Vec<(String, Change)>),
    Do(Change),
    Approve(PrRef),
    /// It's about something that has to load first.
    Load(Need),
}

/// Runs one of [`ACTIONS`] on what's on screen, or on the selected row
/// of a list.
#[must_use]
pub fn act(state: &mut State, action: Action) -> Vec<Cmd> {
    let Some(subject) = subject(state) else {
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
            vec![Cmd::Api(Api::Change(change, By::Act(subject)))]
        }
        Ok(Plan::Approve(pr)) => crate::review::open_approve(state, pr),
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

/// What an action changes: a pull request, an issue, a run or a job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subject {
    Pr(PrRef),
    Issue(RepoId, u64),
    Run(RepoId, u64, Option<u64>),
    /// A job, with its run when its page has it.
    Job(RepoId, Option<u64>, u64),
}

impl Subject {
    /// The subject of a page about one thing.
    fn of(route: &Route) -> Option<Self> {
        Some(match route {
            Route::Pr { pr, .. } => Subject::Pr(pr.clone()),
            Route::Issue { repo, number } => Subject::Issue(repo.clone(), *number),
            Route::WorkflowRun { repo, run, attempt } => Subject::Run(repo.clone(), *run, *attempt),
            Route::Job { repo, run, job, .. } => Subject::Job(repo.clone(), *run, *job),
            _ => return None,
        })
    }

    /// Its page.
    fn route(&self) -> Route {
        match self {
            Subject::Pr(pr) => Route::pr(pr.clone()),
            Subject::Issue(repo, number) => Route::Issue {
                repo: repo.clone(),
                number: *number,
            },
            Subject::Run(repo, run, attempt) => Route::WorkflowRun {
                repo: repo.clone(),
                run: *run,
                attempt: *attempt,
            },
            Subject::Job(repo, run, job) => Route::Job {
                repo: repo.clone(),
                run: *run,
                job: *job,
                step: None,
                query: String::new(),
            },
        }
    }

    /// What planning for it reads.
    fn need(&self) -> Need {
        match self {
            Subject::Pr(pr) => Need::Pr(pr.clone()),
            Subject::Issue(repo, number) => Need::Data(DataKey::Issue(repo.clone(), *number)),
            Subject::Run(repo, run, attempt) => {
                Need::Data(DataKey::Run(repo.clone(), *run, *attempt))
            }
            Subject::Job(repo, _, job) => Need::Data(DataKey::Job(repo.clone(), *job)),
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
            Subject::Issue(..) => action == A::Close,
            Subject::Run(..) | Subject::Job(..) => matches!(action, A::Rerun | A::Close),
        }
    }

    fn name(&self) -> String {
        match self {
            Subject::Pr(pr) => pr.to_string(),
            Subject::Issue(repo, number) => format!("{repo}#{number}"),
            Subject::Run(..) => "the run".to_owned(),
            Subject::Job(..) => "the job".to_owned(),
        }
    }
}

/// What an action would change here: the page's own subject (a pull
/// request, its files, an issue, a run, a job), or on a list, the
/// selected row's.
fn subject(state: &State) -> Option<Subject> {
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

/// Where `action` works, for when it doesn't here.
fn elsewhere(action: Action) -> &'static str {
    match action {
        Action::Close => {
            "Closing works on an issue, a pull request or a running workflow run (or its row)"
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
            let Some(detail) = loaded(state.prs.get(pr), &name)? else {
                return Ok(Plan::Load(subject.need()));
            };
            pr_plan(state, pr, detail, action)
        }
        Subject::Issue(repo, number) => {
            let key = DataKey::Issue(repo.clone(), *number);
            let Some(data) = loaded(state.data.get(&key), &name)? else {
                return Ok(Plan::Load(subject.need()));
            };
            // A number that's a pull request is one.
            let crate::browse::Data::Issue(Some(issue)) = data else {
                return Err(elsewhere(action).to_owned());
            };
            let issue_id = issue.id.clone();
            Ok(match issue.state {
                IssueState::Open | IssueState::Draft if !issue.can_close => {
                    return Err(
                        "GitHub doesn't let you close it: that takes its author or triage access"
                            .into(),
                    );
                }
                IssueState::Open | IssueState::Draft => Plan::Ask(
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
                                issue: issue_id,
                                reason: CloseReason::NotPlanned,
                            },
                        ),
                    ],
                ),
                _ if !issue.can_reopen => {
                    return Err("GitHub doesn't let you reopen it".into());
                }
                IssueState::Closed | IssueState::NotPlanned | IssueState::Merged => Plan::Ask(
                    format!("Reopen {name}?"),
                    Vec::new(),
                    vec![("Reopen".into(), Change::ReopenIssue { issue: issue_id })],
                ),
            })
        }
        Subject::Run(repo, run, attempt) => {
            let key = DataKey::Run(repo.clone(), *run, *attempt);
            if loaded(state.data.get(&key), &name)?.is_none() {
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
            if loaded(state.data.get(&key), &name)?.is_none() {
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
    }
}

fn pr_plan(state: &State, pr: &PrRef, d: &PrDetail, action: Action) -> Result<Plan, String> {
    let open = matches!(d.summary.state, IssueState::Open | IssueState::Draft);
    let key = |a: Action| state.first_key(a);
    let not_open = || match d.summary.state {
        IssueState::Merged => "It's merged".to_owned(),
        _ => format!("It's closed: {} reopens it", key(Action::Close)),
    };
    let readiness = d.readiness();
    match action {
        Action::Merge if readiness == Readiness::Draft => Err(format!(
            "A draft can't be merged: {} marks it ready for review",
            key(Action::ReadyForReview)
        )),
        Action::Merge | Action::UpdateBranch | Action::Approve if !open => Err(not_open()),
        Action::Merge if !d.may.merge => Err(format!("Merging takes write access to {}", pr.repo)),
        Action::Merge if d.merge_methods.is_empty() => {
            Err("The repository allows no way of merging".into())
        }
        Action::Merge | Action::UpdateBranch if readiness == Readiness::Conflicts => Err(format!(
            "It conflicts with {}: resolve that on GitHub",
            d.base_ref
        )),
        Action::Merge if readiness == Readiness::Behind => Err(format!(
            "{} requires its branch up to date with {}: {} updates it",
            pr.repo,
            d.base_ref,
            key(Action::UpdateBranch)
        )),
        Action::Merge if readiness == Readiness::Blocked && !d.may.merge_as_admin => {
            Err("GitHub says it's blocked: a required review or check is missing".into())
        }
        Action::Merge => {
            let choices = d
                .merge_methods
                .iter()
                .map(|&method| {
                    let change = Change::Merge {
                        pr: d.id.clone(),
                        method,
                        head: d.head_oid.clone(),
                    };
                    (method.describe().to_owned(), change)
                })
                .collect();
            let title = format!("Merge {pr}: {}?", d.summary.title);
            Ok(Plan::Ask(title, merge_facts(state, d), choices))
        }
        Action::Close => {
            let (title, label, reopen) = match d.summary.state {
                IssueState::Merged => {
                    return Err("It's merged: a merged pull request can't be reopened".into());
                }
                IssueState::Open | IssueState::Draft if !d.may.close => {
                    return Err(
                        "GitHub doesn't let you close it: that takes its author or triage access"
                            .into(),
                    );
                }
                IssueState::Open | IssueState::Draft => {
                    (format!("Close {pr}?"), "Close pull request", false)
                }
                IssueState::Closed | IssueState::NotPlanned if !d.may.reopen => {
                    return Err("GitHub doesn't let you reopen it".into());
                }
                IssueState::Closed | IssueState::NotPlanned => {
                    (format!("Reopen {pr}?"), "Reopen", true)
                }
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
        Action::ReadyForReview if d.summary.state == IssueState::Draft && !d.may.edit => {
            Err("Marking it ready takes its author or write access".into())
        }
        Action::ReadyForReview if d.summary.state == IssueState::Draft => {
            Ok(Plan::Do(Change::ReadyForReview { pr: d.id.clone() }))
        }
        Action::ReadyForReview if open => Err("It isn't a draft".into()),
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
        Action::Approve if state.viewer.as_deref() == Some(d.summary.author.as_str()) => {
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
    match d.summary.checks {
        Some(ChecksState::Passing) => facts.push(("Checks passed".into(), false)),
        Some(ChecksState::Failing) => facts.push(("Some checks failed".into(), true)),
        Some(ChecksState::Pending) => facts.push(("Checks are still running".into(), true)),
        None => facts.push(("No checks".into(), false)),
    }
    match d.summary.review {
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
        Readiness::Blocked => (
            "A required review or check is missing: you'd merge as an administrator".into(),
            true,
        ),
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
                let by = By::Confirm(confirm.about.clone());
                let cmd = Cmd::Api(Api::Change(change.clone(), by));
                confirm.sending = true;
                confirm.error = None;
                return vec![cmd];
            }
        }
        _ => {}
    }
    Vec::new()
}

/// GitHub's answer to a change, back to what sent it. Whatever was shown
/// is fetched again (a write marks everything stale), so the page ends up
/// as GitHub has it.
#[must_use]
pub fn on_changed(
    state: &mut State,
    change: &Change,
    by: By,
    result: Result<(), ApiError>,
) -> Vec<Cmd> {
    // The dialog or the composer that sent it, if it's still waiting.
    let sender = match (&by, &state.overlay) {
        (By::Confirm(_), Some(Overlay::Confirm(c))) => {
            c.sending
                && c.choices
                    .get(c.selected)
                    .is_some_and(|(_, sent)| sent == change)
        }
        (By::Compose | By::Reply(_), Some(Overlay::Compose(c))) => c.sending,
        _ => false,
    };
    match result {
        Ok(()) => {
            if sender {
                state.overlay = None;
            }
            state.info(done(change));
            match by {
                By::Confirm(about) | By::Act(about) => {
                    keep_row(state, &about);
                    state.awaiting = state.route().map(|route| Awaiting {
                        change: change.clone(),
                        route: route.clone(),
                        about,
                        polls: 0,
                    });
                }
                // It shows in its thread.
                By::Reply(pr) => return vec![Cmd::Api(Api::FetchThreads(pr))],
                By::Compose | By::Star => {}
            }
            state.load_visible(false)
        }
        Err(err) => {
            tracing::warn!(?change, %err, "a change to GitHub failed");
            if let Change::Star { repo, starred, .. } = change {
                nav::set_starred(state, repo, !starred);
            }
            let text = failure(change, &err);
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

/// Keeps `about` on the list on screen, if it's a row there.
fn keep_row(state: &mut State, about: &Subject) {
    let Some(route) = state.route().filter(|r| Subject::of(r).is_none()).cloned() else {
        return;
    };
    let mut kept = match state.kept.take() {
        Some(k) if k.route == route => k,
        _ => Kept {
            route,
            subjects: Vec::new(),
        },
    };
    if !kept.subjects.contains(about) {
        kept.subjects.push(about.clone());
    }
    state.kept = Some(kept);
}

/// `data`, just fetched for `key`, with the rows kept on the list on
/// screen put back where they were if GitHub left them out, and each kept
/// row in the state its own page has.
pub fn keep(state: &State, key: &DataKey, mut data: Data) -> Data {
    let Some(kept) = (state.kept.as_ref()).filter(|k| state.route() == Some(&k.route)) else {
        return data;
    };
    let Data::Search(found) = &mut data else {
        return data;
    };
    let SearchResults::Issues(rows) = &mut **found else {
        return data;
    };
    if !state.needs(&kept.route).contains(&Need::Data(key.clone())) {
        return data;
    }
    let old = match state.picked::<SearchResults>(key) {
        Some(SearchResults::Issues(old)) => old.items.as_slice(),
        _ => &[],
    };
    for subject in &kept.subjects {
        let (repo, number, now) = match subject {
            Subject::Pr(pr) => {
                let d = state.prs.get(pr).and_then(|r| r.data.as_ref());
                (&pr.repo, pr.number, d.map(|d| d.summary.state))
            }
            Subject::Issue(repo, number) => {
                let key = DataKey::Issue(repo.clone(), *number);
                let issue = state.picked::<Option<Box<IssueDetail>>>(&key);
                (repo, *number, issue.and_then(|i| i.as_ref()).map(|i| i.state))
            }
            Subject::Run(..) | Subject::Job(..) => continue,
        };
        let is = |i: &IssueSummary| i.repo == *repo && i.number == number;
        if !rows.items.iter().any(is)
            && let Some((at, row)) = old.iter().enumerate().find(|(_, i)| is(i))
        {
            rows.items.insert(at.min(rows.items.len()), row.clone());
        }
        if let Some(now) = now {
            rows.items.iter_mut().filter(|i| is(i)).for_each(|i| i.state = now);
        }
    }
    data
}

/// The change the page on screen waits to see, if any.
pub fn awaiting_here(state: &State) -> Option<&Awaiting> {
    (state.awaiting.as_ref()).filter(|w| state.route() == Some(&w.route))
}

/// What the status bar says while GitHub hasn't shown a change.
pub fn waiting(state: &State) -> Option<String> {
    let what = match &awaiting_here(state)?.change {
        Change::CancelRun { .. } => "Cancel requested: waiting for GitHub to stop the run",
        Change::Rerun { .. } | Change::RerunJob { .. } => "Waiting for GitHub to start the re-run",
        Change::Merge { .. } => "Waiting for GitHub to show the merge",
        Change::UpdateBranch { .. } => "Waiting for GitHub to update the branch",
        _ => "Waiting for GitHub to show the change",
    };
    Some(what.to_owned())
}

/// What the live refresh fetches again for the change the page waits to
/// see: what the page is about. Past [`POLLS`], it stops waiting.
pub fn poll(state: &mut State) -> Vec<Need> {
    let Some(need) = awaiting_here(state).map(|w| w.about.need()) else {
        return Vec::new();
    };
    if let Some(w) = &mut state.awaiting {
        w.polls += 1;
        if w.polls > POLLS {
            state.awaiting = None;
            let key = state.first_key(Action::Refresh);
            state.info(format!(
                "GitHub doesn't show the change yet: {key} fetches the page again"
            ));
            return Vec::new();
        }
    }
    vec![need]
}

/// Whether the page shows a change GitHub accepted.
enum Shown {
    No,
    Yes,
    /// On another page: the run's latest attempt, the job re-run.
    There(Subject),
}

fn shown(state: &State, change: &Change, about: &Subject) -> Shown {
    let yes = |b: bool| if b { Shown::Yes } else { Shown::No };
    let pr = |pr: &PrRef| state.prs.get(pr).and_then(|r| r.data.as_ref());
    match (change, about) {
        (_, Subject::Pr(r)) => {
            let Some(d) = pr(r) else {
                return Shown::No;
            };
            let open = matches!(d.summary.state, IssueState::Open | IssueState::Draft);
            yes(match change {
                Change::Merge { .. } => d.summary.state == IssueState::Merged,
                Change::SetPrOpen { open: o, .. } => open == *o,
                Change::ReadyForReview { .. } => d.summary.state != IssueState::Draft,
                Change::UpdateBranch { head, .. } => d.head_oid != *head,
                _ => true,
            })
        }
        (Change::CloseIssue { .. } | Change::ReopenIssue { .. }, Subject::Issue(repo, number)) => {
            let key = DataKey::Issue(repo.clone(), *number);
            let issue = state.picked::<Option<Box<ghtui_api::browse::IssueDetail>>>(&key);
            let Some(Some(issue)) = issue else {
                return Shown::No;
            };
            let open = matches!(issue.state, IssueState::Open | IssueState::Draft);
            yes(open == matches!(change, Change::ReopenIssue { .. }))
        }
        (Change::Rerun { .. }, Subject::Run(repo, run, Some(_))) => {
            Shown::There(Subject::Run(repo.clone(), *run, None))
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
                (_, Some(rerun)) => Shown::There(Subject::Job(repo.clone(), *run, rerun)),
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
        _ => Shown::Yes,
    }
}

/// After GitHub's data arrives: stops waiting for a change the page now
/// shows (going to the re-run, if that's where it is), and plans an open
/// confirmation again, so it says what's true now, or closes, saying why,
/// if the change no longer applies.
#[must_use]
pub fn follow(state: &mut State) -> Vec<Cmd> {
    let mut cmds = Vec::new();
    if let Some(w) = awaiting_here(state).cloned() {
        match shown(state, &w.change, &w.about) {
            Shown::No => {}
            Shown::There(about) if Subject::of(&w.route).as_ref() == Some(&w.about) => {
                let route = about.route();
                state.awaiting = Some(Awaiting {
                    route: route.clone(),
                    about,
                    ..w
                });
                cmds.extend(state.replace(route, false, None));
            }
            // The rest of the page (a pull request's commits, or the list
            // the row was on) follows too.
            Shown::Yes | Shown::There(_) => {
                state.awaiting = None;
                cmds.extend(state.ensure_route(&w.route, true));
            }
        }
    }
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
    if confirm.sending {
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

/// What a change is doing while GitHub answers: "Merging".
fn doing(change: &Change) -> &'static str {
    match change {
        Change::Comment { .. } | Change::Reply { .. } => "Posting",
        Change::Star { starred: true, .. } => "Starring",
        Change::Star { .. } => "Unstarring",
        Change::Merge { .. } => "Merging",
        Change::SetPrOpen { open: true, .. } | Change::ReopenIssue { .. } => "Reopening",
        Change::SetPrOpen { .. } | Change::CloseIssue { .. } => "Closing",
        Change::ReadyForReview { .. } => "Marking it ready for review",
        Change::UpdateBranch { .. } => "Updating the branch",
        Change::Rerun { .. } | Change::RerunJob { .. } => "Asking GitHub to re-run it",
        Change::CancelRun { .. } => "Cancelling the run",
    }
}

/// What a change did, once GitHub said so.
fn done(change: &Change) -> String {
    match change {
        Change::Comment { .. } => "Comment posted".into(),
        Change::Reply { .. } => "Reply posted".into(),
        Change::Star {
            repo,
            starred: true,
            ..
        } => format!("Starred {repo}"),
        Change::Star { repo, .. } => format!("Unstarred {repo}"),
        Change::Merge { .. } => "Merged".into(),
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
    }
}

/// The permission a fine-grained token needs for `change`.
pub fn needs(change: &Change) -> &'static str {
    match change {
        Change::Comment { .. } => "Issues or Pull requests: write",
        Change::Star { .. } => "Starring: write",
        Change::Merge { .. } | Change::UpdateBranch { .. } => {
            "Contents: write and Pull requests: write"
        }
        Change::SetPrOpen { .. } | Change::ReadyForReview { .. } | Change::Reply { .. } => {
            "Pull requests: write"
        }
        Change::CloseIssue { .. } | Change::ReopenIssue { .. } => "Issues: write",
        Change::Rerun { .. } | Change::RerunJob { .. } | Change::CancelRun { .. } => {
            "Actions: write"
        }
    }
}

/// Why a change failed, in GitHub's words, and the permission it needs
/// when GitHub says the token lacks one.
fn failure(change: &Change, err: &ApiError) -> String {
    let what = match change {
        Change::Comment { .. } => "the comment",
        Change::Reply { .. } => "the reply",
        Change::Star { starred: true, .. } => "the star",
        Change::Star { .. } => "removing the star",
        Change::Merge { .. } => "the merge",
        Change::SetPrOpen { open: true, .. } | Change::ReopenIssue { .. } => "reopening it",
        Change::SetPrOpen { .. } | Change::CloseIssue { .. } => "closing it",
        Change::ReadyForReview { .. } => "marking it ready for review",
        Change::UpdateBranch { .. } => "updating the branch",
        Change::Rerun { .. } | Change::RerunJob { .. } => "the re-run",
        Change::CancelRun { .. } => "cancelling the run",
    };
    let reason = match err {
        ApiError::GraphQl(messages) => messages.join("; "),
        ApiError::Http { message, .. } => message.clone(),
        err => err.to_string(),
    };
    // GitHub refuses with 403 for other reasons too ("already running"),
    // so only its words say it's a permission.
    let denied = matches!(err, ApiError::Http { .. } | ApiError::GraphQl(_)) && {
        let reason = reason.to_lowercase();
        ["not accessible", "permission", "must have", "forbidden"]
            .iter()
            .any(|s| reason.contains(s))
    };
    let mut text = format!("GitHub refused {what}: {reason}");
    if denied {
        text.push_str(&format!(
            ". Your token may lack the permission: a fine-grained token needs {}; a classic one, the repo scope",
            needs(change)
        ));
    }
    text
}

/// The changes the menu offers for what's on screen: each action, its
/// label, and why it can't be made now, if it can't.
pub fn doables(state: &State) -> Vec<(Action, String, Option<String>)> {
    let Some(subject) = subject(state) else {
        return Vec::new();
    };
    (ACTIONS.into_iter())
        .filter(|&action| subject.fits(action))
        .map(|action| {
            let label = match (action, &subject) {
                (Action::Close, Subject::Run(..) | Subject::Job(..)) => "Cancel the run",
                (Action::Close, _) => "Close or reopen",
                (action, _) => action.description(),
            };
            let why = plan_for(state, &subject, action).err();
            (action, label.to_owned(), why)
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
        let refused = |message: &str| ApiError::Http {
            status: 403,
            message: message.into(),
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
