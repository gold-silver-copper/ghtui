//! Browsing: the data behind each page, and building pages from it.

use std::collections::HashSet;
use std::sync::Arc;

use std::collections::{BTreeSet, HashMap};

use crate::api::browse::{
    Advisory, Blame, Blob, BranchInfo, CheckOutcome, Checks, CommitDetail, CommitInfo, Comparison,
    DeploymentList, DiscussionDetail, DiscussionList, DiscussionsOf, Gist, GistSummary,
    IssueDetail, Job, MilestoneDetail, MilestoneList, PrActivity, Profile, Readme, Refs, Release,
    RepoOverview, RepoSort, RepoSummary, Results, RunSummary, SearchKind, SearchResults,
    StateCounts, TagInfo, TeamDetail, TeamSummary, TreeEntry, UserList, UserSummary, WikiPage,
    Workflow, WorkflowRun,
};
use crate::api::commit_comments::CommitComment;
use crate::api::model::{Capped, PrRef, RepoId};
use crate::api::notifications::Inbox;
use crate::api::project::{Project, ProjectRef, ProjectSummary, ProjectsOf};
use crate::api::security::{Alert, AlertKind, AlertList, AlertRef};
use crate::api::timeline::{InProject, Review};
use crate::ui::Fetched;
use crate::ui::page::{Page, PageLine, Role, Seg};
use crate::ui::pages::{self, Keys, PrTab, ProfileList, ProfileTab};

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

/// One piece of GitHub data a page shows.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DataKey {
    Repo(RepoId),
    /// The README at the root, apart from the overview every repository
    /// page has for its header.
    Readme(RepoId),
    Tree(RepoId, String, String),
    Blob(RepoId, String, String),
    Search(SearchKind, String),
    /// A filtered list's open and closed counts ([`Route::state_counts`]).
    StateCounts(String),
    Issue(RepoId, u64),
    PrActivity(PrRef),
    /// The projects an issue or pull request is in.
    InProjects(RepoId, u64),
    /// A file's lines at a commit, from git: where a review thread is.
    Lines(RepoId, String, String),
    /// A commit's comments, by the revision linked.
    CommitComments(RepoId, String),
    /// All of a pull request's reviews.
    Reviews(PrRef),
    Profile(String),
    /// Every file path at a revision ("Go to file").
    Files(RepoId, String),
    /// Branches and tags.
    Refs(RepoId),
    /// The ref a plain URL's ref-and-path starts with (see
    /// [`Route::Unsplit`]).
    RefIn(RepoId, String),
    /// The latest commit of each entry in a directory.
    LastCommits(RepoId, String, String),
    /// A commit, by the revision linked.
    Commit(RepoId, String),
    /// A revision's commits (of a path, if it isn't empty).
    History(RepoId, String, String),
    /// The checks on a pull request's head.
    PrChecks(PrRef),
    Users(UserList),
    Forks(RepoId),
    Releases(RepoId),
    Release(RepoId, String),
    Tags(RepoId),
    Branches(RepoId),
    /// From git, not GitHub's API.
    Wiki(RepoId, Option<String>),
    Advisories(Option<RepoId>),
    Advisory(Option<RepoId>, String),
    Teams(String),
    Team(String, String),
    Gist(String),
    Gists(String),
    Blame(RepoId, String, String),
    Compare(RepoId, String),
    Deployments(RepoId, Option<String>),
    Milestones(RepoId, bool),
    Milestone(RepoId, u64),
    Discussions(DiscussionsOf, Option<String>),
    Discussion(DiscussionsOf, u64),
    /// A workflow run (an attempt of it, or the latest).
    Run(RepoId, u64, Option<u64>),
    Job(RepoId, u64),
    /// A job's log (not cached: it's big).
    JobLog(RepoId, u64),
    Workflow(RepoId, String),
    /// Workflow runs, filtered as the Actions tab filters them
    /// ([`RunFilter`](crate::api::browse::RunFilter)).
    Runs(RepoId, String),
    CommitChecks(RepoId, String),
    /// Someone's own repositories, in an order.
    OwnerRepos(String, RepoSort),
    /// What someone starred.
    Stars(String),
    /// Your notifications, filtered as the inbox filters them
    /// ([`InboxFilter`](crate::api::notifications::InboxFilter)).
    Notifications(String),
    /// An issue's or pull request's sidebar, with what can go there.
    Sidebar(RepoId, u64),
    /// What a new issue or pull request in a repository can have.
    NewForm(RepoId),
    Projects(ProjectsOf),
    /// A project at a view (its first, if none is named).
    Project(ProjectRef, Option<u64>),
    /// A repository's security alerts of a kind, filtered.
    Alerts(RepoId, AlertKind, String),
    Alert(AlertRef),
    /// Your subscription to a thread or a repository.
    Subscription(crate::api::watch::Watched),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Data {
    Repo(Box<RepoOverview>),
    /// `None`: the repository has no README.
    Readme(Option<Box<Readme>>),
    Tree(Vec<TreeEntry>),
    Blob(Box<Blob>),
    Search(Box<SearchResults>),
    StateCounts(StateCounts),
    /// `None`: the number is a pull request.
    Issue(Option<Box<IssueDetail>>),
    PrActivity(Box<PrActivity>),
    Reviews(Box<Capped<Review>>),
    CommitComments(Box<Capped<CommitComment>>),
    Lines(Arc<Vec<String>>),
    InProjects(Box<Capped<InProject>>),
    Profile(Box<Profile>),
    /// Paths, and whether GitHub cut the list short.
    Files(Arc<Vec<String>>, bool),
    Refs(Box<Refs>),
    /// The ref a ref-and-path starts with, when longer than its first
    /// segment.
    RefIn(Option<String>),
    LastCommits(Arc<HashMap<String, CommitInfo>>),
    Commit(Box<CommitDetail>),
    History(Box<Results<CommitInfo>>),
    Checks(Box<Checks>),
    Users(Box<Results<UserSummary>>),
    /// A page of a list of repositories.
    RepoPage(Box<Results<RepoSummary>>),
    Releases(Box<Results<Release>>),
    Discussions(Box<DiscussionList>),
    Discussion(Box<DiscussionDetail>),
    Run(Box<WorkflowRun>),
    Job(Box<Job>),
    Log(Arc<crate::api::browse::JobLog>),
    Workflow(Box<Workflow>),
    Runs(Box<Results<RunSummary>>),
    Release(Box<Release>),
    Tags(Box<Results<TagInfo>>),
    Branches(Box<Results<BranchInfo>>),
    Wiki(Box<WikiPage>),
    Advisories(Box<Results<Advisory>>),
    Advisory(Box<Advisory>),
    Teams(Box<Results<TeamSummary>>),
    Team(Box<TeamDetail>),
    Gist(Box<Gist>),
    Gists(Box<Results<GistSummary>>),
    Blame(Box<Blame>),
    Compare(Box<Comparison>),
    Deployments(Box<DeploymentList>),
    Milestones(Box<MilestoneList>),
    Milestone(Box<MilestoneDetail>),
    Notifications(Box<Inbox>),
    Sidebar(Box<crate::api::edit::Sidebar>),
    NewForm(Box<crate::api::create::NewForm>),
    Projects(Box<Results<ProjectSummary>>),
    Project(Box<Project>),
    Alerts(Box<AlertList>),
    Alert(Box<Alert>),
    Subscription(Box<crate::api::watch::Subscription>),
}

impl Data {
    /// What was still running when fetched: a job (or its run), a run, a
    /// job's log, checks.
    pub fn running(&self) -> bool {
        match self {
            Data::Log(log) => log.running,
            // Its run going on can still change what it offers.
            Data::Job(job) => {
                job.outcome == CheckOutcome::Pending || job.run.outcome == CheckOutcome::Pending
            }
            Data::Run(run) => run.outcome == CheckOutcome::Pending,
            Data::Checks(c) => c.items.iter().any(|i| i.outcome == CheckOutcome::Pending),
            Data::Runs(r) => r.items.iter().any(|r| r.outcome == CheckOutcome::Pending),
            _ => false,
        }
    }

    /// Where a list's next page starts, if there's more.
    pub fn next_cursor(&self) -> Option<&str> {
        match self {
            Data::Search(r) => next_cursor(r),
            Data::History(r) => r.next.as_deref(),
            Data::Users(r) => r.next.as_deref(),
            Data::RepoPage(r) => r.next.as_deref(),
            Data::Releases(r) => r.next.as_deref(),
            Data::Runs(r) => r.next.as_deref(),
            Data::Discussions(d) => d.results.next.as_deref(),
            Data::Tags(r) => r.next.as_deref(),
            Data::Branches(r) => r.next.as_deref(),
            Data::Teams(r) => r.next.as_deref(),
            Data::Gists(r) => r.next.as_deref(),
            Data::Deployments(d) => d.results.next.as_deref(),
            Data::Milestones(m) => m.results.next.as_deref(),
            Data::Milestone(m) => m.items.next.as_deref(),
            Data::Advisories(r) => r.next.as_deref(),
            Data::Notifications(i) => i.list.next.as_deref(),
            Data::Projects(r) => r.next.as_deref(),
            Data::Project(p) => p.items.next.as_deref(),
            Data::Alerts(a) => a.results.next.as_deref(),
            _ => None,
        }
    }

    /// Appends a list's next page.
    pub fn append(&mut self, more: Data) {
        match (self, more) {
            (Data::Search(a), Data::Search(b)) => append(a, *b),
            (Data::History(a), Data::History(b)) => extend(a, *b, |c| c.oid.clone()),
            (Data::Users(a), Data::Users(b)) => extend(a, *b, |u| u.login.clone()),
            (Data::RepoPage(a), Data::RepoPage(b)) => extend(a, *b, |r| r.repo.clone()),
            (Data::Releases(a), Data::Releases(b)) => extend(a, *b, |r| r.tag.clone()),
            (Data::Runs(a), Data::Runs(b)) => extend(a, *b, |r| r.id),
            (Data::Discussions(a), Data::Discussions(b)) => {
                extend(&mut a.results, b.results, |d| d.number);
            }
            (Data::Tags(a), Data::Tags(b)) => extend(a, *b, |t| t.name.clone()),
            (Data::Branches(a), Data::Branches(b)) => extend(a, *b, |b| b.name.clone()),
            (Data::Teams(a), Data::Teams(b)) => extend(a, *b, |t| t.slug.clone()),
            (Data::Gists(a), Data::Gists(b)) => extend(a, *b, |g| g.id.clone()),
            (Data::Deployments(a), Data::Deployments(b)) => {
                extend(&mut a.results, b.results, |d| {
                    (d.created_at.clone(), d.environment.clone(), d.oid.clone())
                });
            }
            (Data::Milestones(a), Data::Milestones(b)) => {
                extend(&mut a.results, b.results, |m| m.number);
            }
            (Data::Milestone(a), Data::Milestone(b)) => {
                extend(&mut a.items, b.items, |i| (i.repo.clone(), i.number));
            }
            (Data::Advisories(a), Data::Advisories(b)) => extend(a, *b, |a| a.ghsa.clone()),
            (Data::Notifications(a), Data::Notifications(b)) => {
                extend(&mut a.list, b.list, |n| n.id.clone());
                a.searched += b.searched;
                a.cut = b.cut;
            }
            (Data::Projects(a), Data::Projects(b)) => extend(a, *b, |p| p.number),
            (Data::Project(a), Data::Project(b)) => extend(&mut a.items, b.items, |i| i.id.clone()),
            (Data::Alerts(a), Data::Alerts(b)) => extend(&mut a.results, b.results, |a| a.number),
            _ => {}
        }
    }
}

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
        _ => route
            .search()
            .map(|(kind, query)| DataKey::Search(kind, query)),
    }
}

/// What a page needs fetched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Need {
    Pr(PrRef),
    Data(DataKey),
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
pub fn needs(route: &Route) -> Vec<Need> {
    use DataKey as K;
    let header = |repo: &RepoId| Need::Data(K::Repo(repo.clone()));
    let list = route
        .search()
        .map(|(kind, query)| Need::Data(K::Search(kind, query)));
    match route {
        Route::Home | Route::NewGist => Vec::new(),
        Route::Repo(repo) => vec![
            header(repo),
            Need::Data(K::Readme(repo.clone())),
            Need::Data(K::LastCommits(repo.clone(), "HEAD".into(), String::new())),
        ],
        Route::Tree { repo, rev, path } => vec![
            header(repo),
            Need::Data(K::Tree(repo.clone(), rev.clone(), path.clone())),
            Need::Data(K::LastCommits(repo.clone(), rev.clone(), path.clone())),
        ],
        Route::Blob {
            repo, rev, path, ..
        } => vec![
            header(repo),
            Need::Data(K::Blob(repo.clone(), rev.clone(), path.clone())),
        ],
        Route::Blame {
            repo, rev, path, ..
        } => vec![
            header(repo),
            Need::Data(K::Blob(repo.clone(), rev.clone(), path.clone())),
            Need::Data(K::Blame(repo.clone(), rev.clone(), path.clone())),
        ],
        Route::Unsplit { repo, spot, .. } => vec![
            header(repo),
            Need::Data(K::RefIn(repo.clone(), spot.clone())),
        ],
        Route::Issues { repo, .. } | Route::Pulls { repo, .. } => {
            let counts = route.state_counts().map(|q| Need::Data(K::StateCounts(q)));
            std::iter::once(header(repo))
                .chain(list)
                .chain(counts)
                .collect()
        }
        Route::Search { .. } => list.into_iter().collect(),
        Route::Wiki { repo, page } => vec![
            header(repo),
            Need::Data(K::Wiki(repo.clone(), page.clone())),
        ],
        Route::Advisories(repo) | Route::Advisory { repo, .. } => {
            let own = match route {
                Route::Advisory { ghsa, .. } => K::Advisory(repo.clone(), ghsa.clone()),
                _ => K::Advisories(repo.clone()),
            };
            repo.iter().map(header).chain([Need::Data(own)]).collect()
        }
        Route::Gists(_)
        | Route::Teams(_)
        | Route::Notifications { .. }
        | Route::Project { .. }
        | Route::ProjectItem { .. }
        | Route::Projects(ProjectsOf::Owner(_)) => {
            paged(route).map(Need::Data).into_iter().collect()
        }
        Route::Team { org, slug } => vec![Need::Data(K::Team(org.to_lowercase(), slug.clone()))],
        Route::Gist { id, .. } | Route::EditGist { id, .. } => {
            vec![Need::Data(K::Gist(id.clone()))]
        }
        Route::Issue { repo, number } => {
            vec![
                header(repo),
                Need::Data(K::InProjects(repo.clone(), *number)),
                Need::Data(K::Issue(repo.clone(), *number)),
            ]
        }
        Route::NewIssue(repo) | Route::NewPr(repo) => {
            vec![header(repo), Need::Data(K::NewForm(repo.clone()))]
        }
        Route::NewRelease(repo) => vec![
            header(repo),
            Need::Data(K::Tags(repo.clone())),
            Need::Data(K::Branches(repo.clone())),
        ],
        Route::WikiEdit { repo, page } => vec![
            header(repo),
            Need::Data(crate::wiki::form::key(repo, page.as_ref())),
        ],
        Route::Pr {
            pr,
            tab: PrTab::Checks,
        } => vec![
            Need::Pr(pr.clone()),
            Need::Data(K::PrActivity(pr.clone())),
            Need::Data(K::PrChecks(pr.clone())),
        ],
        // The Review tab and a review list every review.
        Route::Pr {
            pr,
            tab: PrTab::Review | PrTab::Reviewed(_),
        } => vec![
            Need::Pr(pr.clone()),
            Need::Data(K::PrActivity(pr.clone())),
            Need::Data(K::Reviews(pr.clone())),
        ],
        Route::Pr {
            pr,
            tab: PrTab::Conversation,
        } => vec![
            Need::Pr(pr.clone()),
            Need::Data(K::InProjects(pr.repo.clone(), pr.number)),
            Need::Data(K::PrActivity(pr.clone())),
        ],
        Route::Pr { pr, .. } => vec![Need::Pr(pr.clone()), Need::Data(K::PrActivity(pr.clone()))],
        Route::Discussions { of, .. } | Route::Discussion { of, .. } => {
            let header = match of {
                DiscussionsOf::Repo(repo) => Some(header(repo)),
                DiscussionsOf::Org(_) => None,
            };
            let own = match route {
                Route::Discussion { number, .. } => Some(K::Discussion(of.clone(), *number)),
                _ => paged(route),
            };
            header.into_iter().chain(own.map(Need::Data)).collect()
        }
        Route::WorkflowRun { repo, run, attempt } => {
            vec![
                header(repo),
                Need::Data(K::Run(repo.clone(), *run, *attempt)),
            ]
        }
        Route::Job { repo, job, .. } => vec![
            header(repo),
            Need::Data(K::Job(repo.clone(), *job)),
            Need::Data(K::JobLog(repo.clone(), *job)),
        ],
        Route::Workflow { repo, file } => std::iter::once(header(repo))
            .chain([Need::Data(K::Workflow(repo.clone(), file.clone()))])
            .chain(paged(route).map(Need::Data))
            .collect(),
        Route::Dispatch { repo, file } => crate::actions::needs(repo, file),
        Route::CommitChecks { repo, oid } => vec![
            header(repo),
            Need::Data(K::Commit(repo.clone(), oid.clone())),
            Need::Data(K::CommitChecks(repo.clone(), oid.clone())),
        ],
        Route::Release { repo, tag } => {
            vec![
                header(repo),
                Need::Data(K::Release(repo.clone(), tag.clone())),
            ]
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
        | Route::Projects(ProjectsOf::Repo(repo)) => std::iter::once(header(repo))
            .chain(paged(route).map(Need::Data))
            .collect(),
        Route::Alert { repo, kind, number } => {
            let a = AlertRef {
                repo: repo.clone(),
                kind: *kind,
                number: *number,
            };
            vec![header(repo), Need::Data(K::Alert(a))]
        }
        Route::Commit { repo, oid, pr } => {
            let mut needs = vec![
                header(repo),
                Need::Data(K::CommitComments(repo.clone(), oid.clone())),
                Need::Data(K::Commit(repo.clone(), oid.clone())),
            ];
            // Its place among the pull request's commits.
            if let Some(number) = pr {
                let pr = PrRef {
                    repo: repo.clone(),
                    number: *number,
                };
                needs.push(Need::Data(K::PrActivity(pr)));
            }
            needs
        }
        Route::Commits { repo, rev, path } => vec![
            header(repo),
            Need::Data(K::History(repo.clone(), rev.clone(), path.clone())),
        ],
        Route::User { login, .. } => std::iter::once(Need::Data(K::Profile(login.to_lowercase())))
            .chain(paged(route).map(Need::Data))
            .collect(),
    }
}

/// Whether `need` is a repository's header on one of its other pages
/// (fetched only when missing).
pub fn is_header(route: &Route, need: &Need) -> bool {
    matches!(need, Need::Data(DataKey::Repo(_))) && !matches!(route, Route::Repo(_))
}

/// A kind of data a page shows, as kept in [`Data`].
pub trait Picked {
    fn pick(data: &Data) -> Option<&Self>;
}

macro_rules! picked {
    ($($variant:ident => $t:ty),* $(,)?) => {$(
        impl Picked for $t {
            fn pick(data: &Data) -> Option<&Self> {
                match data {
                    Data::$variant(x) => Some(std::borrow::Borrow::borrow(x)),
                    _ => None,
                }
            }
        }
    )*};
}

picked!(
    Repo => RepoOverview, Readme => Option<Box<Readme>>, Tree => [TreeEntry], Blob => Blob,
    Search => SearchResults, StateCounts => StateCounts, Issue => Option<Box<IssueDetail>>,
    PrActivity => PrActivity, Reviews => Capped<Review>,
    CommitComments => Capped<CommitComment>, Lines => Vec<String>,
    InProjects => Capped<InProject>,
    Profile => Profile, Refs => Refs, RefIn => Option<String>,
    LastCommits => HashMap<String, CommitInfo>, Commit => CommitDetail,
    History => Results<CommitInfo>, Checks => Checks, Users => Results<UserSummary>,
    RepoPage => Results<RepoSummary>, Releases => Results<Release>,
    Discussions => DiscussionList, Discussion => DiscussionDetail, Run => WorkflowRun, Job => Job,
    Log => crate::api::browse::JobLog, Workflow => Workflow, Runs => Results<RunSummary>,
    Release => Release, Tags => Results<TagInfo>, Branches => Results<BranchInfo>,
    Wiki => WikiPage, Advisories => Results<Advisory>, Advisory => Advisory,
    Teams => Results<TeamSummary>, Team => TeamDetail, Gist => Gist,
    Gists => Results<GistSummary>, Blame => Blame, Compare => Comparison,
    Deployments => DeploymentList, Milestones => MilestoneList, Milestone => MilestoneDetail,
    Notifications => Inbox, Sidebar => crate::api::edit::Sidebar,
    NewForm => crate::api::create::NewForm,
    Projects => Results<ProjectSummary>, Project => Project,
    Alerts => AlertList, Alert => Alert,
    Subscription => crate::api::watch::Subscription,
);

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
        self.needs(route).into_iter().filter_map(|need| match need {
            Need::Pr(pr) => self.prs.get(&pr).map(Remote::status),
            Need::Data(key) => self.data.get(&key).map(Remote::status),
        })
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
            | Route::NewGist
            | Route::EditGist { .. }
            | Route::WikiEdit { .. }
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
            let why = format!("Couldn't refresh: {err}. Showing what was loaded {when}.");
            pages::flash(&mut page, &why);
            page.blank();
        }
        // Where the page was read: a clone at a commit, or GitHub's API.
        if let Some(from) = crate::source::shown(route).and_then(|k| self.sources.get(&k)) {
            let note = vec![Seg::new(from.note(), Role::Meta)];
            page.wrapped(note, 0, crate::ui::page::Frame::None);
        }
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
                if let Some(d) = f.of(self.prs.get(pr)).show(&mut page, &pr.to_string()) {
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
                    crate::release::build(self, &mut page, repo);
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
                    pages::discussion(&mut page, d, fresh, cx);
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
                        Some(key) => ProfileList::Repos(f.get(&key)),
                        None => ProfileList::None,
                    };
                    pages::profile(&mut page, p, *tab, list, now);
                }
            }
        }
        // The sidebar starts beside the page, below the banner.
        if let Some(a) = page.aside.as_mut().filter(|a| !a.lines.is_empty()) {
            a.lines.splice(0..0, vec![PageLine::default(); banner]);
        }
        page
    }
}

/// Appends a page to a list, leaving out what's already there (by `id`):
/// a list that moved between pages repeats items at their edge.
fn extend<T, K: Eq + std::hash::Hash>(a: &mut Results<T>, b: Results<T>, id: impl Fn(&T) -> K) {
    let mut seen: std::collections::HashSet<K> = a.items.iter().map(&id).collect();
    a.items
        .extend(b.items.into_iter().filter(|item| seen.insert(id(item))));
    a.next = b.next;
    // A list GitHub doesn't count says how many each page has.
    a.total = b.total.max(a.items.len() as u64);
}

/// Appends a page of results to what's shown.
pub fn append(results: &mut SearchResults, more: SearchResults) {
    match (results, more) {
        (SearchResults::Repos(a), SearchResults::Repos(b)) => extend(a, b, |r| r.repo.clone()),
        (SearchResults::Issues(a), SearchResults::Issues(b)) => {
            extend(a, b, |i| (i.repo.clone(), i.number));
        }
        (SearchResults::Users(a), SearchResults::Users(b)) => extend(a, b, |u| u.login.clone()),
        (SearchResults::Discussions(a), SearchResults::Discussions(b)) => {
            extend(a, b, |d| (d.repo.clone(), d.summary.number));
        }
        (SearchResults::Commits(a), SearchResults::Commits(b)) => {
            extend(a, b, |c| (c.repo.clone(), c.commit.oid.clone()));
        }
        (SearchResults::Code(a), SearchResults::Code(b)) => {
            extend(a, b, |c| (c.repo.clone(), c.path.clone()));
        }
        _ => {}
    }
}

pub fn next_cursor(results: &SearchResults) -> Option<&str> {
    results.next()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A next page that overlaps the last (the list moved between them)
    /// doesn't show its overlap twice.
    #[test]
    fn a_next_page_leaves_out_what_is_shown() {
        let branch = |n: u32| BranchInfo {
            name: format!("b{n}"),
            ..crate::fixtures::branches().items[0].clone()
        };
        let page = |names: std::ops::Range<u32>, next: bool| Results {
            total: 5,
            items: names.map(branch).collect(),
            next: next.then(|| "c".to_owned()),
        };
        let mut shown = Data::Branches(Box::new(page(0..3, true)));
        shown.append(Data::Branches(Box::new(page(2..5, false))));
        let Data::Branches(r) = shown else { panic!() };
        let names: Vec<&str> = r.items.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["b0", "b1", "b2", "b3", "b4"]);
        assert_eq!(r.next, None);
    }
}
