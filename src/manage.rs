//! What GitHub's sidebar and menus do to an issue, a pull request or a
//! discussion besides editing it: a pull request's branch deleted after
//! it's merged (and restored), its base changed, a merged one reverted;
//! a conversation locked, an issue pinned, transferred or given a branch,
//! its sub-issues and what blocks it; a comment hidden; a new discussion.
//! What a key does here is decided by [`plan`], which the key and the
//! menu both ask, from GitHub's word on what you may do. A change goes to
//! GitHub as a [`Change::Manage`] and the page follows it as
//! [`crate::act`] follows its own ([`shows`]); what makes something new
//! (a revert, a transfer, a discussion) is a job ([`Revert`], [`Transfer`],
//! [`Start`]), whose answer opens
//! what it made.

use crate::route::{Route, discussions, issues, pulls};

use crate::api::browse::{DiscussionDetail, DiscussionsOf, IssueDetail, IssueState, PrState};
use crate::api::change::Change;
use crate::api::manage::{
    Categories, Category, DiscussionReason, HideReason, IssueAt, LockReason, Manage, Target,
};
use crate::api::model::{Capped, NodeId, PrDetail, PrRef, RepoId};
use crate::ui::overlays::PaletteItem;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::act::{Confirm, Step, Subject};
use crate::data::DataKey;
use crate::keymap::Action;
use crate::nav::Doable;
use crate::picker::fuzzy_score;
use crate::state::{Cmd, Overlay, Screen, State};
use crate::ui::page::Link;
use crate::work::{self, Cx, Failure, Job};

/// The actions this module takes as its own.
pub const ACTIONS: [Action; 10] = [
    Action::DeleteBranch,
    Action::ChangeBase,
    Action::Revert,
    Action::Lock,
    Action::Pin,
    Action::Transfer,
    Action::HideComment,
    Action::SubIssues,
    Action::BlockedBy,
    Action::NewDiscussion,
];

// ---- where you are ------------------------------------------------------------------

/// The issue, pull request or discussion on screen (a pull request's
/// files are it too).
fn here(state: &State) -> Option<Subject> {
    match state.screen() {
        Screen::Diff(d) => d.of.pr().cloned().map(Subject::Pr),
        Screen::Page(p) => match &p.route {
            Route::Issues(issues::Issue { repo, number }) => {
                Some(Subject::Issue(repo.clone(), *number))
            }
            Route::Pulls(pulls::Pr { pr, .. }) => Some(Subject::Pr(pr.clone())),
            Route::Discussions(discussions::Discussion { of, number }) => {
                Some(Subject::Discussion(of.clone(), *number))
            }
            _ => None,
        },
    }
}

fn issue<'a>(state: &'a State, repo: &RepoId, number: u64) -> Option<&'a IssueDetail> {
    let key = DataKey::Issue(repo.clone(), number);
    match state.picked::<Option<Box<IssueDetail>>>(&key) {
        Some(Some(d)) => Some(d),
        _ => None,
    }
}

fn pr<'a>(state: &'a State, pr: &PrRef) -> Option<&'a PrDetail> {
    state.pr(pr)
}

fn discussion<'a>(state: &'a State, of: &DiscussionsOf, n: u64) -> Option<&'a DiscussionDetail> {
    state.picked::<DiscussionDetail>(&DataKey::Discussion(of.clone(), n))
}

const LOADING: &str = "It's still loading";

/// The repository new discussions go to here: a repository's
/// discussions, or one of them.
fn discussions_repo(state: &State) -> Option<RepoId> {
    match state.route()? {
        Route::Discussions(discussions::Discussions {
            of: DiscussionsOf::Repo(repo),
            ..
        })
        | Route::Discussions(discussions::Discussion {
            of: DiscussionsOf::Repo(repo),
            ..
        }) => Some(repo.clone()),
        _ => None,
    }
}

/// A hidden or visible comment selected on the page: its node ID, what
/// to call it, and whether it's hidden, may be hidden, may be unhidden.
struct Said {
    id: NodeId,
    name: String,
    hidden: bool,
    can_hide: bool,
    can_unhide: bool,
}

/// The comment selected in the conversation on screen.
fn selected_comment(state: &State, about: &Subject) -> Result<Said, String> {
    let Screen::Page(p) = state.screen() else {
        return Err("Hiding works on a comment in a conversation".into());
    };
    let next = state.first_key(Action::NextItem);
    // A comment's box, or a hidden one's fold, which its anchor names.
    let (by, number) = match p.selected_link() {
        Some(Link::Quote { author, at, .. }) => (Some((author, at)), None),
        Some(Link::Review(crate::ui::page::ReviewLink::Fold(anchor))) => {
            let n = (anchor.strip_prefix("issuecomment-"))
                .or_else(|| anchor.strip_prefix("discussioncomment-"))
                .and_then(|n| n.parse::<u64>().ok());
            (None, n)
        }
        _ => (None, None),
    };
    if by.is_none() && number.is_none() {
        return Err(format!(
            "Select a comment first: {next} goes comment by comment"
        ));
    }
    let is = |c: &crate::api::browse::Comment| match by {
        Some((author, at)) => c.author == *author && c.created_at == *at,
        None => c.id.is_some() && c.id == number,
    };
    let found = match about {
        Subject::Issue(repo, number) => {
            let d = issue(state, repo, *number).ok_or(LOADING)?;
            d.comments().find(|c| is(c)).cloned()
        }
        Subject::Pr(pr) => {
            let a = state.activity(pr).ok_or(LOADING)?;
            a.comments().find(|c| is(c)).cloned()
        }
        Subject::Discussion(of, n) => {
            let d = discussion(state, of, *n).ok_or(LOADING)?;
            let comments = d.answer.as_deref().into_iter().chain(&d.comments);
            comments
                .flat_map(|c| std::iter::once(&c.comment).chain(c.replies.iter()))
                .find(|c| is(c))
                .cloned()
        }
        _ => None,
    };
    let Some(c) = found else {
        return Err("Only a comment can be hidden: select one".into());
    };
    let id = (c.editable.as_ref())
        .map(|e| e.id.clone())
        .ok_or("GitHub didn't say what you may do with it")?;
    Ok(Said {
        id,
        name: format!("{}'s comment", c.author),
        hidden: c.hiding.reason.is_some(),
        can_hide: c.hiding.can_hide,
        can_unhide: c.hiding.can_unhide,
    })
}

// ---- what a key does --------------------------------------------------------------

fn ask(action: Action, about: Subject, title: String, facts: Vec<(String, bool)>) -> Confirm {
    Confirm::new(action, about, title, facts, Vec::new())
}

/// Where `action` works, for when it doesn't here.
fn elsewhere(action: Action) -> String {
    match action {
        Action::DeleteBranch | Action::ChangeBase | Action::Revert => {
            "That works on a pull request".into()
        }
        Action::Lock => "Locking works on an issue, a pull request or a discussion".into(),
        Action::HideComment => {
            "Hiding works on a comment on an issue, a pull request or a discussion".into()
        }
        Action::NewDiscussion => "That works in a repository's discussions".into(),
        _ => "That works on an issue".into(),
    }
}

fn plan(state: &State, action: Action) -> Result<Step, String> {
    if action == Action::NewDiscussion {
        let repo = discussions_repo(state).ok_or_else(|| elsewhere(action))?;
        if state.overview(&repo).is_some_and(|o| !o.has_discussions) {
            return Err(format!("{repo} has discussions turned off"));
        }
        return pick(Pick::Category { repo });
    }
    let about = here(state).ok_or_else(|| elsewhere(action))?;
    crate::act::settled(state)?;
    if action == Action::HideComment {
        let said = selected_comment(state, &about)?;
        return match (said.hidden, said.can_hide, said.can_unhide) {
            (true, _, true) => Ok(Step::Send(
                Change::Manage(Manage::Unhide { comment: said.id }),
                about,
            )),
            (true, _, false) => Err(format!("GitHub doesn't let you unhide {}", said.name)),
            (false, true, _) => pick(Pick::Hide {
                about,
                comment: said.id,
                name: said.name,
            }),
            (false, false, _) => Err(format!(
                "GitHub doesn't let you hide {}: that takes its repository's triage access",
                said.name
            )),
        };
    }
    match &about {
        Subject::Pr(r) => pr_plan(state, r, action, about.clone()),
        Subject::Issue(repo, number) => {
            let d = issue(state, repo, *number).ok_or(LOADING)?;
            issue_plan(d, action, about.clone())
        }
        Subject::Discussion(of, n) => {
            let d = discussion(state, of, *n).ok_or(LOADING)?;
            match action {
                Action::Lock => lock_plan(d.locked, d.can_lock, &d.id, about.clone(), "discussion"),
                _ => Err(elsewhere(action)),
            }
        }
        _ => Err(elsewhere(action)),
    }
}

/// Locking: GitHub's reasons to choose from; unlocking, after asking.
fn lock_plan(
    locked: Option<crate::api::manage::Locked>,
    may: bool,
    id: &NodeId,
    about: Subject,
    what: &str,
) -> Result<Step, String> {
    if !may {
        let verb = if locked.is_some() { "unlock" } else { "lock" };
        return Err(format!(
            "GitHub doesn't let you {verb} it: that takes triage access"
        ));
    }
    if locked.is_none() {
        return pick(Pick::Lock {
            about,
            id: id.clone(),
        });
    }
    let mut confirm = ask(
        Action::Lock,
        about,
        format!("Unlock the conversation on this {what}?"),
        vec![("Everyone will be able to comment again".into(), false)],
    );
    let unlock = Change::Manage(Manage::Unlock {
        subject: id.clone(),
    });
    confirm.choices = vec![("Unlock conversation".into(), unlock)];
    Ok(Step::Ask(Box::new(confirm)))
}

fn pr_plan(state: &State, r: &PrRef, action: Action, about: Subject) -> Result<Step, String> {
    let d = pr(state, r).ok_or(LOADING)?;
    let same_repo =
        (d.head_repo.as_deref()).is_some_and(|h| h.eq_ignore_ascii_case(&r.repo.to_string()));
    match action {
        Action::Lock => lock_plan(d.locked, d.sidebar.labels, &d.id, about, "pull request"),
        Action::DeleteBranch if d.state.is_open() => Err(format!(
            "It's still open: its branch, {}, is deleted once it's merged or closed",
            d.head_ref
        )),
        Action::DeleteBranch if !same_repo => Err(match &d.head_repo {
            Some(fork) => format!("Its branch is in {fork}, not here: its owner deletes it"),
            None => "Its branch's repository is gone".into(),
        }),
        Action::DeleteBranch if d.head_gone && !d.may.merge => Err(format!(
            "Restoring {} takes write access to {}",
            d.head_ref, r.repo
        )),
        Action::DeleteBranch if d.head_gone => Ok(Step::Send(
            Change::Manage(Manage::RestoreBranch {
                repo: r.repo.clone(),
                branch: d.head_ref.clone(),
                oid: d.head_oid.clone(),
            }),
            about,
        )),
        Action::DeleteBranch if !d.may.delete_head => Err(format!(
            "GitHub doesn't let you delete {}: it takes write access, and it may be protected or another pull request's",
            d.head_ref
        )),
        Action::DeleteBranch => Ok(Step::Send(
            Change::Manage(Manage::DeleteBranch {
                repo: r.repo.clone(),
                branch: d.head_ref.clone(),
            }),
            about,
        )),
        Action::ChangeBase if !d.state.is_open() => {
            Err("Only an open pull request's base can change".into())
        }
        Action::ChangeBase if !d.may.edit => {
            Err("Changing its base takes its author or write access".into())
        }
        Action::ChangeBase => pick(Pick::Base {
            pr: r.clone(),
            id: d.id.clone(),
            from: d.base_ref.clone(),
            head: d.head_ref.clone(),
        }),
        Action::Revert if d.state != PrState::Merged => {
            Err("Only a merged pull request can be reverted".into())
        }
        Action::Revert if !d.may.merge => {
            Err(format!("Reverting it takes write access to {}", r.repo))
        }
        Action::Revert => {
            let mut confirm = ask(
                action,
                about,
                format!("Revert #{}?", r.number),
                vec![
                    (
                        format!(
                            "GitHub makes a branch undoing its changes, and a pull request of it into {}",
                            d.base_ref
                        ),
                        false,
                    ),
                    ("The new pull request opens".into(), false),
                ],
            );
            let revert = Change::Manage(Manage::Revert { pr: d.id.clone() });
            confirm.choices = vec![("Revert".into(), revert)];
            Ok(Step::Ask(Box::new(confirm)))
        }
        _ => Err(elsewhere(action)),
    }
}

fn issue_plan(d: &IssueDetail, action: Action, about: Subject) -> Result<Step, String> {
    let at = IssueAt {
        repo: d.repo.clone(),
        number: d.number,
    };
    let triage = d.sidebar.labels;
    match action {
        Action::Lock => lock_plan(d.locked, triage, &d.id, about, "issue"),
        Action::Pin if !d.can_write => {
            Err(format!("Pinning an issue takes write access to {}", d.repo))
        }
        Action::Pin => Ok(Step::Send(
            Change::Manage(Manage::Pin {
                issue: d.id.clone(),
                on: !d.pinned,
            }),
            about,
        )),
        Action::Transfer if !d.can_write => {
            Err(format!("Transferring takes write access to {}", d.repo))
        }
        Action::Transfer => pick(Pick::Transfer {
            issue: at,
            id: d.id.clone(),
        }),
        Action::SubIssues | Action::BlockedBy if !triage => {
            Err("GitHub doesn't let you change it: that takes triage access".into())
        }
        Action::SubIssues => pick(Pick::SubIssues {
            issue: at,
            id: d.id.clone(),
        }),
        Action::BlockedBy => pick(Pick::BlockedBy {
            issue: at,
            id: d.id.clone(),
        }),
        Action::Checkout if !d.can_write => {
            Err(format!("Making a branch takes write access to {}", d.repo))
        }
        Action::Checkout => pick(Pick::Branch {
            issue: at,
            id: d.id.clone(),
            title: d.title.clone(),
        }),
        _ => Err(elsewhere(action)),
    }
}

/// Whether this module takes `action` here: its own anywhere, and
/// checking out on an issue (a branch for it).
/// What managing issues and pull requests adds: its own actions, and
/// checking out a branch for the issue on screen.
pub const FEATURE: crate::features::Feature = crate::features::Feature {
    owns: &ACTIONS,
    takes: |state, action| {
        action == Action::Checkout && matches!(here(state), Some(Subject::Issue(..)))
    },
    act,
    offers: doables,
    ..crate::features::Feature::NONE
};

/// Runs one of its actions, or a branch's checkout for the issue on screen.
#[must_use]
pub fn act(state: &mut State, action: Action) -> Vec<Cmd> {
    crate::act::take(state, plan(state, action))
}

/// Choosing in one of this module's pickers.
#[expect(clippy::unnecessary_wraps, reason = "a plan's step")]
fn pick(pick: Pick) -> Result<Step, String> {
    Ok(Step::Pick(crate::picker::Kind::Manage(pick)))
}

#[must_use]
fn send(state: &mut State, change: Change, about: Subject) -> Vec<Cmd> {
    crate::act::take(state, Ok(Step::Send(change, about)))
}

/// The menu's rows where they apply.
pub fn doables(state: &State, out: &mut Vec<Doable>) {
    let row = |action, label: &str| {
        crate::nav::row("Change", action, label, "", plan(state, action).err())
    };
    if discussions_repo(state).is_some() {
        out.push(row(Action::NewDiscussion, "Start a new discussion"));
    }
    let Some(about) = here(state) else {
        return;
    };
    let locked = match &about {
        Subject::Pr(r) => pr(state, r).map(|d| d.locked.is_some()),
        Subject::Issue(repo, n) => issue(state, repo, *n).map(|d| d.locked.is_some()),
        Subject::Discussion(of, n) => discussion(state, of, *n).map(|d| d.locked.is_some()),
        _ => None,
    };
    let lock = if locked == Some(true) {
        "Unlock the conversation"
    } else {
        "Lock the conversation"
    };
    out.push(row(Action::Lock, lock));
    if let Screen::Page(_) = state.screen() {
        out.push(row(
            Action::HideComment,
            "Hide the selected comment, or unhide it",
        ));
    }
    match &about {
        Subject::Pr(r) => {
            let gone = pr(state, r).is_some_and(|d| d.head_gone);
            let branch = if gone {
                "Restore the branch"
            } else {
                "Delete the branch"
            };
            out.push(row(Action::DeleteBranch, branch));
            out.push(row(Action::ChangeBase, "Change the base branch"));
            out.push(row(Action::Revert, "Revert the pull request"));
        }
        Subject::Issue(repo, n) => {
            let pinned = issue(state, repo, *n).is_some_and(|d| d.pinned);
            out.push(row(
                Action::Pin,
                if pinned {
                    "Unpin the issue"
                } else {
                    "Pin the issue"
                },
            ));
            out.push(row(Action::Transfer, "Transfer the issue"));
            out.push(row(Action::SubIssues, "Add or remove sub-issues"));
            out.push(row(Action::BlockedBy, "Blocked by: add or remove"));
            out.push(row(Action::Checkout, "Create a branch for the issue"));
        }
        _ => {}
    }
}

// ---- pickers --------------------------------------------------------------------

/// What one of this module's pickers lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// GitHub's reasons to lock a conversation.
    Lock {
        about: Subject,
        id: NodeId,
    },
    /// GitHub's reasons to hide a comment.
    Hide {
        about: Subject,
        comment: NodeId,
        name: String,
    },
    /// The branches a pull request's base can be.
    Base {
        pr: PrRef,
        id: NodeId,
        from: String,
        head: String,
    },
    /// The repositories an issue can move to.
    Transfer {
        issue: IssueAt,
        id: NodeId,
    },
    /// The issue one duplicates.
    Duplicate {
        issue: IssueAt,
        id: NodeId,
    },
    SubIssues {
        issue: IssueAt,
        id: NodeId,
    },
    BlockedBy {
        issue: IssueAt,
        id: NodeId,
    },
    /// A new branch's name, for an issue titled `title`.
    Branch {
        issue: IssueAt,
        id: NodeId,
        title: String,
    },
    /// Whether to check out a branch just made.
    Checkout {
        repo: RepoId,
        branch: String,
    },
    /// A new discussion's category.
    Category {
        repo: RepoId,
    },
}

impl Pick {
    pub fn title(&self) -> &'static str {
        match self {
            Pick::Lock { .. } => "Lock conversation",
            Pick::Hide { .. } => "Hide the comment as",
            Pick::Base { .. } => "Change the base branch",
            Pick::Transfer { .. } => "Transfer the issue to",
            Pick::Duplicate { .. } => "Close as a duplicate of",
            Pick::SubIssues { .. } => "Sub-issues",
            Pick::BlockedBy { .. } => "Blocked by",
            Pick::Branch { .. } => "Create a branch for the issue",
            Pick::Checkout { .. } => "Check out the branch?",
            Pick::Category { .. } => "New discussion: choose a category",
        }
    }

    pub fn placeholder(&self) -> &'static str {
        match self {
            Pick::Lock { .. } => "Choose a reason (optional), then ↵",
            Pick::Hide { .. } | Pick::Checkout { .. } => "Choose, then ↵",
            Pick::Base { .. } => "Find a branch",
            Pick::Transfer { .. } => "Find a repository",
            Pick::Duplicate { .. } => "#12, owner/repo#12, a link, or find one",
            Pick::SubIssues { .. } => "#12 adds · ↵ on ✓ removes · alt-↑↓ moves",
            Pick::BlockedBy { .. } => "#12 adds · ↵ on ✓ removes",
            Pick::Branch { .. } => "The branch's name",
            Pick::Category { .. } => "Find a category",
        }
    }

    /// What it lists, loaded when it opens.
    pub fn need(&self) -> Option<DataKey> {
        match self {
            Pick::Base { pr, .. } => Some(DataKey::Refs(pr.repo.clone())),
            Pick::Duplicate { issue, .. }
            | Pick::SubIssues { issue, .. }
            | Pick::BlockedBy { issue, .. } => Some(DataKey::OpenIssues(issue.repo.clone())),
            Pick::Transfer { issue, .. } => Some(DataKey::MoveTargets(issue.repo.owner.clone())),
            Pick::Category { repo } => Some(DataKey::Categories(repo.clone())),
            _ => None,
        }
    }

    /// The text the field starts with.
    pub fn start(&self) -> Option<String> {
        match self {
            Pick::Branch { issue, title, .. } => Some(branch_name(issue.number, title)),
            _ => None,
        }
    }

    /// Whether choosing a row keeps the picker open.
    fn stays(&self) -> bool {
        matches!(self, Pick::SubIssues { .. } | Pick::BlockedBy { .. })
    }
}

/// GitHub's name for an issue's branch: its number, then its title's
/// words, lowercased and joined with dashes.
pub fn branch_name(number: u64, title: &str) -> String {
    let words: Vec<String> = title
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    let mut name = number.to_string();
    for w in words {
        if name.len() + 1 + w.len() > 60 {
            break;
        }
        name.push('-');
        name.push_str(&w);
    }
    name
}

/// What choosing a row does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Chosen {
    /// Send this change, about this.
    Send(Change, Subject),
    /// Ask first.
    Ask(Box<Confirm>),
    /// Check out a branch in your clone.
    Checkout(crate::create::Checkout),
    /// Write the new discussion in the composer, starting with this.
    Write(Starting, String),
}

type Rows = Vec<(PaletteItem, Option<Chosen>)>;

fn item(
    label: impl Into<String>,
    hint: impl Into<String>,
    chosen: Option<Chosen>,
) -> (PaletteItem, Option<Chosen>) {
    let item = PaletteItem {
        label: label.into(),
        hint: hint.into(),
    };
    (item, chosen)
}

/// A list fetched for a picker: its rows, or a row saying why there are
/// none yet.
fn fetched<'a, T: crate::data::Picked>(
    state: &'a State,
    key: &DataKey,
    what: &str,
) -> Result<&'a T, Rows> {
    match state.answer(key) {
        None => Err(vec![item(format!("Loading {what}…"), "", None)]),
        Some(Err(why)) => Err(vec![item(format!("Couldn't load {what}: {why}"), "", None)]),
        Some(Ok(found)) => Ok(found),
    }
}

/// A picker's rows: what each says, and what choosing it does.
pub fn rows(state: &State, pick: &Pick, q: &str) -> Rows {
    match pick {
        Pick::Lock { about, id } => {
            let lock = |reason: Option<LockReason>| {
                Chosen::Send(
                    Change::Manage(Manage::Lock {
                        subject: id.clone(),
                        reason,
                    }),
                    about.clone(),
                )
            };
            let mut rows = Vec::new();
            if fuzzy_score(q, "Lock without a reason").is_some() {
                rows.push(item(
                    "Lock without a reason",
                    "only collaborators can comment",
                    Some(lock(None)),
                ));
            }
            for reason in [
                LockReason::OffTopic,
                LockReason::TooHeated,
                LockReason::Resolved,
                LockReason::Spam,
            ] {
                let said = crate::api::manage::lock_said(reason);
                let label = format!("Lock as {said}");
                if fuzzy_score(q, &label).is_some() {
                    rows.push(item(label, "", Some(lock(Some(reason)))));
                }
            }
            rows
        }
        Pick::Hide {
            about,
            comment,
            name,
        } => (HideReason::ALL.into_iter())
            .filter(|r| fuzzy_score(q, r.label()).is_some())
            .map(|reason| {
                let change = Change::Manage(Manage::Hide {
                    comment: comment.clone(),
                    reason,
                });
                item(
                    reason.label(),
                    format!("hides {name}"),
                    Some(Chosen::Send(change, about.clone())),
                )
            })
            .collect(),
        Pick::Base { pr, id, from, head } => base_rows(state, pr, id, from, head, q),
        Pick::Transfer { issue, id } => transfer_rows(state, issue, id, q),
        Pick::Duplicate { issue, id } => {
            let about = Subject::Issue(issue.repo.clone(), issue.number);
            let close = |of: IssueAt| {
                let change = Change::Manage(Manage::CloseAsDuplicate {
                    issue: id.clone(),
                    of: Some(of),
                });
                Chosen::Send(change, about.clone())
            };
            issue_rows(state, issue, q, &[], |of, title| {
                let label = format!("Duplicate of {} {title}", of.named(&issue.repo));
                item(label, "closes it", Some(close(of)))
            })
        }
        Pick::SubIssues { issue, id } => relation_rows(state, issue, id, q, true),
        Pick::BlockedBy { issue, id } => relation_rows(state, issue, id, q, false),
        Pick::Branch { issue, id, .. } => {
            let name = q.trim();
            if name.is_empty() {
                return vec![item("Type the branch's name", "", None)];
            }
            if name.contains(char::is_whitespace) || name.contains("..") {
                return vec![item("A branch's name has no spaces or ..", "", None)];
            }
            let default = state
                .overview(&issue.repo)
                .and_then(|o| o.default_branch.clone())
                .unwrap_or_else(|| "the default branch".into());
            let change = Change::Manage(Manage::LinkBranch {
                issue: id.clone(),
                repo: issue.repo.clone(),
                name: name.to_owned(),
            });
            let about = Subject::Issue(issue.repo.clone(), issue.number);
            vec![item(
                format!("Create {name} from {default}"),
                format!("linked to #{}", issue.number),
                Some(Chosen::Send(change, about)),
            )]
        }
        Pick::Checkout { repo, branch } => vec![
            item(
                format!("Check out {branch}"),
                "in your clone",
                Some(Chosen::Checkout(crate::create::Checkout {
                    repo: repo.clone(),
                    pr: None,
                    name: branch.clone(),
                    track: Some(branch.clone()),
                })),
            ),
            item("Not now", "", None),
        ],
        Pick::Category { repo } => category_rows(state, repo, q),
    }
}

fn base_rows(state: &State, pr: &PrRef, id: &NodeId, from: &str, head: &str, q: &str) -> Rows {
    let key = DataKey::Refs(pr.repo.clone());
    let Some(refs) = state.picked::<crate::api::browse::Refs>(&key) else {
        let loading = state.remote(&key).is_none_or(|r| r.loading);
        let why = match state.remote(&key).and_then(|r| r.error.as_deref()) {
            Some(err) if !loading => format!("Couldn't load the branches: {err}"),
            _ => "Loading the branches…".into(),
        };
        return vec![item(why, "", None)];
    };
    let mut hits: Vec<(usize, &String)> = (refs.branches.iter())
        .filter(|b| *b != from && *b != head)
        .filter_map(|b| Some((fuzzy_score(q, b)?, b)))
        .collect();
    hits.sort_by_key(|(score, _)| *score);
    let about = Subject::Pr(pr.clone());
    let mut rows: Rows = hits
        .into_iter()
        .map(|(_, base)| {
            let mut confirm = ask(
                Action::ChangeBase,
                about.clone(),
                format!("Change #{}'s base to {base}?", pr.number),
                vec![
                    (format!("From {from} to {base}"), false),
                    (
                        format!("Its commits and changes are then those not in {base}"),
                        false,
                    ),
                ],
            );
            let change = Change::Manage(Manage::ChangeBase {
                pr: id.clone(),
                base: base.clone(),
            });
            confirm.choices = vec![("Change base".into(), change)];
            item(base.clone(), "", Some(Chosen::Ask(Box::new(confirm))))
        })
        .collect();
    if rows.is_empty() {
        rows.push(item("No other branch matches", "", None));
    }
    if refs.branches.left_out() > 0 {
        let more = format!(
            "{} more branches GitHub didn't list",
            refs.branches.left_out()
        );
        rows.push(item(more, "", None));
    }
    rows
}

fn transfer_rows(state: &State, issue: &IssueAt, id: &NodeId, q: &str) -> Rows {
    let owner = &issue.repo.owner;
    let key = DataKey::MoveTargets(owner.clone());
    let targets = match fetched::<Capped<Target>>(state, &key, "the repositories") {
        Ok(t) => t,
        Err(rows) => return rows,
    };
    let about = Subject::Issue(issue.repo.clone(), issue.number);
    let mut rows: Rows = (targets.iter())
        .filter(|t| t.repo != issue.repo)
        .filter(|t| fuzzy_score(q, &t.repo.to_string()).is_some())
        .map(|t| {
            let mut confirm = ask(
                Action::Transfer,
                about.clone(),
                format!("Transfer #{} to {}?", issue.number, t.repo),
                vec![
                    (
                        "Its comments, assignees and labels GitHub can match go with it".into(),
                        false,
                    ),
                    (format!("Its page then is in {}", t.repo), false),
                ],
            );
            let change = Change::Manage(Manage::Transfer {
                issue: id.clone(),
                to: t.id.clone(),
                repo: t.repo.clone(),
            });
            confirm.choices = vec![("Transfer issue".into(), change)];
            item(t.repo.to_string(), "", Some(Chosen::Ask(Box::new(confirm))))
        })
        .collect();
    if rows.is_empty() {
        rows.push(item(
            format!("No other repository of {owner}'s that you can write to has issues on"),
            "an issue moves only within its owner",
            None,
        ));
    }
    if targets.left_out() > 0 {
        let more = format!("{owner}'s first 100 repositories, most recently pushed, are listed");
        rows.push(item(more, "", None));
    }
    rows
}

/// Rows for issues: what's typed (`#12`, a link), then the repository's
/// open issues that match, but `skip` and the issue itself.
fn issue_rows(
    state: &State,
    issue: &IssueAt,
    q: &str,
    skip: &[IssueAt],
    row: impl Fn(IssueAt, &str) -> (PaletteItem, Option<Chosen>),
) -> Rows {
    let mut rows = Vec::new();
    let typed = IssueAt::parse(q, &issue.repo).filter(|t| t != issue && !skip.contains(t));
    if let Some(t) = &typed {
        rows.push(row(t.clone(), ""));
    }
    let key = DataKey::OpenIssues(issue.repo.clone());
    match fetched::<Capped<(u64, String)>>(state, &key, "its open issues") {
        Ok(open) => {
            let listed = (open.iter())
                .map(|(n, title)| {
                    let at = IssueAt {
                        repo: issue.repo.clone(),
                        number: *n,
                    };
                    (at, title)
                })
                .filter(|(at, _)| at != issue && !skip.contains(at) && Some(at) != typed.as_ref())
                .filter(|(at, title)| {
                    typed.is_some() || fuzzy_score(q, &format!("#{} {title}", at.number)).is_some()
                });
            rows.extend(listed.map(|(at, title)| row(at, title)));
        }
        Err(why) => rows.extend(why),
    }
    rows
}

/// Sub-issues (`sub`) or what blocks the issue: those there now, which
/// choosing removes, then what to add.
fn relation_rows(state: &State, at: &IssueAt, id: &NodeId, q: &str, sub: bool) -> Rows {
    let Some(d) = issue(state, &at.repo, at.number) else {
        return vec![item("Loading the issue…", "", None)];
    };
    let about = Subject::Issue(at.repo.clone(), at.number);
    let now = if sub { &d.sub_issues } else { &d.blocked_by };
    let change = |other: IssueAt, add: bool| {
        Change::Manage(if sub {
            Manage::SubIssue {
                parent: id.clone(),
                child: other,
                add,
            }
        } else {
            Manage::BlockedBy {
                issue: id.clone(),
                by: other,
                add,
            }
        })
    };
    let there: Vec<IssueAt> = (now.iter())
        .map(|l| IssueAt {
            repo: l.repo.clone(),
            number: l.number,
        })
        .collect();
    let typed = IssueAt::parse(q, &at.repo);
    let mut rows = Vec::new();
    for (l, other) in now.iter().zip(&there) {
        let label = format!("{} {}", other.named(&at.repo), l.title);
        let matches = match &typed {
            Some(t) => t == other,
            None => fuzzy_score(q, &label).is_some(),
        };
        if !matches {
            continue;
        }
        let verb = if sub {
            "removes it"
        } else {
            "no longer blocked by it"
        };
        rows.push(item(
            format!("✓ {label}"),
            format!("↵ {verb}"),
            Some(Chosen::Send(change(other.clone(), false), about.clone())),
        ));
    }
    if now.left_out() > 0 {
        rows.push(item(format!("and {} more", now.left_out()), "", None));
    }
    let adds = issue_rows(state, at, q, &there, |other, title| {
        let label = if sub {
            format!("Add {} {title}", other.named(&at.repo))
        } else {
            format!("Blocked by {} {title}", other.named(&at.repo))
        };
        item(
            label,
            "",
            Some(Chosen::Send(change(other, true), about.clone())),
        )
    });
    // What's typed first: it's what ↵ takes.
    if typed.is_some() {
        rows.splice(0..0, adds);
    } else {
        rows.extend(adds);
    }
    rows
}

fn category_rows(state: &State, repo: &RepoId, q: &str) -> Rows {
    let key = DataKey::Categories(repo.clone());
    let found = match fetched::<Categories>(state, &key, "its categories") {
        Ok(c) => c,
        Err(rows) => return rows,
    };
    let mut rows: Rows = (found.categories.iter())
        .filter(|c| fuzzy_score(q, &c.name).is_some())
        .map(|c| {
            let hint = match (&c.form, c.answerable) {
                (Some(_), true) => "Q&A · has a form",
                (Some(_), false) => "has a form",
                (None, true) => "Q&A",
                (None, false) => "",
            };
            let label = match c.description.trim() {
                "" => c.name.clone(),
                about => format!("{} · {about}", c.name),
            };
            let starting = Starting {
                repo: repo.clone(),
                repo_id: found.repo.clone(),
                category: c.id.clone(),
                name: c.name.clone(),
            };
            let text = form_text(c);
            item(label, hint, Some(Chosen::Write(starting, text)))
        })
        .collect();
    if found.categories.is_empty() {
        rows.push(item("It has no categories", "", None));
    }
    if found.categories.left_out() > 0 {
        let more = format!("and {} more categories", found.categories.left_out());
        rows.push(item(more, "", None));
    }
    rows
}

/// What a new discussion in `c` starts with: its form's title and a
/// heading per field, as an issue form's.
fn form_text(c: &Category) -> String {
    let Some(form) = &c.form else {
        return String::new();
    };
    let file = format!("{}.yml", c.slug);
    let chooser = crate::create::templates::chooser(&[(file, form.clone())]);
    match chooser.templates.first() {
        Some(t) => crate::create::templates::join(&t.title, &t.body),
        None => String::new(),
    }
}

/// Keys this module's pickers handle themselves: in the sub-issues
/// picker, alt-↑ and alt-↓ move the selected one, and choosing keeps it
/// open. `None` leaves the key to the picker.
#[must_use]
pub fn picker_key(
    state: &mut State,
    key: KeyEvent,
    rows: &mut [(PaletteItem, Option<crate::picker::Choice>)],
) -> Option<Vec<Cmd>> {
    let Some(Overlay::Picker(p)) = &mut state.overlay else {
        return None;
    };
    let crate::picker::Kind::Manage(pick) = &p.kind else {
        return None;
    };
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match (key.code, pick) {
        (KeyCode::Up | KeyCode::Down, Pick::SubIssues { issue, id }) if alt => {
            let up = key.code == KeyCode::Up;
            let child = match rows.get(p.selected) {
                Some((
                    _,
                    Some(crate::picker::Choice::Manage(Chosen::Send(
                        Change::Manage(Manage::SubIssue {
                            child, add: false, ..
                        }),
                        _,
                    ))),
                )) => Some(child.clone()),
                _ => None,
            };
            let (issue, id) = (issue.clone(), id.clone());
            Some(move_sub_issue(state, &issue, &id, child, up))
        }
        (KeyCode::Enter, pick) if pick.stays() => {
            let chosen = rows.get_mut(p.selected).and_then(|(_, c)| c.take());
            match chosen {
                Some(crate::picker::Choice::Manage(chosen)) => Some(choose(state, chosen)),
                _ => Some(Vec::new()),
            }
        }
        _ => None,
    }
}

/// Moves the sub-issue on picker row `selected` up or down, the selection
/// with it.
#[must_use]
fn move_sub_issue(
    state: &mut State,
    at: &IssueAt,
    id: &NodeId,
    child: Option<IssueAt>,
    up: bool,
) -> Vec<Cmd> {
    let Some(d) = issue(state, &at.repo, at.number) else {
        return Vec::new();
    };
    let subs: Vec<IssueAt> = (d.sub_issues.iter())
        .map(|l| IssueAt {
            repo: l.repo.clone(),
            number: l.number,
        })
        .collect();
    let Some((selected, child)) = child.and_then(|c| Some((subs.iter().position(|s| *s == c)?, c)))
    else {
        state.info("Choose one of its sub-issues (✓) to move");
        return Vec::new();
    };
    let sibling = if up {
        selected.checked_sub(1).and_then(|i| subs.get(i))
    } else {
        subs.get(selected + 1)
    };
    let Some(sibling) = sibling.cloned() else {
        state.info(if up {
            "It's the first already"
        } else {
            "It's the last already"
        });
        return Vec::new();
    };
    if let Some(Overlay::Picker(p)) = &mut state.overlay {
        p.selected = if up {
            p.selected.saturating_sub(1)
        } else {
            p.selected + 1
        };
    }
    let change = Change::Manage(Manage::MoveSubIssue {
        parent: id.clone(),
        child,
        sibling,
        before: up,
    });
    send(state, change, Subject::Issue(at.repo.clone(), at.number))
}

/// A row chosen in one of this module's pickers.
#[must_use]
pub fn choose(state: &mut State, chosen: Chosen) -> Vec<Cmd> {
    match chosen {
        Chosen::Send(change, about) => send(state, change, about),
        Chosen::Ask(confirm) => {
            state.overlay = Some(Overlay::Confirm(confirm));
            Vec::new()
        }
        Chosen::Checkout(checkout) => {
            state.info(format!("Checking out {}…", checkout.name));
            work::start(checkout)
        }
        Chosen::Write(starting, text) => {
            let target = crate::compose::Target::NewDiscussion(starting);
            state.compose(target, &text)
        }
    }
}

/// A change chosen in a confirmation that isn't simply sent: a revert or
/// a transfer, which answer with what they made, and a duplicate, which
/// is chosen first. `None` sends it.
#[must_use]
pub fn confirmed(state: &mut State, change: &Change, about: &Subject) -> Option<Vec<Cmd>> {
    let Change::Manage(m) = change else {
        return None;
    };
    let work: work::Work = match (m, about) {
        (Manage::CloseAsDuplicate { issue, of: None }, Subject::Issue(repo, number)) => {
            let pick = Pick::Duplicate {
                issue: IssueAt {
                    repo: repo.clone(),
                    number: *number,
                },
                id: issue.clone(),
            };
            return Some(state.open_picker(crate::picker::Kind::Manage(pick)));
        }
        (Manage::Revert { pr: id }, Subject::Pr(pr)) => Revert {
            pr: pr.clone(),
            id: id.clone(),
        }
        .into(),
        (Manage::Transfer { issue, to, .. }, Subject::Issue(from, number)) => Transfer {
            from: IssueAt {
                repo: from.clone(),
                number: *number,
            },
            issue: issue.clone(),
            to: to.clone(),
        }
        .into(),
        _ => return None,
    };
    if let Some(Overlay::Confirm(c)) = &mut state.overlay {
        c.sent.sending = true;
        c.sent.error = None;
    }
    Some(work::start(work))
}

/// After GitHub took a change: a branch made for an issue is offered to
/// check out in your clone.
#[must_use]
pub fn after(state: &mut State, change: &Change) -> Vec<Cmd> {
    let Change::Manage(Manage::LinkBranch { repo, name, .. }) = change else {
        return Vec::new();
    };
    if let Some(None) = state.clone_of(repo) {
        state.info(format!(
            "Made {name}; ghtui isn't running in a clone of {repo} to check it out"
        ));
        return Vec::new();
    }
    let pick = Pick::Checkout {
        repo: repo.clone(),
        branch: name.clone(),
    };
    state.open_picker(crate::picker::Kind::Manage(pick))
}

// ---- a new discussion --------------------------------------------------------------

/// A new discussion being written: where, and in which category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Starting {
    pub repo: RepoId,
    pub repo_id: NodeId,
    pub category: NodeId,
    /// The category's name.
    pub name: String,
}

impl Starting {
    /// The composer's title.
    pub fn title(&self) -> String {
        format!(
            "New discussion in {} ({}): its title on the first line, then its text",
            self.repo, self.name
        )
    }
}

/// ctrl-s in the composer: starts the discussion.
#[must_use]
pub fn start(state: &mut State, s: &Starting, text: &str) -> Vec<Cmd> {
    let (title, body) = crate::create::templates::split(text);
    let Some(Overlay::Compose(c)) = &mut state.overlay else {
        return Vec::new();
    };
    if title.is_empty() {
        c.sent.error = Some("Write its title on the first line".into());
        return Vec::new();
    }
    if body.trim().is_empty() {
        c.sent.error = Some("Write its text below the title".into());
        return Vec::new();
    }
    c.sent.sending = true;
    c.sent.error = None;
    work::start(Start {
        repo: s.repo.clone(),
        repo_id: s.repo_id.clone(),
        category: s.category.clone(),
        title,
        body,
    })
}

// ---- work in the background --------------------------------------------------------

/// Reverting a merged pull request: GitHub opens a pull request that
/// does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revert {
    pub pr: PrRef,
    pub id: NodeId,
}

/// Moving an issue to another repository of its owner's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transfer {
    pub from: IssueAt,
    pub issue: NodeId,
    pub to: NodeId,
}

/// Starting a discussion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Start {
    pub repo: RepoId,
    pub repo_id: NodeId,
    pub category: NodeId,
    pub title: String,
    pub body: String,
}

/// Its pull request opens.
impl Job for Revert {
    type Out = u64;

    async fn run(&self, cx: &Cx) -> Result<u64, Failure> {
        Ok(cx.gh.revert(&self.id).await?)
    }

    fn done(self, state: &mut State, result: Result<u64, Failure>) -> Vec<Cmd> {
        let number = match result {
            Ok(number) => number,
            Err(why) => return refused(state, format!("GitHub refused the revert: {why}")),
        };
        close_confirm(state);
        state.info(format!("Opened #{number}, which reverts {}", self.pr));
        let made = PrRef {
            repo: self.pr.repo,
            number,
        };
        state.push(Route::pr(made))
    }
}

/// Its page, if it's on screen, moves with it.
impl Job for Transfer {
    type Out = IssueAt;

    async fn run(&self, cx: &Cx) -> Result<IssueAt, Failure> {
        Ok(cx.gh.transfer(&self.issue, &self.to).await?)
    }

    fn done(self, state: &mut State, result: Result<IssueAt, Failure>) -> Vec<Cmd> {
        let (from, to) = match result {
            Ok(to) => (self.from, to),
            Err(why) => return refused(state, format!("GitHub refused the transfer: {why}")),
        };
        close_confirm(state);
        state.info(format!("Moved {from} to {to}"));
        let here = Route::Issues(issues::Issue {
            repo: from.repo,
            number: from.number,
        });
        let there = Route::Issues(issues::Issue {
            repo: to.repo,
            number: to.number,
        });
        if state.route() == Some(&here) {
            state.replace(there, true, None)
        } else {
            Vec::new()
        }
    }
}

/// The new discussion opens.
impl Job for Start {
    type Out = u64;

    async fn run(&self, cx: &Cx) -> Result<u64, Failure> {
        let made = (cx.gh).start_discussion(&self.repo_id, &self.category, &self.title, &self.body);
        Ok(made.await?)
    }

    fn done(self, state: &mut State, result: Result<u64, Failure>) -> Vec<Cmd> {
        let number = match result {
            Ok(number) => number,
            Err(why) => {
                let text = format!("GitHub didn't start the discussion: {why}");
                match &mut state.overlay {
                    Some(Overlay::Compose(c)) => {
                        c.sent.sending = false;
                        c.sent.error = Some(text);
                    }
                    _ => state.error(text),
                }
                return Vec::new();
            }
        };
        if let Some(Overlay::Compose(c)) = &state.overlay
            && let crate::compose::Target::NewDiscussion(_) = c.target
        {
            state.overlay = None;
        }
        state.info(format!("Started discussion #{number}"));
        state.push(Route::Discussions(discussions::Discussion {
            of: DiscussionsOf::Repo(self.repo),
            number,
        }))
    }
}

fn close_confirm(state: &mut State) {
    if let Some(Overlay::Confirm(_)) = state.overlay {
        state.overlay = None;
    }
}

/// Says why GitHub refused, in the confirmation that asked when it's
/// still open.
fn refused(state: &mut State, text: String) -> Vec<Cmd> {
    match &mut state.overlay {
        Some(Overlay::Confirm(c)) if c.sent.sending => {
            c.sent.sending = false;
            c.sent.error = Some(text);
        }
        _ => state.error(text),
    }
    Vec::new()
}

// ---- following GitHub ------------------------------------------------------------

/// Whether a pull request's conversation is fetched again for `change`
/// too (its comments are there).
pub fn polls_activity(change: &Change) -> bool {
    matches!(
        change,
        Change::Manage(Manage::Hide { .. } | Manage::Unhide { .. })
    )
}

fn at(l: &crate::api::timeline::Linked) -> IssueAt {
    IssueAt {
        repo: l.repo.clone(),
        number: l.number,
    }
}

/// Whether what `about` shows, as loaded, shows `m`.
pub fn shows(state: &State, m: &Manage, about: &Subject) -> bool {
    match about {
        Subject::Pr(r) => {
            let Some(d) = pr(state, r) else {
                return false;
            };
            match m {
                Manage::DeleteBranch { .. } => d.head_gone,
                Manage::RestoreBranch { .. } => !d.head_gone,
                Manage::ConvertToDraft { .. } => d.state == PrState::Draft,
                Manage::ChangeBase { base, .. } => d.base_ref == *base,
                Manage::Lock { .. } => d.locked.is_some(),
                Manage::Unlock { .. } => d.locked.is_none(),
                Manage::Hide { comment, .. } | Manage::Unhide { comment } => {
                    let hide = matches!(m, Manage::Hide { .. });
                    state
                        .activity(r)
                        .is_none_or(|a| hidden(a.comments(), comment).is_none_or(|h| h == hide))
                }
                _ => true,
            }
        }
        Subject::Issue(repo, n) => {
            let Some(d) = issue(state, repo, *n) else {
                return false;
            };
            let has = |list: &Capped<crate::api::timeline::Linked>, other: &IssueAt| {
                list.iter().any(|l| at(l) == *other)
            };
            let place = |other: &IssueAt| d.sub_issues.iter().position(|l| at(l) == *other);
            match m {
                Manage::Lock { .. } => d.locked.is_some(),
                Manage::Unlock { .. } => d.locked.is_none(),
                Manage::Pin { on, .. } => d.pinned == *on,
                Manage::Hide { comment, .. } | Manage::Unhide { comment } => {
                    let hide = matches!(m, Manage::Hide { .. });
                    hidden(d.comments(), comment).is_none_or(|h| h == hide)
                }
                Manage::CloseAsDuplicate { .. } => d.state != IssueState::Open,
                // Past those loaded, it can't be seen: don't wait.
                Manage::SubIssue { child, add, .. } => {
                    (*add && d.sub_issues.left_out() > 0) || has(&d.sub_issues, child) == *add
                }
                Manage::BlockedBy { by, add, .. } => {
                    (*add && d.blocked_by.left_out() > 0) || has(&d.blocked_by, by) == *add
                }
                Manage::MoveSubIssue {
                    child,
                    sibling,
                    before,
                    ..
                } => match (place(child), place(sibling)) {
                    (Some(c), Some(s)) => (c < s) == *before,
                    _ => true,
                },
                Manage::LinkBranch { name, .. } => {
                    d.branches.left_out() > 0 || d.branches.iter().any(|b| b == name)
                }
                _ => true,
            }
        }
        Subject::Discussion(of, n) => {
            let Some(d) = discussion(state, of, *n) else {
                return false;
            };
            match m {
                Manage::Lock { .. } => d.locked.is_some(),
                Manage::Unlock { .. } => d.locked.is_none(),
                Manage::CloseDiscussion { .. } => d.closed.is_some(),
                Manage::ReopenDiscussion { .. } => d.closed.is_none(),
                Manage::Hide { comment, .. } | Manage::Unhide { comment } => {
                    let hide = matches!(m, Manage::Hide { .. });
                    let comments = d.answer.as_deref().into_iter().chain(&d.comments);
                    let all =
                        comments.flat_map(|c| std::iter::once(&c.comment).chain(c.replies.iter()));
                    hidden(all, comment).is_none_or(|h| h == hide)
                }
                _ => true,
            }
        }
        _ => true,
    }
}

/// Whether the comment `id` among `comments` is hidden; `None` if it
/// isn't among them.
fn hidden<'a>(
    mut comments: impl Iterator<Item = &'a crate::api::browse::Comment>,
    id: &NodeId,
) -> Option<bool> {
    comments
        .find(|c| c.editable.as_ref().is_some_and(|e| e.id == *id))
        .map(|c| c.hiding.reason.is_some())
}

/// `,x` on a discussion: close it with one of GitHub's reasons, or reopen
/// it; `Ok(None)` until it's loaded.
#[allow(clippy::type_complexity, reason = "act's own plan, in its parts")]
pub fn discussion_close(
    state: &State,
    of: &DiscussionsOf,
    number: u64,
) -> Result<Option<(String, Vec<(String, bool)>, Vec<(String, Change)>)>, String> {
    let Some(d) = discussion(state, of, number) else {
        return Ok(None);
    };
    let id = d.id.clone();
    match d.closed {
        None if !d.can_close => {
            Err("GitHub doesn't let you close it: that takes its author or triage access".into())
        }
        None => {
            let choices = (DiscussionReason::ALL.into_iter())
                .map(|reason| {
                    let change = Change::Manage(Manage::CloseDiscussion {
                        discussion: id.clone(),
                        reason,
                    });
                    (format!("Close as {}", reason.said()), change)
                })
                .collect();
            Ok(Some((format!("Close “{}”?", d.title), Vec::new(), choices)))
        }
        Some(_) if !d.can_reopen => Err("GitHub doesn't let you reopen it".into()),
        Some(_) => {
            let reopen = Change::Manage(Manage::ReopenDiscussion { discussion: id });
            Ok(Some((
                format!("Reopen “{}”?", d.title),
                Vec::new(),
                vec![("Reopen".into(), reopen)],
            )))
        }
    }
}

// ---- what changes say ------------------------------------------------------------

/// What it's doing while GitHub answers.
pub fn doing(m: &Manage) -> &'static str {
    match m {
        Manage::DeleteBranch { .. } => "Deleting the branch",
        Manage::RestoreBranch { .. } => "Restoring the branch",
        Manage::ConvertToDraft { .. } => "Converting it to a draft",
        Manage::ChangeBase { .. } => "Changing the base",
        Manage::Lock { .. } => "Locking the conversation",
        Manage::Unlock { .. } => "Unlocking the conversation",
        Manage::Pin { on: true, .. } => "Pinning it",
        Manage::Pin { .. } => "Unpinning it",
        Manage::Hide { .. } => "Hiding the comment",
        Manage::Unhide { .. } => "Unhiding the comment",
        Manage::CloseAsDuplicate { .. } | Manage::CloseDiscussion { .. } => "Closing",
        Manage::ReopenDiscussion { .. } => "Reopening",
        Manage::Revert { .. } => "Reverting it",
        Manage::Transfer { .. } => "Transferring it",
        Manage::SubIssue { add: true, .. } => "Adding the sub-issue",
        Manage::SubIssue { .. } => "Removing the sub-issue",
        Manage::MoveSubIssue { .. } => "Moving the sub-issue",
        Manage::BlockedBy { add: true, .. } => "Marking it blocked",
        Manage::BlockedBy { .. } => "Marking it no longer blocked",
        Manage::LinkBranch { .. } => "Creating the branch",
    }
}

/// What it did, once GitHub said so.
pub fn done(m: &Manage) -> String {
    match m {
        Manage::DeleteBranch { branch, .. } => format!("Deleted {branch}"),
        Manage::RestoreBranch { branch, .. } => format!("Restored {branch}"),
        Manage::ConvertToDraft { .. } => "Converted to a draft".into(),
        Manage::ChangeBase { base, .. } => format!("Its base is {base}"),
        Manage::Lock {
            reason: Some(reason),
            ..
        } => format!(
            "Locked as {}: only collaborators can comment",
            crate::api::manage::lock_said(*reason)
        ),
        Manage::Lock { .. } => "Locked: only collaborators can comment".into(),
        Manage::Unlock { .. } => "Unlocked".into(),
        Manage::Pin { on: true, .. } => "Pinned".into(),
        Manage::Pin { .. } => "Unpinned".into(),
        Manage::Hide { reason, .. } => format!("Hidden as {}", reason.said()),
        Manage::Unhide { .. } => "Unhidden".into(),
        Manage::CloseAsDuplicate { of: Some(of), .. } => format!("Closed as a duplicate of {of}"),
        Manage::CloseAsDuplicate { .. } => "Closed as a duplicate".into(),
        Manage::CloseDiscussion { reason, .. } => format!("Closed as {}", reason.said()),
        Manage::ReopenDiscussion { .. } => "Reopened".into(),
        Manage::Revert { .. } => "Reverted".into(),
        Manage::Transfer { repo, .. } => format!("Transferred to {repo}"),
        Manage::SubIssue {
            child, add: true, ..
        } => format!("Added {child} as a sub-issue"),
        Manage::SubIssue { child, .. } => format!("Removed {child} from the sub-issues"),
        Manage::MoveSubIssue { child, .. } => format!("Moved {child}"),
        Manage::BlockedBy { by, add: true, .. } => format!("Blocked by {by}"),
        Manage::BlockedBy { by, .. } => format!("No longer blocked by {by}"),
        Manage::LinkBranch { name, .. } => format!("Created {name}"),
    }
}

/// What it is, for "GitHub refused …".
pub fn what(m: &Manage) -> &'static str {
    match m {
        Manage::DeleteBranch { .. } => "deleting the branch",
        Manage::RestoreBranch { .. } => "restoring the branch",
        Manage::ConvertToDraft { .. } => "converting it to a draft",
        Manage::ChangeBase { .. } => "changing the base",
        Manage::Lock { .. } => "locking it",
        Manage::Unlock { .. } => "unlocking it",
        Manage::Pin { on: true, .. } => "pinning it",
        Manage::Pin { .. } => "unpinning it",
        Manage::Hide { .. } => "hiding the comment",
        Manage::Unhide { .. } => "unhiding the comment",
        Manage::CloseAsDuplicate { .. } | Manage::CloseDiscussion { .. } => "closing it",
        Manage::ReopenDiscussion { .. } => "reopening it",
        Manage::Revert { .. } => "the revert",
        Manage::Transfer { .. } => "the transfer",
        Manage::SubIssue { add: true, .. } => "adding the sub-issue",
        Manage::SubIssue { .. } => "removing the sub-issue",
        Manage::MoveSubIssue { .. } => "moving the sub-issue",
        Manage::BlockedBy { .. } => "changing what blocks it",
        Manage::LinkBranch { .. } => "creating the branch",
    }
}

/// The permission a fine-grained token needs for it.
pub fn needs(m: &Manage) -> &'static str {
    match m {
        Manage::DeleteBranch { .. }
        | Manage::RestoreBranch { .. }
        | Manage::LinkBranch { .. }
        | Manage::Revert { .. } => "Contents: write",
        Manage::ConvertToDraft { .. } | Manage::ChangeBase { .. } => "Pull requests: write",
        Manage::CloseDiscussion { .. } | Manage::ReopenDiscussion { .. } => "Discussions: write",
        _ => "Issues or Pull requests: write",
    }
}
