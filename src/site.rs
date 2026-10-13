//! Pages the website has beyond code and conversations: a repository's
//! Insights (contributors and commit activity, counted with git; traffic;
//! the dependency graph), Explore and topics, starred lists, packages;
//! and saved replies for the comment composer.
//!
//! They're one [`Route::Site`](crate::route::Route::Site) and one
//! [`DataKey::Site`](crate::browse::DataKey::Site): this module says what
//! each is (its URL, title, needs and data), and its submodules how it's
//! changed.

#[cfg(test)]
pub(crate) mod fixtures;
pub mod insights;
pub mod lists;
pub mod packages;
pub mod replies;

use crate::api::browse::SearchKind;
use crate::api::explore::{Since, Topic, Topics};
use crate::api::insights::{Activity, Dependencies, Traffic};
use crate::api::lists::{ListPage, MyLists, StarList};
use crate::api::model::{Capped, RepoId};
use crate::api::packages::{Kind, PackageDetail, PackageList, PackageRef, PackagesOf};
use crate::api::replies::SavedReply;
use crate::api::{ApiError, GitHub};
use crate::ui::pages::url as links;

use crate::browse::{Data, DataKey, Need, Picked};

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

impl Page {
    pub fn url(&self) -> String {
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

    pub fn title(&self) -> String {
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

    /// The repository it's a page of.
    pub fn repo(&self) -> Option<&RepoId> {
        match self {
            Page::Insights { repo, .. } | Page::Packages(PackagesOf::Repo(repo)) => Some(repo),
            _ => None,
        }
    }

    /// One level up.
    pub fn up(&self) -> crate::route::Route {
        use crate::route::Route;
        match self {
            Page::Insights { repo, .. } | Page::Packages(PackagesOf::Repo(repo)) => {
                Route::Repo(repo.clone())
            }
            Page::Explore { .. } | Page::Topic(_) => Route::Home,
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
        }
    }

    /// The list it loads more of. Explore's search is from a day, today.
    pub fn paged(&self) -> Option<DataKey> {
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
            Page::StarList { login, slug } => {
                site(Key::StarList(login.to_lowercase(), slug.clone()))
            }
            Page::Package { at, .. } => site(Key::Package(at.clone())),
            _ => return None,
        })
    }

    /// What it needs fetched.
    pub fn needs(&self) -> Vec<Need> {
        let data = |key| Need::Data(site(key));
        let header = |repo: &RepoId| Need::Data(DataKey::Repo(repo.clone()));
        let paged = self.paged().map(Need::Data);
        match self {
            Page::Insights { repo, tab } => {
                let own = match tab {
                    InsightsTab::Contributors | InsightsTab::Commits => Key::Activity(repo.clone()),
                    InsightsTab::Traffic => Key::Traffic(repo.clone()),
                    InsightsTab::Dependencies => Key::Dependencies(repo.clone()),
                };
                vec![header(repo), data(own)]
            }
            Page::Explore { .. } => paged.into_iter().chain([data(Key::Featured)]).collect(),
            Page::Topic(name) => std::iter::once(data(Key::Topic(name.to_lowercase())))
                .chain(paged)
                .collect(),
            Page::StarList { login, .. } => {
                let profile = Need::Data(DataKey::Profile(login.to_lowercase()));
                std::iter::once(profile).chain(paged).collect()
            }
            Page::Packages(of) => {
                let own = data(Key::Packages(of.clone()));
                match of {
                    PackagesOf::Repo(repo) => vec![header(repo), own],
                    PackagesOf::Owner(login) => {
                        vec![Need::Data(DataKey::Profile(login.to_lowercase())), own]
                    }
                }
            }
            Page::Package { .. } => paged.into_iter().collect(),
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

// ---- data -----------------------------------------------------------------------------

/// What these pages read.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Key {
    /// Commits by week and author: counted by git where it can.
    Activity(RepoId),
    Traffic(RepoId),
    Dependencies(RepoId),
    /// GitHub's featured topics.
    Featured,
    /// A topic, by its (lowercase) name.
    Topic(String),
    /// Someone's lists, by their lowercase login.
    StarLists(String),
    /// A list, by its owner's lowercase login and its slug.
    StarList(String, String),
    /// Your lists, as they stand for a repository.
    MyLists(RepoId),
    Packages(PackagesOf),
    Package(PackageRef),
    SavedReplies,
}

/// What they read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Activity(Activity),
    Traffic(Traffic),
    Dependencies(Dependencies),
    Topics(Topics),
    Topic(Topic),
    StarLists(Capped<StarList>),
    StarList(ListPage),
    MyLists(MyLists),
    Packages(PackageList),
    Package(PackageDetail),
    SavedReplies(Capped<SavedReply>),
}

/// `key` as a page's data key.
pub fn site(key: Key) -> DataKey {
    DataKey::Site(key)
}

macro_rules! picked {
    ($($variant:ident => $t:ty),* $(,)?) => {$(
        impl Picked for $t {
            fn pick(data: &Data) -> Option<&Self> {
                match data {
                    Data::Site(v) => match &**v {
                        Value::$variant(x) => Some(x),
                        _ => None,
                    },
                    _ => None,
                }
            }
        }
    )*};
}

picked!(
    Activity => Activity, Traffic => Traffic, Dependencies => Dependencies,
    Topics => Topics, Topic => Topic, StarLists => Capped<StarList>, StarList => ListPage,
    MyLists => MyLists, Packages => PackageList, Package => PackageDetail,
    SavedReplies => Capped<SavedReply>,
);

impl Key {
    /// Whether git answers it ([`crate::source`]).
    pub fn by_git(&self) -> bool {
        matches!(self, Key::Activity(_))
    }

    /// The repository git reads for it.
    pub fn git_repo(&self) -> Option<&RepoId> {
        match self {
            Key::Activity(repo) => Some(repo),
            _ => None,
        }
    }

    /// How long git gets before the API answers.
    pub fn git_wait(&self) -> u64 {
        20
    }
}

impl Value {
    /// Where a list's next page starts.
    pub fn next_cursor(&self) -> Option<&str> {
        match self {
            Value::StarList(l) => l.repos.next.as_deref(),
            Value::Package(p) => p.versions.next.as_deref(),
            _ => None,
        }
    }

    /// Appends a list's next page.
    pub fn append(&mut self, more: Value) {
        match (self, more) {
            (Value::StarList(a), Value::StarList(b)) => {
                crate::browse::extend(&mut a.repos, b.repos, |r| r.repo.clone());
            }
            (Value::Package(a), Value::Package(b)) => {
                crate::browse::extend(&mut a.versions, b.versions, |v| v.id);
            }
            _ => {}
        }
    }
}

fn value(v: Value) -> Data {
    Data::Site(Box::new(v))
}

/// `key`'s data from GitHub's API; for a list, the page after `after`.
pub async fn fetch(gh: &GitHub, key: &Key, after: Option<String>) -> Result<Data, ApiError> {
    Ok(value(match key {
        Key::Activity(repo) => Value::Activity(gh.activity(repo).await?),
        Key::Traffic(repo) => Value::Traffic(gh.traffic(repo).await?),
        Key::Dependencies(repo) => Value::Dependencies(gh.dependencies(repo).await?),
        Key::Featured => Value::Topics(gh.featured_topics().await?),
        Key::Topic(name) => Value::Topic(gh.topic(name).await?),
        Key::StarLists(login) => Value::StarLists(gh.star_lists(login).await?),
        Key::StarList(login, slug) => Value::StarList(gh.star_list(login, slug, after).await?),
        Key::MyLists(repo) => Value::MyLists(gh.my_lists(repo).await?),
        Key::Packages(of) => Value::Packages(gh.packages(of).await?),
        Key::Package(p) => Value::Package(gh.package(p, after).await?),
        Key::SavedReplies => Value::SavedReplies(gh.saved_replies().await?),
    }))
}

/// What the cache has for `key`, and when it was fetched.
pub fn cached(gh: &GitHub, key: &Key) -> Option<(Data, u64)> {
    use crate::api::{explore, insights, lists, packages, replies};
    fn at<T>(
        c: Option<crate::store::Cached<T>>,
        wrap: impl FnOnce(T) -> Value,
    ) -> Option<(Data, u64)> {
        c.map(|c| (value(wrap(c.value)), c.fetched_at))
    }
    match key {
        Key::Activity(repo) => at(gh.cached(&insights::keys::activity(repo)), Value::Activity),
        Key::Traffic(repo) => at(gh.cached(&insights::keys::traffic(repo)), Value::Traffic),
        Key::Dependencies(repo) => at(
            gh.cached(&insights::keys::dependencies(repo)),
            Value::Dependencies,
        ),
        Key::Featured => at(gh.cached(explore::keys::FEATURED), Value::Topics),
        Key::Topic(name) => at(gh.cached(&explore::keys::topic(name)), Value::Topic),
        Key::StarLists(login) => at(gh.cached(&lists::keys::lists(login)), Value::StarLists),
        Key::StarList(login, slug) => {
            at(gh.cached(&lists::keys::list(login, slug)), Value::StarList)
        }
        // Your lists are asked anew: what you choose starts from them.
        Key::MyLists(_) => None,
        Key::Packages(of) => at(gh.cached(&packages::keys::packages(of)), Value::Packages),
        Key::Package(p) => at(gh.cached(&packages::keys::package(p)), Value::Package),
        Key::SavedReplies => at(gh.cached(replies::keys::REPLIES), Value::SavedReplies),
    }
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
    pub fn needs(&self) -> Vec<Need> {
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
            Pick::Lists(repo) => Some(site(Key::MyLists(repo.clone()))),
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

// ---- pages ----------------------------------------------------------------------------

/// Builds page `p` from what's loaded; `retry` is the key that tries
/// again.
pub fn build(
    state: &crate::state::State,
    page: &mut crate::ui::page::Page,
    p: &Page,
    retry: &str,
    cx: crate::ui::pages::PageCtx<'_>,
) {
    use crate::api::browse::SearchResults;
    use crate::state::Remote;
    use crate::ui::pages;
    let get = |key: &DataKey| Remote::fetched(state.data.get(key), retry);
    let now = cx.now;
    match p {
        Page::Insights { repo, tab } => {
            pages::insights_nav(page, repo, *tab);
            match tab {
                InsightsTab::Contributors | InsightsTab::Commits => {
                    let key = site(Key::Activity(repo.clone()));
                    let f = get(&key).pick(<Activity as Picked>::pick);
                    if let Some(a) = f.show(page, "the commits") {
                        if *tab == InsightsTab::Contributors {
                            pages::contributors(page, repo, a);
                        } else {
                            pages::commit_activity(page, a);
                        }
                    }
                }
                InsightsTab::Traffic => {
                    let key = site(Key::Traffic(repo.clone()));
                    let f = get(&key).pick(<Traffic as Picked>::pick);
                    if let Some(t) = f.show(page, "the traffic") {
                        pages::traffic(page, t);
                    }
                }
                InsightsTab::Dependencies => {
                    pages::dependents_note(page, repo);
                    let key = site(Key::Dependencies(repo.clone()));
                    let f = get(&key).pick(<Dependencies as Picked>::pick);
                    if let Some(d) = f.show(page, "the dependency graph") {
                        pages::dependencies(page, d);
                    }
                }
            }
        }
        Page::Explore { language, since } => {
            let topics = get(&site(Key::Featured)).pick(<Topics as Picked>::pick);
            if let Some(key) = p.paged() {
                let repos = get(&key).pick(<SearchResults as Picked>::pick);
                pages::explore(page, language.as_deref(), *since, repos, topics, cx);
            }
        }
        Page::Topic(name) => {
            let topic = get(&site(Key::Topic(name.to_lowercase())));
            let topic = topic.pick(<Topic as Picked>::pick);
            if let (Some(t), Some(key)) = (topic.show(page, "the topic"), p.paged()) {
                let repos = get(&key).pick(<SearchResults as Picked>::pick);
                pages::topic(page, t, repos, now);
            }
        }
        Page::StarList { login, .. } => {
            let Some(key) = p.paged() else {
                return;
            };
            if let Some(l) = get(&key)
                .pick(<ListPage as Picked>::pick)
                .show(page, "the list")
            {
                pages::star_list(page, login, l, now);
            }
        }
        Page::Packages(of) => {
            let f = get(&site(Key::Packages(of.clone())));
            if let Some(list) = f
                .pick(<PackageList as Picked>::pick)
                .show(page, "the packages")
            {
                pages::packages(page, of, list, now);
            }
        }
        Page::Package { at, version } => {
            let f = get(&site(Key::Package(at.clone())));
            if let Some(d) = f
                .pick(<PackageDetail as Picked>::pick)
                .show(page, "the package")
            {
                let delete = state.first_key(crate::keymap::Action::DeleteComment);
                pages::package(page, d, *version, &delete, now);
            }
        }
    }
}
