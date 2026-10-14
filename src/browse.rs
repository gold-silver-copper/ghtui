//! Browsing: the data behind each page, and building pages from it.

use std::collections::HashSet;

use std::collections::BTreeSet;

use crate::api::browse::{
    CommitDetail, CommitInfo, DiscussionsOf, Gist, IssueDetail, PrActivity, Release, RepoOverview,
    SearchResults, StateCounts, UserList, WikiPage,
};
use crate::api::model::{PrDetail, PrRef, RepoId};
use crate::api::project::{Project, ProjectsOf};
use crate::api::security::AlertRef;
use crate::ui::Fetched;
use crate::ui::page::{Page, PageLine, Role, Seg};
use crate::ui::pages::{self, Keys, PrTab, ProfileList, ProfileTab};

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
        Route::Commits { repo, rev, path } => {
            Some(DataKey::History(repo.clone(), rev.clone(), path.clone()))
        }
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
        Route::Tags(repo) => Some(DataKey::Tags(repo.clone())),
        Route::Branches(repo) => Some(DataKey::Branches(repo.clone())),
        Route::Teams(org) => Some(DataKey::Teams(org.to_lowercase())),
        Route::Gists(login) => Some(DataKey::Gists(login.to_lowercase())),
        Route::Compare { repo, spec } => Some(DataKey::Compare(repo.clone(), spec.clone())),
        Route::Deployments { repo, environment } => {
            Some(DataKey::Deployments(repo.clone(), environment.clone()))
        }
        Route::Milestones { repo, closed } => Some(DataKey::Milestones(repo.clone(), *closed)),
        Route::Milestone { repo, number } => Some(DataKey::Milestone(repo.clone(), *number)),
        Route::Advisories(repo) => Some(DataKey::Advisories(repo.clone())),
        Route::Notifications { query } => Some(DataKey::Notifications(query.clone())),
        Route::Projects(of) => Some(DataKey::Projects(of.clone())),
        Route::Project { project, view } | Route::ProjectItem { project, view, .. } => {
            Some(DataKey::Project(project.clone(), *view))
        }
        Route::Alerts { repo, kind, query } => {
            Some(DataKey::Alerts(repo.clone(), *kind, query.clone()))
        }
        Route::Site(page) => page.paged(),
        Route::Labels(_) => crate::housekeeping::paged(route),
        _ => route
            .search()
            .map(|(kind, query)| DataKey::Search(kind, query)),
    }
}

/// The pull request whose review a page shows at a glance: its Review
/// tab's. Its diff's threads and drafts load with it.
pub(crate) fn glanced(route: &Route) -> Option<&PrRef> {
    match route {
        Route::Pr {
            pr,
            tab: PrTab::Review,
        } => Some(pr),
        _ => None,
    }
}

/// What a page needs fetched, but Home's (its sections are the state's:
/// [`State::needs`]).
pub fn needs(route: &Route) -> Vec<DataKey> {
    use DataKey as K;
    let header = |repo: &RepoId| K::Repo(repo.clone());
    let list = route.search().map(|(kind, query)| K::Search(kind, query));
    match route {
        Route::Home | Route::NewGist | Route::Offline => Vec::new(),
        Route::Repo(repo) => vec![
            header(repo),
            K::Readme(repo.clone()),
            K::LastCommits(repo.clone(), "HEAD".into(), String::new()),
        ],
        Route::Tree { repo, rev, path } => vec![
            header(repo),
            K::Tree(repo.clone(), rev.clone(), path.clone()),
            K::LastCommits(repo.clone(), rev.clone(), path.clone()),
        ],
        Route::Blob {
            repo, rev, path, ..
        } => vec![
            header(repo),
            K::Blob(repo.clone(), rev.clone(), path.clone()),
        ],
        Route::Blame {
            repo, rev, path, ..
        } => vec![
            header(repo),
            K::Blob(repo.clone(), rev.clone(), path.clone()),
            K::Blame(repo.clone(), rev.clone(), path.clone()),
        ],
        Route::Unsplit { repo, spot, .. } => {
            vec![header(repo), K::RefIn(repo.clone(), spot.clone())]
        }
        Route::Issues { repo, .. } | Route::Pulls { repo, .. } => {
            let counts = route.state_counts().map(K::StateCounts);
            std::iter::once(header(repo))
                .chain(list)
                .chain(counts)
                .collect()
        }
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
        Route::Issue { repo, number } => {
            vec![
                header(repo),
                K::InProjects(repo.clone(), *number),
                K::Issue(repo.clone(), *number),
            ]
        }
        Route::NewIssue(repo) | Route::NewPr(repo) => {
            vec![header(repo), K::NewForm(repo.clone())]
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
        Route::Pr {
            pr,
            tab: PrTab::Checks,
        } => vec![
            DataKey::Pr(pr.clone()),
            K::PrActivity(pr.clone()),
            K::PrChecks(pr.clone()),
        ],
        // The Review tab and a review list every review.
        Route::Pr {
            pr,
            tab: PrTab::Review | PrTab::Reviewed(_),
        } => vec![
            DataKey::Pr(pr.clone()),
            K::PrActivity(pr.clone()),
            K::Reviews(pr.clone()),
        ],
        Route::Pr {
            pr,
            tab: PrTab::Conversation,
        } => vec![
            DataKey::Pr(pr.clone()),
            K::InProjects(pr.repo.clone(), pr.number),
            K::PrActivity(pr.clone()),
        ],
        Route::Pr { pr, .. } => vec![DataKey::Pr(pr.clone()), K::PrActivity(pr.clone())],
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
        Route::CommitChecks { repo, oid } => vec![
            header(repo),
            K::Commit(repo.clone(), oid.clone()),
            K::CommitChecks(repo.clone(), oid.clone()),
        ],
        Route::Release { repo, tag } => {
            vec![header(repo), K::Release(repo.clone(), tag.clone())]
        }
        Route::Actions { repo, .. }
        | Route::Stargazers(repo)
        | Route::Watchers(repo)
        | Route::Forks(repo)
        | Route::Releases(repo)
        | Route::Tags(repo)
        | Route::Branches(repo)
        | Route::Compare { repo, .. }
        | Route::Deployments { repo, .. }
        | Route::Milestones { repo, .. }
        | Route::Milestone { repo, .. }
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
        Route::Commit { repo, oid, pr } => {
            let mut needs = vec![
                header(repo),
                K::CommitComments(repo.clone(), oid.clone()),
                K::Commit(repo.clone(), oid.clone()),
            ];
            // Its place among the pull request's commits.
            if let Some(number) = pr {
                let pr = PrRef {
                    repo: repo.clone(),
                    number: *number,
                };
                needs.push(K::PrActivity(pr));
            }
            needs
        }
        Route::Commits { repo, rev, path } => vec![
            header(repo),
            K::History(repo.clone(), rev.clone(), path.clone()),
        ],
        Route::User { login, tab } => {
            // A user's lists show above what they starred.
            let lists = (*tab == ProfileTab::Stars).then(|| K::StarLists(login.to_lowercase()));
            std::iter::once(K::Profile(login.to_lowercase()))
                .chain(lists)
                .chain(paged(route))
                .collect()
        }
        Route::Site(page) => page.needs(),
        Route::Labels(_) | Route::Housekeeping(_) => crate::housekeeping::needs(route),
    }
}

/// Whether `need` is a repository's header on one of its other pages
/// (fetched only when missing).
pub fn is_header(route: &Route, key: &DataKey) -> bool {
    matches!(key, DataKey::Repo(_)) && !matches!(route, Route::Repo(_))
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
        self.of(self.state.data.get(key)).pick(T::pick)
    }

    /// The list the route loads more of.
    fn paged<T: Picked + ?Sized>(&self, route: &Route) -> Fetched<'a, T> {
        let remote = paged(route).and_then(|key| self.state.data.get(&key));
        self.of(remote).pick(T::pick)
    }
}

impl State {
    pub fn get(&self, key: &DataKey) -> Option<&Data> {
        self.data.get(key).and_then(|r| r.data.as_ref())
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
            let route = Route::Commit {
                repo: repo.clone(),
                oid: c.oid.clone(),
                pr: Some(number),
            };
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
        self.data.get(&DataKey::Pr(pr.clone()))
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
        let mut page = match route {
            Route::Issue { .. }
            | Route::Pr {
                tab: PrTab::Conversation,
                ..
            } => Page::with_aside(reading),
            Route::Repo(_) => Page::with_aside(lists),
            Route::Blob { path, .. } if pages::is_markdown(path) => Page::new(reading),
            Route::Wiki { .. }
            | Route::Discussion { .. }
            | Route::Release { .. }
            | Route::Advisory { .. }
            | Route::Alert { .. }
            | Route::NewIssue(_)
            | Route::NewPr(_)
            | Route::NewRelease(_)
            | Route::EditRelease { .. }
            | Route::NewGist
            | Route::EditGist { .. }
            | Route::WikiEdit { .. }
            | Route::Housekeeping(_)
            | Route::Offline
            | Route::Dispatch { .. }
            | Route::ProjectItem { .. }
            | Route::Pr {
                tab: PrTab::Review | PrTab::Reviewed(_),
                ..
            } => Page::new(reading),
            Route::Pr {
                tab: PrTab::Checks, ..
            }
            | Route::CommitChecks { .. }
            | Route::Actions { .. }
            | Route::WorkflowRun { .. }
            | Route::Job { .. }
            | Route::Workflow { .. }
            | Route::Blob { .. }
            | Route::Blame { .. }
            | Route::Branches(_)
            | Route::Tags(_)
            | Route::Project { .. }
            | Route::Deployments { .. } => Page::new(width),
            _ => Page::new(lists),
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
        match route {
            Route::Home => {
                let sections: Vec<pages::HomeSection<'_>> = self
                    .home
                    .iter()
                    .map(|s| pages::HomeSection {
                        title: &s.title,
                        search: s.search.as_ref().map(|q| pages::SectionSearch {
                            kind: q.kind,
                            query: &q.query,
                            rows: q.rows,
                            results: f.get(&q.key()),
                        }),
                    })
                    .collect();
                pages::home(&mut page, &sections, icons, now);
            }
            Route::Repo(repo) => {
                let overview = f.get(&DataKey::Repo(repo.clone()));
                #[expect(clippy::disallowed_methods, reason = "extra: shown once loaded")]
                pages::repo_title(&mut page, repo, overview.ready_unchecked());
                if let Some(o) = overview.show(&mut page, "the repository") {
                    let key = DataKey::LastCommits(repo.clone(), "HEAD".into(), String::new());
                    #[expect(clippy::disallowed_methods, reason = "extra: shown once loaded")]
                    let commits = f.get(&key).ready_unchecked();
                    let readme = f.get(&DataKey::Readme(repo.clone()));
                    pages::repo_code(&mut page, repo, o, commits, readme, cx);
                }
            }
            Route::Tree { repo, rev, path } => {
                let key = DataKey::LastCommits(repo.clone(), rev.clone(), path.clone());
                #[expect(clippy::disallowed_methods, reason = "extra: shown once loaded")]
                let commits = f.get(&key).ready_unchecked();
                let dir = pages::Listing {
                    repo,
                    rev,
                    path,
                    entries: f.get(&DataKey::Tree(repo.clone(), rev.clone(), path.clone())),
                    commits,
                };
                pages::repo_dir(&mut page, dir, cx);
            }
            Route::Blob {
                repo,
                rev,
                path,
                lines,
            } => {
                let key = DataKey::Blob(repo.clone(), rev.clone(), path.clone());
                if let Some(blob) = f.get(&key).show(&mut page, "the file") {
                    let file = pages::FileAt {
                        repo,
                        rev,
                        path,
                        lines: *lines,
                    };
                    pages::file(&mut page, file, blob, keys);
                }
            }
            Route::Blame {
                repo,
                rev,
                path,
                lines,
            } => {
                let blob = f.get(&DataKey::Blob(repo.clone(), rev.clone(), path.clone()));
                let blame = f.get(&DataKey::Blame(repo.clone(), rev.clone(), path.clone()));
                let file = pages::FileAt {
                    repo,
                    rev,
                    path,
                    lines: *lines,
                };
                pages::blame(&mut page, file, blob, blame, keys, now);
            }
            // Shown until the ref is known, then replaced (`State::settle`).
            Route::Unsplit { repo, spot, .. } => {
                let key = DataKey::RefIn(repo.clone(), spot.clone());
                let _ = f.get::<Option<String>>(&key).show(&mut page, "the branch");
            }
            Route::Issues { repo, query } | Route::Pulls { repo, query } => {
                let is_pr = matches!(route, Route::Pulls { .. });
                // A filtered list counts what the filter finds, as GitHub's
                // does; until it's counted, no counts show.
                #[expect(clippy::disallowed_methods, reason = "extra: shown once loaded")]
                let counts = match route.state_counts() {
                    Some(q) => (f.get::<StateCounts>(&DataKey::StateCounts(q)))
                        .ready_unchecked()
                        .map(|c| (c.open, c.closed)),
                    None => (f.get::<RepoOverview>(&DataKey::Repo(repo.clone())))
                        .ready_unchecked()
                        .map(|o| {
                            if is_pr {
                                (o.open_prs, o.closed_prs)
                            } else {
                                (o.open_issues, o.closed_issues)
                            }
                        }),
                };
                let results = f.paged(route).pick(|r| match r {
                    SearchResults::Issues(r) => Some(r),
                    _ => None,
                });
                // Pinned issues, above the list, as on GitHub.
                #[expect(clippy::disallowed_methods, reason = "extra: shown once loaded")]
                let overview = f
                    .get::<RepoOverview>(&DataKey::Repo(repo.clone()))
                    .ready_unchecked();
                if let Some(o) = overview.filter(|_| !is_pr) {
                    pages::pinned_issues(&mut page, repo, &o.pinned_issues, icons);
                }
                // Labels and milestones, beside the list as on GitHub.
                pages::labels_and_milestones(&mut page, repo);
                pages::issue_list(&mut page, query, counts, is_pr, results, cx);
            }
            Route::Search { kind, query } => {
                let results = f.paged(route);
                pages::search(&mut page, *kind, query, results, icons, keys, now);
            }
            Route::Issue { repo, number } => {
                let issue =
                    f.get::<Option<Box<IssueDetail>>>(&DataKey::Issue(repo.clone(), *number));
                let none = HashSet::new();
                let fresh = crate::live::page::fresh(self, route).unwrap_or(&none);
                match issue.show(&mut page, &format!("{repo}#{number}")) {
                    Some(Some(issue)) => {
                        let lines = pages::LinesAt::new();
                        let mut marks = crate::talk::marks(self, None, &lines);
                        marks.comments = fresh;
                        let projects = f.get(&DataKey::InProjects(repo.clone(), *number));
                        pages::issue(&mut page, issue, projects, marks, cx);
                    }
                    // Redirecting to the pull request.
                    Some(None) => page.line(vec![Seg::new("Loading…", Role::Meta)]),
                    None => {}
                }
            }
            Route::Pr { pr, tab } => {
                let activity = f.get(&DataKey::PrActivity(pr.clone()));
                let reviews = f.get(&DataKey::Reviews(pr.clone()));
                let lines = crate::talk::lines_at(self, pr);
                let marks = crate::talk::marks(self, Some(pr), &lines);
                let detail = f.get::<PrDetail>(&DataKey::Pr(pr.clone()));
                if let Some(d) = detail.show(&mut page, &pr.to_string()) {
                    match tab {
                        PrTab::Conversation => {
                            let projects = DataKey::InProjects(pr.repo.clone(), pr.number);
                            let projects = f.get(&projects);
                            pages::pr_conversation(&mut page, pr, d, activity, projects, marks, cx);
                        }
                        PrTab::Commits => pages::pr_commits(&mut page, pr, d, activity, now),
                        PrTab::Checks => {
                            let checks = f.get(&DataKey::PrChecks(pr.clone()));
                            pages::pr_checks(&mut page, pr, d, checks, now);
                        }
                        PrTab::Review => {
                            let diff = self.diffs.get(&DiffOf::Pr(pr.clone()));
                            let inputs = diff.map(DiffState::inputs);
                            let glance = pages::Glance {
                                threads: Remote::fetched(inputs.map(|i| &i.threads), &retry),
                                drafts: inputs
                                    .filter(|i| i.review_loaded)
                                    .map(|i| i.review.pending.as_slice()),
                                fresh: marks.threads,
                                reviews,
                                new_reviews: marks.reviews,
                            };
                            pages::pr_review(&mut page, pr, d, activity, glance, cx);
                        }
                        PrTab::Reviewed(id) => {
                            pages::pr_reviewed(&mut page, pr, d, *id, activity, reviews, marks, cx);
                        }
                    }
                }
            }
            Route::NewIssue(repo) | Route::NewPr(repo) => {
                let key = DataKey::NewForm(repo.clone());
                if let Some(d) = f.get(&key).show(&mut page, &title) {
                    crate::create::build(self, &mut page, route, d);
                }
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
            Route::Tags(repo) => pages::tags(&mut page, repo, f.paged(route), now),
            Route::Branches(repo) => pages::branches(&mut page, repo, f.paged(route), icons, now),
            Route::Milestones { repo, closed } => {
                pages::milestones(&mut page, repo, f.paged(route), *closed, now);
            }
            Route::Milestone { repo, .. } => {
                if let Some(m) = f.paged(route).show(&mut page, &title) {
                    pages::milestone(&mut page, repo, m, icons, now);
                }
            }
            Route::Compare { repo, spec } => {
                if let Some(c) = f.paged(route).show(&mut page, &title) {
                    pages::compare(&mut page, repo, spec, c, now);
                }
            }
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
            Route::CommitChecks { repo, oid } => {
                let checks = f.get(&DataKey::CommitChecks(repo.clone(), oid.clone()));
                pages::commit_checks(&mut page, checks, now);
            }
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
            Route::Commits { repo, rev, path } => {
                pages::commit_history(&mut page, repo, rev, path, f.paged(route), now);
            }
            Route::Commit { repo, oid, pr } => {
                let key = DataKey::Commit(repo.clone(), oid.clone());
                if let Some(c) = f.get::<CommitDetail>(&key).show(&mut page, &title) {
                    if let Some(at) = pr.and_then(|n| self.pr_commit_at(repo, n, &c.oid)) {
                        pages::pr_commit_nav(&mut page, &at);
                    }
                    let files = Route::Commit {
                        repo: repo.clone(),
                        oid: c.oid.clone(),
                        pr: None,
                    };
                    let files = format!("{}#files", files.url());
                    pages::commit(&mut page, repo, c, &files, now);
                    let comments = DataKey::CommitComments(repo.clone(), oid.clone());
                    pages::commit_comments(&mut page, repo, c, f.get(&comments), cx);
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
            Route::Site(p) => crate::site::build(self, &mut page, p, &retry, cx),
            Route::Labels(_) | Route::Housekeeping(_) => {
                crate::housekeeping::build(self, &mut page, route, now);
            }
            Route::Offline => crate::offline::build(self, &mut page, now),
        }
        // The sidebar starts beside the page, below the banner.
        if let Some(a) = page.aside.as_mut().filter(|a| !a.lines.is_empty()) {
            a.lines.splice(0..0, vec![PageLine::default(); banner]);
        }
        page
    }
}
