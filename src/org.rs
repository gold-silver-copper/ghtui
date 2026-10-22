//! An organization's people and teams, changed as on the website: its
//! owners invite members (and cancel invitations), change a member's role
//! and remove them, on its People tab; whoever GitHub lets makes a team on
//! its Teams page; a team's maintainers (and the owners) add and remove
//! its members and repositories, and change their roles, on its page.
//!
//! The keys are the Settings tab's (`a` adds, `e` changes a role, `D`
//! removes), taken on these pages ([`here`]). Each change asks first,
//! goes to GitHub as a [`Change::Org`], and the page follows GitHub until
//! it shows it ([`shows`]). What GitHub doesn't let you do, or your
//! token's scopes don't, is said, with the command that adds the scope.

use crate::act::{Step, Subject};
use crate::api::browse::{Results, TeamDetail, TeamSummary};
use crate::api::change::Change;
use crate::api::model::RepoId;
use crate::api::org::{Member, OrgAccess, OrgChange, OrgRole};
use crate::api::settings::Permission;
use crate::data::DataKey;
use crate::keymap::Action;
use crate::nav::Doable;
use crate::route::{Dest, Route, Target, people};
use crate::state::{Cmds, State};
use crate::ui::overlays::PaletteItem;
use crate::ui::page::Link;
use crate::ui::pages::{ProfileTab, SettingLink};

/// The actions this module takes on its pages: the Settings tab's.
const TAKES: [Action; 3] = [
    Action::AddSetting,
    Action::EditSetting,
    Action::DeleteSetting,
];

/// What an organization's pages add: inviting, roles and removing on its
/// People tab, a new team on its Teams page, a team's members and
/// repositories on its page.
pub const FEATURE: crate::features::Feature = crate::features::Feature {
    takes: |state, action| TAKES.contains(&action) && state.route().and_then(here).is_some(),
    act,
    offers: doables,
    ..crate::features::Feature::NONE
};

/// One of an organization's pages its people and teams are changed on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum At {
    /// Its People tab: members, and (for its owners) invitations.
    People(String),
    Teams(String),
    Team(String, String),
}

/// Where `route` is among an organization's pages, if it's one.
pub fn here(route: &Route) -> Option<At> {
    Some(match route {
        Route::People(people::User {
            login,
            tab: ProfileTab::People,
        }) => At::People(login.clone()),
        Route::People(people::Teams(org)) => At::Teams(org.clone()),
        Route::People(people::Team { org, slug }) => At::Team(org.clone(), slug.clone()),
        _ => return None,
    })
}

impl At {
    fn org(&self) -> &str {
        match self {
            At::People(org) | At::Teams(org) | At::Team(org, _) => org,
        }
    }

    /// Its page.
    pub fn route(&self) -> Route {
        Route::People(match self {
            At::People(org) => people::User {
                login: org.clone(),
                tab: ProfileTab::People,
            },
            At::Teams(org) => people::Teams(org.clone()),
            At::Team(org, slug) => people::Team {
                org: org.clone(),
                slug: slug.clone(),
            },
        })
    }

    /// What shows a change there.
    pub fn need(&self) -> DataKey {
        match self {
            At::People(org) => DataKey::OrgAccess(org.to_lowercase()),
            At::Teams(org) => DataKey::Teams(org.to_lowercase()),
            At::Team(org, slug) => DataKey::Team(org.to_lowercase(), slug.clone()),
        }
    }

    /// What else shows it: a People tab's members.
    pub fn also(&self) -> Option<DataKey> {
        match self {
            At::People(org) => Some(DataKey::Members(org.to_lowercase())),
            At::Teams(_) | At::Team(..) => None,
        }
    }

    pub fn name(&self) -> String {
        match self {
            At::People(org) => format!("@{org}'s people"),
            At::Teams(org) => format!("@{org}'s teams"),
            At::Team(org, slug) => format!("@{org}/{slug}"),
        }
    }
}

// ---- what's loaded ------------------------------------------------------------------------

const LOADING: &str = "It's still loading";

/// `key`'s data, or why there's none: it's loading, or it couldn't load.
pub(crate) fn loaded<'a, T: crate::data::Picked + ?Sized>(
    state: &'a State,
    key: &DataKey,
    what: &str,
) -> Result<&'a T, String> {
    if let Some(found) = state.picked::<T>(key) {
        return Ok(found);
    }
    match state.remote(key) {
        Some(r) if !r.loading && r.error.is_some() => Err(format!(
            "Couldn't load {what}: {}",
            r.error.as_deref().unwrap_or_default()
        )),
        _ => Err(LOADING.into()),
    }
}

fn access<'a>(state: &'a State, org: &str) -> Option<&'a OrgAccess> {
    state.picked(&DataKey::OrgAccess(org.to_lowercase()))
}

/// What you may do in `org`, or why it isn't known.
fn may<'a>(state: &'a State, org: &str) -> Result<&'a OrgAccess, String> {
    let what = format!("what you may do in @{org}");
    loaded(state, &DataKey::OrgAccess(org.to_lowercase()), &what)
}

fn members<'a>(state: &'a State, org: &str) -> Option<&'a Results<Member>> {
    state.picked(&DataKey::Members(org.to_lowercase()))
}

fn team<'a>(state: &'a State, org: &str, slug: &str) -> Option<&'a TeamDetail> {
    state.picked(&DataKey::Team(org.to_lowercase(), slug.to_owned()))
}

fn teams<'a>(state: &'a State, org: &str) -> Option<&'a Results<TeamSummary>> {
    state.picked(&DataKey::Teams(org.to_lowercase()))
}

/// What the selected row is.
enum Row {
    Person(String),
    Repo(RepoId),
    /// An organization's invitation: its ID, and whom it's to.
    Invitation(u64, Option<String>),
    /// A child team, or something else that isn't changed here.
    Other,
}

fn selected(state: &State) -> Option<Row> {
    let link = state.screen().page()?.selected_link()?;
    Some(match link {
        Link::Setting(SettingLink::OrgInvitation(id, login)) => Row::Invitation(*id, login.clone()),
        Link::Url(url) => match Dest::from_url(url).target {
            Target::Page(Route::People(people::User { login, .. })) => Row::Person(login),
            Target::Page(Route::Code(crate::route::code::Repo(repo))) => Row::Repo(repo),
            _ => Row::Other,
        },
        _ => Row::Other,
    })
}

fn select(state: &State, what: &str) -> String {
    let next = state.first_key(Action::Down);
    format!("Select {what} first: {next} goes row by row")
}

// ---- what a key does --------------------------------------------------------------------

/// Runs one of the actions it takes.
fn act(state: &mut State, action: Action) -> Cmds {
    crate::act::take(state, plan(state, action))
}

/// Asking first about one change.
fn ask(
    action: Action,
    at: &At,
    title: String,
    facts: Vec<(String, bool)>,
    choice: (&str, OrgChange),
) -> Step {
    let about = Subject::Org(at.clone());
    crate::act::ask_one(action, about, title, facts, choice.0, Change::Org(choice.1))
}

#[expect(clippy::unnecessary_wraps, reason = "a plan's step")]
fn pick(pick: Pick) -> Result<Step, String> {
    Ok(Step::Pick(crate::picker::Kind::Org(pick)))
}

fn plan(state: &State, action: Action) -> Result<Step, String> {
    let at = state
        .route()
        .and_then(here)
        .ok_or("That works on an organization's people and teams")?;
    crate::act::settled(state)?;
    match &at {
        At::People(org) => people_plan(state, &at, org, action),
        At::Teams(org) => teams_plan(state, org, action),
        At::Team(org, slug) => team_plan(state, &at, org, slug, action),
    }
}

/// Why you can't change `org`'s people: you don't own it, or your token
/// can't.
fn owner(state: &State, org: &str) -> Result<(), String> {
    let a = may(state, org)?;
    if !a.admin {
        return Err(format!(
            "Changing @{org}'s people takes being one of its owners, which GitHub doesn't say you are"
        ));
    }
    a.lacks.clone().map_or(Ok(()), Err)
}

fn people_plan(state: &State, at: &At, org: &str, action: Action) -> Result<Step, String> {
    owner(state, org)?;
    if action == Action::AddSetting {
        return pick(Pick::Invite {
            org: org.to_owned(),
        });
    }
    match (action, selected(state)) {
        (Action::EditSetting, Some(Row::Person(login))) => {
            let now = (members(state, org).ok_or(LOADING)?.items.iter())
                .find(|m| m.login.eq_ignore_ascii_case(&login))
                .and_then(|m| m.role);
            pick(Pick::Role {
                org: org.to_owned(),
                login,
                now,
            })
        }
        (Action::EditSetting, Some(Row::Invitation(..))) => {
            let delete = state.first_key(Action::DeleteSetting);
            Err(format!(
                "GitHub doesn't change an invitation's role: cancel it ({delete}) and invite them again"
            ))
        }
        (Action::DeleteSetting, Some(Row::Person(login))) => {
            let mut facts = vec![(
                format!("They lose access to @{org}'s repositories and teams"),
                true,
            )];
            if state
                .viewer
                .as_deref()
                .is_some_and(|me| me.eq_ignore_ascii_case(&login))
            {
                facts.push(("That's you: you leave it".into(), true));
            }
            Ok(ask(
                action,
                at,
                format!("Remove @{login} from @{org}?"),
                facts,
                (
                    "Remove",
                    OrgChange::RemoveMember {
                        org: org.to_owned(),
                        login,
                    },
                ),
            ))
        }
        (Action::DeleteSetting, Some(Row::Invitation(id, login))) => {
            let invitations = access(state, org).and_then(|a| a.invitations.as_ref());
            let who = (invitations.and_then(|i| i.as_ref().ok()))
                .and_then(|l| l.items.iter().find(|i| i.id == id))
                .map_or_else(
                    || login.map(|l| format!("@{l}")).unwrap_or_default(),
                    crate::api::org::OrgInvitation::who,
                );
            Ok(ask(
                action,
                at,
                format!("Cancel the invitation to {who}?"),
                vec![(format!("They won't be able to join @{org} with it"), false)],
                (
                    "Cancel invitation",
                    OrgChange::CancelInvitation {
                        org: org.to_owned(),
                        id,
                        who,
                    },
                ),
            ))
        }
        _ => Err(select(state, "a member or an invitation")),
    }
}

fn teams_plan(state: &State, org: &str, action: Action) -> Result<Step, String> {
    if action != Action::AddSetting {
        return Err("A team's members and repositories are changed on its page: ↵ opens it".into());
    }
    let a = may(state, org)?;
    if !a.create_teams {
        return Err(format!("GitHub doesn't let you make teams in @{org}"));
    }
    if let Some(lacks) = &a.lacks {
        return Err(lacks.clone());
    }
    pick(Pick::TeamName {
        org: org.to_owned(),
    })
}

fn team_plan(
    state: &State,
    at: &At,
    org: &str,
    slug: &str,
    action: Action,
) -> Result<Step, String> {
    let key = DataKey::Team(org.to_lowercase(), slug.to_owned());
    let t: &TeamDetail = loaded(state, &key, "the team")?;
    if !t.can_admin {
        return Err(format!(
            "Changing @{org}/{slug} takes maintaining it, or owning @{org}, which GitHub doesn't say you do"
        ));
    }
    if let Some(lacks) = access(state, org).and_then(|a| a.lacks.clone()) {
        return Err(lacks);
    }
    let (org, team) = (org.to_owned(), slug.to_owned());
    if action == Action::AddSetting {
        return pick(Pick::AddTo { org, team });
    }
    match (action, selected(state)) {
        (Action::EditSetting, Some(Row::Person(login))) => {
            let now = Some(t.maintainers.iter().any(|m| m.eq_ignore_ascii_case(&login)));
            pick(Pick::TeamRole {
                org,
                team,
                login,
                now,
            })
        }
        (Action::EditSetting, Some(Row::Repo(repo))) => {
            let now = (t.repos.iter())
                .find(|r| r.repo == repo)
                .and_then(|r| r.permission);
            pick(Pick::RepoRole {
                org,
                team,
                repo,
                now: Some(now),
            })
        }
        (Action::DeleteSetting, Some(Row::Person(login))) => Ok(ask(
            action,
            at,
            format!("Remove @{login} from @{org}/{team}?"),
            vec![(
                format!("They lose what @{org}/{team} gives them, and stay in @{org}"),
                true,
            )],
            (
                "Remove from team",
                OrgChange::RemoveTeamMember { org, team, login },
            ),
        )),
        (Action::DeleteSetting, Some(Row::Repo(repo))) => Ok(ask(
            action,
            at,
            format!("Remove {repo} from @{org}/{team}?"),
            vec![(
                format!("@{org}/{team}'s members lose the access the team gave them to it"),
                true,
            )],
            (
                "Remove repository",
                OrgChange::RemoveTeamRepo { org, team, repo },
            ),
        )),
        _ => Err(select(state, "a member or a repository")),
    }
}

/// The menu's rows on an organization's pages.
fn doables(state: &State, out: &mut Vec<Doable>) {
    let Some(at) = state.route().and_then(here) else { return };
    let row = |action, label: &'static str, short: &'static str| {
        crate::nav::row("Change", action, label, short, plan(state, action).err())
    };
    match at {
        At::People(_) => out.extend([
            row(Action::AddSetting, "Invite a member", "invite"),
            row(Action::EditSetting, "Change their role", "role"),
            row(
                Action::DeleteSetting,
                "Remove them, or cancel the invitation",
                "",
            ),
        ]),
        At::Teams(_) => out.push(row(Action::AddSetting, "New team", "new team")),
        At::Team(..) => out.extend([
            row(Action::AddSetting, "Add a member or a repository", "add"),
            row(
                Action::EditSetting,
                "Change their role, or the team's on the repository",
                "role",
            ),
            row(
                Action::DeleteSetting,
                "Remove them, or the repository, from the team",
                "",
            ),
        ]),
    }
}

/// The key hints a page of these names, as they're bound.
pub fn hint(state: &State, at: &At) -> String {
    let [add, edit, delete] = TAKES.map(|a| state.first_key(a));
    match at {
        At::People(_) => {
            format!(
                "{add} invites someone, {edit} changes the selected one's role, {delete} removes them"
            )
        }
        At::Teams(_) => format!("{add} makes a team"),
        At::Team(..) => format!(
            "{add} adds a member or a repository, {edit} changes the selected one's role, {delete} removes it"
        ),
    }
}

// ---- pickers ------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// Someone to invite, typed.
    Invite { org: String },
    /// A role in the organization for someone: a member's new one, or
    /// (`now` none and not a member) an invitation's.
    Role {
        org: String,
        login: String,
        now: Option<OrgRole>,
    },
    /// A new team's name, typed.
    TeamName { org: String },
    /// A new team's visibility.
    Privacy { org: String, name: String },
    /// Someone or a repository to add to a team, typed.
    AddTo { org: String, team: String },
    /// A role in a team (maintainer or not; `now` is theirs, `None` for
    /// someone added).
    TeamRole {
        org: String,
        team: String,
        login: String,
        now: Option<bool>,
    },
    /// The team's role on a repository (`now`: it has access, with this).
    RepoRole {
        org: String,
        team: String,
        repo: RepoId,
        now: Option<Option<Permission>>,
    },
}

impl Pick {
    pub fn title(&self) -> &'static str {
        match self {
            Pick::Invite { .. } => "Invite a member",
            Pick::Role { now: None, .. } => "Invite them as",
            Pick::TeamRole { now: None, .. } => "Add them as",
            Pick::Role { .. } | Pick::TeamRole { .. } => "Change their role to",
            Pick::TeamName { .. } => "New team",
            Pick::Privacy { .. } => "Who can see it",
            Pick::AddTo { .. } => "Add to the team",
            Pick::RepoRole { now: None, .. } => "Give the team",
            Pick::RepoRole { .. } => "Change the team's role to",
        }
    }

    pub fn placeholder(&self) -> &'static str {
        match self {
            Pick::Invite { .. } => "Their GitHub login",
            Pick::TeamName { .. } => "The team's name",
            Pick::AddTo { .. } => "A login, or a repository (name or owner/name)",
            Pick::Role { .. }
            | Pick::Privacy { .. }
            | Pick::TeamRole { .. }
            | Pick::RepoRole { .. } => "Choose",
        }
    }
}

fn next(pick: Pick) -> Option<Step> {
    Some(Step::Pick(crate::picker::Kind::Org(pick)))
}

/// A login as typed, without its `@`, if it can be one.
fn login(q: &str) -> Result<&str, String> {
    let login = q.trim().trim_start_matches('@');
    if login.is_empty() {
        Err("Type their GitHub login".into())
    } else if crate::route::valid_login(login) {
        Ok(login)
    } else {
        Err(format!("“{login}” isn't a GitHub login"))
    }
}

/// A picker's rows: what each says, and what choosing it does.
pub fn rows(state: &State, pick: &Pick, q: &str) -> Vec<(PaletteItem, Option<Step>)> {
    let row =
        |label: String, hint: &str, step: Option<Step>| (crate::picker::item(label, hint), step);
    match pick {
        Pick::Invite { org } => match login(q) {
            Err(why) => vec![row(why, "", None)],
            Ok(login) => {
                let member = members(state, org)
                    .and_then(|m| m.items.iter().find(|m| m.login.eq_ignore_ascii_case(login)));
                if member.is_some() {
                    let edit = state.first_key(Action::EditSetting);
                    return vec![row(
                        format!("@{login} is a member already: {edit} on them changes their role"),
                        "",
                        None,
                    )];
                }
                let then = Pick::Role {
                    org: org.clone(),
                    login: login.to_owned(),
                    now: None,
                };
                vec![row(
                    format!("Invite @{login}"),
                    "then choose their role",
                    next(then),
                )]
            }
        },
        Pick::Role { org, login, now } => (OrgRole::ALL.into_iter())
            .filter(|r| crate::picker::fuzzy_score(q, r.name()).is_some())
            .map(|r| {
                if *now == Some(r) {
                    return row(format!("✓ {} (now)", r.name()), r.about(), None);
                }
                let at = At::People(org.clone());
                let invite = now.is_none();
                let (title, fact, label) = if invite {
                    (
                        format!("Invite @{login} to @{org} as {}?", article(r)),
                        format!("GitHub emails them; once they accept, they {}", r.about()),
                        "Invite",
                    )
                } else {
                    (
                        format!("Make @{login} {} of @{org}?", article(r)),
                        format!("They {}", r.about()),
                        "Change role",
                    )
                };
                let change = OrgChange::SetMember {
                    org: org.clone(),
                    login: login.clone(),
                    role: r,
                    invite,
                };
                let fact = (fact, r == OrgRole::Owner);
                let step = ask(Action::EditSetting, &at, title, vec![fact], (label, change));
                row(r.name().to_owned(), r.about(), Some(step))
            })
            .collect(),
        Pick::TeamName { org } => {
            let name = q.trim();
            if name.is_empty() {
                return vec![row("Type the team's name".into(), "", None)];
            }
            let taken = teams(state, org)
                .is_some_and(|t| t.items.iter().any(|t| t.name.eq_ignore_ascii_case(name)));
            if taken {
                return vec![row(
                    format!("@{org} has a team named {name} already"),
                    "",
                    None,
                )];
            }
            let then = Pick::Privacy {
                org: org.clone(),
                name: name.to_owned(),
            };
            vec![row(
                format!("New team “{name}”"),
                "then who can see it",
                next(then),
            )]
        }
        Pick::Privacy { org, name } => [
            (
                false,
                "Visible",
                "every member of the organization can see it",
            ),
            (
                true,
                "Secret",
                "only its members and the organization's owners can see it",
            ),
        ]
        .into_iter()
        .filter(|(_, label, _)| crate::picker::fuzzy_score(q, label).is_some())
        .map(|(secret, label, about)| {
            let at = At::Teams(org.clone());
            let change = OrgChange::CreateTeam {
                org: org.clone(),
                name: name.clone(),
                secret,
            };
            let title = format!("Make the team “{name}” in @{org}?");
            let fact = (format!("{label}: {about}"), false);
            let step = ask(
                Action::AddSetting,
                &at,
                title,
                vec![fact],
                ("Create team", change),
            );
            row(label.to_owned(), about, Some(step))
        })
        .collect(),
        Pick::AddTo { org, team: slug } => add_rows(state, org, slug, q),
        Pick::TeamRole {
            org,
            team: slug,
            login,
            now,
        } => [(false, "Member"), (true, "Maintainer")]
            .into_iter()
            .filter(|(_, label)| crate::picker::fuzzy_score(q, label).is_some())
            .map(|(maintainer, label)| {
                let about = if maintainer {
                    "can add and remove its members, and change its name and description"
                } else {
                    "has the team's access to its repositories"
                };
                if *now == Some(maintainer) {
                    return row(format!("✓ {label} (now)"), about, None);
                }
                let at = At::Team(org.clone(), slug.clone());
                let add = now.is_none();
                let title = if add {
                    format!(
                        "Add @{login} to @{org}/{slug} as a {}?",
                        label.to_lowercase()
                    )
                } else {
                    format!("Make @{login} a {} of @{org}/{slug}?", label.to_lowercase())
                };
                let mut facts = vec![(format!("A {}: {about}", label.to_lowercase()), false)];
                let outside = members(state, org).is_some_and(|m| {
                    m.next.is_none() && !m.items.iter().any(|m| m.login.eq_ignore_ascii_case(login))
                });
                if add && outside {
                    facts.push((
                        format!("They aren't in @{org}: GitHub invites them to it"),
                        true,
                    ));
                }
                let change = OrgChange::SetTeamMember {
                    org: org.clone(),
                    team: slug.clone(),
                    login: login.clone(),
                    maintainer,
                    add,
                };
                let choice = if add { "Add to team" } else { "Change role" };
                let step = ask(Action::EditSetting, &at, title, facts, (choice, change));
                row(label.to_owned(), about, Some(step))
            })
            .collect(),
        Pick::RepoRole {
            org,
            team: slug,
            repo,
            now,
        } => (Permission::ALL.into_iter())
            .filter(|p| crate::picker::fuzzy_score(q, p.name()).is_some())
            .map(|p| {
                if *now == Some(Some(p)) {
                    return row(format!("✓ {} (now)", p.name()), p.about(), None);
                }
                let at = At::Team(org.clone(), slug.clone());
                let add = now.is_none();
                let title = if add {
                    format!("Give @{org}/{slug} {} access to {repo}?", p.name())
                } else {
                    format!("Change @{org}/{slug}'s access to {repo} to {}?", p.name())
                };
                let fact = (
                    format!("Its members can {}", p.about()),
                    p == Permission::Admin,
                );
                let change = OrgChange::SetTeamRepo {
                    org: org.clone(),
                    team: slug.clone(),
                    repo: repo.clone(),
                    permission: p,
                    add,
                };
                let choice = if add { "Add repository" } else { "Change role" };
                let step = ask(
                    Action::EditSetting,
                    &at,
                    title,
                    vec![fact],
                    (choice, change),
                );
                row(p.name().to_owned(), p.about(), Some(step))
            })
            .collect(),
    }
}

/// "a member", "an owner".
fn article(r: OrgRole) -> String {
    match r {
        OrgRole::Owner => "an owner".into(),
        OrgRole::Member => "a member".into(),
    }
}

/// What can be added to a team for what's typed: someone, or one of the
/// organization's repositories (or another's, named in full).
fn add_rows(state: &State, org: &str, slug: &str, q: &str) -> Vec<(PaletteItem, Option<Step>)> {
    let q = q.trim();
    let row =
        |label: String, hint: &str, step: Option<Step>| (crate::picker::item(label, hint), step);
    if q.is_empty() {
        return vec![row("Type a login, or a repository's name".into(), "", None)];
    }
    let t = team(state, org, slug);
    let mut out = Vec::new();
    let repo = if q.contains('/') {
        RepoId::parse(q)
    } else {
        RepoId::parse(&format!("{org}/{q}"))
    };
    if !q.contains('/')
        && let Ok(login) = login(q)
    {
        let there = t.is_some_and(|t| {
            t.members
                .iter()
                .any(|m| m.login.eq_ignore_ascii_case(login))
        });
        if there {
            out.push(row(format!("@{login} is in the team already"), "", None));
        } else {
            let then = Pick::TeamRole {
                org: org.to_owned(),
                team: slug.to_owned(),
                login: login.to_owned(),
                now: None,
            };
            out.push(row(
                format!("Add @{login}"),
                "then choose their role",
                next(then),
            ));
        }
    }
    if let Some(repo) = repo.filter(|_| !q.starts_with('@')) {
        let there = t.is_some_and(|t| t.repos.iter().any(|r| r.repo == repo));
        if there {
            out.push(row(format!("{repo} is the team's already"), "", None));
        } else {
            let then = Pick::RepoRole {
                org: org.to_owned(),
                team: slug.to_owned(),
                repo: repo.clone(),
                now: None,
            };
            out.push(row(
                format!("Give it access to {repo}"),
                "then choose its role",
                next(then),
            ));
        }
    }
    if out.is_empty() {
        out.push(row(
            format!("“{q}” is neither a login nor a repository"),
            "",
            None,
        ));
    }
    out
}

// ---- following GitHub ---------------------------------------------------------------

/// Whether the page `at`, as loaded, shows `c`. A list cut short that
/// doesn't show what was added may have it further on: that's as good as
/// GitHub says.
pub fn shows(state: &State, c: &OrgChange, at: &At) -> bool {
    use OrgChange as C;
    let org = at.org();
    let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    match c {
        C::SetMember {
            login,
            role,
            invite: true,
            ..
        } => {
            let Some(a) = access(state, org) else { return false };
            let invited = (a.invitations.as_ref().and_then(|i| i.as_ref().ok())).is_some_and(|l| {
                l.more
                    || l.items
                        .iter()
                        .any(|i| i.login.as_deref().is_some_and(|l| same(l, login)))
            });
            invited || member_is(state, org, login, Some(*role))
        }
        C::SetMember { login, role, .. } => member_is(state, org, login, Some(*role)),
        C::RemoveMember { login, .. } => {
            members(state, org).is_some_and(|m| !m.items.iter().any(|m| same(&m.login, login)))
        }
        C::CancelInvitation { id, .. } => access(state, org).is_some_and(|a| {
            (a.invitations.as_ref().and_then(|i| i.as_ref().ok()))
                .is_none_or(|l| l.items.iter().all(|i| i.id != *id))
        }),
        C::CreateTeam { name, .. } => teams(state, org).is_some_and(|t| {
            t.items.iter().any(|t| same(&t.name, name))
                || (t.next.is_some()
                    && t.items
                        .last()
                        .is_some_and(|last| last.name.to_lowercase() < name.to_lowercase()))
        }),
        C::SetTeamMember {
            team: slug,
            login,
            maintainer,
            ..
        } => team(state, org, slug).is_some_and(|t| {
            let listed = t.members.iter().any(|m| same(&m.login, login));
            let maintains = t.maintainers.iter().any(|m| same(m, login));
            (listed && maintains == *maintainer) || (!listed && t.members.left_out() > 0)
        }),
        C::RemoveTeamMember {
            team: slug, login, ..
        } => {
            team(state, org, slug).is_some_and(|t| !t.members.iter().any(|m| same(&m.login, login)))
        }
        C::SetTeamRepo {
            team: slug,
            repo,
            permission,
            ..
        } => {
            team(state, org, slug).is_some_and(|t| match t.repos.iter().find(|r| r.repo == *repo) {
                Some(r) => r.permission == Some(*permission),
                None => t.repos.left_out() > 0,
            })
        }
        C::RemoveTeamRepo {
            team: slug, repo, ..
        } => team(state, org, slug).is_some_and(|t| !t.repos.iter().any(|r| r.repo == *repo)),
        // Answered on their own pages ([`crate::invited`]).
        C::AnswerRepo { .. } | C::JoinOrg { .. } => true,
    }
}

/// Whether the People tab shows `login` with `role`, or may further on.
fn member_is(state: &State, org: &str, login: &str, role: Option<OrgRole>) -> bool {
    members(state, org).is_some_and(|m| {
        match m.items.iter().find(|m| m.login.eq_ignore_ascii_case(login)) {
            Some(m) => m.role == role,
            None => m.next.is_some(),
        }
    })
}

// ---- what changes say ------------------------------------------------------------

/// What it's doing while GitHub answers, and what it is, for "GitHub refused …".
pub fn said(c: &OrgChange) -> (&'static str, &'static str) {
    use OrgChange as C;
    match c {
        C::SetMember { invite: true, .. } => ("Inviting them", "the invitation"),
        C::SetMember { .. } | C::SetTeamMember { add: false, .. } => {
            ("Changing their role", "the new role")
        }
        C::RemoveMember { .. } | C::RemoveTeamMember { .. } => ("Removing them", "removing them"),
        C::CancelInvitation { .. } => ("Cancelling the invitation", "cancelling the invitation"),
        C::CreateTeam { .. } => ("Making the team", "the new team"),
        C::SetTeamMember { .. } => ("Adding them to the team", "adding them"),
        C::SetTeamRepo { add: true, .. } => ("Adding the repository", "adding the repository"),
        C::SetTeamRepo { .. } => ("Changing the team's role", "the team's new role"),
        C::RemoveTeamRepo { .. } => ("Removing the repository", "removing the repository"),
        C::AnswerRepo { accept: true, .. } | C::JoinOrg { .. } => {
            ("Accepting the invitation", "accepting it")
        }
        C::AnswerRepo { .. } => ("Declining the invitation", "declining it"),
    }
}

pub fn done(c: &OrgChange) -> String {
    use OrgChange as C;
    match c {
        C::SetMember {
            org,
            login,
            role,
            invite: true,
        } => format!("Invited @{login} to @{org} as {}", article(*role)),
        C::SetMember {
            org, login, role, ..
        } => format!("@{login} is {} of @{org}", article(*role)),
        C::RemoveMember { org, login } => format!("Removed @{login} from @{org}"),
        C::CancelInvitation { who, .. } => format!("Cancelled the invitation to {who}"),
        C::CreateTeam { org, name, .. } => format!("Made the team “{name}” in @{org}"),
        C::SetTeamMember {
            org,
            team,
            login,
            maintainer,
            add,
        } => {
            let role = if *maintainer {
                "a maintainer"
            } else {
                "a member"
            };
            let verb = if *add { "Added" } else { "Made" };
            format!("{verb} @{login} {role} of @{org}/{team}")
        }
        C::RemoveTeamMember { org, team, login } => format!("Removed @{login} from @{org}/{team}"),
        C::SetTeamRepo {
            org,
            team,
            repo,
            permission,
            ..
        } => format!("@{org}/{team}: {} on {repo}", permission.name()),
        C::RemoveTeamRepo { org, team, repo } => format!("Removed {repo} from @{org}/{team}"),
        C::AnswerRepo {
            repo, accept: true, ..
        } => format!("You're a collaborator on {repo} now"),
        C::AnswerRepo { repo, .. } => format!("Declined the invitation to {repo}"),
        C::JoinOrg { org } => format!("You're a member of @{org} now"),
    }
}

/// The permission a fine-grained token needs.
pub fn needs(c: &OrgChange) -> &'static str {
    use OrgChange as C;
    match c {
        C::AnswerRepo { .. } | C::JoinOrg { .. } => "classic tokens only",
        C::SetTeamRepo { .. } | C::RemoveTeamRepo { .. } => {
            "Members: write and Administration: write"
        }
        _ => "Members: write",
    }
}

/// The scope a classic token needs for `c`, and the command that adds it.
pub fn scope(c: &OrgChange) -> String {
    let scope = match c {
        OrgChange::AnswerRepo { .. } => "repo",
        OrgChange::JoinOrg { .. } => "user",
        _ => "admin:org",
    };
    let refresh = crate::api::refresh(scope);
    match needs(c) {
        "classic tokens only" => {
            format!("it takes a classic token with the {scope} scope (`{refresh}` adds it)")
        }
        needs => format!(
            "a classic token needs the {scope} scope (`{refresh}` adds it); a fine-grained one, {needs}"
        ),
    }
}

/// An organization's people and teams as GitHub might send them, for tests.
#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    use crate::api::org::OrgInvitation;
    use crate::api::settings::Listed;

    /// Three members: an owner, a member, and one whose role is hidden.
    pub fn members() -> Results<Member> {
        let m = |login: &str, role| Member {
            login: login.into(),
            name: None,
            role,
        };
        Results {
            total: 3,
            items: vec![
                m("octocat", Some(OrgRole::Owner)),
                m("hubot", Some(OrgRole::Member)),
                m("monalisa", Some(OrgRole::Member)),
            ],
            next: None,
        }
    }

    /// What an owner (`admin`) or a member may do, with an owner's one
    /// invitation waiting.
    pub fn access(admin: bool) -> OrgAccess {
        let invitation = OrgInvitation {
            id: 41,
            login: Some("newbie".into()),
            email: None,
            role: "Member".into(),
            created_at: "2026-10-01T00:00:00Z".into(),
            inviter: Some("octocat".into()),
            failed: false,
        };
        OrgAccess {
            admin,
            create_teams: admin,
            member: true,
            lacks: None,
            invitations: admin.then(|| {
                Ok(Listed {
                    items: vec![invitation],
                    more: false,
                })
            }),
        }
    }
}
