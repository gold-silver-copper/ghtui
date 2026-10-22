//! GitHub's data, as pages read it: one [`DataKey`] for each piece, and
//! the [`Data`] it is.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

pub mod load;

use crate::api::browse::{
    Advisory, Blame, Blob, BranchInfo, CheckOutcome, Checks, CommitDetail, CommitInfo, Comparison,
    DeploymentList, DiscussionDetail, DiscussionList, DiscussionsOf, Gist, GistSummary,
    IssueDetail, Job, JobLog, MilestoneDetail, MilestoneList, PrActivity, Profile, Readme, Refs,
    Release, RepoOverview, RepoSort, RepoSummary, Results, RunSummary, SearchKind, SearchResults,
    StateCounts, TagInfo, TeamDetail, TeamSummary, TreeEntry, UserList, UserSummary, WikiPage,
    Workflow, WorkflowRun,
};
use crate::api::codeowners::Codeowners;
use crate::api::commit_comments::CommitComment;
use crate::api::create::NewForm;
use crate::api::edit::Sidebar;
use crate::api::explore::{Topic, Topics};
use crate::api::housekeeping::{BranchRules, FileLook, LabelInfo, Owners, Templates};
use crate::api::insights::{Activity, Dependencies, Traffic};
use crate::api::lists::{ListPage, MyLists, StarList};
use crate::api::manage::{Categories, Target};
use crate::api::mention::Mentionable;
use crate::api::model::{Capped, PrDetail, PrRef, RepoId};
use crate::api::notifications::{Inbox, Unread};
use crate::api::org::{Invitations, Member, OrgAccess};
use crate::api::packages::{PackageDetail, PackageList, PackageRef, PackagesOf};
use crate::api::project::{InProject, Project, ProjectRef, ProjectSummary, ProjectsOf};
use crate::api::release::TagLook;
use crate::api::replies::SavedReply;
use crate::api::security::{Alert, AlertKind, AlertList, AlertRef};
use crate::api::settings::{Settings, SettingsPage};
use crate::api::timeline::Review;
use crate::api::watch::{Subscription, Watched};
use crate::create::HeadState;

/// One piece of GitHub data a page shows.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DataKey {
    Repo(RepoId),
    /// The README at the root, apart from the overview every repository
    /// page has for its header.
    Readme(RepoId),
    Tree(RepoId, String, String),
    Blob(RepoId, String, String),
    /// Who owns what: the CODEOWNERS file at a revision.
    Codeowners(RepoId, String),
    Search(SearchKind, String),
    /// A filtered list's open and closed counts
    /// ([`crate::route::issues`]).
    StateCounts(String),
    Issue(RepoId, u64),
    Pr(PrRef),
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
    /// [`code::Unsplit`](crate::route::code::Page::Unsplit)).
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
    /// An organization's members, with their roles, by its lowercase
    /// login.
    Members(String),
    /// What you may do in an organization, and its owners' invitations.
    OrgAccess(String),
    /// The invitations sent to you.
    Invitations,
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
    Subscription(Watched),
    /// A page of a repository's Settings tab.
    Settings(RepoId, SettingsPage),
    /// Commits by week and author: counted by git where it can.
    Activity(RepoId),
    Traffic(RepoId),
    Dependencies(RepoId),
    /// GitHub's featured topics.
    Featured,
    /// A topic, by its (lowercase) name.
    Topic(String),
    /// Someone's starred lists, by their lowercase login.
    StarLists(String),
    /// A starred list, by its owner's lowercase login and its slug.
    StarList(String, String),
    /// Your starred lists, as they stand for a repository.
    MyLists(RepoId),
    Packages(PackagesOf),
    Package(PackageRef),
    SavedReplies,
    Labels(RepoId),
    /// You, and the organizations you may make repositories for.
    Owners,
    /// GitHub's `.gitignore` and licence templates.
    Templates,
    /// A repository's open issues, for a picker.
    OpenIssues(RepoId),
    /// An owner's repositories an issue can move to.
    MoveTargets(String),
    /// Where a repository's new discussions go.
    Categories(RepoId),
    /// The clone ghtui runs in, if it's one of the repository's: from git.
    Clone(RepoId),
    /// Where a new pull request's head stands against its base: in the
    /// clone ghtui runs in, and on GitHub.
    Head {
        repo: RepoId,
        base: String,
        head: String,
    },
    /// What GitHub has of a tag.
    TagLook(RepoId, String),
    /// What GitHub's rules say of deleting a branch.
    BranchRules(RepoId, String),
    /// A file at a revision, where its form starts.
    FileLook(RepoId, String, String),
    /// Who and what a repository's comments can mention.
    Mentionable(RepoId),
    /// How many of your notifications are unread, for the header.
    Unread,
    /// Repositories the search box suggests for what's typed.
    Suggest(String),
}

impl DataKey {
    /// Whether its data is kept in the cache, to show while it's asked
    /// for again and while GitHub can't be reached.
    pub fn kept(&self) -> bool {
        !matches!(
            self,
            // Files can be large; so can a job's log.
            DataKey::Blob(..)
                | DataKey::JobLog(..)
                | DataKey::Files(..)
                // A thread's lines and a wiki come from git: the clone is
                // their cache.
                | DataKey::Lines(..)
                | DataKey::Wiki(..)
                // A commit's comments revalidate with their ETag.
                | DataKey::CommitComments(..)
                // Where a ref ends is asked anew each session.
                | DataKey::RefIn(..)
                // What these offer to choose from is what they have now.
                | DataKey::InProjects(..)
                | DataKey::Sidebar(..)
                | DataKey::NewForm(..)
                | DataKey::Subscription(..)
                | DataKey::MyLists(..)
                | DataKey::Owners
                | DataKey::OrgAccess(_)
                | DataKey::Invitations
                | DataKey::Templates
                | DataKey::OpenIssues(_)
                | DataKey::MoveTargets(_)
                | DataKey::Categories(_)
                | DataKey::TagLook(..)
                | DataKey::BranchRules(..)
                | DataKey::FileLook(..)
                | DataKey::Mentionable(_)
                | DataKey::Unread
                | DataKey::Suggest(_)
                // Git answers these.
                | DataKey::Clone(_)
                | DataKey::Head { .. }
        )
    }

    /// Whether the cache's copy shows while it's asked for again. A pull
    /// request's shows only while GitHub can't be reached: the diff starts
    /// from the base it says, which may have moved since.
    pub fn cache_first(&self) -> bool {
        !matches!(self, DataKey::Pr(_))
    }

    /// Where its data is kept in the cache, if it is.
    fn cache_id(&self) -> Option<String> {
        let key = self.kept().then(|| serde_json::to_string(self).ok())??;
        Some(format!("data:{key}"))
    }
}

/// A kind of data a page shows, as kept in [`Data`].
pub trait Picked {
    fn pick(data: &Data) -> Option<&Self>;
}

/// Any kind, as it is.
impl Picked for Data {
    fn pick(data: &Data) -> Option<&Self> {
        Some(data)
    }
}

/// The [`Data`] enum, and how each kind is [`Picked`] from it: a kind
/// after `=>` is also made into its [`Data`] by `From`, one after `as`
/// (unsized) isn't. A paged list says, in brackets, which of its fields
/// is the [`Results`] (none: it is) and the id that dedupes its pages.
macro_rules! data {
    ($(
        $(#[$meta:meta])* $variant:ident($($field:ty),+)
        $(=> $picked:ty $([$(.$list:ident)? $id:expr])?)?
        $(as $unsized:ty)?,
    )*) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        pub enum Data {
            $($(#[$meta])* $variant($($field),+),)*
        }

        $($(
            data!(@pick $variant $picked);

            impl From<$picked> for Data {
                fn from(x: $picked) -> Self {
                    Data::$variant(x.into())
                }
            }
        )?)*

        $($(data!(@pick $variant $unsized);)?)*

        impl Data {
            /// Where a paged list's next page starts, if there's more.
            fn paged_next(&self) -> Option<&str> {
                match self {
                    $($($(Data::$variant(x) => x$(.$list)?.next.as_deref(),)?)?)*
                    _ => None,
                }
            }

            /// Appends a paged list's next page.
            fn append_paged(&mut self, more: Data) {
                match (self, more) {
                    $($($(
                        (Data::$variant(a), Data::$variant(b)) => {
                            extend(&mut (**a)$(.$list)?, (*b)$(.$list)?, $id);
                        }
                    )?)?)*
                    _ => {}
                }
            }
        }
    };
    (@pick $variant:ident $kind:ty) => {
        impl Picked for $kind {
            fn pick(data: &Data) -> Option<&Self> {
                match data {
                    Data::$variant(x) => Some(std::borrow::Borrow::borrow(x)),
                    _ => None,
                }
            }
        }
    };
}

data! {
    Repo(Box<RepoOverview>) => RepoOverview,
    /// `None`: the repository has no README.
    Readme(Option<Box<Readme>>) => Option<Box<Readme>>,
    Tree(Vec<TreeEntry>) as [TreeEntry],
    Blob(Box<Blob>) => Blob,
    Codeowners(Box<Codeowners>) => Codeowners,
    Search(Box<SearchResults>) => SearchResults,
    StateCounts(StateCounts) => StateCounts,
    /// `None`: the number is a pull request.
    Issue(Option<Box<IssueDetail>>) => Option<Box<IssueDetail>>,
    Pr(Box<PrDetail>) => PrDetail,
    PrActivity(Box<PrActivity>) => PrActivity,
    Reviews(Box<Capped<Review>>) => Capped<Review>,
    CommitComments(Box<Capped<CommitComment>>) => Capped<CommitComment>,
    Lines(Arc<Vec<String>>) => Vec<String>,
    InProjects(Box<Capped<InProject>>) => Capped<InProject>,
    Profile(Box<Profile>) => Profile,
    /// Paths, and whether GitHub cut the list short.
    Files(Arc<Vec<String>>, bool),
    Refs(Box<Refs>) => Refs,
    /// The ref a ref-and-path starts with, when longer than its first
    /// segment.
    RefIn(Option<String>) => Option<String>,
    LastCommits(Arc<HashMap<String, CommitInfo>>) => HashMap<String, CommitInfo>,
    Commit(Box<CommitDetail>) => CommitDetail,
    History(Box<Results<CommitInfo>>) => Results<CommitInfo> [|c| c.oid.clone()],
    Checks(Box<Checks>) => Checks,
    Users(Box<Results<UserSummary>>) => Results<UserSummary> [|u| u.login.clone()],
    /// A page of a list of repositories.
    RepoPage(Box<Results<RepoSummary>>) => Results<RepoSummary> [|r| r.repo.clone()],
    Releases(Box<Results<Release>>) => Results<Release> [|r| r.tag.clone()],
    Discussions(Box<DiscussionList>) => DiscussionList [.results |d| d.number],
    Discussion(Box<DiscussionDetail>) => DiscussionDetail,
    Run(Box<WorkflowRun>) => WorkflowRun,
    Job(Box<Job>) => Job,
    #[serde(skip)]
    Log(Arc<JobLog>) => JobLog,
    Workflow(Box<Workflow>) => Workflow,
    Runs(Box<Results<RunSummary>>) => Results<RunSummary> [|r| r.id],
    Release(Box<Release>) => Release,
    Tags(Box<Results<TagInfo>>) => Results<TagInfo> [|t| t.name.clone()],
    Branches(Box<Results<BranchInfo>>) => Results<BranchInfo> [|b| b.name.clone()],
    Wiki(Box<WikiPage>) => WikiPage,
    Advisories(Box<Results<Advisory>>) => Results<Advisory> [|a| a.ghsa.clone()],
    Advisory(Box<Advisory>) => Advisory,
    Teams(Box<Results<TeamSummary>>) => Results<TeamSummary> [|t| t.slug.clone()],
    Team(Box<TeamDetail>) => TeamDetail,
    Members(Box<Results<Member>>) => Results<Member> [|m| m.login.clone()],
    #[serde(skip)]
    OrgAccess(Box<OrgAccess>) => OrgAccess,
    #[serde(skip)]
    Invitations(Box<Invitations>) => Invitations,
    Gist(Box<Gist>) => Gist,
    Gists(Box<Results<GistSummary>>) => Results<GistSummary> [|g| g.id.clone()],
    Blame(Box<Blame>) => Blame,
    Compare(Box<Comparison>) => Comparison,
    Deployments(Box<DeploymentList>) => DeploymentList [.results |d| (d.created_at.clone(), d.environment.clone(), d.oid.clone())],
    Milestones(Box<MilestoneList>) => MilestoneList [.results |m| m.number],
    Milestone(Box<MilestoneDetail>) => MilestoneDetail [.items |i| (i.repo.clone(), i.number)],
    Notifications(Box<Inbox>) => Inbox,
    #[serde(skip)]
    Sidebar(Box<Sidebar>) => Sidebar,
    #[serde(skip)]
    NewForm(Box<NewForm>) => NewForm,
    Projects(Box<Results<ProjectSummary>>) => Results<ProjectSummary> [|p| p.number],
    Project(Box<Project>) => Project [.items |i| i.id.clone()],
    Alerts(Box<AlertList>) => AlertList [.results |a| a.number],
    Alert(Box<Alert>) => Alert,
    #[serde(skip)]
    Subscription(Box<Subscription>) => Subscription,
    Settings(Box<Settings>) => Settings,
    Activity(Box<Activity>) => Activity,
    Traffic(Box<Traffic>) => Traffic,
    Dependencies(Box<Dependencies>) => Dependencies,
    Topics(Box<Topics>) => Topics,
    Topic(Box<Topic>) => Topic,
    StarLists(Box<Capped<StarList>>) => Capped<StarList>,
    StarList(Box<ListPage>) => ListPage [.repos |r| r.repo.clone()],
    MyLists(Box<MyLists>) => MyLists,
    Packages(Box<PackageList>) => PackageList,
    Package(Box<PackageDetail>) => PackageDetail [.versions |v| v.id],
    SavedReplies(Box<Capped<SavedReply>>) => Capped<SavedReply>,
    Labels(Box<Results<LabelInfo>>) => Results<LabelInfo> [|l| l.name.clone()],
    Owners(Box<Owners>) => Owners,
    Templates(Box<Templates>) => Templates,
    #[serde(skip)]
    OpenIssues(Box<Capped<(u64, String)>>) => Capped<(u64, String)>,
    #[serde(skip)]
    MoveTargets(Box<Capped<Target>>) => Capped<Target>,
    #[serde(skip)]
    Categories(Box<Categories>) => Categories,
    /// `None`: ghtui doesn't run in a clone of it.
    #[serde(skip)]
    Clone(Option<Box<crate::git::local::Clone>>) => Option<Box<crate::git::local::Clone>>,
    #[serde(skip)]
    Head(Box<HeadState>) => HeadState,
    #[serde(skip)]
    TagLook(Box<TagLook>) => TagLook,
    #[serde(skip)]
    BranchRules(Box<BranchRules>) => BranchRules,
    #[serde(skip)]
    FileLook(Box<FileLook>) => FileLook,
    #[serde(skip)]
    Mentionable(Box<Mentionable>) => Mentionable,
    #[serde(skip)]
    Unread(Unread) => Unread,
    #[serde(skip)]
    Suggestions(Vec<RepoSummary>) as [RepoSummary],
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
            Data::Search(r) => r.next(),
            Data::Notifications(i) => i.list.next.as_deref(),
            _ => self.paged_next(),
        }
    }

    /// Appends a list's next page.
    pub fn append(&mut self, more: Data) {
        match (self, more) {
            (Data::Search(a), Data::Search(b)) => append(a, *b),
            (Data::Notifications(a), Data::Notifications(b)) => {
                extend(&mut a.list, b.list, |n| n.id.clone());
                a.searched += b.searched;
                a.cut = b.cut;
            }
            (this, more) => this.append_paged(more),
        }
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
fn append(results: &mut SearchResults, more: SearchResults) {
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
        let mut shown = Data::from(page(0..3, true));
        shown.append(Data::from(page(2..5, false)));
        let Data::Branches(r) = shown else { panic!() };
        let names: Vec<&str> = r.items.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["b0", "b1", "b2", "b3", "b4"]);
        assert_eq!(r.next, None);
    }
}
