//! Pages the website has beyond code and conversations: a repository's
//! Insights (contributors and commit activity, counted with git; traffic;
//! the dependency graph), Explore and topics, starred lists, packages;
//! and saved replies for the comment composer.
//!
//! They're one [`Route::Site`](crate::route::Route::Site): this module
//! says what each is (its URL, title, needs and how it's built), and its
//! submodules how it's changed.

#[cfg(test)]
pub(crate) mod fixtures;
pub mod insights;
pub mod lists;
pub mod packages;
pub mod replies;

use crate::api::GitHub;
use crate::api::browse::SearchKind;
use crate::api::explore::{Since, Topic};
use crate::api::lists::ListPage;
use crate::api::model::RepoId;
use crate::api::packages::{Kind, PackageRef, PackagesOf};
use crate::route::code;
use crate::ui::pages::url as links;

use crate::browse::{Build, Width};
use crate::chrome::{Chrome, Section};
use crate::data::DataKey;
use crate::route::{Family, Route, Target, app};
use crate::state::State;
use crate::ui::pages::ProfileTab;

/// One of these pages.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Page {
    /// A repository's Insights, at a tab.
    Insights { repo: RepoId, tab: InsightsTab },
    /// New repositories, most starred (the nearest the API comes to
    /// Trending), in a language if given, and GitHub's featured topics.
    Explore {
        language: Option<String>,
        since: Since,
    },
    /// A topic and its repositories.
    Topic(String),
    /// One of someone's starred lists.
    StarList { login: String, slug: String },
    /// An owner's packages, or a repository's.
    Packages(PackagesOf),
    /// A package and its versions, at one of them (else its newest).
    Package {
        at: PackageRef,
        version: Option<u64>,
    },
}

/// Insights' tabs, as GitHub's sidebar lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InsightsTab {
    Contributors,
    Commits,
    Traffic,
    Dependencies,
}

impl InsightsTab {
    pub const ALL: [InsightsTab; 4] = [
        InsightsTab::Contributors,
        InsightsTab::Commits,
        InsightsTab::Traffic,
        InsightsTab::Dependencies,
    ];

    pub fn name(self) -> &'static str {
        match self {
            InsightsTab::Contributors => "Contributors",
            InsightsTab::Commits => "Commit activity",
            InsightsTab::Traffic => "Traffic",
            InsightsTab::Dependencies => "Dependency graph",
        }
    }

    /// Its path under the repository.
    fn path(self) -> &'static str {
        match self {
            InsightsTab::Contributors => "graphs/contributors",
            InsightsTab::Commits => "graphs/commit-activity",
            InsightsTab::Traffic => "graphs/traffic",
            InsightsTab::Dependencies => "network/dependencies",
        }
    }
}

impl Family for Page {
    fn url(&self) -> String {
        match self {
            Page::Insights { repo, tab } => format!("{}/{}", links::repo(repo), tab.path()),
            Page::Explore { language, since } => {
                let mut url = format!("{}/trending", links::base());
                if let Some(language) = language {
                    url.push('/');
                    url.push_str(&links::encode_path(&language_slug(language)));
                }
                if *since != Since::Daily {
                    url.push_str(&format!("?since={}", since.slug()));
                }
                url
            }
            Page::Topic(name) => format!("{}/topics/{}", links::base(), links::encode_path(name)),
            Page::StarList { login, slug } => {
                format!(
                    "{}/stars/{login}/lists/{}",
                    links::base(),
                    links::encode_path(slug)
                )
            }
            Page::Packages(PackagesOf::Owner(login)) => {
                format!("{}?tab=packages", links::user(login))
            }
            Page::Packages(PackagesOf::Repo(repo)) => format!("{}/packages", links::repo(repo)),
            Page::Package { at, version: None } => format!(
                "{}/users/{}/packages/{}/package/{}",
                links::base(),
                at.owner,
                at.kind.slug(),
                links::encode_path(&at.name)
            ),
            Page::Package {
                at,
                version: Some(id),
            } => format!(
                "{}/users/{}/packages/{}/{}/{id}",
                links::base(),
                at.owner,
                at.kind.slug(),
                links::encode_path(&at.name)
            ),
        }
    }

    fn title(&self) -> String {
        match self {
            Page::Insights { repo, tab } => format!("{repo} · {}", tab.name()),
            Page::Explore { language: None, .. } => "Explore".to_owned(),
            Page::Explore {
                language: Some(l), ..
            } => format!("Explore · {l}"),
            Page::Topic(name) => format!("Topic: {name}"),
            Page::StarList { login, slug } => format!("@{login}'s list {slug}"),
            Page::Packages(PackagesOf::Owner(login)) => format!("@{login} · Packages"),
            Page::Packages(PackagesOf::Repo(repo)) => format!("{repo} · Packages"),
            Page::Package { at, .. } => format!("Package {}", at.name),
        }
    }

    fn repo(&self) -> Option<&RepoId> {
        match self {
            Page::Insights { repo, .. } | Page::Packages(PackagesOf::Repo(repo)) => Some(repo),
            _ => None,
        }
    }

    fn up(&self, _: &State) -> Option<Route> {
        Some(match self {
            Page::Insights { repo, .. } | Page::Packages(PackagesOf::Repo(repo)) => {
                Route::Code(code::Repo(repo.clone()))
            }
            Page::Explore { .. } | Page::Topic(_) => Route::App(app::Home),
            Page::StarList { login, .. } => Route::User {
                login: login.clone(),
                tab: crate::ui::pages::ProfileTab::Stars,
            },
            Page::Packages(PackagesOf::Owner(login)) => Route::user(login),
            Page::Package { at, version: None } => {
                Route::Site(Page::Packages(PackagesOf::Owner(at.owner.clone())))
            }
            Page::Package { at, .. } => Route::Site(Page::Package {
                at: at.clone(),
                version: None,
            }),
        })
    }

    /// Explore's search is from a day, today.
    fn paged(&self) -> Option<DataKey> {
        Some(match self {
            Page::Explore { language, since } => DataKey::Search(
                SearchKind::Repos,
                crate::api::explore::new_and_starred(
                    language.as_deref(),
                    *since,
                    crate::store::now(),
                ),
            ),
            Page::Topic(name) => {
                DataKey::Search(SearchKind::Repos, crate::api::explore::topic_repos(name))
            }
            Page::StarList { login, slug } => DataKey::StarList(login.to_lowercase(), slug.clone()),
            Page::Package { at, .. } => DataKey::Package(at.clone()),
            _ => return None,
        })
    }

    fn needs(&self, _: &State) -> Vec<DataKey> {
        let data = |key| key;
        let header = |repo: &RepoId| DataKey::Repo(repo.clone());
        let paged = self.paged();
        match self {
            Page::Insights { repo, tab } => {
                let own = match tab {
                    InsightsTab::Contributors | InsightsTab::Commits => {
                        DataKey::Activity(repo.clone())
                    }
                    InsightsTab::Traffic => DataKey::Traffic(repo.clone()),
                    InsightsTab::Dependencies => DataKey::Dependencies(repo.clone()),
                };
                vec![header(repo), data(own)]
            }
            Page::Explore { .. } => paged.into_iter().chain([data(DataKey::Featured)]).collect(),
            Page::Topic(name) => std::iter::once(data(DataKey::Topic(name.to_lowercase())))
                .chain(paged)
                .collect(),
            Page::StarList { login, .. } => {
                let profile = DataKey::Profile(login.to_lowercase());
                std::iter::once(profile).chain(paged).collect()
            }
            Page::Packages(of) => {
                let own = data(DataKey::Packages(of.clone()));
                match of {
                    PackagesOf::Repo(repo) => vec![header(repo), own],
                    PackagesOf::Owner(login) => {
                        vec![DataKey::Profile(login.to_lowercase()), own]
                    }
                }
            }
            Page::Package { .. } => paged.into_iter().collect(),
        }
    }

    fn width(&self) -> Width {
        Width::Lists
    }

    fn shown_source(&self) -> Option<DataKey> {
        match self {
            Page::Insights {
                repo,
                tab: InsightsTab::Contributors | InsightsTab::Commits,
            } => Some(DataKey::Activity(repo.clone())),
            _ => None,
        }
    }

    fn section(&self) -> Section {
        match self {
            Page::Insights { .. } => Section::Insights,
            _ => Section::Code,
        }
    }

    fn chrome(&self, state: &State, c: &mut Chrome) {
        let explore = || {
            Target::Page(Route::Site(Page::Explore {
                language: None,
                since: crate::api::explore::Since::default(),
            }))
        };
        let user = |login: &str| Some(Target::Page(Route::user(login)));
        match self {
            Page::Explore { .. } => c.crumb("Explore", None),
            Page::Topic(name) => {
                c.crumb("Explore", Some(explore()));
                c.crumb(&format!("#{name}"), None);
            }
            Page::StarList { login, .. } => {
                c.crumb(login, user(login));
                let stars = Route::User {
                    login: login.clone(),
                    tab: ProfileTab::Stars,
                };
                c.crumb("Stars", Some(Target::Page(stars)));
                let list = (self.paged())
                    .and_then(|k| state.picked::<ListPage>(&k))
                    .map_or_else(|| "List".to_owned(), |l| l.list.name.clone());
                c.crumb(&list, None);
            }
            Page::Packages(PackagesOf::Repo(repo)) => state.repo_chrome(repo, Section::Code, c),
            Page::Packages(of) => {
                let login = of.owner();
                c.crumb(login, None);
                state.profile_tabs(login, None, None, c);
                let here = Target::Page(Route::Site(self.clone()));
                c.active = c.tabs.iter().position(|(_, t)| *t == here);
            }
            Page::Package { at, .. } => {
                c.crumb(&at.owner, user(&at.owner));
                let list = Route::Site(Page::Packages(PackagesOf::Owner(at.owner.clone())));
                c.crumb("Packages", Some(Target::Page(list)));
                c.crumb(&at.name, None);
            }
            Page::Insights { repo, .. } => state.repo_chrome(repo, Section::Insights, c),
        }
    }

    fn build(&self, b: Build<'_>, page: &mut crate::ui::page::Page) {
        use crate::ui::pages;
        let now = b.cx.now;
        match self {
            Page::Insights { repo, tab } => {
                pages::insights_nav(page, repo, *tab);
                match tab {
                    InsightsTab::Contributors | InsightsTab::Commits => {
                        let key = DataKey::Activity(repo.clone());
                        let f = b.get(&key);
                        if let Some(a) = f.show(page, "the commits") {
                            if *tab == InsightsTab::Contributors {
                                pages::contributors(page, repo, a);
                            } else {
                                pages::commit_activity(page, a);
                            }
                        }
                    }
                    InsightsTab::Traffic => {
                        let key = DataKey::Traffic(repo.clone());
                        let f = b.get(&key);
                        if let Some(t) = f.show(page, "the traffic") {
                            pages::traffic(page, t);
                        }
                    }
                    InsightsTab::Dependencies => {
                        pages::dependents_note(page, repo);
                        let key = DataKey::Dependencies(repo.clone());
                        let f = b.get(&key);
                        if let Some(d) = f.show(page, "the dependency graph") {
                            pages::dependencies(page, d);
                        }
                    }
                }
            }
            Page::Explore { language, since } => {
                let topics = b.get(&DataKey::Featured);
                if let Some(key) = self.paged() {
                    let repos = b.get(&key);
                    pages::explore(page, language.as_deref(), *since, repos, topics, b.cx);
                }
            }
            Page::Topic(name) => {
                let topic = b.get::<Topic>(&DataKey::Topic(name.to_lowercase()));
                if let (Some(t), Some(key)) = (topic.show(page, "the topic"), self.paged()) {
                    let repos = b.get(&key);
                    pages::topic(page, t, repos, now);
                }
            }
            Page::StarList { login, .. } => {
                let Some(key) = self.paged() else {
                    return;
                };
                if let Some(l) = b.get(&key).show(page, "the list") {
                    pages::star_list(page, login, l, now);
                }
            }
            Page::Packages(of) => {
                let f = b.get(&DataKey::Packages(of.clone()));
                if let Some(list) = f.show(page, "the packages") {
                    pages::packages(page, of, list, now);
                }
            }
            Page::Package { at, version } => {
                let f = b.get(&DataKey::Package(at.clone()));
                if let Some(d) = f.show(page, "the package") {
                    let delete = b.state.first_key(crate::keymap::Action::DeleteComment);
                    pages::package(page, d, *version, &delete, now);
                }
            }
        }
    }
}

/// A language as GitHub's Trending URLs write it (`c++`, `visual-basic-.net`).
fn language_slug(language: &str) -> String {
    language.trim().to_lowercase().replace(' ', "-")
}

/// A language from a Trending URL.
fn language_of(slug: &str) -> String {
    slug.replace('-', " ")
}

/// The page a GitHub URL's path (`segments`) names, if it's one of these.
pub fn parse(segments: &[&str], param: impl Fn(&str) -> Option<String>) -> Option<Page> {
    let valid = crate::route::valid_login;
    let owner = |s: &str| RepoId::parse(&format!("{s}/x")).is_some();
    Some(match segments {
        ["explore"] | ["trending"] => Page::Explore {
            language: None,
            since: since(&param),
        },
        ["trending", language] => Page::Explore {
            language: Some(language_of(language)),
            since: since(&param),
        },
        ["topics"] => Page::Explore {
            language: None,
            since: Since::Daily,
        },
        ["topics", name] if valid_topic(name) => Page::Topic((*name).to_lowercase()),
        ["stars", login, "lists", slug] if valid(login) && !slug.is_empty() => Page::StarList {
            login: (*login).to_owned(),
            slug: (*slug).to_owned(),
        },
        ["orgs" | "users", login, "packages"] if valid(login) => {
            Page::Packages(PackagesOf::Owner((*login).to_owned()))
        }
        // A package's owner is as its repository's is: one a repository
        // path accepts.
        ["orgs" | "users", login, "packages", kind, "package", name] if owner(login) => {
            Page::Package {
                at: PackageRef {
                    owner: (*login).to_owned(),
                    kind: Kind::from_slug(kind)?,
                    name: (*name).to_owned(),
                },
                version: None,
            }
        }
        // One of its versions.
        ["orgs" | "users", login, "packages", kind, name, id] if owner(login) => Page::Package {
            at: PackageRef {
                owner: (*login).to_owned(),
                kind: Kind::from_slug(kind)?,
                name: (*name).to_owned(),
            },
            version: Some(id.parse().ok()?),
        },
        [login] if param("tab").as_deref() == Some("packages") && valid(login) => {
            Page::Packages(PackagesOf::Owner((*login).to_owned()))
        }
        [o, r, rest @ ..] if !crate::route::is_reserved(o) => {
            let repo = RepoId::parse(&format!("{o}/{r}"))?;
            let tab = |tab| Page::Insights {
                repo: repo.clone(),
                tab,
            };
            match rest {
                ["graphs", "contributors"] => tab(InsightsTab::Contributors),
                ["graphs", "commit-activity"] => tab(InsightsTab::Commits),
                ["graphs", "traffic"] => tab(InsightsTab::Traffic),
                ["network", "dependencies"] => tab(InsightsTab::Dependencies),
                ["packages"] => Page::Packages(PackagesOf::Repo(repo)),
                ["pkgs", kind, name] => Page::Package {
                    at: PackageRef {
                        owner: repo.owner,
                        kind: Kind::from_slug(kind)?,
                        name: (*name).to_owned(),
                    },
                    version: None,
                },
                _ => return None,
            }
        }
        _ => return None,
    })
}

fn since(param: &impl Fn(&str) -> Option<String>) -> Since {
    param("since")
        .and_then(|s| Since::from_slug(&s))
        .unwrap_or_default()
}

/// A topic's name, as GitHub allows them: lowercase letters, digits and
/// hyphens.
fn valid_topic(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 50
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '+' || c == '#')
}

#[cfg(test)]
pub(crate) fn arb() -> impl proptest::strategy::Strategy<Value = Route> {
    use crate::route::arb::{login, repo};
    use proptest::prelude::*;
    let package = (
        login(),
        prop::sample::select(Kind::ALL.to_vec()),
        "[a-z0-9][a-z0-9._-]{0,11}",
    )
        .prop_map(|(owner, kind, name)| PackageRef { owner, kind, name });
    let page = prop_oneof![
        (repo(), prop::sample::select(InsightsTab::ALL.to_vec()))
            .prop_map(|(repo, tab)| Page::Insights { repo, tab }),
        (
            prop::option::of("[a-z0-9+#]{1,10}"),
            prop::sample::select(Since::ALL.to_vec())
        )
            .prop_map(|(language, since)| Page::Explore { language, since }),
        "[a-z0-9][a-z0-9-]{0,15}".prop_map(Page::Topic),
        (login(), "[a-z0-9][a-z0-9-]{0,15}")
            .prop_map(|(login, slug)| Page::StarList { login, slug }),
        login().prop_map(|l| Page::Packages(PackagesOf::Owner(l))),
        repo().prop_map(|r| Page::Packages(PackagesOf::Repo(r))),
        (package, prop::option::of(1..u64::MAX))
            .prop_map(|(at, version)| Page::Package { at, version }),
    ];
    page.prop_map(Route::Site)
}

// ---- writes ---------------------------------------------------------------------------

/// Changes these pages make, run in the background ([`crate::writes`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Work {
    Lists(lists::Work),
    DeleteList(lists::Delete),
    DeleteVersion(packages::Delete),
}

/// Their answers.
#[derive(Debug)]
pub enum Done {
    Lists(lists::Work, Result<(), String>),
    DeleteList(lists::Delete, Result<(), String>),
    DeleteVersion(packages::Delete, Result<(), String>),
}

impl Work {
    pub fn failed(&self, why: String) -> Done {
        match self {
            Work::Lists(w) => Done::Lists(w.clone(), Err(why)),
            Work::DeleteList(d) => Done::DeleteList(d.clone(), Err(why)),
            Work::DeleteVersion(d) => Done::DeleteVersion(d.clone(), Err(why)),
        }
    }

    pub async fn run(self, gh: &GitHub) -> Done {
        match self {
            Work::Lists(w) => {
                let result = w.clone().run(gh).await;
                Done::Lists(w, result)
            }
            Work::DeleteList(d) => {
                let result = d.clone().run(gh).await;
                Done::DeleteList(d, result)
            }
            Work::DeleteVersion(d) => {
                let result = d.clone().run(gh).await;
                Done::DeleteVersion(d, result)
            }
        }
    }
}

impl Done {
    pub fn wrote(&self) -> bool {
        match self {
            Done::Lists(_, r) | Done::DeleteList(_, r) | Done::DeleteVersion(_, r) => r.is_ok(),
        }
    }
}

#[must_use]
pub fn on_done(state: &mut crate::state::State, done: Done) -> Vec<crate::state::Cmd> {
    match done {
        Done::Lists(w, result) => lists::on_done(state, w, result),
        Done::DeleteList(d, result) => lists::on_deleted(state, d, result),
        Done::DeleteVersion(d, result) => packages::on_deleted(state, d, result),
    }
}

/// What the page shows once GitHub has a change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expected {
    Lists(lists::Expected),
    /// A package without this version.
    VersionGone(PackageRef, u64),
}

impl Expected {
    pub fn needs(&self) -> Vec<DataKey> {
        match self {
            Expected::Lists(e) => e.needs(),
            Expected::VersionGone(at, _) => packages::needs(at),
        }
    }

    pub fn waiting(&self) -> &'static str {
        match self {
            Expected::Lists(e) => e.waiting(),
            Expected::VersionGone(..) => "Waiting for GitHub to show the version gone",
        }
    }

    pub fn shown(&self, state: &crate::state::State) -> bool {
        match self {
            Expected::Lists(e) => e.shown(state),
            Expected::VersionGone(at, id) => packages::gone(state, at, *id),
        }
    }
}

/// Answers to these pages' questions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    DeleteList,
    /// Delete the version with this ID.
    DeleteVersion(u64),
}

impl Answer {
    pub fn busy(self) -> &'static str {
        match self {
            Answer::DeleteList => "Deleting the list…",
            Answer::DeleteVersion(_) => "Deleting the version…",
        }
    }
}

/// The question about `route` was answered.
#[must_use]
pub fn answer(
    state: &mut crate::state::State,
    route: &crate::route::Route,
    answer: Answer,
) -> Vec<crate::state::Cmd> {
    match answer {
        Answer::DeleteList => lists::delete(state, route),
        Answer::DeleteVersion(id) => packages::delete(state, route, id),
    }
}

/// These pages' pickers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// Your lists, for a repository.
    Lists(RepoId),
}

impl Pick {
    pub fn title(&self) -> &'static str {
        match self {
            Pick::Lists(_) => "Your lists",
        }
    }

    pub fn placeholder(&self) -> &'static str {
        match self {
            Pick::Lists(_) => "Filter, or type a new list's name · ↵ adds or takes out",
        }
    }

    pub fn need(&self) -> Option<DataKey> {
        match self {
            Pick::Lists(repo) => Some(DataKey::MyLists(repo.clone())),
        }
    }
}

/// What choosing one of their rows does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Chosen {
    Lists(lists::Work),
}

pub fn rows(
    state: &crate::state::State,
    pick: &Pick,
    q: &str,
) -> Vec<(
    crate::ui::overlays::PaletteItem,
    Option<crate::writes::Chosen>,
)> {
    match pick {
        Pick::Lists(repo) => lists::rows(state, repo, q),
    }
}

#[must_use]
pub fn choose(state: &mut crate::state::State, chosen: Chosen) -> Vec<crate::state::Cmd> {
    match chosen {
        Chosen::Lists(work) => lists::choose(state, work),
    }
}

/// The menu's rows on these pages.
pub fn doables(
    state: &crate::state::State,
    route: &crate::route::Route,
    out: &mut Vec<crate::nav::Doable>,
) {
    lists::doables(state, route, out);
    packages::doables(state, route, out);
}

/// Whether `,D` here is these pages': a package's, or a list's.
pub fn deletes_here(route: Option<&crate::route::Route>) -> bool {
    matches!(
        route,
        Some(crate::route::Route::Site(
            Page::Package { .. } | Page::StarList { .. }
        ))
    )
}

/// `,D` here.
#[must_use]
pub fn ask_delete(state: &mut crate::state::State) -> Vec<crate::state::Cmd> {
    match state.route() {
        Some(crate::route::Route::Site(Page::StarList { .. })) => lists::ask_delete(state),
        _ => packages::ask_delete(state),
    }
}
