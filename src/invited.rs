//! The invitations sent to you: to collaborate on a repository (accepted
//! or declined), and to join an organization (accepted: GitHub's API
//! can't decline one). They're listed at the top of your notifications,
//! and each has its page, at the website's link (`/owner/repo/invitations`,
//! `/orgs/org/invitation`). Answering asks first, as GitHub's page asks
//! you to choose, goes to GitHub as a [`Change::Org`], and the page
//! follows GitHub until the invitation is gone ([`shows`]).

use crate::act::{Step, Subject};
use crate::api::change::Change;
use crate::api::org::{Invitations, OrgChange};
use crate::browse::Build;
use crate::data::DataKey;
use crate::keymap::Action;
use crate::nav::Doable;
use crate::route::people::{self, Invite};
use crate::route::{Dest, Route, Target, search};
use crate::state::{Cmds, State};
use crate::ui::page::{Link, Page};

/// What the invitations add: answering one, on its page or its row.
pub const FEATURE: crate::features::Feature = crate::features::Feature {
    owns: &[Action::AnswerInvitation],
    act: |state, _| answer(state, None),
    offers: doables,
    ..crate::features::Feature::NONE
};

/// Where an invitation was answered: the inbox (by its filter), or its
/// page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum At {
    Inbox(String),
    Page(Invite),
}

impl At {
    pub fn route(&self) -> Route {
        match self {
            At::Inbox(query) => Route::Search(search::Notifications {
                query: query.clone(),
            }),
            At::Page(invite) => Route::People(people::Invitation(invite.clone())),
        }
    }
}

/// The invitation on screen: its page's, or the inbox's selected row's.
fn here(state: &State) -> Option<(At, Invite)> {
    match state.route()? {
        Route::People(people::Invitation(invite)) => {
            Some((At::Page(invite.clone()), invite.clone()))
        }
        Route::Search(search::Notifications { query }) => {
            let url = state.screen().page()?.selected_link()?.url()?;
            match Dest::from_url(url).target {
                Target::Page(Route::People(people::Invitation(invite))) => {
                    Some((At::Inbox(query.clone()), invite))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn invitations(state: &State) -> Option<&Invitations> {
    state.picked(&DataKey::Invitations)
}

/// Answers the invitation on screen, asking first; `accept` puts that
/// answer first (a page's buttons), else accepting is.
pub fn answer(state: &mut State, accept: Option<bool>) -> Cmds {
    crate::act::take(state, plan(state, accept))
}

fn plan(state: &State, accept: Option<bool>) -> Result<Step, String> {
    let (at, invite) = here(state)
        .ok_or("That answers an invitation sent to you: from your notifications, or its page")?;
    crate::act::settled(state)?;
    let all = crate::org::loaded::<Invitations>(state, &DataKey::Invitations, "your invitations")?;
    let action = Action::AnswerInvitation;
    let about = Subject::Invited(at);
    match invite {
        Invite::Repo(repo) => {
            let i = (all.repos.iter())
                .find(|i| i.repo == repo)
                .ok_or_else(|| format!("No invitation to {repo} is waiting for you"))?;
            let by = i
                .inviter
                .as_deref()
                .map_or_else(String::new, |l| format!(" by @{l}"));
            if i.expired {
                return Err(format!(
                    "The invitation to {repo}{by} has expired: they can invite you again"
                ));
            }
            let can = i.permission.map_or_else(
                || "You'll have the access it gives".to_owned(),
                |p| format!("With {} access, you can {}", p.name(), p.about()),
            );
            let facts = vec![(format!("You were invited{by}"), false), (can, false)];
            let change = |accept| {
                Change::Org(OrgChange::AnswerRepo {
                    id: i.id,
                    repo: repo.clone(),
                    accept,
                })
            };
            let mut choices = vec![
                ("Accept invitation".to_owned(), change(true)),
                ("Decline".to_owned(), change(false)),
            ];
            if accept == Some(false) {
                choices.reverse();
            }
            let title = format!("Join {repo}?");
            let ask = crate::act::confirm(action, about, title, facts, choices);
            Ok(Step::Ask(Box::new(ask)))
        }
        Invite::Org(org) => {
            let orgs = all.orgs.as_ref().map_err(Clone::clone)?;
            let i = (orgs.iter())
                .find(|i| i.org.eq_ignore_ascii_case(&org))
                .ok_or_else(|| format!("No invitation to @{org} is waiting for you"))?;
            let decline = || {
                let open = state.first_key(Action::OpenInBrowser);
                format!(
                    "GitHub's API can't decline an organization's invitation: that's on github.com ({open} opens it)"
                )
            };
            if accept == Some(false) {
                return Err(decline());
            }
            let role = i.role.name().to_lowercase();
            let facts = vec![
                (
                    format!(
                        "As {} {role}, you {}",
                        if role == "owner" { "an" } else { "a" },
                        i.role.about()
                    ),
                    false,
                ),
                (decline(), false),
            ];
            let choices = vec![(
                format!("Join @{org}"),
                Change::Org(OrgChange::JoinOrg { org: i.org.clone() }),
            )];
            let title = format!("Join @{org}?");
            let ask = crate::act::confirm(action, about, title, facts, choices);
            Ok(Step::Ask(Box::new(ask)))
        }
    }
}

/// The menu's row, where there's an invitation to answer.
fn doables(state: &State, out: &mut Vec<Doable>) {
    if here(state).is_none() {
        return;
    }
    let why = plan(state, None).err();
    let row = crate::nav::row(
        "Change",
        Action::AnswerInvitation,
        "Accept or decline the invitation",
        "answer",
        why,
    );
    out.push(row);
}

/// Whether the page shows `c`: the invitation answered is gone.
pub fn shows(state: &State, c: &OrgChange) -> bool {
    let Some(all) = invitations(state) else { return false };
    match c {
        OrgChange::AnswerRepo { id, .. } => all.repos.iter().all(|i| i.id != *id),
        OrgChange::JoinOrg { org } => !(all.orgs.as_ref())
            .is_ok_and(|orgs| orgs.iter().any(|i| i.org.eq_ignore_ascii_case(org))),
        _ => true,
    }
}

/// An invitation's page.
pub fn build(b: Build<'_>, page: &mut Page, invite: &Invite) {
    let Some(all) = b
        .get::<Invitations>(&DataKey::Invitations)
        .show(page, "your invitations")
    else {
        return;
    };
    crate::ui::pages::invitation(page, invite, all, b.cx.now);
}

/// Following one of an invitation page's buttons.
pub fn follow_link(state: &mut State, accept: bool) -> Cmds {
    answer(state, Some(accept))
}

/// The page's buttons, as links.
pub fn button(accept: bool) -> Link {
    Link::Setting(crate::ui::pages::SettingLink::Answer(accept))
}

/// Invitations as GitHub might send them, for tests.
#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    use crate::api::model::RepoId;
    use crate::api::org::{OrgInvited, OrgRole, RepoInvitation};
    use crate::api::settings::Permission;

    /// No invitations waiting.
    pub fn none() -> Invitations {
        Invitations {
            repos: Vec::new(),
            orgs: Ok(Vec::new()),
        }
    }

    /// One repository's invitation, from octocat with write access, and
    /// one organization's.
    pub fn invitations() -> Invitations {
        Invitations {
            repos: vec![RepoInvitation {
                id: 77,
                repo: RepoId::new("octocat", "secret-plans"),
                inviter: Some("octocat".into()),
                permission: Some(Permission::Write),
                created_at: "2026-10-01T00:00:00Z".into(),
                expired: false,
            }],
            orgs: Ok(vec![OrgInvited {
                org: "octo-org".into(),
                role: OrgRole::Member,
            }]),
        }
    }
}
