//! Changing GitHub from a page: merging, closing and reopening, marking a
//! draft ready, updating a branch, approving, re-running and cancelling
//! runs. What an action does where you are is decided once, by [`plan`]:
//! the key, the menu and the command palette all ask it, so they can't
//! disagree. Every change goes to GitHub as a [`Change`] and comes back as
//! [`Msg::Changed`], handled by [`on_changed`].

use crossterm::event::{KeyCode, KeyEvent};
use ghtui_api::ApiError;
use ghtui_api::browse::{CheckOutcome, IssueState, Job, WorkflowRun};
use ghtui_api::change::{Change, CloseReason, MergeMethod};
use ghtui_api::model::{
    ChecksState, MergeState, Mergeable, PrDetail, PrRef, RepoId, ReviewDecision,
};

use crate::browse::DataKey;
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
    Action::Close,
    Action::Rerun,
];

/// A change to confirm: what it is, what's worth knowing first, and the
/// ways to make it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirm {
    pub title: String,
    /// Each with whether it's a problem.
    pub facts: Vec<(String, bool)>,
    pub choices: Vec<(String, Change)>,
    pub selected: usize,
    pub sending: bool,
    pub error: Option<String>,
}

impl Confirm {
    fn new(title: String, facts: Vec<(String, bool)>, choices: Vec<(String, Change)>) -> Self {
        Self {
            title,
            facts,
            choices,
            selected: 0,
            sending: false,
            error: None,
        }
    }

    /// What's happening while GitHub answers.
    pub fn busy(&self) -> Option<String> {
        let (_, change) = self.choices.get(self.selected).filter(|_| self.sending)?;
        Some(format!("{}…", doing(change)))
    }
}

/// What an action does here.
#[derive(Debug)]
enum Plan {
    Ask(Confirm),
    Do(Change),
    Approve(PrRef),
}

/// Why `action` can't change anything here, if it can't.
pub fn unavailable(state: &State, action: Action) -> Option<String> {
    plan(state, action).err()
}

/// Runs one of [`ACTIONS`].
#[must_use]
pub fn act(state: &mut State, action: Action) -> Vec<Cmd> {
    match plan(state, action) {
        Err(why) => {
            state.info(why);
            Vec::new()
        }
        Ok(Plan::Ask(confirm)) => {
            state.overlay = Some(Overlay::Confirm(Box::new(confirm)));
            Vec::new()
        }
        Ok(Plan::Do(change)) => {
            state.info(format!("{}…", doing(&change)));
            vec![Cmd::Api(Api::Change(change))]
        }
        Ok(Plan::Approve(pr)) => crate::review::open_approve(state, pr),
    }
}

/// What the screen is about, as far as changing it goes.
enum Subject {
    Pr(PrRef),
    Issue(RepoId, u64),
    Run(RepoId, u64, Option<u64>),
    Job(RepoId, u64),
}

fn subject(state: &State) -> Option<Subject> {
    match state.screen() {
        Screen::Diff(d) => d.of.pr().map(|pr| Subject::Pr(pr.clone())),
        Screen::Page(p) => match &p.route {
            Route::Pr { pr, .. } => Some(Subject::Pr(pr.clone())),
            Route::Issue { repo, number } => Some(Subject::Issue(repo.clone(), *number)),
            Route::WorkflowRun { repo, run, attempt } => {
                Some(Subject::Run(repo.clone(), *run, *attempt))
            }
            Route::Job { repo, job, .. } => Some(Subject::Job(repo.clone(), *job)),
            _ => None,
        },
    }
}

fn plan(state: &State, action: Action) -> Result<Plan, String> {
    let elsewhere = || match action {
        Action::Close => "Closing works on an issue, a pull request or a running workflow run",
        Action::Rerun => "Re-running works on a workflow run or a job",
        _ => "That works on a pull request",
    };
    match subject(state).ok_or_else(|| elsewhere().to_owned())? {
        Subject::Pr(pr) => {
            let detail = state
                .prs
                .get(&pr)
                .and_then(|r| r.data.as_ref())
                .ok_or("The pull request hasn't loaded yet")?;
            pr_plan(state, &pr, detail, action).ok_or_else(|| elsewhere().to_owned())?
        }
        Subject::Issue(repo, number) => {
            let key = DataKey::Issue(repo.clone(), number);
            let issue = state
                .picked::<Option<Box<ghtui_api::browse::IssueDetail>>>(&key)
                .and_then(Option::as_deref)
                .ok_or("The issue hasn't loaded yet")?;
            if action != Action::Close {
                return Err(elsewhere().to_owned());
            }
            let name = format!("{repo}#{number}");
            let issue_id = issue.id.clone();
            Ok(Plan::Ask(match issue.state {
                IssueState::Open | IssueState::Draft => Confirm::new(
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
                IssueState::Closed | IssueState::NotPlanned | IssueState::Merged => Confirm::new(
                    format!("Reopen {name}?"),
                    Vec::new(),
                    vec![("Reopen".into(), Change::ReopenIssue { issue: issue_id })],
                ),
            }))
        }
        Subject::Run(repo, run, attempt) => {
            let run = state
                .picked::<WorkflowRun>(&DataKey::Run(repo.clone(), run, attempt))
                .ok_or("The run hasn't loaded yet")?;
            run_plan(&repo, run.id, run.outcome, None, action)
                .ok_or_else(|| elsewhere().to_owned())?
        }
        Subject::Job(repo, job) => {
            let job = state
                .picked::<Job>(&DataKey::Job(repo.clone(), job))
                .ok_or("The job hasn't loaded yet")?;
            run_plan(&repo, job.run_id, job.outcome, Some(job), action)
                .ok_or_else(|| elsewhere().to_owned())?
        }
    }
}

/// `None`: `action` isn't for pull requests.
fn pr_plan(
    state: &State,
    pr: &PrRef,
    d: &PrDetail,
    action: Action,
) -> Option<Result<Plan, String>> {
    let open = matches!(d.summary.state, IssueState::Open | IssueState::Draft);
    let key = |a: Action| state.first_key(a);
    let not_open = || match d.summary.state {
        IssueState::Merged => "It's merged".to_owned(),
        _ => format!("It's closed: {} reopens it", key(Action::Close)),
    };
    Some(match action {
        Action::Merge if d.summary.state == IssueState::Draft => Err(format!(
            "A draft can't be merged: {} marks it ready for review",
            key(Action::ReadyForReview)
        )),
        Action::Merge | Action::UpdateBranch | Action::Approve if !open => Err(not_open()),
        Action::Merge if d.merge_methods.is_empty() => {
            Err("The repository allows no way of merging".into())
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
            Ok(Plan::Ask(Confirm::new(
                title,
                merge_facts(state, d),
                choices,
            )))
        }
        Action::Close => {
            let (title, label, reopen) = match d.summary.state {
                IssueState::Merged => {
                    return Some(Err(
                        "It's merged: a merged pull request can't be reopened".into()
                    ));
                }
                IssueState::Open | IssueState::Draft => {
                    (format!("Close {pr}?"), "Close pull request", false)
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
            Ok(Plan::Ask(Confirm::new(
                title,
                facts,
                vec![(label.into(), change)],
            )))
        }
        Action::ReadyForReview if d.summary.state == IssueState::Draft => {
            Ok(Plan::Do(Change::ReadyForReview { pr: d.id.clone() }))
        }
        Action::ReadyForReview if open => Err("It isn't a draft".into()),
        Action::ReadyForReview => Err(not_open()),
        Action::UpdateBranch if d.merge_state == MergeState::Dirty => Err(format!(
            "It conflicts with {}: resolve that on GitHub",
            d.base_ref
        )),
        // GitHub's own "Update branch" button: behind, and the repository
        // suggests updating (or requires it).
        Action::UpdateBranch if !d.can_update_branch => Err(format!(
            "GitHub doesn't offer to update it: it's up to date with {}, or the repository doesn't suggest updating branches",
            d.base_ref
        )),
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
                    "Its branch, {}, is behind {}; updating pushes to it",
                    d.head_ref, d.base_ref
                ),
                false,
            )];
            Ok(Plan::Ask(Confirm::new(
                format!("Update {pr}'s branch?"),
                facts,
                choices,
            )))
        }
        Action::Approve if state.viewer.as_deref() == Some(d.summary.author.as_str()) => {
            Err("You can't approve your own pull request".into())
        }
        Action::Approve => Ok(Plan::Approve(pr.clone())),
        _ => return None,
    })
}

/// What's worth knowing before merging.
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
    let update = state.first_key(Action::UpdateBranch);
    let (text, problem) = match (d.merge_state, d.mergeable) {
        (MergeState::Dirty, _) | (_, Mergeable::Conflicting) => {
            (format!("It conflicts with {}", d.base_ref), true)
        }
        (MergeState::Behind, _) => (
            format!("The branch is behind {} ({update} updates it)", d.base_ref),
            true,
        ),
        (MergeState::Blocked, _) => (
            "GitHub says it's blocked: a required review or check is missing".into(),
            true,
        ),
        (MergeState::Unstable, _) => (
            "Some checks didn't pass, though none is required".into(),
            true,
        ),
        (MergeState::Unknown, _) | (_, Mergeable::Unknown) => (
            "GitHub is still working out whether it can merge".into(),
            false,
        ),
        (MergeState::Clean | MergeState::HasHooks | MergeState::Draft, Mergeable::Yes) => {
            ("Ready to merge".into(), false)
        }
    };
    facts.push((text, problem));
    // Behind, though nothing requires it to be up to date.
    if d.can_update_branch && !matches!(d.merge_state, MergeState::Behind | MergeState::Dirty) {
        facts.push((
            format!("The branch is behind {} ({update} updates it)", d.base_ref),
            false,
        ));
    }
    facts
}

/// Cancelling or re-running a run, from its page (`job: None`) or one of
/// its jobs'. `None`: `action` isn't for runs.
fn run_plan(
    repo: &RepoId,
    run: u64,
    outcome: CheckOutcome,
    job: Option<&Job>,
    action: Action,
) -> Option<Result<Plan, String>> {
    let running = outcome == CheckOutcome::Pending;
    Some(match action {
        Action::Close if running => {
            let change = Change::CancelRun {
                repo: repo.clone(),
                run,
            };
            Ok(Plan::Ask(Confirm::new(
                format!("Cancel run {run}?"),
                vec![("Jobs still running stop where they are".into(), false)],
                vec![("Cancel run".into(), change)],
            )))
        }
        Action::Close => Err("It has finished: there's nothing to cancel".into()),
        Action::Rerun if running => Err("It's still running: X cancels it".into()),
        Action::Rerun => {
            let rerun = |failed_only| Change::Rerun {
                repo: repo.clone(),
                run,
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
            if matches!(outcome, CheckOutcome::Failure | CheckOutcome::Cancelled) {
                choices.push(("Re-run failed jobs".to_owned(), rerun(true)));
            }
            choices.push(("Re-run all jobs".to_owned(), rerun(false)));
            Ok(Plan::Ask(Confirm::new(
                format!("Re-run run {run}?"),
                Vec::new(),
                choices,
            )))
        }
        _ => return None,
    })
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
                confirm.sending = true;
                confirm.error = None;
                return vec![Cmd::Api(Api::Change(change.clone()))];
            }
        }
        _ => {}
    }
    Vec::new()
}

/// GitHub's answer to a change. Whatever was shown is fetched again (a
/// write marks everything stale), so the page ends up as GitHub has it.
#[must_use]
pub fn on_changed(state: &mut State, change: &Change, result: Result<(), ApiError>) -> Vec<Cmd> {
    match result {
        Ok(()) => {
            if matches!(
                state.overlay,
                Some(Overlay::Confirm(_) | Overlay::Compose(_))
            ) {
                state.overlay = None;
            }
            state.info(done(change));
            state.load_visible(false)
        }
        Err(err) => {
            tracing::warn!(?change, %err, "a change to GitHub failed");
            if let Change::Star { repo, starred, .. } = change {
                nav::set_starred(state, repo, !starred);
            }
            let text = failure(change, &err);
            match &mut state.overlay {
                Some(Overlay::Confirm(confirm)) => {
                    confirm.sending = false;
                    confirm.error = Some(text);
                }
                Some(Overlay::Compose(compose)) if compose.sending => {
                    compose.sending = false;
                    compose.error = Some(text);
                }
                _ => state.error(text),
            }
            Vec::new()
        }
    }
}

/// What a change is doing while GitHub answers: "Merging".
fn doing(change: &Change) -> &'static str {
    match change {
        Change::Comment { .. } => "Posting",
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
        Change::SetPrOpen { .. } | Change::ReadyForReview { .. } => "Pull requests: write",
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
    use Action as A;
    let (actions, run): (&[Action], bool) = match subject(state) {
        Some(Subject::Pr(_)) => (
            &[
                A::Merge,
                A::Approve,
                A::ReadyForReview,
                A::UpdateBranch,
                A::Close,
            ],
            false,
        ),
        Some(Subject::Issue(..)) => (&[A::Close], false),
        Some(Subject::Run(..) | Subject::Job(..)) => (&[A::Rerun, A::Close], true),
        None => (&[], false),
    };
    actions
        .iter()
        .map(|&action| {
            let label = match action {
                A::Close if run => "Cancel the run",
                A::Close => "Close or reopen",
                action => action.description(),
            };
            (action, label.to_owned(), unavailable(state, action))
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
