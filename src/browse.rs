//! Browsing: the data behind each page, and building pages from it.

use crate::route::Family;
use crate::route::code;
use std::collections::HashSet;

use std::collections::BTreeSet;

use crate::api::browse::{
    CommitInfo, DiscussionsOf, Gist, PrActivity, Release, RepoOverview, UserList, WikiPage,
};
use crate::api::model::{PrDetail, PrRef, RepoId};
use crate::api::project::{Project, ProjectsOf};
use crate::api::security::AlertRef;
use crate::ui::Fetched;
use crate::ui::page::{Page, PageLine, Role, Seg};
use crate::ui::pages::{self, Keys, ProfileList, ProfileTab};

use crate::data::{Data, DataKey, Picked};
use crate::diff_screen::{DiffOf, DiffState};
use crate::keymap::Action;
use crate::route::Route;
use crate::state::{Remote, State};

/// Widest a page of reading (a conversation, a Markdown file) gets, its
/// sidebar included; wider terminals center it, as GitHub does.
const READING: u16 = 160;
/// Widest a page of lists gets: room for their columns, without a row's
/// two ends drifting apart.
const LISTS: u16 = 200;

/// The list a page loads more of.
pub fn paged(route: &Route) -> Option<DataKey> {
    match route {
        Route::Stargazers(repo) => Some(DataKey::Users(UserList::Stargazers(repo.clone()))),
        Route::Watchers(repo) => Some(DataKey::Users(UserList::Watchers(repo.clone()))),
        Route::Forks(repo) => Some(DataKey::Forks(repo.clone())),
        Route::Releases(repo) => Some(DataKey::Releases(repo.clone())),
        Route::Discussions { of, category } => {
            Some(DataKey::Discussions(of.clone(), category.clone()))
        }
        Route::Actions { repo, query } => Some(DataKey::Runs(repo.clone(), query.clone())),
        Route::Workflow { repo, file } => {
            Some(DataKey::Runs(repo.clone(), format!("workflow:{file}")))
        }
        Route::User { login, tab } => {
            let login = login.to_lowercase();
            Some(match tab {
                ProfileTab::Overview => return None,
                ProfileTab::Repositories(sort) => DataKey::OwnerRepos(login, *sort),
                ProfileTab::Stars => DataKey::Stars(login),
                ProfileTab::Followers => DataKey::Users(UserList::Followers(login)),
                ProfileTab::Following => DataKey::Users(UserList::Following(login)),
                ProfileTab::People => DataKey::Users(UserList::People(login)),
            })
        }
        Route::Teams(org) => Some(DataKey::Teams(org.to_lowercase())),
        Route::Gists(login) => Some(DataKey::Gists(login.to_lowercase())),
        Route::Deployments { repo, environment } => {
            Some(DataKey::Deployments(repo.clone(), environment.clone()))
        }
        Route::Advisories(repo) => Some(DataKey::Advisories(repo.clone())),
        Route::Notifications { query } => Some(DataKey::Notifications(query.clone())),
        Route::Projects(of) => Some(DataKey::Projects(of.clone())),
        Route::Project { project, view } | Route::ProjectItem { project, view, .. } => {
            Some(DataKey::Project(project.clone(), *view))
        }
        Route::Alerts { repo, kind, query } => {
            Some(DataKey::Alerts(repo.clone(), *kind, query.clone()))
        }
        Route::App(p) => p.paged(),
        Route::Code(p) => p.paged(),
        Route::Issues(p) => p.paged(),
        Route::Pulls(p) => p.paged(),
        Route::Site(p) => p.paged(),
        Route::Housekeeping(p) => p.paged(),
        _ => route
            .search()
            .map(|(kind, query)| DataKey::Search(kind, query)),
    }
}

/// The pull request whose review a page shows at a glance: its Review
/// tab's.
pub(crate) fn glanced(route: &Route) -> Option<&PrRef> {
    match route {
        Route::Pulls(p) => p.glanced(),
        _ => None,
    }
}

/// What a page needs fetched, but Home's (its sections are the state's:
/// [`State::needs`]).
pub(crate) fn needs(route: &Route) -> Vec<DataKey> {
    use DataKey as K;
    let header = |repo: &RepoId| K::Repo(repo.clone());
    let list = route.search().map(|(kind, query)| K::Search(kind, query));
    match route {
        Route::App(_)
        | Route::Code(_)
        | Route::Issues(_)
        | Route::Pulls(_)
        | Route::NewGist
        | Route::Site(_)
        | Route::Housekeeping(_) => Vec::new(),
        Route::Search { .. } => list.into_iter().collect(),
        Route::Wiki { repo, page } => vec![header(repo), K::Wiki(repo.clone(), page.clone())],
        Route::Advisories(repo) | Route::Advisory { repo, .. } => {
            let own = match route {
                Route::Advisory { ghsa, .. } => K::Advisory(repo.clone(), ghsa.clone()),
                _ => K::Advisories(repo.clone()),
            };
            repo.iter().map(header).chain([own]).collect()
        }
        Route::Gists(_)
        | Route::Teams(_)
        | Route::Notifications { .. }
        | Route::Project { .. }
        | Route::ProjectItem { .. }
        | Route::Projects(ProjectsOf::Owner(_)) => paged(route).into_iter().collect(),
        Route::Team { org, slug } => vec![K::Team(org.to_lowercase(), slug.clone())],
        Route::Gist { id, .. } | Route::EditGist { id, .. } => {
            vec![K::Gist(id.clone())]
        }
        Route::NewRelease(repo) => vec![
            header(repo),
            K::Tags(repo.clone()),
            K::Branches(repo.clone()),
        ],
        Route::EditRelease { repo, tag } => vec![
            header(repo),
            K::Release(repo.clone(), tag.clone()),
            K::Tags(repo.clone()),
            K::Branches(repo.clone()),
        ],
        Route::WikiEdit { repo, page } => {
            vec![header(repo), crate::wiki::form::key(repo, page.as_ref())]
        }
        Route::Discussions { of, .. } | Route::Discussion { of, .. } => {
            let header = match of {
                DiscussionsOf::Repo(repo) => Some(header(repo)),
                DiscussionsOf::Org(_) => None,
            };
            let own = match route {
                Route::Discussion { number, .. } => Some(K::Discussion(of.clone(), *number)),
                _ => paged(route),
            };
            header.into_iter().chain(own).collect()
        }
        Route::WorkflowRun { repo, run, attempt } => {
            vec![header(repo), K::Run(repo.clone(), *run, *attempt)]
        }
        Route::Job { repo, job, .. } => vec![
            header(repo),
            K::Job(repo.clone(), *job),
            K::JobLog(repo.clone(), *job),
        ],
        Route::Workflow { repo, file } => std::iter::once(header(repo))
            .chain([K::Workflow(repo.clone(), file.clone())])
            .chain(paged(route))
            .collect(),
        Route::Dispatch { repo, file } => crate::actions::needs(repo, file),
        Route::Release { repo, tag } => {
            vec![header(repo), K::Release(repo.clone(), tag.clone())]
        }
        Route::Actions { repo, .. }
        | Route::Stargazers(repo)
        | Route::Watchers(repo)
        | Route::Forks(repo)
        | Route::Releases(repo)
        | Route::Deployments { repo, .. }
        | Route::Alerts { repo, .. }
        | Route::Projects(ProjectsOf::Repo(repo)) => {
            std::iter::once(header(repo)).chain(paged(route)).collect()
        }
        Route::Alert { repo, kind, number } => {
            let a = AlertRef {
                repo: repo.clone(),
                kind: *kind,
                number: *number,
            };
            vec![header(repo), K::Alert(a)]
        }
        Route::Settings { repo, page } => {
            vec![header(repo), K::Settings(repo.clone(), *page)]
        }
        Route::User { login, tab } => {
            // A user's lists show above what they starred.
            let lists = (*tab == ProfileTab::Stars).then(|| K::StarLists(login.to_lowercase()));
            std::iter::once(K::Profile(login.to_lowercase()))
                .chain(lists)
                .chain(paged(route))
                .collect()
        }
    }
}

/// Whether `need` is a repository's header on one of its other pages
/// (fetched only when missing).
pub fn is_header(route: &Route, key: &DataKey) -> bool {
    matches!(key, DataKey::Repo(_)) && !matches!(route, Route::Code(code::Repo(_)))
}

/// How wide a page gets: reading stays narrow enough to follow, lists
/// get room for their columns, and output and tables take the whole
/// width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Width {
    Reading,
    Lists,
    Full,
    /// Reading, with a sidebar.
    ReadingAside,
    /// Lists, with a sidebar.
    ListsAside,
}

/// How wide `route`'s page gets.
fn width_of(route: &Route) -> Width {
    match route {
        Route::Wiki { .. }
        | Route::Discussion { .. }
        | Route::Release { .. }
        | Route::Advisory { .. }
        | Route::Alert { .. }
        | Route::NewRelease(_)
        | Route::EditRelease { .. }
        | Route::NewGist
        | Route::EditGist { .. }
        | Route::WikiEdit { .. }
        | Route::Dispatch { .. }
        | Route::ProjectItem { .. } => Width::Reading,
        Route::Actions { .. }
        | Route::WorkflowRun { .. }
        | Route::Job { .. }
        | Route::Workflow { .. }
        | Route::Project { .. }
        | Route::Deployments { .. } => Width::Full,
        Route::App(p) => p.width(),
        Route::Code(p) => p.width(),
        Route::Issues(p) => p.width(),
        Route::Pulls(p) => p.width(),
        Route::Site(p) => p.width(),
        Route::Housekeeping(p) => p.width(),
        _ => Width::Lists,
    }
}

/// What a page is built from: the state, and what every page builder
/// needs.
#[derive(Clone, Copy)]
pub struct Build<'a> {
    pub state: &'a State,
    pub cx: pages::PageCtx<'a>,
    /// The key that tries again.
    pub retry: &'a str,
}

impl<'a> Build<'a> {
    /// `key`'s data, as the page shows it: loading, failed (and why) or
    /// ready.
    pub fn get<T: Picked + ?Sized>(&self, key: &DataKey) -> Fetched<'a, T> {
        Remote::fetched(self.state.remote(key), self.retry).pick(T::pick)
    }

    /// The list the page loads more of.
    pub fn list<T: Picked + ?Sized>(&self, key: Option<DataKey>) -> Fetched<'a, T> {
        let remote = key.and_then(|key| self.state.remote(&key));
        Remote::fetched(remote, self.retry).pick(T::pick)
    }
}

/// A page's needs as the page shows them, failing with the key that
/// tries again.
struct Needs<'a> {
    state: &'a State,
    retry: &'a str,
}

impl<'a> Needs<'a> {
    fn of<T>(&self, remote: Option<&'a Remote<T>>) -> Fetched<'a, T> {
        Remote::fetched(remote, self.retry)
    }

    fn get<T: Picked + ?Sized>(&self, key: &DataKey) -> Fetched<'a, T> {
        self.of(self.state.remote(key)).pick(T::pick)
    }

    /// The list the route loads more of.
    fn paged<T: Picked + ?Sized>(&self, route: &Route) -> Fetched<'a, T> {
        let remote = paged(route).and_then(|key| self.state.remote(&key));
        self.of(remote).pick(T::pick)
    }
}

impl State {
    /// How fetching `key` is going, once it's started.
    pub fn remote(&self, key: &DataKey) -> Option<&Remote<Data>> {
        #[cfg(test)]
        reads::record(key);
        self.data.get(key)
    }

    pub fn get(&self, key: &DataKey) -> Option<&Data> {
        self.remote(key).and_then(|r| r.data.as_ref())
    }

    /// `key`'s data, if it's there: for what isn't a page.
    pub fn picked<T: Picked + ?Sized>(&self, key: &DataKey) -> Option<&T> {
        self.get(key).and_then(T::pick)
    }

    /// Where commit `oid` is among pull request `number`'s commits, once
    /// they're in, with links to the ones either side.
    pub fn pr_commit_at(&self, repo: &RepoId, number: u64, oid: &str) -> Option<pages::PrCommitAt> {
        let pr = PrRef {
            repo: repo.clone(),
            number,
        };
        let commits = &self
            .picked::<PrActivity>(&DataKey::PrActivity(pr.clone()))?
            .commits;
        let i = commits
            .items
            .iter()
            .position(|c| c.oid.starts_with(oid) || oid.starts_with(&c.oid))?;
        let link = |c: &CommitInfo| {
            let route = Route::Code(code::Commit {
                repo: repo.clone(),
                oid: c.oid.clone(),
                pr: Some(number),
            });
            route.url()
        };
        Some(pages::PrCommitAt {
            n: commits.left_out() + i as u64 + 1,
            of: commits.total(),
            pr: pages::url::pull(&pr),
            number,
            previous: i
                .checked_sub(1)
                .and_then(|i| commits.items.get(i))
                .map(link),
            next: commits.items.get(i + 1).map(link),
        })
    }

    pub fn overview(&self, repo: &RepoId) -> Option<&RepoOverview> {
        self.picked(&DataKey::Repo(repo.clone()))
    }

    pub fn pr(&self, pr: &PrRef) -> Option<&PrDetail> {
        self.picked(&DataKey::Pr(pr.clone()))
    }

    /// How fetching pull request `pr` is going.
    pub fn pr_remote(&self, pr: &PrRef) -> Option<&Remote<Data>> {
        self.remote(&DataKey::Pr(pr.clone()))
    }

    pub fn activity(&self, pr: &PrRef) -> Option<&PrActivity> {
        self.picked(&DataKey::PrActivity(pr.clone()))
    }

    /// Which state of its diff's inputs a page shows: a pull request's
    /// Review tab shows the diff's threads and drafts.
    pub(crate) fn diff_version(&self, route: &Route) -> Option<u64> {
        let of = DiffOf::Pr(glanced(route)?.clone());
        self.diffs.get(&of).map(DiffState::version)
    }

    /// How a page's fetches are going (the ones that have started).
    pub(crate) fn fetches(&self, route: &Route) -> impl Iterator<Item = Remote<()>> {
        (self.needs(route).into_iter()).filter_map(|key| self.data.get(&key).map(Remote::status))
    }

    /// Why the page's needs that show a copy couldn't refresh it, each
    /// reason once (one outage fails them all alike).
    fn stale_errors(&self, route: &Route) -> Option<String> {
        let fetches = self.fetches(route).filter(|r| r.data.is_some());
        let errors = BTreeSet::from_iter(fetches.filter_map(|r| r.error));
        (!errors.is_empty()).then(|| Vec::from_iter(errors).join("; "))
    }

    /// When the oldest cached copy on the page was fetched, while one is
    /// shown in place of fresh data.
    pub fn page_cached_at(&self, route: &Route) -> Option<u64> {
        self.fetches(route).filter_map(|r| r.cached_at).min()
    }

    /// One row per list item.
    pub fn compact(&self) -> bool {
        match self.density {
            crate::config::Density::Compact => true,
            crate::config::Density::Comfortable => false,
            crate::config::Density::Auto => self.size.1 < 30,
        }
    }

    pub fn page_loading(&self, route: &Route) -> bool {
        self.fetches(route).any(|r| r.loading)
    }

    /// The first key that runs `action` here, as people write it.
    pub fn first_key(&self, action: Action) -> String {
        self.key_here(action)
            .unwrap_or_else(|| format!(":{}", action.name().replace('_', " ")))
    }

    /// Builds the page for `route`, `width` columns wide in all.
    pub fn build_page(&self, route: &Route, width: u16, now: u64) -> Page {
        let icons = self.icons;
        let (comment, find_file, filter) = (
            self.first_key(Action::Comment),
            self.first_key(Action::FindFile),
            self.first_key(Action::Search),
        );
        let archive = self.first_key(Action::ArchiveItem);
        let keys = Keys {
            comment: &comment,
            find_file: &find_file,
            filter: &filter,
            archive: &archive,
        };
        let cx = pages::PageCtx { icons, keys, now };
        // Reading stays narrow enough to follow, lists get room for their
        // columns, and output and tables take the whole width.
        let (reading, lists) = (width.min(READING), width.min(LISTS));
        let mut page = match width_of(route) {
            Width::Reading => Page::new(reading),
            Width::Lists => Page::new(lists),
            Width::Full => Page::new(width),
            Width::ReadingAside => Page::with_aside(reading),
            Width::ListsAside => Page::with_aside(lists),
        };
        page.compact = self.compact();
        if let Some(err) = self.stale_errors(route) {
            // A flash banner on top; what's below is the cached copy.
            let when = self
                .page_cached_at(route)
                .map_or_else(|| "before".to_owned(), |at| crate::ui::time::ago(at, now));
            let why = crate::offline::banner(self, &when).unwrap_or_else(|| {
                format!("Couldn't refresh: {err}. Showing what was loaded {when}.")
            });
            pages::flash(&mut page, &why);
            page.blank();
        }
        // Where the page was read: a clone at a commit, or GitHub's API.
        if let Some(from) = crate::source::shown(route).and_then(|k| self.sources.get(&k)) {
            let note = vec![Seg::new(from.note(now), Role::Meta)];
            page.wrapped(note, 0, crate::ui::page::Frame::None);
        }
        // Writes made here, waiting for GitHub.
        crate::offline::queued_here(self, route, &mut page, now);
        let banner = page.lines.len();
        let retry = self.first_key(Action::Refresh);
        let f = Needs {
            state: self,
            retry: &retry,
        };
        let title = route.title();
        let b = Build {
            state: self,
            cx,
            retry: &retry,
        };
        match route {
            Route::App(p) => p.build(b, &mut page),
            Route::Code(p) => p.build(b, &mut page),
            Route::Issues(p) => p.build(b, &mut page),
            Route::Pulls(p) => p.build(b, &mut page),
            Route::Search { kind, query } => {
                let results = f.paged(route);
                pages::search(&mut page, *kind, query, results, icons, keys, now);
            }
            Route::NewRelease(repo) => {
                let header = f.get::<RepoOverview>(&DataKey::Repo(repo.clone()));
                if header.show(&mut page, &title).is_some() {
                    crate::release::build(self, &mut page, route, repo);
                }
            }
            Route::EditRelease { repo, tag } => {
                let key = DataKey::Release(repo.clone(), tag.clone());
                if f.get::<Release>(&key).show(&mut page, &title).is_some() {
                    crate::release::build(self, &mut page, route, repo);
                }
            }
            Route::NewGist => crate::gist::build(self, &mut page, route),
            Route::EditGist { id, .. } => {
                if f.get::<Gist>(&DataKey::Gist(id.clone()))
                    .show(&mut page, &title)
                    .is_some()
                {
                    crate::gist::build(self, &mut page, route);
                }
            }
            Route::WikiEdit { repo, page: name } => {
                let key = crate::wiki::form::key(repo, name.as_ref());
                if f.get::<WikiPage>(&key).show(&mut page, &title).is_some() {
                    crate::wiki::form::build(self, &mut page, route);
                }
            }
            Route::Stargazers(repo) => {
                // GitHub lists none of a big repository's stargazers.
                let stars = self.overview(repo).map_or(0, |o| o.summary.stars);
                let empty = match stars {
                    0 => "Nobody here yet.".to_owned(),
                    n => format!(
                        "GitHub doesn't list this repository's {} stargazers.",
                        pages::compact(n)
                    ),
                };
                pages::people_list(&mut page, "Stargazers", f.paged(route), &empty);
            }
            Route::Watchers(_) => {
                pages::people_list(&mut page, "Watchers", f.paged(route), "Nobody here yet.");
            }
            Route::Forks(_) => pages::repos(&mut page, "Forks", f.paged(route), now),
            Route::Releases(repo) => pages::releases(&mut page, repo, f.paged(route), now),
            Route::Wiki { repo, page: name } => {
                let key = DataKey::Wiki(repo.clone(), name.clone());
                if let Some(w) = f.get(&key).show(&mut page, &title) {
                    pages::wiki(&mut page, repo, w);
                }
            }
            Route::Advisories(repo) => {
                if let Some(repo) = repo {
                    pages::security_nav(&mut page, repo, None);
                }
                pages::advisories(&mut page, repo.as_ref(), f.paged(route), now);
            }
            Route::Alerts { repo, kind, query } => {
                pages::security_nav(&mut page, repo, Some(*kind));
                pages::alerts(&mut page, repo, *kind, query, f.paged(route), cx);
            }
            Route::Alert { repo, kind, number } => {
                let key = DataKey::Alert(AlertRef {
                    repo: repo.clone(),
                    kind: *kind,
                    number: *number,
                });
                if let Some(a) = f.get(&key).show(&mut page, &title) {
                    pages::alert(&mut page, repo, a, cx);
                }
            }
            Route::Settings { repo, page: at } => {
                let key = DataKey::Settings(repo.clone(), *at);
                crate::settings::build(self, &mut page, repo, *at, f.get(&key), now);
            }
            Route::Advisory { repo, ghsa } => {
                let key = DataKey::Advisory(repo.clone(), ghsa.clone());
                if let Some(a) = f.get(&key).show(&mut page, ghsa) {
                    pages::advisory(&mut page, a, now);
                }
            }
            Route::Teams(org) => pages::teams(&mut page, org, f.paged(route)),
            Route::Team { org, slug } => {
                let key = DataKey::Team(org.to_lowercase(), slug.clone());
                if let Some(t) = f.get(&key).show(&mut page, &title) {
                    pages::team(&mut page, org, t);
                }
            }
            Route::Gist { id, .. } => {
                if let Some(g) = f.get(&DataKey::Gist(id.clone())).show(&mut page, &title) {
                    pages::gist(&mut page, g, now);
                }
            }
            Route::Gists(login) => pages::gists(&mut page, login, f.paged(route), now),
            Route::Deployments { repo, environment } => {
                let env = environment.as_deref();
                pages::deployments(&mut page, repo, f.paged(route), env, now);
            }
            Route::Discussions { of, category } => {
                let list = f.paged(route);
                let base = Route::Discussions {
                    of: of.clone(),
                    category: None,
                };
                pages::discussions(&mut page, &base.url(), list, category.as_deref(), now);
            }
            Route::Discussion { of, number } => {
                let key = DataKey::Discussion(of.clone(), *number);
                let none = HashSet::new();
                let fresh = crate::live::page::fresh(self, route).unwrap_or(&none);
                if let Some(d) = f.get(&key).show(&mut page, &title) {
                    pages::discussion(&mut page, d, fresh, &self.talk.unfolded, cx);
                }
            }
            Route::WorkflowRun { repo, run, attempt } => {
                let key = DataKey::Run(repo.clone(), *run, *attempt);
                if let Some(r) = f.get(&key).show(&mut page, &title) {
                    pages::workflow_run(&mut page, repo, r, now);
                }
            }
            Route::Job {
                repo,
                job,
                step,
                query,
                ..
            } => {
                let key = DataKey::Job(repo.clone(), *job);
                if let Some(j) = f.get(&key).show(&mut page, &title) {
                    let log = f.get(&DataKey::JobLog(repo.clone(), *job));
                    let at = pages::JobAt {
                        step: *step,
                        query,
                        keys,
                        follows: crate::live::page::follows(self, repo, *job),
                    };
                    pages::job(&mut page, repo, j, log, at, now);
                }
            }
            Route::Workflow { repo, file } => {
                let wf = f.get(&DataKey::Workflow(repo.clone(), file.clone()));
                pages::workflow(&mut page, repo, wf, f.paged(route), now);
            }
            Route::Dispatch { repo, file } => crate::actions::build(self, &mut page, repo, file),
            Route::Release { repo, tag } => {
                let key = DataKey::Release(repo.clone(), tag.clone());
                if let Some(r) = f.get(&key).show(&mut page, &title) {
                    pages::release(&mut page, repo, r, now);
                }
            }
            Route::Actions { repo, query } => {
                pages::actions(&mut page, repo, query, f.paged(route), cx);
            }
            Route::Notifications { query } => {
                pages::notifications(&mut page, query, f.paged(route), cx);
            }
            Route::Projects(of) => pages::projects(&mut page, of, f.paged(route), now),
            Route::Project { project, view } => {
                if let Some(p) = f.paged(route).show(&mut page, &title) {
                    pages::project_view(&mut page, project, *view, p, cx);
                }
            }
            Route::ProjectItem {
                project,
                view,
                item,
            } => {
                if let Some(p) = f.paged::<Project>(route).show(&mut page, &title) {
                    pages::project_item(&mut page, project, *view, p, *item, cx);
                }
            }
            Route::User { login, tab } => {
                let key = DataKey::Profile(login.to_lowercase());
                if let Some(p) = f.get(&key).show(&mut page, &format!("@{login}")) {
                    let list = match paged(route) {
                        Some(key @ DataKey::Users(_)) => ProfileList::People(f.get(&key)),
                        Some(key) if *tab == ProfileTab::Stars => {
                            let lists = DataKey::StarLists(login.to_lowercase());
                            let lists = f.get(&lists);
                            ProfileList::Starred(lists, f.get(&key))
                        }
                        Some(key) => ProfileList::Repos(f.get(&key)),
                        None => ProfileList::None,
                    };
                    pages::profile(&mut page, p, *tab, list, now);
                }
            }
            Route::Site(p) => p.build(b, &mut page),
            Route::Housekeeping(p) => p.build(b, &mut page),
        }
        // The sidebar starts beside the page, below the banner.
        if let Some(a) = page.aside.as_mut().filter(|a| !a.lines.is_empty()) {
            a.lines.splice(0..0, vec![PageLine::default(); banner]);
        }
        page
    }
}

/// What building a page reads, recorded in tests to check a page reads
/// only what it needs.
#[cfg(test)]
pub(crate) mod reads {
    use std::cell::RefCell;

    use crate::data::DataKey;

    thread_local! {
        static READS: RefCell<Option<Vec<DataKey>>> = const { RefCell::new(None) };
    }

    pub(crate) fn record(key: &DataKey) {
        READS.with_borrow_mut(|reads| {
            if let Some(reads) = reads {
                reads.push(key.clone());
            }
        });
    }

    /// What `f` reads.
    pub(crate) fn during<R>(f: impl FnOnce() -> R) -> (R, Vec<DataKey>) {
        READS.set(Some(Vec::new()));
        let out = f();
        (out, READS.take().unwrap_or_default())
    }
}
