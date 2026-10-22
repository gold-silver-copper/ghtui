//! People: a profile at one of its tabs, an organization's teams, a
//! repository's stargazers, watchers and forks, an invitation sent to
//! you, and the repositories you watch.

use super::{Family, RESERVED, Route, Target, Url, valid_login, valid_slug};
use crate::api::browse::{RepoSort, UserList};
use crate::api::model::RepoId;
use crate::browse::Build;
use crate::chrome::Chrome;
use crate::data::DataKey;
use crate::state::State;
use crate::ui::pages::{self, ProfileList, ProfileTab, url as links};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Page {
    User {
        login: String,
        tab: ProfileTab,
    },
    Stargazers(RepoId),
    Watchers(RepoId),
    Forks(RepoId),
    /// An organization's teams.
    Teams(String),
    Team {
        org: String,
        slug: String,
    },
    /// An invitation sent to you ([`crate::invited`]).
    Invitation(Invite),
    /// The repositories you watch.
    Watching,
}

/// What an invitation sent to you is to join.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Invite {
    Repo(RepoId),
    Org(String),
}

pub use Page::*;

impl Family for Page {
    fn url(&self) -> String {
        match self {
            User { login, tab } => match tab {
                ProfileTab::Overview => links::user(login),
                ProfileTab::Repositories(sort) => {
                    let sort = match sort {
                        RepoSort::Updated => "",
                        RepoSort::Name => "&sort=name",
                        RepoSort::Stars => "&sort=stargazers",
                    };
                    format!("{}?tab=repositories{sort}", links::user(login))
                }
                ProfileTab::Stars => format!("{}?tab=stars", links::user(login)),
                ProfileTab::Followers => format!("{}?tab=followers", links::user(login)),
                ProfileTab::Following => format!("{}?tab=following", links::user(login)),
                ProfileTab::People => format!("{}/orgs/{login}/people", links::base()),
            },
            Stargazers(repo) => format!("{}/stargazers", links::repo(repo)),
            Watchers(repo) => format!("{}/watchers", links::repo(repo)),
            Forks(repo) => format!("{}/forks", links::repo(repo)),
            Teams(org) => format!("{}/orgs/{org}/teams", links::base()),
            Team { org, slug } => format!("{}/orgs/{org}/teams/{slug}", links::base()),
            Invitation(Invite::Repo(repo)) => format!("{}/invitations", links::repo(repo)),
            Invitation(Invite::Org(org)) => format!("{}/orgs/{org}/invitation", links::base()),
            Watching => format!("{}/watching", links::base()),
        }
    }

    fn title(&self) -> String {
        match self {
            User { login, .. } => format!("@{login}"),
            Watching => "Watching".to_owned(),
            Stargazers(repo) => format!("{repo} · Stargazers"),
            Watchers(repo) => format!("{repo} · Watchers"),
            Forks(repo) => format!("{repo} · Forks"),
            Teams(org) => format!("@{org} · Teams"),
            Team { org, slug } => format!("@{org}/{slug}"),
            Invitation(Invite::Repo(repo)) => format!("Invitation to {repo}"),
            Invitation(Invite::Org(org)) => format!("Invitation to @{org}"),
        }
    }

    fn repo(&self) -> Option<&RepoId> {
        match self {
            Stargazers(repo) | Watchers(repo) | Forks(repo) => Some(repo),
            User { .. } | Teams(_) | Team { .. } | Invitation(_) | Watching => None,
        }
    }

    fn up(&self, _: &State) -> Option<Route> {
        Some(match self {
            User { .. } | Watching => Route::App(super::app::Home),
            Stargazers(repo) | Watchers(repo) | Forks(repo) => Route::code(repo.clone()),
            Teams(login) => Route::user(login),
            Team { org, .. } => Route::People(Teams(org.clone())),
            Invitation(_) => Route::Search(super::search::Notifications {
                query: String::new(),
            }),
        })
    }

    fn needs(&self) -> Vec<DataKey> {
        match self {
            User { login, tab } => {
                // A user's lists show above what they starred.
                let lists =
                    (*tab == ProfileTab::Stars).then(|| DataKey::StarLists(login.to_lowercase()));
                // What you may change of an organization's people.
                let access =
                    (*tab == ProfileTab::People).then(|| DataKey::OrgAccess(login.to_lowercase()));
                std::iter::once(DataKey::Profile(login.to_lowercase()))
                    .chain(lists)
                    .chain(access)
                    .chain(self.paged())
                    .collect()
            }
            Stargazers(repo) | Watchers(repo) | Forks(repo) => {
                std::iter::once(DataKey::Repo(repo.clone()))
                    .chain(self.paged())
                    .collect()
            }
            Teams(org) => (self.paged().into_iter())
                .chain([DataKey::OrgAccess(org.to_lowercase())])
                .collect(),
            Watching => self.paged().into_iter().collect(),
            Team { org, slug } => vec![
                DataKey::Team(org.to_lowercase(), slug.clone()),
                DataKey::OrgAccess(org.to_lowercase()),
            ],
            Invitation(_) => vec![DataKey::Invitations],
        }
    }

    fn paged(&self) -> Option<DataKey> {
        Some(match self {
            User { login, tab } => {
                let login = login.to_lowercase();
                match tab {
                    ProfileTab::Overview => return None,
                    ProfileTab::Repositories(sort) => DataKey::OwnerRepos(login, *sort),
                    ProfileTab::Stars => DataKey::Stars(login),
                    ProfileTab::Followers => DataKey::Users(UserList::Followers(login)),
                    ProfileTab::Following => DataKey::Users(UserList::Following(login)),
                    ProfileTab::People => DataKey::Members(login),
                }
            }
            Stargazers(repo) => DataKey::Users(UserList::Stargazers(repo.clone())),
            Watchers(repo) => DataKey::Users(UserList::Watchers(repo.clone())),
            Forks(repo) => DataKey::Forks(repo.clone()),
            Teams(org) => DataKey::Teams(org.to_lowercase()),
            Watching => DataKey::Watching,
            Team { .. } | Invitation(_) => return None,
        })
    }

    fn chrome(&self, state: &State, c: &mut Chrome) {
        let user = |login: &str| Some(Target::Page(Route::user(login)));
        match self {
            User { login, tab } => {
                c.crumb(login, None);
                state.profile_tabs(login, Some(*tab), None, c);
            }
            Teams(org) => {
                c.crumb(org, user(org));
                c.crumb("Teams", None);
            }
            Watching => c.crumb("Watching", None),
            Team { org, slug } => {
                c.crumb(org, user(org));
                c.crumb(
                    "Teams",
                    Some(Target::Page(Route::People(Teams(org.clone())))),
                );
                c.crumb(slug, None);
            }
            Stargazers(repo) | Watchers(repo) | Forks(repo) => {
                state.repo_chrome(repo, self.section(), c);
            }
            Invitation(_) => c.crumb("Invitation", None),
        }
    }

    fn build(&self, b: Build<'_>, page: &mut crate::ui::page::Page) {
        let now = b.cx.now;
        match self {
            User { login, tab } => {
                let key = DataKey::Profile(login.to_lowercase());
                let hint = crate::org::hint(b.state, &crate::org::At::People(login.clone()));
                if let Some(p) = b.get(&key).show(page, &self.title()) {
                    let list = match self.paged() {
                        Some(key @ DataKey::Users(_)) => ProfileList::People(b.get(&key)),
                        Some(key @ DataKey::Members(_)) => {
                            let access = DataKey::OrgAccess(login.to_lowercase());
                            ProfileList::Members(b.get(&key), b.state.picked(&access), &hint)
                        }
                        Some(key) if *tab == ProfileTab::Stars => {
                            let lists = b.get(&DataKey::StarLists(login.to_lowercase()));
                            ProfileList::Starred(lists, b.get(&key))
                        }
                        Some(key) => ProfileList::Repos(b.get(&key)),
                        None => ProfileList::None,
                    };
                    pages::profile(page, p, *tab, list, now);
                }
            }
            Stargazers(repo) => {
                // GitHub lists none of a big repository's stargazers.
                let stars = b.state.overview(repo).map_or(0, |o| o.summary.stars);
                let empty = match stars {
                    0 => "Nobody here yet.".to_owned(),
                    n => format!(
                        "GitHub doesn't list this repository's {} stargazers.",
                        pages::compact(n)
                    ),
                };
                pages::people_list(page, "Stargazers", b.list(self.paged()), &empty);
            }
            Watchers(_) => {
                pages::people_list(page, "Watchers", b.list(self.paged()), "Nobody here yet.");
            }
            Forks(_) => pages::repos(page, "Forks", b.list(self.paged()), now),
            Watching => pages::repos(page, "Watching", b.list(self.paged()), now),
            Teams(org) => pages::teams(page, org, b.list(self.paged())),
            Team { org, slug } => {
                let key = DataKey::Team(org.to_lowercase(), slug.clone());
                if let Some(t) = b.get(&key).show(page, &self.title()) {
                    pages::team(page, org, t);
                }
            }
            Invitation(invite) => crate::invited::build(b, page, invite),
        }
    }
}

/// Profiles and teams, at the top of GitHub's paths.
pub(super) fn parse(url: &Url<'_>) -> Option<Target> {
    let tab = url.param("tab");
    let repos = || {
        ProfileTab::Repositories(match url.param("sort").as_deref() {
            Some("name") => RepoSort::Name,
            Some("stargazers") => RepoSort::Stars,
            _ => RepoSort::Updated,
        })
    };
    let user = |login: &str, tab| User {
        login: login.to_owned(),
        tab,
    };
    let page = match url.segments {
        ["stars", login] => user(login, ProfileTab::Stars),
        ["watching"] => Watching,
        ["orgs", login] => user(login, ProfileTab::Overview),
        // A new team is made on the list (`a`).
        ["orgs", org, "teams"] | ["orgs", org, "teams", "new"] => Teams((*org).to_owned()),
        // Its members, repositories and child teams are on its page.
        ["orgs", org, "teams", slug, ..] if valid_slug(slug) => Team {
            org: (*org).to_owned(),
            slug: (*slug).to_owned(),
        },
        // Its owners see the invitations waiting under its people.
        ["orgs", login, "people"] | ["orgs", login, "people", "pending_invitations"] => {
            user(login, ProfileTab::People)
        }
        ["orgs", org, "invitation"] => Invitation(Invite::Org((*org).to_owned())),
        ["orgs", login, "repositories"] => user(login, repos()),
        [login] if !RESERVED.contains(login) && valid_login(login) => {
            let tab = match tab.as_deref() {
                None | Some("overview") => ProfileTab::Overview,
                Some("repositories") => repos(),
                Some("stars") => ProfileTab::Stars,
                Some("followers") => ProfileTab::Followers,
                Some("following") => ProfileTab::Following,
                // Packages need a scope gh doesn't grant; projects are
                // the projects'.
                Some(_) => return None,
            };
            user(login, tab)
        }
        _ => return None,
    };
    Some(Target::Page(Route::People(page)))
}

pub(super) fn parse_repo(repo: &RepoId, rest: &[&str], _: &Url<'_>) -> Option<Target> {
    let repo = repo.clone();
    let page = match rest {
        ["stargazers"] => Stargazers(repo),
        ["watchers"] => Watchers(repo),
        ["forks"] | ["network", "members"] => Forks(repo),
        ["invitations"] => Invitation(Invite::Repo(repo)),
        _ => return None,
    };
    Some(Target::Page(Route::People(page)))
}

#[cfg(test)]
pub(crate) fn arb() -> impl proptest::strategy::Strategy<Value = Route> {
    use crate::route::arb::{login, repo, segment};
    use proptest::prelude::*;
    let tabs = prop::sample::select(vec![
        ProfileTab::Overview,
        ProfileTab::Repositories(RepoSort::Updated),
        ProfileTab::Repositories(RepoSort::Name),
        ProfileTab::Repositories(RepoSort::Stars),
        ProfileTab::Stars,
        ProfileTab::Followers,
        ProfileTab::Following,
        ProfileTab::People,
    ]);
    let page = prop_oneof![
        (login(), tabs)
            .prop_filter("not a reserved path", |(l, _)| !RESERVED
                .contains(&l.as_str()))
            .prop_map(|(login, tab)| User { login, tab }),
        repo().prop_map(Stargazers),
        repo().prop_map(Watchers),
        repo().prop_map(Forks),
        login().prop_map(Teams),
        (login(), segment())
            .prop_filter("not the new-team form", |(_, slug)| slug != "new")
            .prop_map(|(org, slug)| Team { org, slug }),
        repo().prop_map(|r| Invitation(Invite::Repo(r))),
        login().prop_map(|o| Invitation(Invite::Org(o))),
        Just(Watching),
    ];
    page.prop_map(Route::People)
}
