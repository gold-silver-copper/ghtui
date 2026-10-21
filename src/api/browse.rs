//! Browsing GitHub: repositories (overview, directories, files, README),
//! issues, profiles, search and pull request conversations.
//!
//! The wire types are typed against GitHub's schema; the models after them
//! are what the app renders and caches.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::api::edit::{Editable, SidebarPermits, WireReactionGroup, reactions};
use crate::api::model::{
    Capped, ChecksState, Label, MilestoneRef, NodeId, PrStatus, RepoId, ReviewDecision, author,
    count, labels,
};
use crate::api::queries::{
    Actor, CommentCount, CommitCount, CommitOid, DateTime, FollowCount, FollowingCount,
    GitObjectId, IssueCount, LabelConnection, NumberVariablesFields, PageInfo, PrCount, RefCount,
    RepositoryName, StatusState, Uri, UserCount, fragments, nodes,
};
use crate::api::timeline;
use crate::schema::schema;

pub use crate::api::model::{IssueState, PrState, Status};
pub use crate::api::person::{Login, Person};

// ---- models ------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoSummary {
    pub repo: RepoId,
    pub description: Option<String>,
    pub stars: u64,
    pub forks: u64,
    pub language: Option<String>,
    /// `#rrggbb`.
    pub language_color: Option<String>,
    /// ISO 8601.
    pub pushed_at: Option<String>,
    pub private: bool,
    pub fork: bool,
    pub archived: bool,
}

/// How a check came out. Ordered worst first, for sorting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CheckOutcome {
    Failure,
    Pending,
    Cancelled,
    Neutral,
    Skipped,
    Success,
}

/// One check run or commit status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckItem {
    pub name: String,
    /// The workflow or app it belongs to.
    pub group: String,
    pub outcome: CheckOutcome,
    /// ISO 8601.
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    /// Its title or description.
    pub summary: Option<String>,
    /// Its details page (logs, re-runs).
    pub url: Option<String>,
    /// What a check run says about lines of code, worst first.
    #[serde(default)]
    pub annotations: Capped<crate::api::workflows::Annotation>,
}

/// The checks on a commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checks {
    pub oid: String,
    pub items: Capped<CheckItem>,
}

/// A commit's page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitDetail {
    pub oid: String,
    pub headline: String,
    /// The message after the headline.
    pub body: String,
    pub author: Person,
    /// ISO 8601.
    pub authored_at: String,
    /// Who committed it, when that isn't the author (rebased, applied).
    pub committer: Option<Person>,
    pub committed_at: String,
    pub parents: Capped<String>,
    pub additions: u64,
    pub deletions: u64,
    pub changed_files: Option<u64>,
    /// Whether GitHub verified its signature; `None` when unsigned.
    pub verified: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitInfo {
    pub oid: String,
    pub headline: String,
    pub author: Person,
    /// ISO 8601.
    pub date: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryKind {
    Dir,
    File,
    Symlink,
    Submodule,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeEntry {
    pub name: String,
    pub path: String,
    pub kind: EntryKind,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoOverview {
    pub summary: RepoSummary,
    pub homepage: Option<String>,
    pub watchers: u64,
    pub open_issues: u64,
    pub open_prs: u64,
    pub closed_issues: u64,
    pub closed_prs: u64,
    pub license: Option<String>,
    pub topics: Capped<String>,
    pub default_branch: Option<String>,
    pub last_commit: Option<CommitInfo>,
    pub commits: u64,
    /// The repository this one was forked from.
    pub parent: Option<String>,
    pub starred: bool,
    /// You may push to it, and re-run and cancel its workflows.
    pub can_write: bool,
    /// You administer it: its Settings tab is yours.
    #[serde(default)]
    pub can_admin: bool,
    /// Node ID, for starring.
    pub id: NodeId,
    pub has_issues: bool,
    pub has_discussions: bool,
    pub has_wiki: bool,
    pub branches: u64,
    pub tags: u64,
    /// The issues pinned at the top of its Issues tab.
    pub pinned_issues: Capped<timeline::Linked>,
    /// Root directory at the default branch (empty for empty repos).
    pub entries: Vec<TreeEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Readme {
    pub path: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Blob {
    pub path: String,
    /// `None` for binary files.
    pub text: Option<String>,
    pub size: u64,
    pub truncated: bool,
}

/// An issue or pull request in a list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueSummary {
    pub repo: RepoId,
    pub number: u64,
    pub title: String,
    pub kind: IssueKind,
    pub author: String,
    pub updated_at: String,
    pub comments: u64,
    pub labels: Capped<Label>,
    pub created_at: String,
}

/// A row is an issue or a pull request, and only a pull request has a
/// review and checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IssueKind {
    Issue(IssueState),
    Pr {
        /// What its checks are asked by.
        id: NodeId,
        state: PrState,
        review: Option<ReviewDecision>,
        /// Reviewers whose latest word is an approval.
        approvals: u64,
        /// Where known.
        checks: Option<ChecksState>,
    },
}

impl IssueSummary {
    pub fn status(&self) -> Status {
        match self.kind {
            IssueKind::Issue(state) => Status::Issue(state),
            IssueKind::Pr { state, .. } => Status::Pr(state),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserSummary {
    pub login: String,
    pub name: Option<String>,
    pub bio: Option<String>,
    pub is_org: bool,
}

/// A release, as listed (no notes or assets) or on its page.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    /// What REST names it by (to edit or delete it, and its assets).
    #[serde(default)]
    pub id: Option<u64>,
    pub name: String,
    pub tag: String,
    /// ISO 8601; drafts aren't published.
    pub published_at: Option<String>,
    pub prerelease: bool,
    pub draft: bool,
    pub latest: bool,
    pub author: Option<String>,
    /// Its notes, as markdown (on its page).
    pub notes: Option<String>,
    pub assets: Capped<Asset>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    pub downloads: u64,
    pub url: String,
}

/// A tag and the commit it points at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagInfo {
    pub name: String,
    pub oid: Option<String>,
    /// ISO 8601: when its commit was committed.
    pub date: Option<String>,
}

/// How a profile's repositories are ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum RepoSort {
    #[default]
    Updated,
    Name,
    Stars,
}

impl RepoSort {
    pub fn label(self) -> &'static str {
        match self {
            RepoSort::Updated => "Last updated",
            RepoSort::Name => "Name",
            RepoSort::Stars => "Stars",
        }
    }

    /// The next order, round and round.
    pub fn next(self) -> RepoSort {
        match self {
            RepoSort::Updated => RepoSort::Name,
            RepoSort::Name => RepoSort::Stars,
            RepoSort::Stars => RepoSort::Updated,
        }
    }
}

/// A list of people: a repository's stargazers or watchers, someone's
/// followers or who they follow, an organization's public members.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UserList {
    Stargazers(RepoId),
    Watchers(RepoId),
    Followers(String),
    Following(String),
    People(String),
}

/// One page of search results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Results<T> {
    pub total: u64,
    pub items: Vec<T>,
    /// Cursor for the next page, if there is one.
    pub next: Option<String>,
}

/// A list's page and its total, for checking they add up.
pub trait Page {
    /// Items on this page, and how many GitHub counts in all.
    fn counts(&self) -> (usize, u64);
}

impl<T> Page for Results<T> {
    fn counts(&self) -> (usize, u64) {
        (self.items.len(), self.total)
    }
}

macro_rules! page_in {
    ($($list:ty => $field:ident),* $(,)?) => {$(
        impl Page for $list {
            fn counts(&self) -> (usize, u64) {
                self.$field.counts()
            }
        }
    )*};
}
page_in! {
    MilestoneList => results,
    MilestoneDetail => items,
    DeploymentList => results,
    DiscussionList => results,
    crate::api::notifications::Inbox => list,
}

impl<T> Results<T> {
    /// A page of a connection: its total, its cursor, and the nodes `item`
    /// makes something of.
    pub(crate) fn from_page<N>(
        total: i32,
        page: PageInfo,
        list: Option<Vec<Option<N>>>,
        item: impl FnMut(N) -> Option<T>,
    ) -> Self {
        Self {
            total: count(total),
            items: nodes(list).filter_map(item).collect(),
            next: page.next(),
        }
    }

    /// A page of a list GitHub doesn't count.
    pub(crate) fn uncounted(items: Vec<T>, next: Option<String>) -> Self {
        Self {
            total: 0,
            items,
            next,
        }
    }
}

/// No results (without asking `T` for a default).
impl<T> Default for Results<T> {
    fn default() -> Self {
        Self {
            total: 0,
            items: Vec::new(),
            next: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SearchKind {
    Repos,
    /// Issues and pull requests (unless the query says `is:issue` or `is:pr`).
    Issues,
    /// Pull requests only.
    Pulls,
    Users,
    Discussions,
    Commits,
    Code,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchResults {
    Repos(Results<RepoSummary>),
    Issues(Results<IssueSummary>),
    Users(Results<UserSummary>),
    Discussions(Results<DiscussionHit>),
    Commits(Results<CommitHit>),
    Code(Results<CodeHit>),
}

/// `$e` of whichever kind of results `$s` holds, as `$r`.
macro_rules! each_kind {
    ($s:expr, $r:ident => $e:expr) => {
        match $s {
            SearchResults::Repos($r) => $e,
            SearchResults::Issues($r) => $e,
            SearchResults::Users($r) => $e,
            SearchResults::Discussions($r) => $e,
            SearchResults::Commits($r) => $e,
            SearchResults::Code($r) => $e,
        }
    };
}

impl SearchResults {
    /// How many there are in all, and how many are here.
    pub fn counts(&self) -> (u64, usize) {
        each_kind!(self, r => (r.total, r.items.len()))
    }

    pub fn next(&self) -> Option<&str> {
        each_kind!(self, r => r.next.as_deref())
    }
}

/// A discussion a search found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscussionHit {
    pub repo: RepoId,
    pub summary: DiscussionSummary,
}

/// A commit a search found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitHit {
    pub repo: RepoId,
    pub commit: CommitInfo,
}

/// A file a code search found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeHit {
    pub repo: RepoId,
    pub path: String,
    /// The blob's ID: the file as it was indexed (empty from git).
    pub sha: String,
    /// The lines that matched, by number, where git searched (GitHub's
    /// search doesn't say).
    #[serde(default)]
    pub lines: Vec<(u32, String)>,
    /// The commit git searched at; GitHub's search reads the default
    /// branch.
    #[serde(default)]
    pub rev: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    /// GitHub's number for it, which its links' anchors use
    /// (`#issuecomment-…`, `#discussioncomment-…`).
    pub id: Option<u64>,
    pub author: String,
    pub body: String,
    pub created_at: String,
    /// What edits name it by, what GitHub lets you do to it, and its
    /// reactions; `None` where ghtui doesn't edit it.
    pub editable: Option<Editable>,
    /// Whether it's hidden and why, and whether you may hide or unhide it.
    pub hiding: crate::api::manage::Hiding,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueDetail {
    pub repo: RepoId,
    pub number: u64,
    pub title: String,
    pub body: String,
    pub state: IssueState,
    pub author: String,
    pub created_at: String,
    pub labels: Capped<Label>,
    pub assignees: Capped<String>,
    pub milestone: Option<MilestoneRef>,
    /// The newest comments and events, oldest first, of how many there
    /// are in all.
    pub timeline: Capped<timeline::Item>,
    /// Comments in all, loaded or not.
    pub comment_count: u64,
    /// The pull requests that will close it (GitHub's "Development").
    pub closed_by: Capped<timeline::Linked>,
    /// Branches made for it.
    pub branches: Capped<String>,
    pub parent: Option<timeline::Linked>,
    pub sub_issues: Capped<timeline::Linked>,
    pub progress: Option<timeline::Progress>,
    /// The issues that block it, and those it blocks.
    pub blocked_by: Capped<timeline::Linked>,
    pub blocking: Capped<timeline::Linked>,
    pub issue_type: Option<String>,
    /// Node ID, for commenting.
    pub id: NodeId,
    pub can_close: bool,
    pub can_reopen: bool,
    /// Its conversation is locked: only collaborators comment.
    pub locked: Option<crate::api::manage::Locked>,
    /// Pinned to its repository's Issues tab.
    pub pinned: bool,
    /// You can write to its repository: pin it, move it.
    pub can_write: bool,
    /// Its description, as edits name it.
    pub editable: Editable,
    pub sidebar: SidebarPermits,
}

impl IssueDetail {
    /// The comments loaded, oldest first.
    pub fn comments(&self) -> impl Iterator<Item = &Comment> {
        comments_in(&self.timeline)
    }

    /// The first comment loaded, to change in a test.
    #[cfg(test)]
    pub(crate) fn first_comment(&mut self) -> Option<&mut Comment> {
        self.timeline.iter_mut().find_map(|i| match i {
            timeline::Item::Comment(c) => Some(c),
            _ => None,
        })
    }
}

/// The comments among a timeline's items.
fn comments_in(timeline: &[timeline::Item]) -> impl Iterator<Item = &Comment> {
    timeline.iter().filter_map(|i| match i {
        timeline::Item::Comment(c) => Some(c),
        _ => None,
    })
}

/// Everything on a PR's Conversation and Commits tabs that `PrDetail`
/// doesn't carry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrActivity {
    /// Node ID, for commenting.
    pub id: NodeId,
    /// The newest comments, reviews and events, oldest first, of how
    /// many there are in all.
    pub timeline: Capped<timeline::Item>,
    /// Comments in all, loaded or not.
    pub comment_count: u64,
    /// The newest commits, oldest first, of how many there are in all.
    pub commits: Capped<CommitInfo>,
    /// Who has been asked for a review and hasn't given one since.
    pub requested: Capped<timeline::Requested>,
    /// Each reviewer's latest review.
    pub latest: Capped<timeline::Latest>,
    /// The issues it will close when it merges.
    pub closing: Capped<timeline::Linked>,
    /// Its merge commit, once merged.
    pub merge_commit: Option<String>,
    /// Its review threads, which its reviews' comments start.
    pub threads: Vec<crate::api::model::ReviewThread>,
}

impl PrActivity {
    /// The comments loaded, oldest first.
    pub fn comments(&self) -> impl Iterator<Item = &Comment> {
        comments_in(&self.timeline)
    }

    /// The reviews loaded, oldest first.
    pub fn reviews(&self) -> impl Iterator<Item = &timeline::Review> {
        self.timeline.iter().filter_map(|i| match i {
            timeline::Item::Review(r) => Some(r),
            _ => None,
        })
    }
}

/// Someone asked for a review: a person (or a bot), or a team.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reviewer {
    User(String),
    /// `org/slug`.
    Team(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub login: String,
    pub name: Option<String>,
    pub bio: Option<String>,
    pub company: Option<String>,
    pub location: Option<String>,
    pub website: Option<String>,
    pub followers: Option<u64>,
    pub following: Option<u64>,
    pub is_org: bool,
    pub pinned: Vec<RepoSummary>,
    pub repos: Capped<RepoSummary>,
    pub star_count: u64,
    /// The profile README, as markdown.
    pub readme: Option<String>,
    /// "🌴 On vacation".
    pub status: Option<String>,
    pub pronouns: Option<String>,
    /// Social accounts: (how it's shown, link).
    pub socials: Vec<(String, String)>,
    /// Organizations a user belongs to (the public ones).
    pub orgs: Capped<String>,
    /// An organization's verified domain badge.
    pub verified: bool,
    /// An organization's first public members.
    pub people: Capped<String>,
    /// A user's contributions in the last year.
    pub contributions: Option<Contributions>,
}

/// The contribution graph and recent activity on a user's profile.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contributions {
    pub total: u64,
    /// Oldest first, each starting on a Sunday (the first may start later).
    pub weeks: Vec<Week>,
    /// Newest month first.
    pub activity: Vec<MonthActivity>,
    /// Where its counts may be short.
    pub short: Short,
}

/// For each kind of activity, the month (`YYYY-MM`) in and before which
/// GitHub has more than was fetched: only the newest are.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Short {
    pub commits: Option<String>,
    pub pulls: Option<String>,
    pub issues: Option<String>,
    pub reviews: Option<String>,
}

impl Short {
    /// Counts may be short in every month.
    pub const EVERY_MONTH: &str = "9999-12";

    /// Whether a count in `month` may be short, given the month a kind is
    /// short from.
    pub fn in_month(from: Option<&String>, month: &str) -> bool {
        from.is_some_and(|from| month <= from.as_str())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Week {
    /// `YYYY-MM-DD` of its first day.
    pub start: String,
    /// Each day's level, 0 (none) to 4 (most).
    pub days: Vec<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonthActivity {
    /// `YYYY-MM`.
    pub month: String,
    /// Commits per repository, most first.
    pub commits: Vec<(String, u64)>,
    pub pulls: Vec<Contributed>,
    pub issues: Vec<Contributed>,
    pub reviews: Vec<Contributed>,
}

/// An issue or pull request someone opened or reviewed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contributed {
    /// `owner/name`.
    pub repo: String,
    pub number: u64,
    pub title: String,
}

// ---- wire types: shared fragments ---------------------------------------------

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Repository")]
pub struct RepoCard {
    pub name_with_owner: String,
    pub description: Option<String>,
    pub stargazer_count: i32,
    pub fork_count: i32,
    pub primary_language: Option<Language>,
    pub pushed_at: Option<DateTime>,
    pub is_private: bool,
    pub is_fork: bool,
    pub is_archived: bool,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct Language {
    pub name: String,
    pub color: Option<String>,
}

// A conversation's lists: the newest page (`last:`, oldest first within
// it) and how many there are, so what's left out can be said.
fragments! {
    counted:
    PrCommits = "PullRequestCommitConnection" => PrCommitNode,
}

// Lists of nodes, and their nodes' types.
fragments! {
    counted:
    Topics = "RepositoryTopicConnection" => RepoTopic,
    Assignees = "UserConnection" => UserLogin,
    CommitParents = "CommitConnection" => CommitOid,
    OrgLogins = "OrganizationConnection" => OrgLogin,
    MemberList = "OrganizationMemberConnection" => UserLogin,
    RepoList = "RepositoryConnection" => RepoCard,
    AssetList = "ReleaseAssetConnection" => WireAsset,
}

fragments! {
    total_count: i32;
    StarCount = "StarredRepositoryConnection",
}

fragments! {
    nodes:
    Pinned = "PinnableItemConnection" => PinnedItem,
}

// Pages of lists, and how many there are in all.
fragments! {
    paged:
    CommitHistory = "CommitHistoryConnection" => CommitCard,
    RollupContexts = "StatusCheckRollupContextConnection" => RollupContext,
    PagedRepos = "RepositoryConnection" => RepoCard,
    PagedStars = "StarredRepositoryConnection" => RepoCard,
    ReleaseList = "ReleaseConnection" => WireRelease,
    TagRefs = "RefConnection" => TagRef,
}

// The roots of the queries below, and what they open in a repository.
fragments! {
    root "Query" repository(owner, name);
    RepoQuery("RepoVariables") => RepoFull,
    /// A directory listing or file by `<ref>:<path>`.
    ObjectQuery("RepoVariables") => RepoObject,
    IssueQuery("NumberVariables") => RepoIssue,
    PrActivityQuery("NumberVariables") => RepoPrActivity,
    /// A commit by revision (`<sha>`, short or full).
    CommitQuery("RepoVariables") => RepoCommit,
    /// A revision's commits, newest first (touching `path`, if given).
    HistoryQuery("HistoryVariables") => RepoHistory,
    /// The checks on a pull request's head.
    PrChecksQuery("NumberVariables") => RepoPrChecks,
    ContextsQuery("ContextsVariables") => ContextsRepo,
    /// The checks on a commit, by revision.
    CommitChecksQuery("RepoVariables") => RepoCommitChecks,
    /// A repository's forks, most starred first.
    ForksQuery("ListVariables") => RepoForks,
    /// A repository's releases, newest first.
    ReleasesQuery("ListVariables") => RepoReleases,
    /// One release, with its notes and assets.
    ReleaseQuery("ReleaseVariables") => RepoRelease,
    /// A repository's tags, most recently committed first.
    TagsQuery("ListVariables") => RepoTags,
}
fragments! {
    root "Repository" object(expression);
    RepoObject("RepoVariables") => GitObject,
    RepoCommit("RepoVariables") => CommitObject,
    RepoHistory("HistoryVariables") => HistoryObject,
    ContextsRepo("ContextsVariables") => ContextsTarget,
    RepoCommitChecks("RepoVariables") => ChecksTarget,
}
fragments! {
    root "Repository" pull_request(number);
    RepoPrActivity("NumberVariables") => WirePrActivity,
    RepoPrChecks("NumberVariables") => PrChecks,
}
fragments! {
    root "Repository" issue_or_pull_request(number);
    RepoIssue("NumberVariables") => IssueOrPr,
}

// ---- repository ---------------------------------------------------------------------

#[derive(cynic::QueryVariables, Debug)]
pub struct RepoVariables {
    pub owner: String,
    pub name: String,
    /// `HEAD:` for the root of the default branch.
    pub expression: String,
}

impl RepoVariables {
    /// `repo`'s object at `expression`.
    pub fn at(repo: &RepoId, expression: impl Into<String>) -> Self {
        Self {
            owner: repo.owner.clone(),
            name: repo.name.clone(),
            expression: expression.into(),
        }
    }
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Repository", variables = "RepoVariables")]
pub struct RepoFull {
    pub id: cynic::Id,
    #[cynic(spread)]
    pub card: RepoCard,
    pub homepage_url: Option<Uri>,
    pub watchers: UserCount,
    #[arguments(states: [OPEN])]
    pub issues: IssueCount,
    #[arguments(states: [OPEN])]
    pub pull_requests: PrCount,
    #[cynic(rename = "issues", alias)]
    #[arguments(states: [CLOSED])]
    pub closed_issues: IssueCount,
    #[cynic(rename = "pullRequests", alias)]
    #[arguments(states: [CLOSED, MERGED])]
    pub closed_pull_requests: PrCount,
    pub license_info: Option<License>,
    #[arguments(first: 20)]
    pub repository_topics: Topics,
    pub default_branch_ref: Option<BranchRef>,
    pub parent: Option<RepositoryName>,
    pub viewer_has_starred: bool,
    pub viewer_permission: Option<crate::api::queries::RepositoryPermission>,
    pub has_issues_enabled: bool,
    pub has_discussions_enabled: bool,
    pub has_wiki_enabled: bool,
    #[cynic(rename = "refs", alias)]
    #[arguments(refPrefix: "refs/heads/")]
    pub branch_count: Option<RefCount>,
    #[cynic(rename = "refs", alias)]
    #[arguments(refPrefix: "refs/tags/")]
    pub tag_count: Option<RefCount>,
    /// The issues pinned at the top of its Issues tab (GitHub pins up to 3).
    #[arguments(first: 3)]
    pub pinned_issues: Option<timeline::PinnedIssues>,
    #[arguments(expression: $expression)]
    pub object: Option<GitObject>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct License {
    pub name: String,
    pub spdx_id: Option<String>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RepositoryTopic")]
pub struct RepoTopic {
    pub topic: Topic,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct Topic {
    pub name: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Ref")]
pub struct BranchRef {
    pub name: String,
    pub target: Option<RefTarget>,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "GitObject")]
pub enum RefTarget {
    Commit(HeadCommit),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Commit")]
pub struct HeadCommit {
    #[cynic(spread)]
    pub commit: CommitCard,
    #[arguments(first: 0)]
    pub history: CommitCount,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct GitActor {
    pub name: Option<String>,
    pub user: Option<UserLogin>,
}

// Objects by the one field asked of them.
fragments! {
    login: String;
    UserLogin = "User",
    BotLogin = "Bot",
    OrgLogin = "Organization",
}
fragments! {
    name: String;
    RefName = "Ref",
    AppName = "App",
    WorkflowName = "Workflow",
    TypeName = "IssueType",
}
fragments! {
    number: i32;
    PrNumber = "PullRequest",
    IssueNumber = "Issue",
}
// Its `org/slug`.
fragments! { combined_slug: String; TeamSlug = "Team" }

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequestCommit")]
pub struct PrCommitNode {
    pub commit: CommitCard,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum GitObject {
    Tree(Tree),
    Blob(BlobObject),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct Tree {
    pub entries: Option<Vec<WireEntry>>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "TreeEntry")]
pub struct WireEntry {
    pub name: String,
    pub path: Option<String>,
    #[cynic(rename = "type")]
    pub kind: String,
    pub mode: i32,
    pub size: i32,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Blob")]
pub struct BlobObject {
    pub text: Option<String>,
    pub is_binary: Option<bool>,
    pub byte_size: i32,
    pub is_truncated: bool,
}

// ---- search ----------------------------------------------------------------------

#[derive(cynic::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchType {
    Discussion,
    Issue,
    IssueAdvanced,
    IssueHybrid,
    IssueSemantic,
    Repository,
    User,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct BrowseSearchVariables {
    pub query: String,
    pub kind: SearchType,
    pub first: i32,
    pub after: Option<String>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Query", variables = "BrowseSearchVariables")]
pub struct BrowseSearch {
    #[arguments(query: $query, type: $kind, first: $first, after: $after)]
    pub search: BrowseSearchConnection,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "SearchResultItemConnection")]
pub struct BrowseSearchConnection {
    pub issue_count: i32,
    pub repository_count: i32,
    pub user_count: i32,
    pub page_info: PageInfo,
    pub nodes: Option<Vec<Option<BrowseItem>>>,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct StateCountsVariables {
    pub open: String,
    pub closed: String,
}

/// How many issues or pull requests two searches find, and nothing else.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Query", variables = "StateCountsVariables")]
pub struct StateCountsQuery {
    #[cynic(rename = "search", alias)]
    #[arguments(query: $open, type: ISSUE, first: 0)]
    pub open: SearchCount,
    #[cynic(rename = "search", alias)]
    #[arguments(query: $closed, type: ISSUE, first: 0)]
    pub closed: SearchCount,
}

fragments! { issue_count: i32; SearchCount = "SearchResultItemConnection" }

/// A filtered issue or pull request list's open and closed counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateCounts {
    pub open: u64,
    pub closed: u64,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "SearchResultItem")]
pub enum BrowseItem {
    Repository(RepoCard),
    Issue(IssueCard),
    PullRequest(PrCard),
    User(UserCard),
    Organization(OrgCard),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Issue")]
pub struct IssueCard {
    pub number: i32,
    pub title: String,
    pub state: WireIssueState,
    pub state_reason: Option<StateReason>,
    pub author: Option<Actor>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
    pub comments: CommentCount,
    #[arguments(first: 6)]
    pub labels: Option<LabelConnection>,
    pub repository: RepositoryName,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequest")]
pub struct PrCard {
    /// For its checks, read apart (see `GitHub::search`).
    pub id: cynic::Id,
    pub number: i32,
    pub title: String,
    #[cynic(spread)]
    pub status: PrStatus,
    pub author: Option<Actor>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
    pub review_decision: Option<ReviewDecision>,
    /// Each reviewer's latest approval or change request.
    #[arguments(first: 20)]
    pub latest_opinionated_reviews: Option<timeline::Opinions>,
    pub comments: CommentCount,
    #[arguments(first: 6)]
    pub labels: Option<LabelConnection>,
    pub repository: RepositoryName,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "User")]
pub struct UserCard {
    pub login: String,
    pub name: Option<String>,
    pub bio: Option<String>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Organization")]
pub struct OrgCard {
    pub login: String,
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(cynic::Enum, Debug, Clone, Copy, PartialEq, Eq)]
#[cynic(graphql_type = "IssueState")]
pub enum WireIssueState {
    Closed,
    Open,
}

#[derive(cynic::Enum, Debug, Clone, Copy, PartialEq, Eq)]
#[cynic(graphql_type = "IssueStateReason")]
pub enum StateReason {
    Completed,
    Duplicate,
    NotPlanned,
    Reopened,
}

// ---- issue ------------------------------------------------------------------------

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "IssueOrPullRequest")]
pub enum IssueOrPr {
    Issue(Box<IssueFull>),
    PullRequest(PrNumber),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Issue")]
pub struct IssueFull {
    pub id: cynic::Id,
    pub number: i32,
    pub title: String,
    pub body: String,
    pub state: WireIssueState,
    pub state_reason: Option<StateReason>,
    pub author: Option<Actor>,
    pub created_at: DateTime,
    #[arguments(first: 20)]
    pub labels: Option<LabelConnection>,
    #[arguments(first: 10)]
    pub assignees: Assignees,
    pub milestone: Option<crate::api::queries::MilestoneName>,
    pub comments: CommentCount,
    #[arguments(last: 100, itemTypes: [ISSUE_COMMENT, CLOSED_EVENT, REOPENED_EVENT, CROSS_REFERENCED_EVENT, REFERENCED_EVENT, LABELED_EVENT, UNLABELED_EVENT, ASSIGNED_EVENT, UNASSIGNED_EVENT, MILESTONED_EVENT, DEMILESTONED_EVENT, ADDED_TO_PROJECT_V2_EVENT, REMOVED_FROM_PROJECT_V2_EVENT, PROJECT_V2_ITEM_STATUS_CHANGED_EVENT, RENAMED_TITLE_EVENT, LOCKED_EVENT, UNLOCKED_EVENT, PINNED_EVENT, UNPINNED_EVENT, TRANSFERRED_EVENT, MARKED_AS_DUPLICATE_EVENT, UNMARKED_AS_DUPLICATE_EVENT, SUB_ISSUE_ADDED_EVENT, SUB_ISSUE_REMOVED_EVENT, PARENT_ISSUE_ADDED_EVENT, PARENT_ISSUE_REMOVED_EVENT, ISSUE_TYPE_ADDED_EVENT, ISSUE_TYPE_CHANGED_EVENT, ISSUE_TYPE_REMOVED_EVENT, CONVERTED_TO_DISCUSSION_EVENT])]
    pub timeline_items: timeline::IssueTimeline,
    #[arguments(first: 10, includeClosedPrs: true)]
    pub closed_by_pull_requests_references: Option<timeline::ClosedBy>,
    #[arguments(first: 10)]
    pub linked_branches: timeline::Branches,
    pub parent: Option<timeline::LinkedIssue>,
    #[arguments(first: 50)]
    pub sub_issues: timeline::Issues,
    pub sub_issues_summary: timeline::WireProgress,
    #[arguments(first: 20)]
    pub blocked_by: timeline::Issues,
    #[arguments(first: 20)]
    pub blocking: timeline::Issues,
    pub issue_type: Option<TypeName>,
    pub repository: RepoAccess,
    pub locked: bool,
    pub active_lock_reason: Option<timeline::LockReason>,
    pub is_pinned: Option<bool>,
    pub viewer_can_close: bool,
    pub viewer_can_reopen: bool,
    pub viewer_can_update: bool,
    pub viewer_can_react: bool,
    pub viewer_can_label: bool,
    pub viewer_can_assign: bool,
    pub viewer_can_set_milestone: bool,
    pub reaction_groups: Option<Vec<WireReactionGroup>>,
}

/// A repository's name, and what you may do in it.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Repository")]
pub struct RepoAccess {
    pub name_with_owner: String,
    pub viewer_permission: Option<crate::api::queries::RepositoryPermission>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "IssueComment")]
pub struct WireComment {
    pub id: cynic::Id,
    /// `databaseId` overflows GraphQL's `Int` on newer comments.
    pub full_database_id: Option<crate::api::queries::BigInt>,
    pub author: Option<Actor>,
    pub body: String,
    pub created_at: DateTime,
    pub viewer_can_update: bool,
    pub viewer_can_delete: bool,
    pub viewer_can_react: bool,
    pub reaction_groups: Option<Vec<WireReactionGroup>>,
    pub is_minimized: bool,
    pub minimized_reason: Option<String>,
    pub viewer_can_minimize: bool,
    pub viewer_can_unminimize: bool,
}

// ---- PR activity -----------------------------------------------------------------

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequest")]
pub struct WirePrActivity {
    pub id: cynic::Id,
    pub comments: CommentCount,
    #[arguments(last: 100, itemTypes: [ISSUE_COMMENT, PULL_REQUEST_REVIEW, PULL_REQUEST_COMMIT, CLOSED_EVENT, REOPENED_EVENT, MERGED_EVENT, CROSS_REFERENCED_EVENT, REFERENCED_EVENT, LABELED_EVENT, UNLABELED_EVENT, ASSIGNED_EVENT, UNASSIGNED_EVENT, MILESTONED_EVENT, DEMILESTONED_EVENT, ADDED_TO_PROJECT_V2_EVENT, REMOVED_FROM_PROJECT_V2_EVENT, PROJECT_V2_ITEM_STATUS_CHANGED_EVENT, REVIEW_REQUESTED_EVENT, REVIEW_REQUEST_REMOVED_EVENT, REVIEW_DISMISSED_EVENT, RENAMED_TITLE_EVENT, READY_FOR_REVIEW_EVENT, CONVERT_TO_DRAFT_EVENT, HEAD_REF_FORCE_PUSHED_EVENT, BASE_REF_FORCE_PUSHED_EVENT, HEAD_REF_DELETED_EVENT, HEAD_REF_RESTORED_EVENT, BASE_REF_CHANGED_EVENT, LOCKED_EVENT, UNLOCKED_EVENT, MARKED_AS_DUPLICATE_EVENT, UNMARKED_AS_DUPLICATE_EVENT, AUTO_MERGE_ENABLED_EVENT, AUTO_MERGE_DISABLED_EVENT, ADDED_TO_MERGE_QUEUE_EVENT, REMOVED_FROM_MERGE_QUEUE_EVENT])]
    pub timeline_items: timeline::PrTimeline,
    #[arguments(last: 100)]
    pub commits: PrCommits,
    #[arguments(first: 100)]
    pub review_requests: Option<timeline::Requests>,
    #[arguments(first: 100)]
    pub latest_reviews: Option<timeline::LatestReviews>,
    #[arguments(first: 10)]
    pub closing_issues_references: Option<timeline::Issues>,
    pub merge_commit: Option<crate::api::queries::CommitOid>,
    /// Your pending review: GitHub shows it only to you.
    #[arguments(first: 1, states: [PENDING])]
    #[cynic(rename = "reviews", alias)]
    pub pending_review: Option<timeline::PendingReviews>,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum RequestedReviewer {
    User(UserLogin),
    Bot(BotLogin),
    Team(TeamSlug),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Commit")]
pub struct CommitCard {
    pub oid: GitObjectId,
    /// The whole message: GitHub cuts `messageHeadline` at 72 columns.
    pub message: String,
    pub committed_date: DateTime,
    pub author: Option<GitActor>,
}

// ---- commits -------------------------------------------------------------------------

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "GitObject")]
pub enum CommitObject {
    Commit(Box<WireCommit>),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Commit")]
pub struct WireCommit {
    pub oid: GitObjectId,
    pub message: String,
    pub authored_date: DateTime,
    pub committed_date: DateTime,
    pub author: Option<GitActor>,
    pub committer: Option<GitActor>,
    #[arguments(first: 5)]
    pub parents: CommitParents,
    pub additions: i32,
    pub deletions: i32,
    pub changed_files_if_available: Option<i32>,
    pub signature: Option<WireSignature>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "GitSignature")]
pub struct WireSignature {
    pub is_valid: bool,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct HistoryVariables {
    pub owner: String,
    pub name: String,
    pub expression: String,
    pub path: Option<String>,
    pub after: Option<String>,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "GitObject", variables = "HistoryVariables")]
pub enum HistoryObject {
    Commit(HistoryCommit),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Commit", variables = "HistoryVariables")]
pub struct HistoryCommit {
    #[arguments(first: 30, after: $after, path: $path)]
    pub history: CommitHistory,
}

impl CommitHistory {
    pub(crate) fn into_results(self) -> Results<CommitInfo> {
        Results::from_page(self.total_count, self.page_info, self.nodes, |c| {
            Some(c.into_info())
        })
    }
}

// ---- checks --------------------------------------------------------------------------

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequest")]
pub struct PrChecks {
    /// What the newest commit should be.
    pub head_ref_oid: GitObjectId,
    #[arguments(last: 1)]
    pub commits: PrHeadCommits,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequestCommitConnection")]
pub struct PrHeadCommits {
    pub nodes: Option<Vec<Option<PrHeadCommit>>>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequestCommit")]
pub struct PrHeadCommit {
    pub commit: ChecksCommit,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "GitObject")]
pub enum ChecksTarget {
    Commit(ChecksCommit),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Commit")]
pub struct ChecksCommit {
    pub oid: GitObjectId,
    pub status_check_rollup: Option<WireRollup>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "StatusCheckRollup")]
pub struct WireRollup {
    #[arguments(first: 100)]
    pub contexts: RollupContexts,
}

/// A later page of a commit's checks (by its oid in `expression`): a
/// re-run on a later page is a check's newest run.
#[derive(cynic::QueryVariables, Debug)]
pub struct ContextsVariables {
    pub owner: String,
    pub name: String,
    pub expression: String,
    pub after: Option<String>,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "GitObject", variables = "ContextsVariables")]
pub enum ContextsTarget {
    Commit(ContextsCommit),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Commit", variables = "ContextsVariables")]
pub struct ContextsCommit {
    pub status_check_rollup: Option<ContextsRollup>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "StatusCheckRollup", variables = "ContextsVariables")]
pub struct ContextsRollup {
    #[arguments(first: 100, after: $after)]
    pub contexts: RollupContexts,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "StatusCheckRollupContext")]
pub enum RollupContext {
    CheckRun(Box<WireCheckRun>),
    StatusContext(WireStatusContext),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "CheckRun")]
pub struct WireCheckRun {
    pub name: String,
    pub status: CheckStatusState,
    pub conclusion: Option<CheckConclusionState>,
    pub started_at: Option<DateTime>,
    pub completed_at: Option<DateTime>,
    pub title: Option<String>,
    pub details_url: Option<Uri>,
    pub check_suite: Option<WireSuite>,
    #[arguments(first: 50)]
    pub annotations: Option<crate::api::workflows::WireAnnotations>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "CheckSuite")]
pub struct WireSuite {
    pub app: Option<AppName>,
    pub workflow_run: Option<WireWorkflowRun>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "WorkflowRun")]
pub struct WireWorkflowRun {
    pub workflow: WorkflowName,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "StatusContext")]
pub struct WireStatusContext {
    pub context: String,
    pub state: StatusState,
    pub description: Option<String>,
    pub target_url: Option<Uri>,
    pub created_at: DateTime,
}

#[derive(cynic::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckStatusState {
    Completed,
    InProgress,
    Pending,
    Queued,
    Requested,
    Waiting,
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckConclusionState {
    ActionRequired,
    Cancelled,
    Failure,
    Neutral,
    Skipped,
    Stale,
    StartupFailure,
    Success,
    TimedOut,
    #[cynic(fallback)]
    Other,
}

impl CheckOutcome {
    /// How a check run, job or step came out, from GraphQL or REST alike:
    /// pending until it completes (a REST job without a status is queued),
    /// and a failure if it completed in a way not known here or not said,
    /// so a finished job is never taken for a running one.
    pub(crate) fn of(
        status: Option<CheckStatusState>,
        conclusion: Option<CheckConclusionState>,
    ) -> Self {
        use CheckConclusionState as C;
        match status {
            Some(CheckStatusState::Completed) => match conclusion.unwrap_or(C::Other) {
                C::Success => Self::Success,
                C::Skipped => Self::Skipped,
                C::Neutral | C::Stale => Self::Neutral,
                C::Cancelled => Self::Cancelled,
                C::Failure | C::TimedOut | C::StartupFailure | C::ActionRequired | C::Other => {
                    Self::Failure
                }
            },
            _ => Self::Pending,
        }
    }
}

/// A page of a commit's rollup contexts; a commit with no rollup has none.
pub(crate) fn rollup_contexts(rollup: Option<WireRollup>) -> Results<RollupContext> {
    rollup
        .map(|r| r.contexts.into_results())
        .unwrap_or_default()
}

impl RollupContexts {
    pub(crate) fn into_results(self) -> Results<RollupContext> {
        Results::from_page(self.total_count, self.page_info, self.nodes, Some)
    }
}

/// A commit's checks from its rollup's contexts, the newest run of each.
pub(crate) fn checks(oid: String, contexts: Capped<RollupContext>) -> Checks {
    let left_out = contexts.left_out();
    let items = contexts
        .items
        .into_iter()
        .filter_map(|c| match c {
            RollupContext::CheckRun(run) => Some(run.into_item()),
            RollupContext::StatusContext(s) => Some(s.into_item()),
            RollupContext::Other => None,
        })
        .collect();
    let items = latest_runs(items);
    // Those past the pages read may be earlier runs too; count them as
    // checks still to show.
    let total = items.len() as u64 + left_out;
    Checks {
        oid,
        items: Capped::new(items, total),
    }
}

/// The latest run of each check. A workflow that runs again on the same
/// commit (a re-run, or one a label or title edit triggers again) leaves
/// every run in the rollup; GitHub shows only the newest. One not started
/// yet is the newest.
fn latest_runs(items: Vec<CheckItem>) -> Vec<CheckItem> {
    use std::collections::hash_map::{Entry, HashMap};
    fn newness(i: &CheckItem) -> (bool, Option<&str>) {
        (i.started_at.is_none(), i.started_at.as_deref())
    }
    // Where each check is kept, so the first-seen order stays.
    let mut at: HashMap<(String, String), usize> = HashMap::new();
    let mut out: Vec<CheckItem> = Vec::new();
    for item in items {
        match at.entry((item.group.clone(), item.name.clone())) {
            Entry::Occupied(e) => {
                if let Some(kept) = out.get_mut(*e.get())
                    && newness(&item) > newness(kept)
                {
                    *kept = item;
                }
            }
            Entry::Vacant(e) => {
                e.insert(out.len());
                out.push(item);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {

    /// A tag's commit, lightweight or annotated; a tree's entries by kind,
    /// directories first, then by name whatever its case.
    #[test]
    fn tags_resolve_to_commits_and_trees_list_dirs_first() {
        let tags: TagRefs = serde_json::from_value(serde_json::json!({
            "totalCount": 3,
            "pageInfo": {"hasNextPage": true, "endCursor": "c"},
            "nodes": [
                {"name": "light", "target": {"__typename": "Commit", "oid": "a1", "committedDate": "2026-01-01T00:00:00Z"}},
                {"name": "annotated", "target": {"__typename": "Tag", "target": {"__typename": "Commit", "oid": "b2", "committedDate": "2026-02-01T00:00:00Z"}}},
                {"name": "tree", "target": {"__typename": "Tree"}}
            ]
        }))
        .unwrap();
        let tags = tags.into_results();
        let got: Vec<(&str, Option<&str>)> = tags
            .items
            .iter()
            .map(|t| (t.name.as_str(), t.oid.as_deref()))
            .collect();
        assert_eq!(
            got,
            [
                ("light", Some("a1")),
                ("annotated", Some("b2")),
                ("tree", None)
            ]
        );
        assert_eq!((tags.total, tags.next.as_deref()), (3, Some("c")));

        let tree: Tree = serde_json::from_value(serde_json::json!({"entries": [
            {"name": "b.txt", "path": "b.txt", "type": "blob", "mode": 0o100_644, "size": 5},
            {"name": "link", "path": "link", "type": "blob", "mode": 0o120_000, "size": 4},
            {"name": "Zdir", "path": "Zdir", "type": "tree", "mode": 0o040_000, "size": 0},
            {"name": "sub", "path": "sub", "type": "commit", "mode": 0o160_000, "size": 0},
            {"name": "a.txt", "path": null, "type": "blob", "mode": 0o100_644, "size": 1}
        ]}))
        .unwrap();
        let got: Vec<(String, EntryKind, Option<u64>)> = entries(tree)
            .into_iter()
            .map(|e| (e.path, e.kind, e.size))
            .collect();
        assert_eq!(
            got,
            [
                ("Zdir".into(), EntryKind::Dir, None),
                ("a.txt".into(), EntryKind::File, Some(1)),
                ("b.txt".into(), EntryKind::File, Some(5)),
                ("link".into(), EntryKind::Symlink, None),
                ("sub".into(), EntryKind::Submodule, None),
            ]
        );
    }

    /// A month's activity: commits summed per repository, most first; a PR
    /// reviewed twice once; months newest first, three of them. Where
    /// GitHub has more than came, the counts say they may be short: every
    /// month's commits with more repositories than fetched, and a list's
    /// from its oldest month on.
    #[test]
    fn activity_is_grouped_by_month_and_says_where_its_short() {
        let item = |at: &str, number: u32| serde_json::json!({"occurredAt": at, "pullRequest": {"number": number, "title": "t", "repository": {"nameWithOwner": "o/r"}}});
        let commits = |repo: &str, days: &[(&str, u32)]| {
            serde_json::json!({"repository": {"nameWithOwner": repo}, "contributions": {
                "totalCount": days.len(),
                "nodes": days.iter().map(|(at, n)| serde_json::json!({"occurredAt": at, "commitCount": n})).collect::<Vec<_>>()
            }})
        };
        let wire: WireContributions = serde_json::from_value(serde_json::json!({
            "contributionCalendar": {"totalContributions": 9, "weeks": []},
            "commitContributionsByRepository": [
                commits("o/a", &[("2026-10-02T00:00:00Z", 2), ("2026-10-01T00:00:00Z", 3)]),
                commits("o/b", &[("2026-10-03T00:00:00Z", 7)]),
            ],
            "totalRepositoriesWithContributedCommits": 3,
            "pullRequestContributions": {"totalCount": 50, "nodes": [
                item("2026-10-02T00:00:00Z", 1), item("2026-09-20T00:00:00Z", 2)
            ]},
            "issueContributions": {"totalCount": 0, "nodes": []},
            "pullRequestReviewContributions": {"totalCount": 4, "nodes": [
                item("2026-09-05T00:00:00Z", 5), item("2026-09-04T00:00:00Z", 5),
                item("2026-08-01T00:00:00Z", 6), item("2026-07-01T00:00:00Z", 8)
            ]},
        }))
        .unwrap();
        let c = wire.into_contributions();
        let months: Vec<&str> = c.activity.iter().map(|m| m.month.as_str()).collect();
        assert_eq!(months, ["2026-10", "2026-09", "2026-08"]);
        assert_eq!(
            c.activity[0].commits,
            [("o/b".to_owned(), 7), ("o/a".to_owned(), 5)]
        );
        assert_eq!(c.activity[1].reviews.len(), 1);
        assert_eq!(c.short.commits.as_deref(), Some(Short::EVERY_MONTH));
        assert_eq!(c.short.pulls.as_deref(), Some("2026-09"));
        assert_eq!(c.short.reviews, None);
        assert!(Short::in_month(c.short.pulls.as_ref(), "2026-09"));
        assert!(!Short::in_month(c.short.pulls.as_ref(), "2026-10"));
    }

    /// How REST names a run's, job's or step's outcome (found by mutation
    /// testing: no test pinned it), decoded as REST spells it.
    #[test]
    fn rest_statuses_and_conclusions_map_to_outcomes() {
        for (status, conclusion, outcome) in [
            ("completed", Some("success"), CheckOutcome::Success),
            ("completed", Some("skipped"), CheckOutcome::Skipped),
            ("completed", Some("cancelled"), CheckOutcome::Cancelled),
            ("completed", Some("neutral"), CheckOutcome::Neutral),
            ("completed", Some("stale"), CheckOutcome::Neutral),
            ("completed", Some("failure"), CheckOutcome::Failure),
            ("completed", Some("timed_out"), CheckOutcome::Failure),
            ("completed", Some("something_new"), CheckOutcome::Failure),
            ("completed", None, CheckOutcome::Failure),
            ("in_progress", None, CheckOutcome::Pending),
            ("queued", None, CheckOutcome::Pending),
        ] {
            let job: rest_actions::Job = serde_json::from_value(serde_json::json!({
                "id": 1, "run_id": 2, "name": "build", "status": status,
                "conclusion": conclusion, "started_at": null, "completed_at": null,
                "steps": [{"number": 1, "name": "test", "status": status,
                    "conclusion": conclusion, "started_at": null, "completed_at": null}],
            }))
            .unwrap();
            let run = || -> rest_actions::Run {
                serde_json::from_value(serde_json::json!({
                    "id": 1, "run_number": 3, "event": "push", "head_sha": "abc",
                    "status": status, "conclusion": conclusion,
                }))
                .unwrap()
            };
            let job = job.into_job(run().into_job_run(None));
            assert_eq!(job.outcome, outcome, "{status} {conclusion:?}");
            assert_eq!(
                job.run.outcome, outcome,
                "job's run {status} {conclusion:?}"
            );
            assert_eq!(
                job.steps[0].outcome, outcome,
                "step {status} {conclusion:?}"
            );
            assert_eq!(
                run().into_summary().outcome,
                outcome,
                "run {status} {conclusion:?}"
            );
        }
    }

    /// A search's next page and counts, whatever it searched.
    #[test]
    fn search_results_page_and_count() {
        let user = UserSummary {
            login: "a".into(),
            name: None,
            bio: None,
            is_org: false,
        };
        let users = Results {
            total: 40,
            items: vec![user; 2],
            next: Some("c2".into()),
        };
        let r = SearchResults::Users(users);
        assert_eq!((r.next(), r.counts()), (Some("c2"), (40, 2)));
        let empty = SearchResults::Repos(Results::default());
        assert_eq!((empty.next(), empty.counts()), (None, (0, 0)));
    }

    /// A commit's checks past the first page are counted, not dropped;
    /// blank summaries are none.
    #[test]
    fn checks_count_what_wasnt_fetched() {
        let run = |name: &str, title: &str| {
            serde_json::json!({"__typename": "CheckRun", "name": name, "status": "COMPLETED",
                "conclusion": "SUCCESS", "startedAt": null, "completedAt": null, "title": title,
                "detailsUrl": null, "checkSuite": null, "annotations": null})
        };
        let contexts: RollupContexts = serde_json::from_value(serde_json::json!({
            "totalCount": 5, "pageInfo": {"hasNextPage": true, "endCursor": "c"},
            "nodes": [run("a", "  "), run("b", "Built")]
        }))
        .unwrap();
        let fetched = contexts.into_results();
        let checks = checks("abc".into(), Capped::new(fetched.items, fetched.total));
        assert_eq!((checks.items.len(), checks.items.total()), (2, 5));
        assert_eq!(checks.items[0].summary, None);
        assert_eq!(checks.items[1].summary.as_deref(), Some("Built"));
    }

    /// Only a line that starts with GitHub's timestamp has one.
    #[test]
    fn log_lines_split_off_their_timestamps() {
        use super::log::split;
        assert_eq!(
            split("2026-10-05T17:42:01.1234567Z hello there"),
            (Some("2026-10-05T17:42:01.1234567Z"), "hello there")
        );
        assert_eq!(
            split("\u{feff}2026-10-05T17:42:01.1Z x"),
            (Some("2026-10-05T17:42:01.1Z"), "x")
        );
        for line in [
            "hello there",
            "2026-10-05 short",
            "2026/10/05T17:42:01.1234Z x",
            "20261005T17:42:01.12345Z x",
            "2026-10-05T17:42:01.12345+ x",
        ] {
            assert_eq!(split(line).0, None, "{line}");
        }
    }

    /// A long log is cut at a line's start; the lines cut keep their
    /// timestamps, and the ones that start steps their text.
    #[test]
    fn a_long_log_is_cut_at_a_line() {
        use super::log;
        assert_eq!(log::cut("a\nb\n"), (String::new(), "a\nb\n"));
        let filler = "x".repeat(100);
        let mut full = String::from("2026-10-05T17:42:00.1Z ##[group]Run ./build\nno time\n");
        while full.len() <= log::KEEP + 1000 {
            full.push_str(&format!("2026-10-05T17:42:01.1Z {filler}\n"));
        }
        let (cut, kept) = log::cut(&full);
        assert!(kept.len() <= log::KEEP && kept.starts_with("2026-"));
        assert_eq!(
            cut.lines().count() + kept.lines().count(),
            full.lines().count()
        );
        let mut lines = cut.lines();
        assert_eq!(
            lines.next(),
            Some("2026-10-05T17:42:00.1Z ##[group]Run ./build")
        );
        assert_eq!(lines.next(), Some(""));
        assert_eq!(lines.next(), Some("2026-10-05T17:42:01.1Z"));
    }

    use super::*;

    /// A branch's pull request is one in its own repository from that
    /// branch: GitHub also lists forks' PRs whose head it is.
    #[test]
    fn a_branchs_pr_is_its_own() {
        let branch: wire_branches::Branch = serde_json::from_value(serde_json::json!({
            "name": "trunk",
            "target": null,
            "associatedPullRequests": { "nodes": [
                { "number": 1, "state": "MERGED", "isDraft": false, "headRefName": "trunk",
                  "repository": { "nameWithOwner": "someone/cli-fork" } },
                { "number": 9, "state": "OPEN", "isDraft": false, "headRefName": "other",
                  "repository": { "nameWithOwner": "cli/cli" } },
                { "number": 7, "state": "CLOSED", "isDraft": false, "headRefName": "trunk",
                  "repository": { "nameWithOwner": "CLI/cli" } },
            ] },
        }))
        .unwrap();
        let info = branch.into_info(&RepoId::new("cli", "cli"), Some("trunk"));
        assert_eq!(info.pr, Some((7, PrState::Closed)));
        assert!(info.default);
    }

    /// A branch whose tip has no author shows only its headline, not a
    /// made-up "unknown"; a git name is still shown.
    #[test]
    fn a_branch_tip_without_an_author_names_nobody() {
        let author = |author: serde_json::Value| {
            let branch: wire_branches::Branch = serde_json::from_value(serde_json::json!({
                "name": "trunk",
                "target": { "oid": "abc", "messageHeadline": "Fix",
                    "committedDate": "2026-01-01T00:00:00Z", "author": author },
                "associatedPullRequests": { "nodes": [] },
            }))
            .unwrap();
            branch.into_info(&RepoId::new("cli", "cli"), None).author
        };
        assert_eq!(author(serde_json::Value::Null), None);
        assert_eq!(
            author(serde_json::json!({ "name": "", "user": null })),
            None
        );
        assert_eq!(
            author(serde_json::json!({ "name": "Jane", "user": null })),
            Some(Person::Git("Jane".into()))
        );
    }

    /// A gist file without a name (GitHub's schema allows it) doesn't
    /// spoil the list.
    #[test]
    fn gist_files_may_lack_names() {
        let gists: wire::Connection<wire_gists::Gist> = serde_json::from_value(serde_json::json!({
            "totalCount": 1,
            "pageInfo": { "hasNextPage": false, "endCursor": null },
            "nodes": [{ "name": "abc", "description": null, "updatedAt": "2026-10-01T00:00:00Z",
                "stargazerCount": 0, "files": [{ "name": null }, { "name": "a.rs" }],
                "comments": { "totalCount": 0 } }],
        }))
        .unwrap();
        let gists = gists.into_results(wire_gists::Gist::into_summary);
        assert_eq!(gists.items[0].files, ["a.rs"]);
    }

    /// A comparison listing no commits (the head is behind or at the
    /// base) is to the merge base, from the base when asked directly.
    #[test]
    fn a_comparison_without_commits_is_to_the_merge_base() {
        let compare: rest_compare::Compare = serde_json::from_value(serde_json::json!({
            "status": "behind", "ahead_by": 0, "behind_by": 1, "total_commits": 0,
            "base_commit": { "sha": "base" }, "merge_base_commit": { "sha": "mb" },
            "commits": [],
        }))
        .unwrap();
        let c = compare.into_comparison(true);
        assert_eq!((c.from.as_str(), c.to.as_str()), ("base", "mb"));
    }

    fn run(group: &str, name: &str, started: Option<&str>, outcome: CheckOutcome) -> CheckItem {
        CheckItem {
            name: name.into(),
            group: group.into(),
            outcome,
            started_at: started.map(|s| format!("2026-09-28T{s}Z")),
            completed_at: None,
            summary: None,
            url: None,
            annotations: Capped::default(),
        }
    }

    /// Re-runs, and runs a label edit triggered again, show once: the
    /// newest, which a run not yet started is.
    #[test]
    fn only_the_latest_run_of_a_check_shows() {
        use CheckOutcome::{Failure, Pending, Success};
        let items = vec![
            run("CI", "test", Some("15:46:00"), Failure),
            run("CI", "test", Some("15:52:00"), Success),
            run("PR", "check-title", Some("15:58:00"), Success),
            run("PR", "check-title", Some("16:01:00"), Failure),
            run("PR", "check-title", Some("15:59:00"), Success),
            run("Release", "test", Some("10:00:00"), Success),
            run("Lint", "fmt", Some("10:00:00"), Success),
            run("Lint", "fmt", None, Pending),
        ];
        let latest: Vec<String> = latest_runs(items)
            .iter()
            .map(|i| format!("{}/{} {:?}", i.group, i.name, i.outcome))
            .collect();
        assert_eq!(
            latest,
            [
                "CI/test Success",
                "PR/check-title Failure",
                "Release/test Success",
                "Lint/fmt Pending",
            ]
        );
    }

    /// A commit found by search whose author has no account is by its
    /// author's git name, not by whoever committed it (cherry-picked, applied).
    #[test]
    fn a_found_commit_by_an_unlinked_author_isnt_by_its_committer() {
        let found: super::rest_search::Commit = serde_json::from_value(serde_json::json!({
            "sha": "abc123",
            "repository": {"full_name": "o/r"},
            "author": null,
            "commit": {
                "message": "Fix the thing",
                "author": {"name": "Jane Doe", "date": "2026-01-01T00:00:00Z"},
                "committer": {"name": "Bob Maintainer", "date": "2026-02-01T00:00:00Z"}
            }
        }))
        .unwrap();
        let hit = found.into_hit().unwrap();
        assert_eq!(hit.commit.author, super::Person::Git("Jane Doe".into()));
        assert_eq!(hit.commit.date, "2026-02-01T00:00:00Z");
    }

    /// A commit's committer is shown unless they read as its author, or are
    /// nobody or GitHub's own web-flow account; a git name that happens to
    /// read "web-flow" is somebody.
    #[test]
    fn a_commits_committer_is_hidden_only_for_web_flows_account() {
        let committer = |c: serde_json::Value| {
            let wire: super::WireCommit = serde_json::from_value(serde_json::json!({
                "oid": "abc123",
                "message": "Fix",
                "authoredDate": "2026-01-01T00:00:00Z",
                "committedDate": "2026-01-02T00:00:00Z",
                "author": {"name": "Jane", "user": {"login": "jane"}},
                "committer": c,
                "parents": {"totalCount": 0, "nodes": []},
                "additions": 0,
                "deletions": 0,
                "changedFilesIfAvailable": null,
                "signature": null
            }))
            .unwrap();
            wire.into_detail().committer
        };
        let web_flow = serde_json::json!({"name": "GitHub", "user": {"login": "web-flow"}});
        assert_eq!(committer(web_flow), None);
        assert_eq!(
            committer(serde_json::json!({"name": "web-flow", "user": null})),
            Some(super::Person::Git("web-flow".into()))
        );
        let jane = serde_json::json!({"name": "J", "user": {"login": "jane"}});
        assert_eq!(committer(jane), None);
        // Shown as the author is: "jane", whether by login or by git name.
        assert_eq!(
            committer(serde_json::json!({"name": "jane", "user": null})),
            None
        );
        assert_eq!(committer(serde_json::json!(null)), None);
    }
}

impl WireCheckRun {
    fn into_item(self) -> CheckItem {
        let outcome = CheckOutcome::of(Some(self.status), self.conclusion);
        let suite = self.check_suite;
        let group = suite
            .as_ref()
            .and_then(|s| s.workflow_run.as_ref())
            .map(|w| w.workflow.name.clone())
            .or_else(|| suite.and_then(|s| s.app).map(|a| a.name))
            .unwrap_or_else(|| "Checks".into());
        CheckItem {
            name: self.name,
            group,
            outcome,
            started_at: self.started_at.map(|d| d.0),
            completed_at: self.completed_at.map(|d| d.0),
            summary: self.title.filter(|t| !t.trim().is_empty()),
            url: self.details_url.map(|u| u.0),
            annotations: (self.annotations)
                .map(|a| {
                    let a = a.into_capped();
                    let total = a.total();
                    Capped::new(crate::api::workflows::sorted(a.items), total)
                })
                .unwrap_or_default(),
        }
    }
}

impl WireStatusContext {
    fn into_item(self) -> CheckItem {
        let outcome = match ChecksState::from(self.state) {
            ChecksState::Passing => CheckOutcome::Success,
            ChecksState::Failing => CheckOutcome::Failure,
            ChecksState::Pending => CheckOutcome::Pending,
        };
        CheckItem {
            name: self.context,
            group: "Statuses".into(),
            outcome,
            started_at: Some(self.created_at.0),
            completed_at: None,
            summary: self.description.filter(|d| !d.trim().is_empty()),
            url: self.target_url.map(|u| u.0),
            annotations: Capped::default(),
        }
    }
}

// ---- actions --------------------------------------------------------------------------------

/// A workflow run and its jobs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRun {
    pub id: u64,
    /// The workflow's name.
    pub name: String,
    /// What it ran for: the commit's headline or the pull request's title.
    pub title: String,
    pub number: u64,
    pub attempt: u64,
    /// `push`, `pull_request`, `schedule`…
    pub event: String,
    pub branch: Option<String>,
    pub sha: String,
    pub outcome: CheckOutcome,
    pub actor: Option<String>,
    /// ISO 8601.
    pub started_at: Option<String>,
    pub updated_at: Option<String>,
    /// The workflow file, `.github/workflows/ci.yml`.
    pub path: String,
    pub jobs: Capped<JobSummary>,
    /// What it made to keep.
    #[serde(default)]
    pub artifacts: Capped<crate::api::workflows::Artifact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobSummary {
    pub id: u64,
    pub name: String,
    pub outcome: CheckOutcome,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

/// How many of a directory's entries get their latest commit, as on
/// GitHub (whose file list stops at 1,000).
pub const LAST_COMMIT_ENTRIES: usize = 1000;

/// A job's log. A long one is cut to its last 2 MB (where failures are);
/// the lines cut are kept as their timestamps, and the ones that start a
/// step whole, so the rest still have their steps and line numbers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JobLog {
    /// The lines cut, reduced as above.
    pub cut: String,
    pub text: String,
    /// The job hadn't finished, so there was no log to fetch yet.
    pub running: bool,
}

/// A job's log lines, as GitHub writes them.
pub mod log {
    /// Bytes of log kept.
    pub const KEEP: usize = 2 << 20;

    /// A log line without its timestamp, and when it was written (ISO 8601).
    pub fn split(line: &str) -> (Option<&str>, &str) {
        let line = line.trim_start_matches('\u{feff}');
        match line.split_once(' ') {
            Some((at, rest))
                if at.len() >= 20 && at.ends_with('Z') && at.as_bytes().get(4) == Some(&b'-') =>
            {
                (Some(at), rest)
            }
            _ => (None, line),
        }
    }

    /// Whether a line (without its timestamp) is one that starts a step:
    /// `##[group]Run …`, `Post job cleanup.`, `Cleaning up orphan processes`.
    pub fn starts_step(text: &str) -> bool {
        text.starts_with("##[group]Run ")
            || text == "Post job cleanup."
            || text == "Cleaning up orphan processes"
    }

    /// `log` cut to its last [`KEEP`] bytes, at a line's start: the lines
    /// cut, reduced to what places the rest, and the lines kept.
    pub fn cut(log: &str) -> (String, &str) {
        if log.len() <= KEEP {
            return (String::new(), log);
        }
        let from = log.len() - KEEP;
        let start = log
            .get(from..)
            .and_then(|tail| tail.find('\n'))
            .map_or(log.len(), |i| from + i + 1);
        let (head, kept) = log.split_at(start);
        let mut cut = String::new();
        for line in head.lines() {
            match split(line) {
                (Some(at), text) if starts_step(text) => {
                    cut.push_str(at);
                    cut.push(' ');
                    cut.push_str(text);
                }
                (Some(at), _) => cut.push_str(at),
                (None, _) => {}
            }
            cut.push('\n');
        }
        (cut, kept)
    }
}

/// A job: its steps (its log comes separately).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub id: u64,
    pub run_id: u64,
    pub name: String,
    pub outcome: CheckOutcome,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub steps: Vec<Step>,
    /// The attempt of its run it's in.
    pub attempt: u64,
    /// Its run, as it is now: a job can end while its run goes on.
    pub run: JobRun,
    /// What it says about lines of code, worst first.
    #[serde(default)]
    pub annotations: Capped<crate::api::workflows::Annotation>,
}

/// The run a job is in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobRun {
    /// The workflow's name.
    pub name: String,
    pub number: u64,
    pub outcome: CheckOutcome,
    /// Its latest attempt.
    pub attempt: u64,
    /// The job's ID in the latest attempt, once it's re-run there.
    pub rerun: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub number: u32,
    pub name: String,
    pub outcome: CheckOutcome,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

/// A workflow (its runs come separately, a page at a time).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workflow {
    pub name: String,
    pub path: String,
    /// Why it's disabled, in words ("disabled manually"); `None` while
    /// it's active.
    pub disabled: Option<String>,
}

/// A run in a list of runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunSummary {
    pub id: u64,
    pub title: String,
    /// Its workflow's name.
    pub workflow: String,
    pub number: u64,
    pub event: String,
    pub branch: Option<String>,
    pub outcome: CheckOutcome,
    pub actor: Option<String>,
    pub created_at: Option<String>,
    pub started_at: Option<String>,
    pub updated_at: Option<String>,
}

/// The statuses runs filter by (`is:failure`), as REST names them.
pub const RUN_STATUSES: [&str; 14] = [
    "completed",
    "action_required",
    "cancelled",
    "failure",
    "neutral",
    "skipped",
    "stale",
    "success",
    "timed_out",
    "in_progress",
    "queued",
    "requested",
    "waiting",
    "pending",
];

/// A filter of workflow runs in the words of GitHub's Actions tab
/// (`workflow:CI branch:main is:failure event:push actor:octocat`), as
/// REST's list of runs takes it: one of each kind.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RunFilter {
    /// A workflow, by its name, file name or ID.
    pub workflow: Option<String>,
    /// REST's parameters: `branch`, `status`, `event`, `actor`.
    pub params: Vec<(&'static str, String)>,
    /// Words runs can't be filtered by.
    pub ignored: Vec<String>,
}

impl RunFilter {
    /// Reads a filter; a later word of a kind replaces an earlier one.
    pub fn parse(query: &str) -> RunFilter {
        let mut filter = RunFilter::default();
        for word in filter_words(query) {
            let (key, value) = word.split_once(':').unwrap_or_default();
            let param = match key {
                _ if value.is_empty() => None,
                "workflow" => {
                    filter.workflow = Some(value.to_owned());
                    continue;
                }
                "is" if RUN_STATUSES.contains(&value) => Some("status"),
                "branch" => Some("branch"),
                "event" => Some("event"),
                "actor" => Some("actor"),
                _ => None,
            };
            match param {
                Some(param) => {
                    let value = value.to_owned();
                    filter.params.retain(|(p, _)| *p != param);
                    filter.params.push((param, value));
                }
                None => filter.ignored.push(word),
            }
        }
        filter
    }
}

/// A filter's words, a quoted part's spaces kept and its quotes dropped
/// (`workflow:"Build and test"`).
pub(crate) fn filter_words(query: &str) -> Vec<String> {
    let (mut words, mut word, mut quoted) = (Vec::new(), String::new(), false);
    for c in query.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => words.push(std::mem::take(&mut word)),
            c => word.push(c),
        }
    }
    words.push(word);
    words.retain(|w| !w.is_empty());
    words
}

/// The shapes GraphQL responses read as raw JSON share.
pub(crate) mod wire {
    use serde::Deserialize;

    use super::Results;

    #[derive(Deserialize)]
    pub struct Name {
        pub name: String,
    }

    /// A REST commit's git author or committer.
    #[derive(Deserialize)]
    pub struct Signature {
        pub name: Option<String>,
        pub date: Option<String>,
    }

    #[derive(Deserialize)]
    pub struct Message {
        pub message: String,
        pub author: Option<Signature>,
        pub committer: Option<Signature>,
    }

    /// A commit as REST lists them (compare, search).
    #[derive(Deserialize)]
    pub struct RestCommit {
        pub sha: String,
        pub commit: Message,
        pub author: Option<super::UserLogin>,
    }

    /// Which signature dates a REST commit.
    #[derive(Clone, Copy)]
    pub enum Dated {
        Authored,
        /// When committed, else when authored.
        Committed,
    }

    impl RestCommit {
        /// By GitHub's account for it, else its author's git name (never the
        /// committer's).
        pub fn into_info(self, dated: Dated) -> super::CommitInfo {
            let Message {
                message,
                author,
                committer,
            } = self.commit;
            let committed = match dated {
                Dated::Authored => None,
                Dated::Committed => committer.and_then(|c| c.date),
            };
            let (name, authored) = author.map_or((None, None), |a| (a.name, a.date));
            let author = super::GitActor {
                name,
                user: self.author,
            };
            super::CommitInfo {
                headline: message.lines().next().unwrap_or_default().to_owned(),
                author: Some(author).into(),
                date: committed.or(authored).unwrap_or_default(),
                oid: self.sha,
            }
        }
    }

    /// A commit as raw GraphQL lists them (blame, a path's last commit).
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Commit {
        pub oid: String,
        pub message_headline: String,
        pub committed_date: String,
        pub author: Option<super::GitActor>,
    }

    impl Commit {
        pub fn into_info(self) -> super::CommitInfo {
            super::CommitInfo {
                author: self.author.into(),
                oid: self.oid,
                headline: self.message_headline,
                date: self.committed_date,
            }
        }
    }

    /// Someone in a list of people (stargazers, followers, members).
    #[derive(Deserialize)]
    pub struct Person {
        pub login: String,
        pub name: Option<String>,
        pub bio: Option<String>,
    }

    impl Person {
        /// A blank name or bio is none.
        pub fn into_summary(self) -> super::UserSummary {
            let text = |s: Option<String>| s.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
            super::UserSummary {
                login: self.login,
                name: text(self.name),
                bio: text(self.bio),
                is_org: false,
            }
        }
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct RepoName {
        pub name_with_owner: String,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Count {
        pub total_count: u64,
    }

    /// A connection's nodes, without paging.
    #[derive(Deserialize)]
    pub struct Nodes<T> {
        pub nodes: Vec<Option<T>>,
    }

    /// The nodes that aren't null.
    impl<T> IntoIterator for Nodes<T> {
        type Item = T;
        type IntoIter = std::iter::Flatten<std::vec::IntoIter<Option<T>>>;

        fn into_iter(self) -> Self::IntoIter {
            self.nodes.into_iter().flatten()
        }
    }

    /// Some of a connection's nodes, and how many there are in all.
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Counted<T> {
        pub total_count: u64,
        pub nodes: Vec<Option<T>>,
    }

    impl<T> Counted<T> {
        /// The nodes that aren't null, each made an item by `item`.
        pub fn into_capped<U>(
            self,
            item: impl FnMut(T) -> Option<U>,
        ) -> crate::api::model::Capped<U> {
            let items = self.nodes.into_iter().flatten().filter_map(item).collect();
            crate::api::model::Capped::new(items, self.total_count)
        }
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct PageInfo {
        pub has_next_page: bool,
        pub end_cursor: Option<String>,
    }

    /// A page of a connection, and how many there are in all (a search's
    /// count of discussions included).
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Connection<T> {
        #[serde(alias = "discussionCount")]
        pub total_count: u64,
        pub page_info: PageInfo,
        pub nodes: Vec<Option<T>>,
    }

    impl<T> Connection<T> {
        /// The page as results, each node made an item by `item`.
        pub fn into_results<U>(self, mut item: impl FnMut(T) -> U) -> Results<U> {
            self.filter_results(|node| Some(item(node)))
        }

        /// The page as results, the nodes `item` makes nothing of left out.
        pub fn filter_results<U>(self, item: impl FnMut(T) -> Option<U>) -> Results<U> {
            let PageInfo {
                has_next_page,
                end_cursor,
            } = self.page_info;
            Results {
                total: self.total_count,
                items: self.nodes.into_iter().flatten().filter_map(item).collect(),
                next: end_cursor.filter(|_| has_next_page),
            }
        }
    }
}

pub(crate) mod rest_actions {
    use serde::de::IntoDeserializer;
    use serde::{Deserialize, Deserializer};

    pub use super::UserLogin as Actor;
    use super::{CheckConclusionState, CheckStatusState};

    /// REST spells GraphQL's enum values in lower case.
    fn upper<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<T>, D::Error> {
        Option::<String>::deserialize(d)?
            .map(|s| T::deserialize(s.to_uppercase().into_deserializer()))
            .transpose()
    }

    #[derive(Deserialize)]
    pub struct Run {
        pub id: u64,
        pub name: Option<String>,
        pub display_title: Option<String>,
        pub run_number: u64,
        pub run_attempt: Option<u64>,
        pub event: String,
        pub head_branch: Option<String>,
        pub head_sha: String,
        #[serde(default, deserialize_with = "upper")]
        pub status: Option<CheckStatusState>,
        #[serde(default, deserialize_with = "upper")]
        pub conclusion: Option<CheckConclusionState>,
        pub actor: Option<Actor>,
        pub run_started_at: Option<String>,
        pub created_at: Option<String>,
        pub updated_at: Option<String>,
        #[serde(default)]
        pub path: String,
    }

    #[derive(Deserialize)]
    pub struct Runs {
        pub total_count: u64,
        pub workflow_runs: Vec<Run>,
    }

    #[derive(Deserialize)]
    pub struct Step {
        pub number: u32,
        pub name: String,
        #[serde(default, deserialize_with = "upper")]
        pub status: Option<CheckStatusState>,
        #[serde(default, deserialize_with = "upper")]
        pub conclusion: Option<CheckConclusionState>,
        pub started_at: Option<String>,
        pub completed_at: Option<String>,
    }

    #[derive(Deserialize)]
    pub struct Job {
        pub id: u64,
        pub run_id: u64,
        pub name: String,
        #[serde(default, deserialize_with = "upper")]
        pub status: Option<CheckStatusState>,
        #[serde(default, deserialize_with = "upper")]
        pub conclusion: Option<CheckConclusionState>,
        pub started_at: Option<String>,
        pub completed_at: Option<String>,
        #[serde(default)]
        pub steps: Vec<Step>,
        pub run_attempt: Option<u64>,
    }

    #[derive(Deserialize)]
    pub struct Jobs {
        pub jobs: Vec<Job>,
        #[serde(default)]
        pub total_count: u64,
    }

    #[derive(Deserialize)]
    pub struct Workflow {
        pub id: u64,
        pub name: String,
        pub path: String,
        pub state: String,
    }

    #[derive(Deserialize)]
    pub struct Workflows {
        pub total_count: u64,
        pub workflows: Vec<Workflow>,
    }
}

impl rest_actions::Run {
    pub(crate) fn into_run(self, jobs: Capped<rest_actions::Job>) -> WorkflowRun {
        let outcome = CheckOutcome::of(self.status, self.conclusion);
        WorkflowRun {
            id: self.id,
            name: self.name.unwrap_or_else(|| "Workflow".into()),
            title: self.display_title.unwrap_or_default(),
            number: self.run_number,
            attempt: self.run_attempt.unwrap_or(1),
            event: self.event,
            branch: self.head_branch,
            sha: self.head_sha,
            outcome,
            actor: self.actor.map(|a| a.login),
            started_at: self.run_started_at.or(self.created_at),
            updated_at: self.updated_at,
            path: self.path,
            artifacts: Capped::default(),
            jobs: jobs.map(|j| JobSummary {
                outcome: CheckOutcome::of(j.status, j.conclusion),
                id: j.id,
                name: j.name,
                started_at: j.started_at,
                completed_at: j.completed_at,
            }),
        }
    }

    /// What a job says of its run, with the job's ID in its latest
    /// attempt if it was re-run there.
    pub(crate) fn into_job_run(self, rerun: Option<u64>) -> JobRun {
        JobRun {
            name: self.name.unwrap_or_else(|| "Workflow".into()),
            number: self.run_number,
            outcome: CheckOutcome::of(self.status, self.conclusion),
            attempt: self.run_attempt.unwrap_or(1),
            rerun,
        }
    }

    pub(crate) fn into_summary(self) -> RunSummary {
        let outcome = CheckOutcome::of(self.status, self.conclusion);
        RunSummary {
            id: self.id,
            title: self.display_title.unwrap_or_default(),
            workflow: self.name.unwrap_or_else(|| "Workflow".into()),
            number: self.run_number,
            event: self.event,
            branch: self.head_branch,
            outcome,
            actor: self.actor.map(|a| a.login),
            started_at: self.run_started_at.or_else(|| self.created_at.clone()),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

impl rest_actions::Job {
    pub(crate) fn outcome(&self) -> CheckOutcome {
        CheckOutcome::of(self.status, self.conclusion)
    }

    pub(crate) fn into_job(self, run: JobRun) -> Job {
        Job {
            outcome: self.outcome(),
            attempt: self.run_attempt.unwrap_or(1),
            run,
            annotations: Capped::default(),
            id: self.id,
            run_id: self.run_id,
            name: self.name,
            started_at: self.started_at,
            completed_at: self.completed_at,
            steps: self
                .steps
                .into_iter()
                .map(|s| Step {
                    outcome: CheckOutcome::of(s.status, s.conclusion),
                    number: s.number,
                    name: s.name,
                    started_at: s.started_at,
                    completed_at: s.completed_at,
                })
                .collect(),
        }
    }
}

// ---- branches ---------------------------------------------------------------------------------

/// A branch, and its latest commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchInfo {
    pub name: String,
    pub default: bool,
    pub oid: Option<String>,
    pub headline: Option<String>,
    /// `None` when the branch points at no commit, or its commit names nobody.
    pub author: Option<Person>,
    /// ISO 8601: when its latest commit was committed.
    pub date: Option<String>,
    /// Its most recent pull request: number and state.
    pub pr: Option<(u64, PrState)>,
}

pub(crate) mod wire_branches {
    use serde::Deserialize;

    pub use super::wire::{Nodes, RepoName};
    use super::{GitActor, PrStatus};

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Commit {
        pub oid: Option<String>,
        pub message_headline: Option<String>,
        pub committed_date: Option<String>,
        pub author: Option<GitActor>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Pr {
        pub number: u64,
        #[serde(flatten)]
        pub status: PrStatus,
        pub head_ref_name: String,
        pub repository: RepoName,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Branch {
        pub name: String,
        pub target: Option<Commit>,
        pub associated_pull_requests: Nodes<Pr>,
    }
}

impl wire_branches::Branch {
    /// The branch, marked the default if it's `default`, with its newest
    /// pull request in `repo` (GitHub's list also has forks' PRs whose
    /// head is this branch).
    pub(crate) fn into_info(self, repo: &RepoId, default: Option<&str>) -> BranchInfo {
        let (oid, headline, date, author) = match self.target {
            Some(c) => (
                c.oid,
                c.message_headline,
                c.committed_date,
                Some(Person::from(c.author)).filter(|a| *a != Person::Unknown),
            ),
            None => (None, None, None, None),
        };
        let (name, full) = (self.name.as_str(), repo.to_string());
        let ours = |p: &wire_branches::Pr| {
            p.head_ref_name == name && p.repository.name_with_owner.eq_ignore_ascii_case(&full)
        };
        let pr = self
            .associated_pull_requests
            .into_iter()
            .find(ours)
            .map(|p| (p.number, p.status.into()));
        BranchInfo {
            default: default == Some(self.name.as_str()),
            name: self.name,
            oid,
            headline,
            author,
            date,
            pr,
        }
    }
}

// ---- milestones -------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MilestoneInfo {
    pub number: u64,
    pub title: String,
    pub description: String,
    /// ISO 8601.
    pub due_on: Option<String>,
    pub closed: bool,
    pub closed_at: Option<String>,
    pub updated_at: String,
    /// Its issues and pull requests, open and done.
    pub open: u64,
    pub done: u64,
}

/// A page of milestones, and how many are open and closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MilestoneList {
    pub open: u64,
    pub closed: u64,
    pub results: Results<MilestoneInfo>,
}

/// A milestone and a page of its issues and pull requests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MilestoneDetail {
    pub info: MilestoneInfo,
    pub items: Results<IssueSummary>,
    /// Its title has quotes, which GitHub's search can't match, so its
    /// issues and pull requests weren't looked up.
    pub unsearchable: bool,
}

pub(crate) mod wire_milestones {
    use serde::Deserialize;

    pub use super::wire::Count;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Milestone {
        pub number: u64,
        pub title: String,
        pub description: Option<String>,
        pub due_on: Option<String>,
        pub closed: bool,
        pub closed_at: Option<String>,
        pub updated_at: String,
        pub open_issues: Count,
        pub done_issues: Count,
        pub open_prs: Count,
        pub done_prs: Count,
    }
}

impl wire_milestones::Milestone {
    pub(crate) fn into_info(self) -> MilestoneInfo {
        MilestoneInfo {
            number: self.number,
            title: self.title,
            description: self.description.unwrap_or_default(),
            due_on: self.due_on,
            closed: self.closed,
            closed_at: self.closed_at,
            updated_at: self.updated_at,
            open: self.open_issues.total_count + self.open_prs.total_count,
            done: self.done_issues.total_count + self.done_prs.total_count,
        }
    }
}

// ---- deployments ------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentInfo {
    pub environment: String,
    pub outcome: CheckOutcome,
    /// GitHub's word for its state: `active`, `failure`, `inactive`…
    pub state: String,
    pub created_at: String,
    pub creator: Option<String>,
    pub branch: Option<String>,
    pub oid: String,
    /// Where its log is (usually a workflow run's job).
    pub log_url: Option<String>,
    /// What it deployed to.
    pub environment_url: Option<String>,
}

/// A page of deployments, and the environments to filter by.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentList {
    pub environments: Capped<String>,
    pub results: Results<DeploymentInfo>,
}

pub(crate) mod wire_deployments {
    use serde::Deserialize;

    pub use super::{UserLogin, wire::Name};

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Status {
        pub log_url: Option<String>,
        pub environment_url: Option<String>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Deployment {
        pub environment: Option<String>,
        pub state: Option<String>,
        pub created_at: String,
        pub creator: Option<UserLogin>,
        #[serde(rename = "ref")]
        pub branch: Option<Name>,
        pub commit_oid: String,
        pub latest_status: Option<Status>,
    }
}

impl wire_deployments::Deployment {
    pub(crate) fn into_info(self) -> DeploymentInfo {
        let state = self.state.unwrap_or_default().to_lowercase();
        let outcome = match state.as_str() {
            "active" | "success" => CheckOutcome::Success,
            "failure" | "error" => CheckOutcome::Failure,
            "inactive" | "destroyed" => CheckOutcome::Neutral,
            "abandoned" => CheckOutcome::Cancelled,
            _ => CheckOutcome::Pending,
        };
        let (log_url, environment_url) = self
            .latest_status
            .map_or((None, None), |s| (s.log_url, s.environment_url));
        DeploymentInfo {
            environment: self.environment.unwrap_or_default(),
            outcome,
            state: state.replace('_', " "),
            created_at: self.created_at,
            creator: self.creator.map(|c| c.login),
            branch: self.branch.map(|r| r.name),
            oid: self.commit_oid,
            log_url,
            environment_url,
        }
    }
}
// ---- comparisons ------------------------------------------------------------------------------

/// Two revisions compared: the commits from one to the other, and what
/// the diff between them is from and to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comparison {
    pub ahead: u64,
    pub behind: u64,
    /// Oldest first.
    pub commits: Capped<CommitInfo>,
    /// The diff's ends: the merge base (or, for `a..b`, the base) and the
    /// head, as full commit IDs.
    pub from: String,
    pub to: String,
    pub files: u64,
    /// GitHub lists at most 300 files, so the counts may be short.
    pub files_capped: bool,
    pub additions: u64,
    pub deletions: u64,
}

/// How many files GitHub lists in a comparison at most.
const COMPARE_FILES: usize = 300;

/// How many commits GitHub lists in a comparison at most (the newest).
pub const COMPARE_COMMITS: u64 = 250;

pub(crate) mod rest_compare {
    use serde::Deserialize;

    pub use super::wire::RestCommit as Commit;

    #[derive(Deserialize)]
    pub struct Sha {
        pub sha: String,
    }

    #[derive(Deserialize)]
    pub struct File {
        #[serde(default)]
        pub additions: u64,
        #[serde(default)]
        pub deletions: u64,
    }

    #[derive(Deserialize)]
    pub struct Compare {
        /// Names the comparison's ends by short SHA:
        /// `…/compare/owner:bcf4368...owner:efd1e47`.
        #[serde(default)]
        pub permalink_url: String,
        pub ahead_by: u64,
        pub behind_by: u64,
        pub total_commits: u64,
        pub base_commit: Sha,
        pub merge_base_commit: Sha,
        pub commits: Vec<Commit>,
        #[serde(default)]
        pub files: Vec<File>,
    }
}

impl rest_compare::Compare {
    /// `direct`: `a..b`, the diff from the base itself, not the merge base.
    pub(crate) fn into_comparison(self, direct: bool) -> Comparison {
        // The newest commit listed is the head; with none, the head is
        // behind or at the base, so it's the merge base.
        let to = self
            .commits
            .last()
            .map_or_else(|| self.merge_base_commit.sha.clone(), |c| c.sha.clone());
        let from = if direct {
            self.base_commit.sha
        } else {
            self.merge_base_commit.sha
        };
        Comparison {
            ahead: self.ahead_by,
            behind: self.behind_by,
            to,
            from,
            files: self.files.len() as u64,
            files_capped: self.files.len() >= COMPARE_FILES,
            additions: self.files.iter().map(|f| f.additions).sum(),
            deletions: self.files.iter().map(|f| f.deletions).sum(),
            commits: Capped::new(
                self.commits
                    .into_iter()
                    .map(|c| c.into_info(wire::Dated::Authored))
                    .collect(),
                self.total_commits,
            ),
        }
    }
}
// ---- blame ------------------------------------------------------------------------------------

/// Which commit last changed each line of a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Blame {
    pub ranges: Vec<BlameRange>,
}

/// Lines `start..=end` (from 1), last changed by one commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlameRange {
    pub start: u32,
    pub end: u32,
    /// How recent, 1 (newest) to 10 (oldest).
    pub age: u8,
    pub commit: CommitInfo,
}

pub(crate) mod wire_blame {
    use serde::Deserialize;

    use super::wire::Commit;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Range {
        pub starting_line: u32,
        pub ending_line: u32,
        pub age: u8,
        pub commit: Commit,
    }

    #[derive(Deserialize)]
    pub struct Blame {
        pub ranges: Vec<Range>,
    }
}

impl wire_blame::Blame {
    pub(crate) fn into_blame(self) -> Blame {
        Blame {
            ranges: self
                .ranges
                .into_iter()
                .map(|r| BlameRange {
                    start: r.starting_line,
                    end: r.ending_line,
                    age: r.age,
                    commit: r.commit.into_info(),
                })
                .collect(),
        }
    }
}
// ---- gists -------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GistFile {
    pub name: String,
    pub language: Option<String>,
    pub size: u64,
    /// `None` past the API's size limit.
    pub text: Option<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gist {
    pub id: String,
    pub owner: Option<String>,
    pub description: String,
    pub public: bool,
    pub created_at: String,
    pub updated_at: String,
    pub comments: u64,
    pub files: Vec<GistFile>,
}

/// A gist in a list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GistSummary {
    pub id: String,
    pub description: String,
    pub files: Vec<String>,
    pub updated_at: String,
    pub stars: u64,
    pub comments: u64,
}

pub(crate) mod rest_gists {
    use std::collections::BTreeMap;

    use serde::Deserialize;

    pub use super::UserLogin;

    #[derive(Deserialize)]
    pub struct File {
        pub language: Option<String>,
        #[serde(default)]
        pub size: u64,
        pub content: Option<String>,
        #[serde(default)]
        pub truncated: bool,
    }

    #[derive(Deserialize)]
    pub struct Gist {
        pub id: String,
        pub owner: Option<UserLogin>,
        pub description: Option<String>,
        pub public: bool,
        pub created_at: String,
        pub updated_at: String,
        #[serde(default)]
        pub comments: u64,
        pub files: BTreeMap<String, File>,
    }
}

impl rest_gists::Gist {
    pub(crate) fn into_gist(self) -> Gist {
        Gist {
            id: self.id,
            owner: self.owner.map(|o| o.login),
            description: self.description.unwrap_or_default(),
            public: self.public,
            created_at: self.created_at,
            updated_at: self.updated_at,
            comments: self.comments,
            files: self
                .files
                .into_iter()
                .map(|(name, f)| GistFile {
                    name,
                    language: f.language,
                    size: f.size,
                    text: f.content,
                    truncated: f.truncated,
                })
                .collect(),
        }
    }
}

pub(crate) mod wire_gists {
    use serde::Deserialize;

    pub use super::wire::Count;

    #[derive(Deserialize)]
    pub struct Name {
        pub name: Option<String>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Gist {
        pub name: String,
        pub description: Option<String>,
        pub updated_at: String,
        pub stargazer_count: u64,
        pub files: Option<Vec<Option<Name>>>,
        pub comments: Count,
    }
}

impl wire_gists::Gist {
    pub(crate) fn into_summary(self) -> GistSummary {
        GistSummary {
            id: self.name,
            description: self.description.unwrap_or_default(),
            files: self
                .files
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|f| f.name)
                .collect(),
            updated_at: self.updated_at,
            stars: self.stargazer_count,
            comments: self.comments.total_count,
        }
    }
}
// ---- teams -------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamSummary {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub secret: bool,
    pub members: u64,
    pub repos: u64,
}

/// A repository a team has access to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamRepo {
    pub repo: RepoId,
    pub description: String,
    pub stars: u64,
}

/// A team: its members, repositories and child teams (the first 50 of
/// each).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamDetail {
    pub team: TeamSummary,
    pub parent: Option<TeamSummary>,
    pub members: Capped<UserSummary>,
    pub repos: Capped<TeamRepo>,
    pub children: Capped<TeamSummary>,
}

pub(crate) mod wire_teams {
    use serde::Deserialize;

    pub use super::wire::{Count, Counted};

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Team {
        pub slug: String,
        pub name: String,
        pub description: Option<String>,
        pub privacy: Option<String>,
        pub members: Option<Count>,
        pub repositories: Option<Count>,
    }

    #[derive(Deserialize)]
    pub struct Member {
        pub login: String,
        pub name: Option<String>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Repo {
        pub name_with_owner: String,
        pub description: Option<String>,
        pub stargazer_count: u64,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Detail {
        #[serde(flatten)]
        pub team: Team,
        pub parent_team: Option<Team>,
        #[serde(rename = "memberList")]
        pub member_list: Counted<Member>,
        #[serde(rename = "repoList")]
        pub repo_list: Counted<Repo>,
        pub child_teams: Counted<Team>,
    }
}

impl wire_teams::Team {
    pub(crate) fn into_summary(self) -> TeamSummary {
        TeamSummary {
            slug: self.slug,
            name: self.name,
            description: self.description.unwrap_or_default(),
            secret: self.privacy.as_deref() == Some("SECRET"),
            members: self.members.map_or(0, |c| c.total_count),
            repos: self.repositories.map_or(0, |c| c.total_count),
        }
    }
}

impl wire_teams::Detail {
    pub(crate) fn into_detail(self) -> TeamDetail {
        TeamDetail {
            team: self.team.into_summary(),
            parent: self.parent_team.map(wire_teams::Team::into_summary),
            members: self.member_list.into_capped(|m| {
                Some(UserSummary {
                    login: m.login,
                    name: m.name,
                    bio: None,
                    is_org: false,
                })
            }),
            repos: self.repo_list.into_capped(|r| {
                Some(TeamRepo {
                    repo: RepoId::parse(&r.name_with_owner)?,
                    description: r.description.unwrap_or_default(),
                    stars: r.stargazer_count,
                })
            }),
            children: self.child_teams.into_capped(|t| Some(t.into_summary())),
        }
    }
}
// ---- security advisories ----------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvisoryPackage {
    pub ecosystem: String,
    pub name: String,
    pub vulnerable: Option<String>,
    pub patched: Option<String>,
}

#[derive(Debug, Clone, Default, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
    #[serde(other)]
    #[default]
    Unknown,
}

/// A security advisory: GitHub's reviewed database's, or a repository's.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Advisory {
    pub ghsa: String,
    pub cve: Option<String>,
    pub summary: String,
    pub description: String,
    pub severity: Severity,
    pub published_at: Option<String>,
    pub updated_at: Option<String>,
    pub withdrawn_at: Option<String>,
    /// The CVSS score (as GitHub writes it) and vector.
    pub cvss: Option<(String, String)>,
    /// `CWE-79 Cross-site Scripting`…
    pub cwes: Vec<String>,
    pub packages: Vec<AdvisoryPackage>,
    pub credits: Vec<String>,
    pub references: Vec<String>,
}

pub(crate) mod rest_advisories {
    use serde::Deserialize;

    pub use super::UserLogin;

    #[derive(Deserialize)]
    pub struct Package {
        pub ecosystem: Option<String>,
        pub name: Option<String>,
    }

    #[derive(Deserialize)]
    pub struct Vulnerability {
        pub package: Option<Package>,
        pub vulnerable_version_range: Option<String>,
        #[serde(alias = "first_patched_version")]
        pub patched_versions: Option<String>,
    }

    #[derive(Deserialize)]
    pub struct Cvss {
        pub score: Option<f64>,
        pub vector_string: Option<String>,
    }

    #[derive(Deserialize)]
    pub struct Cwe {
        pub cwe_id: String,
        pub name: String,
    }

    #[derive(Deserialize)]
    pub struct Credit {
        pub login: Option<String>,
        pub user: Option<UserLogin>,
    }

    #[derive(Deserialize)]
    pub struct Advisory {
        pub ghsa_id: String,
        pub cve_id: Option<String>,
        pub summary: String,
        pub description: Option<String>,
        pub severity: Option<super::Severity>,
        pub published_at: Option<String>,
        pub updated_at: Option<String>,
        pub withdrawn_at: Option<String>,
        pub cvss: Option<Cvss>,
        #[serde(default)]
        pub cwes: Option<Vec<Cwe>>,
        #[serde(default)]
        pub vulnerabilities: Option<Vec<Vulnerability>>,
        #[serde(default)]
        pub credits: Option<Vec<Credit>>,
        #[serde(default)]
        pub references: Option<Vec<String>>,
    }
}

impl rest_advisories::Advisory {
    pub(crate) fn into_advisory(self) -> Advisory {
        Advisory {
            ghsa: self.ghsa_id,
            cve: self.cve_id,
            summary: self.summary,
            description: self.description.unwrap_or_default(),
            severity: self.severity.unwrap_or(Severity::Unknown),
            published_at: self.published_at,
            updated_at: self.updated_at,
            withdrawn_at: self.withdrawn_at,
            cvss: self
                .cvss
                .and_then(|c| Some((format!("{:.1}", c.score?), c.vector_string?))),
            cwes: self
                .cwes
                .into_iter()
                .flatten()
                .map(|c| format!("{} {}", c.cwe_id, c.name))
                .collect(),
            packages: self
                .vulnerabilities
                .into_iter()
                .flatten()
                .map(|v| {
                    let package = v.package;
                    AdvisoryPackage {
                        ecosystem: package
                            .as_ref()
                            .and_then(|p| p.ecosystem.clone())
                            .unwrap_or_default(),
                        name: package.and_then(|p| p.name).unwrap_or_default(),
                        vulnerable: v.vulnerable_version_range,
                        patched: v.patched_versions,
                    }
                })
                .collect(),
            credits: self
                .credits
                .into_iter()
                .flatten()
                .filter_map(|c| c.login.or(c.user.map(|u| u.login)))
                .collect(),
            references: self.references.unwrap_or_default(),
        }
    }
}
// ---- wikis -------------------------------------------------------------------------------------

/// A wiki page (or, without one, the wiki's list of pages), read from
/// the wiki's git repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WikiPage {
    /// Its title (the file's name, without its extension).
    pub title: Option<String>,
    pub text: Option<String>,
    /// Whether the text is Markdown (wikis may use other markups).
    pub markdown: bool,
    /// Every page's title, in order.
    pub pages: Vec<String>,
    pub sidebar: Option<String>,
    /// The commit it was read at, and the page's file there: what an
    /// edit is made on.
    pub commit: Option<String>,
    pub file: Option<String>,
}
// ---- search: discussions, commits, code -------------------------------------------------------

pub(crate) mod rest_search {
    use serde::Deserialize;

    use super::wire::RestCommit;

    #[derive(Deserialize)]
    pub struct Repo {
        pub full_name: String,
    }

    #[derive(Deserialize)]
    pub struct Commit {
        pub repository: Repo,
        #[serde(flatten)]
        pub commit: RestCommit,
    }

    #[derive(Deserialize)]
    pub struct Code {
        pub path: String,
        pub sha: String,
        pub repository: Repo,
    }

    #[derive(Deserialize)]
    pub struct Page<T> {
        pub total_count: u64,
        /// The search timed out and found only some of them.
        #[serde(default)]
        pub incomplete_results: bool,
        pub items: Vec<T>,
    }
}

impl rest_search::Commit {
    pub(crate) fn into_hit(self) -> Option<CommitHit> {
        Some(CommitHit {
            repo: RepoId::parse(&self.repository.full_name)?,
            commit: self.commit.into_info(wire::Dated::Committed),
        })
    }
}

impl rest_search::Code {
    pub(crate) fn into_hit(self) -> Option<CodeHit> {
        Some(CodeHit {
            repo: RepoId::parse(&self.repository.full_name)?,
            path: self.path,
            sha: self.sha,
            lines: Vec::new(),
            rev: None,
        })
    }
}
// ---- discussions ------------------------------------------------------------------------------

/// Whose discussions: a repository's, or an organization's (which GitHub
/// keeps in one of its repositories).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DiscussionsOf {
    Repo(RepoId),
    Org(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscussionCategory {
    pub name: String,
    pub slug: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscussionSummary {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub category: String,
    pub comments: u64,
    pub answered: bool,
    pub upvotes: u64,
    /// ISO 8601.
    pub updated_at: String,
}

/// A page of discussions, and the categories to filter by.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscussionList {
    pub categories: Capped<DiscussionCategory>,
    pub results: Results<DiscussionSummary>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscussionDetail {
    /// What comments and edits name it by.
    pub id: NodeId,
    /// The repository it's in.
    pub repo: RepoId,
    pub number: u64,
    pub title: String,
    pub body: String,
    pub author: String,
    pub created_at: String,
    pub category: String,
    /// Its category takes answers (Q&A): its comments are answers, and one
    /// can be marked the answer.
    pub answerable: bool,
    pub answered: bool,
    /// Only those with write access can comment.
    pub locked: Option<crate::api::manage::Locked>,
    /// Closed, and why (`None` inside: GitHub didn't say).
    pub closed: Option<Option<crate::api::manage::DiscussionReason>>,
    pub can_close: bool,
    pub can_reopen: bool,
    /// Lock or unlock it: triage access.
    pub can_lock: bool,
    pub upvotes: u64,
    /// The post: what GitHub lets you do to it, and its reactions.
    pub editable: Editable,
    /// The comment marked as the answer (shown at the top), and who
    /// marked it.
    pub answer: Option<Box<DiscussionComment>>,
    pub answer_by: Option<String>,
    /// The first 50 comments, of how many there are.
    pub comments: Capped<DiscussionComment>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscussionComment {
    /// Its editable is always there: what replies, reactions and marking
    /// the answer name it by.
    pub comment: Comment,
    pub upvotes: u64,
    /// The answer to a question.
    pub answer: bool,
    /// GitHub lets you mark it the answer, or unmark it.
    pub can_mark: bool,
    pub can_unmark: bool,
    /// The first 30 replies, of how many there are.
    pub replies: Capped<Comment>,
}

pub(crate) mod wire_discussions {
    use serde::Deserialize;

    pub use super::wire::{Count, Counted, Name, RepoName};
    pub use crate::api::queries::Actor;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Summary {
        pub number: u64,
        pub title: String,
        pub author: Option<Actor>,
        pub category: Name,
        pub comments: Count,
        pub is_answered: Option<bool>,
        pub upvote_count: u64,
        pub updated_at: String,
    }

    #[derive(Deserialize)]
    pub struct Category {
        pub id: String,
        pub name: String,
        pub slug: String,
    }

    /// A reply, or what a comment and a reply share.
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Reply {
        pub id: crate::api::model::NodeId,
        pub database_id: Option<u64>,
        pub author: Option<Actor>,
        pub body: String,
        pub created_at: String,
        pub viewer_can_update: bool,
        pub viewer_can_delete: bool,
        pub viewer_can_react: bool,
        pub reaction_groups: Option<Vec<crate::api::edit::WireReactionGroup>>,
        pub is_minimized: bool,
        pub minimized_reason: Option<String>,
        pub viewer_can_minimize: bool,
        pub viewer_can_unminimize: bool,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct WireComment {
        #[serde(flatten)]
        pub said: Reply,
        pub is_answer: bool,
        pub upvote_count: u64,
        pub viewer_can_mark_as_answer: bool,
        pub viewer_can_unmark_as_answer: bool,
        pub replies: Option<Counted<Reply>>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct InCategory {
        pub name: String,
        pub is_answerable: bool,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Detail {
        pub id: crate::api::model::NodeId,
        pub number: u64,
        pub title: String,
        pub body: String,
        pub author: Option<Actor>,
        pub created_at: String,
        pub category: InCategory,
        pub is_answered: Option<bool>,
        pub locked: bool,
        pub active_lock_reason: Option<crate::api::timeline::LockReason>,
        pub closed: bool,
        pub state_reason: Option<String>,
        pub viewer_can_close: bool,
        pub viewer_can_reopen: bool,
        pub viewer_can_label: bool,
        pub upvote_count: u64,
        pub viewer_can_update: bool,
        pub viewer_can_react: bool,
        pub reaction_groups: Option<Vec<crate::api::edit::WireReactionGroup>>,
        pub answer: Option<WireComment>,
        pub answer_chosen_by: Option<Actor>,
        pub comments: Counted<WireComment>,
    }

    /// A discussion a search found, and where.
    #[derive(Deserialize)]
    pub struct Hit {
        pub repository: RepoName,
        #[serde(flatten)]
        pub summary: Summary,
    }

    /// A discussion a search found, by its link (none for another kind).
    #[derive(Deserialize)]
    pub struct Found {
        #[serde(default)]
        pub url: String,
        pub repository: Option<RepoName>,
    }
}

impl wire_discussions::Hit {
    pub(crate) fn into_hit(self) -> Option<DiscussionHit> {
        Some(DiscussionHit {
            repo: RepoId::parse(&self.repository.name_with_owner)?,
            summary: self.summary.into_summary(),
        })
    }
}

impl wire_discussions::Summary {
    pub(crate) fn into_summary(self) -> DiscussionSummary {
        DiscussionSummary {
            number: self.number,
            title: self.title,
            author: author(self.author),
            category: self.category.name,
            comments: self.comments.total_count,
            answered: self.is_answered.unwrap_or(false),
            upvotes: self.upvote_count,
            updated_at: self.updated_at,
        }
    }
}

impl wire_discussions::Detail {
    pub(crate) fn into_detail(self, repo: RepoId) -> DiscussionDetail {
        let said = |r: wire_discussions::Reply| Comment {
            id: r.database_id,
            author: author(r.author),
            body: r.body,
            created_at: r.created_at,
            editable: Some(Editable {
                id: r.id,
                can_update: r.viewer_can_update,
                can_delete: r.viewer_can_delete,
                can_react: r.viewer_can_react,
                reactions: reactions(r.reaction_groups),
            }),
            hiding: crate::api::manage::Hiding {
                reason: r
                    .is_minimized
                    .then(|| r.minimized_reason.unwrap_or_default()),
                can_hide: r.viewer_can_minimize,
                can_unhide: r.viewer_can_unminimize,
            },
        };
        let comment = |c: wire_discussions::WireComment| DiscussionComment {
            upvotes: c.upvote_count,
            answer: c.is_answer,
            can_mark: c.viewer_can_mark_as_answer,
            can_unmark: c.viewer_can_unmark_as_answer,
            replies: (c.replies).map_or_else(Capped::default, |r| r.into_capped(|r| Some(said(r)))),
            comment: said(c.said),
        };
        DiscussionDetail {
            editable: Editable {
                id: self.id.clone(),
                can_update: self.viewer_can_update,
                can_delete: false,
                can_react: self.viewer_can_react,
                reactions: reactions(self.reaction_groups),
            },
            id: self.id,
            repo,
            number: self.number,
            title: self.title,
            body: self.body,
            author: author(self.author),
            created_at: self.created_at,
            category: self.category.name,
            answerable: self.category.is_answerable,
            answered: self.is_answered.unwrap_or(false),
            locked: (self.locked).then_some(crate::api::manage::Locked {
                reason: self.active_lock_reason,
            }),
            closed: (self.closed).then(|| {
                (self.state_reason.as_deref())
                    .and_then(crate::api::manage::DiscussionReason::from_wire)
            }),
            can_close: self.viewer_can_close,
            can_reopen: self.viewer_can_reopen,
            can_lock: self.viewer_can_label,
            upvotes: self.upvote_count,
            answer: self.answer.map(|a| Box::new(comment(a))),
            answer_by: self.answer_chosen_by.map(|a| a.login),
            comments: self.comments.into_capped(|c| Some(comment(c))),
        }
    }
}

// ---- profiles ------------------------------------------------------------------------

#[derive(cynic::Scalar, Debug, Clone)]
pub struct Date(pub String);

/// A profile README: `<login>/<login>`'s for a user, `.github`'s
/// `profile/README.md` for an organization.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Repository")]
pub struct UserReadmeRepo {
    #[arguments(expression: "HEAD:README.md")]
    pub object: Option<ReadmeObject>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Repository")]
pub struct OrgReadmeRepo {
    #[arguments(expression: "HEAD:profile/README.md")]
    pub object: Option<ReadmeObject>,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "GitObject")]
pub enum ReadmeObject {
    Blob(ReadmeBlob),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Blob")]
pub struct ReadmeBlob {
    pub text: Option<String>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "UserStatus")]
pub struct WireStatus {
    pub emoji: Option<String>,
    pub message: Option<String>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "SocialAccountConnection")]
pub struct SocialList {
    pub nodes: Option<Vec<Option<WireSocial>>>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "SocialAccount")]
pub struct WireSocial {
    pub display_name: String,
    pub url: Uri,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "ContributionsCollection")]
pub struct WireContributions {
    pub contribution_calendar: WireCalendar,
    /// The 10 repositories with the most, and how many there are.
    #[arguments(maxRepositories: 10)]
    pub commit_contributions_by_repository: Vec<WireRepoCommits>,
    pub total_repositories_with_contributed_commits: i32,
    // These lists are newest first, so `first` is the newest.
    #[arguments(first: 20)]
    pub pull_request_contributions: PrContributions,
    #[arguments(first: 20)]
    pub issue_contributions: IssueContributions,
    #[arguments(first: 20)]
    pub pull_request_review_contributions: ReviewContributions,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "ContributionCalendar")]
pub struct WireCalendar {
    pub total_contributions: i32,
    pub weeks: Vec<WireWeek>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "ContributionCalendarWeek")]
pub struct WireWeek {
    pub contribution_days: Vec<WireDay>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "ContributionCalendarDay")]
pub struct WireDay {
    pub date: Date,
    pub contribution_level: ContributionLevel,
}

#[derive(cynic::Enum, Debug, Clone, Copy)]
pub enum ContributionLevel {
    None,
    FirstQuartile,
    SecondQuartile,
    ThirdQuartile,
    FourthQuartile,
}

// A profile's contributions of each kind, and how many there are.
fragments! {
    counted:
    CommitContributions = "CreatedCommitContributionConnection" => WireCommitContribution,
    PrContributions = "CreatedPullRequestContributionConnection" => WirePrContribution,
    IssueContributions = "CreatedIssueContributionConnection" => WireIssueContribution,
    ReviewContributions = "CreatedPullRequestReviewContributionConnection" => WireReviewContribution,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "CommitContributionsByRepository")]
pub struct WireRepoCommits {
    pub repository: RepositoryName,
    #[arguments(first: 100)]
    pub contributions: CommitContributions,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "CreatedCommitContribution")]
pub struct WireCommitContribution {
    pub occurred_at: DateTime,
    pub commit_count: i32,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "CreatedPullRequestContribution")]
pub struct WirePrContribution {
    pub occurred_at: DateTime,
    pub pull_request: ContributedPr,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "CreatedIssueContribution")]
pub struct WireIssueContribution {
    pub occurred_at: DateTime,
    pub issue: ContributedIssue,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "CreatedPullRequestReviewContribution")]
pub struct WireReviewContribution {
    pub occurred_at: DateTime,
    pub pull_request: ContributedPr,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequest")]
pub struct ContributedPr {
    pub number: i32,
    pub title: String,
    pub repository: RepositoryName,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Issue")]
pub struct ContributedIssue {
    pub number: i32,
    pub title: String,
    pub repository: RepositoryName,
}

/// How many months of activity a profile lists.
const ACTIVITY_MONTHS: usize = 3;

impl WireContributions {
    pub(crate) fn into_contributions(self) -> Contributions {
        let calendar = self.contribution_calendar;
        let weeks = calendar
            .weeks
            .into_iter()
            .filter_map(|w| {
                let start = w.contribution_days.first()?.date.0.clone();
                let days = w
                    .contribution_days
                    .iter()
                    .map(|d| match d.contribution_level {
                        ContributionLevel::None => 0,
                        ContributionLevel::FirstQuartile => 1,
                        ContributionLevel::SecondQuartile => 2,
                        ContributionLevel::ThirdQuartile => 3,
                        ContributionLevel::FourthQuartile => 4,
                    })
                    .collect();
                Some(Week { start, days })
            })
            .collect();
        let mut months: BTreeMap<String, MonthActivity> = BTreeMap::new();
        let mut short = Short::default();
        if usize::try_from(self.total_repositories_with_contributed_commits).unwrap_or_default()
            > self.commit_contributions_by_repository.len()
        {
            // Another repository's commits could be in any month.
            short.commits = Some(Short::EVERY_MONTH.to_owned());
        }
        for repo in self.commit_contributions_by_repository {
            let contributions: Vec<WireCommitContribution> =
                nodes(repo.contributions.nodes).collect();
            let at = contributions.iter().map(|c| &c.occurred_at);
            short.commits = short.commits.max(cut(repo.contributions.total_count, at));
            let name = repo.repository.name_with_owner;
            for c in contributions {
                let (m, n) = (month_of(&mut months, &c.occurred_at), count(c.commit_count));
                match m.commits.iter_mut().find(|(r, _)| *r == name) {
                    Some((_, total)) => *total += n,
                    None => m.commits.push((name.clone(), n)),
                }
            }
        }
        let item = |repo: RepositoryName, number: i32, title: String| Contributed {
            repo: repo.name_with_owner,
            number: count(number),
            title,
        };
        let pulls: Vec<_> = nodes(self.pull_request_contributions.nodes).collect();
        let issues: Vec<_> = nodes(self.issue_contributions.nodes).collect();
        let reviews: Vec<_> = nodes(self.pull_request_review_contributions.nodes).collect();
        let total = self.pull_request_contributions.total_count;
        short.pulls = cut(total, pulls.iter().map(|p| &p.occurred_at));
        let total = self.issue_contributions.total_count;
        short.issues = cut(total, issues.iter().map(|i| &i.occurred_at));
        let total = self.pull_request_review_contributions.total_count;
        short.reviews = cut(total, reviews.iter().map(|r| &r.occurred_at));
        for p in pulls {
            let pr = p.pull_request;
            let c = item(pr.repository, pr.number, pr.title);
            month_of(&mut months, &p.occurred_at).pulls.push(c);
        }
        for i in issues {
            let issue = i.issue;
            let c = item(issue.repository, issue.number, issue.title);
            month_of(&mut months, &i.occurred_at).issues.push(c);
        }
        for r in reviews {
            let pr = r.pull_request;
            let c = item(pr.repository, pr.number, pr.title);
            let m = month_of(&mut months, &r.occurred_at);
            if !m
                .reviews
                .iter()
                .any(|r| r.repo == c.repo && r.number == c.number)
            {
                m.reviews.push(c);
            }
        }
        let mut months: Vec<MonthActivity> =
            months.into_values().rev().take(ACTIVITY_MONTHS).collect();
        for m in &mut months {
            m.commits
                .sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        }
        Contributions {
            total: count(calendar.total_contributions),
            weeks,
            activity: months,
            short,
        }
    }
}

/// The `YYYY-MM` a contribution is filed under.
fn month(at: &DateTime) -> String {
    at.0.get(..7).unwrap_or_default().to_owned()
}

/// The month `at` is in, filed the first time it's asked for.
fn month_of<'m>(
    months: &'m mut BTreeMap<String, MonthActivity>,
    at: &DateTime,
) -> &'m mut MonthActivity {
    let key = month(at);
    months.entry(key.clone()).or_insert_with(|| MonthActivity {
        month: key,
        ..MonthActivity::default()
    })
}

/// A list cut short has all of its kind from its oldest month on; in that
/// month and before, there may be more: that month, if `total` says so.
fn cut<'a>(total: i32, at: impl Iterator<Item = &'a DateTime>) -> Option<String> {
    let at: Vec<&DateTime> = at.collect();
    let oldest = at.iter().min_by(|a, b| a.0.cmp(&b.0));
    oldest
        .filter(|_| usize::try_from(total).unwrap_or_default() > at.len())
        .map(|a| month(a))
}

#[derive(cynic::QueryVariables, Debug)]
pub struct ProfileVariables {
    pub login: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Query", variables = "ProfileVariables")]
pub struct ProfileQuery {
    #[arguments(login: $login)]
    pub user: Option<UserFull>,
    #[arguments(login: $login)]
    pub organization: Option<OrgFull>,
    #[cynic(rename = "repository", alias)]
    #[arguments(owner: $login, name: $login)]
    pub user_readme: Option<UserReadmeRepo>,
    #[cynic(rename = "repository", alias)]
    #[arguments(owner: $login, name: ".github")]
    pub org_readme: Option<OrgReadmeRepo>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "User")]
pub struct UserFull {
    pub login: String,
    pub name: Option<String>,
    pub bio: Option<String>,
    pub company: Option<String>,
    pub location: Option<String>,
    pub website_url: Option<Uri>,
    pub followers: FollowCount,
    pub following: FollowingCount,
    #[arguments(first: 6, types: [REPOSITORY])]
    pub pinned_items: Pinned,
    #[arguments(first: 30, orderBy: { field: PUSHED_AT, direction: DESC }, ownerAffiliations: [OWNER])]
    pub repositories: RepoList,
    pub starred_repositories: StarCount,
    pub status: Option<WireStatus>,
    pub pronouns: Option<String>,
    #[arguments(first: 10)]
    pub social_accounts: SocialList,
    #[arguments(first: 20)]
    pub organizations: OrgLogins,
    pub contributions_collection: WireContributions,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Organization")]
pub struct OrgFull {
    pub login: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub website_url: Option<Uri>,
    #[arguments(first: 6, types: [REPOSITORY])]
    pub pinned_items: Pinned,
    #[arguments(first: 30, orderBy: { field: PUSHED_AT, direction: DESC })]
    pub repositories: RepoList,
    pub is_verified: bool,
    #[arguments(first: 12)]
    pub members_with_role: MemberList,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "PinnableItem")]
pub enum PinnedItem {
    Repository(RepoCard),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct ListVariables {
    pub owner: String,
    pub name: String,
    pub after: Option<String>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Repository", variables = "ListVariables")]
pub struct RepoForks {
    #[arguments(first: 30, after: $after, orderBy: { field: STARGAZERS, direction: DESC })]
    pub forks: PagedRepos,
}

impl PagedRepos {
    pub(crate) fn into_results(self) -> Results<RepoSummary> {
        Results::from_page(
            self.total_count,
            self.page_info,
            self.nodes,
            RepoCard::into_summary,
        )
    }
}

#[derive(cynic::Enum, Debug, Clone, Copy)]
pub enum RepositoryOrderField {
    CreatedAt,
    Name,
    PushedAt,
    Stargazers,
    UpdatedAt,
}

#[derive(cynic::Enum, Debug, Clone, Copy)]
pub enum OrderDirection {
    Asc,
    Desc,
}

#[derive(cynic::InputObject, Debug)]
pub struct RepositoryOrder {
    pub field: RepositoryOrderField,
    pub direction: OrderDirection,
}

impl From<RepoSort> for RepositoryOrder {
    fn from(sort: RepoSort) -> Self {
        let (field, direction) = match sort {
            RepoSort::Updated => (RepositoryOrderField::PushedAt, OrderDirection::Desc),
            RepoSort::Name => (RepositoryOrderField::Name, OrderDirection::Asc),
            RepoSort::Stars => (RepositoryOrderField::Stargazers, OrderDirection::Desc),
        };
        RepositoryOrder { field, direction }
    }
}

#[derive(cynic::QueryVariables, Debug)]
pub struct OwnerReposVariables {
    pub login: String,
    pub after: Option<String>,
    pub order: RepositoryOrder,
}

/// A user's or organization's own repositories.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Query", variables = "OwnerReposVariables")]
pub struct OwnerReposQuery {
    #[arguments(login: $login)]
    pub repository_owner: Option<OwnerRepos>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RepositoryOwner", variables = "OwnerReposVariables")]
pub struct OwnerRepos {
    #[arguments(first: 30, after: $after, orderBy: $order, ownerAffiliations: [OWNER])]
    pub repositories: PagedRepos,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct LoginPageVariables {
    pub login: String,
    pub after: Option<String>,
}

/// What a user starred, most recently first.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Query", variables = "LoginPageVariables")]
pub struct StarredQuery {
    #[arguments(login: $login)]
    pub user: Option<UserStarred>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "User", variables = "LoginPageVariables")]
pub struct UserStarred {
    #[arguments(first: 30, after: $after, orderBy: { field: STARRED_AT, direction: DESC })]
    pub starred_repositories: PagedStars,
}

impl PagedStars {
    pub(crate) fn into_results(self) -> Results<RepoSummary> {
        Results::from_page(
            self.total_count,
            self.page_info,
            self.nodes,
            RepoCard::into_summary,
        )
    }
}

// ---- releases and tags ---------------------------------------------------------------------

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Repository", variables = "ListVariables")]
pub struct RepoReleases {
    #[arguments(first: 20, after: $after, orderBy: { field: CREATED_AT, direction: DESC })]
    pub releases: ReleaseList,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Release")]
pub struct WireRelease {
    pub database_id: Option<i32>,
    pub name: Option<String>,
    pub tag_name: String,
    pub published_at: Option<DateTime>,
    pub is_prerelease: bool,
    pub is_draft: bool,
    pub is_latest: bool,
    pub author: Option<UserLogin>,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct ReleaseVariables {
    pub owner: String,
    pub name: String,
    pub tag: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Repository", variables = "ReleaseVariables")]
pub struct RepoRelease {
    #[arguments(tagName: $tag)]
    pub release: Option<WireReleaseFull>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Release")]
pub struct WireReleaseFull {
    #[cynic(spread)]
    pub card: WireRelease,
    pub description: Option<String>,
    #[arguments(first: 50)]
    pub release_assets: AssetList,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "ReleaseAsset")]
pub struct WireAsset {
    pub name: String,
    pub size: i32,
    pub download_count: i32,
    pub download_url: Uri,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Repository", variables = "ListVariables")]
pub struct RepoTags {
    #[arguments(refPrefix: "refs/tags/", first: 30, after: $after, orderBy: { field: TAG_COMMIT_DATE, direction: DESC })]
    pub refs: Option<TagRefs>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Ref")]
pub struct TagRef {
    pub name: String,
    pub target: Option<TagTarget>,
}

/// A lightweight tag points at a commit; an annotated one at a tag
/// object that points at it.
#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "GitObject")]
pub enum TagTarget {
    Commit(TagCommit),
    Tag(AnnotatedTag),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Commit")]
pub struct TagCommit {
    pub oid: GitObjectId,
    pub committed_date: DateTime,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Tag")]
pub struct AnnotatedTag {
    pub target: TaggedObject,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "GitObject")]
pub enum TaggedObject {
    Commit(TagCommit),
    #[cynic(fallback)]
    Other,
}

impl WireRelease {
    fn into_release(self) -> Release {
        Release {
            id: self.database_id.and_then(|id| u64::try_from(id).ok()),
            name: self
                .name
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| self.tag_name.clone()),
            tag: self.tag_name,
            published_at: self.published_at.map(|d| d.0),
            prerelease: self.is_prerelease,
            draft: self.is_draft,
            latest: self.is_latest,
            author: self.author.map(|a| a.login),
            notes: None,
            assets: Vec::new().into(),
        }
    }
}

impl ReleaseList {
    pub(crate) fn into_results(self) -> Results<Release> {
        Results::from_page(self.total_count, self.page_info, self.nodes, |r| {
            Some(r.into_release())
        })
    }
}

impl WireReleaseFull {
    pub(crate) fn into_release(self) -> Release {
        Release {
            notes: self.description.filter(|d| !d.trim().is_empty()),
            assets: Capped::from_nodes(
                self.release_assets.total_count,
                self.release_assets.nodes,
                |a| Asset {
                    name: a.name,
                    size: count(a.size),
                    downloads: count(a.download_count),
                    url: a.download_url.0,
                },
            ),
            ..self.card.into_release()
        }
    }
}

impl TagRefs {
    pub(crate) fn into_results(self) -> Results<TagInfo> {
        Results::from_page(self.total_count, self.page_info, self.nodes, |r| {
            let commit = match r.target {
                Some(TagTarget::Commit(c)) => Some(c),
                Some(TagTarget::Tag(t)) => match t.target {
                    TaggedObject::Commit(c) => Some(c),
                    TaggedObject::Other => None,
                },
                _ => None,
            };
            Some(TagInfo {
                name: r.name,
                oid: commit.as_ref().map(|c| c.oid.0.clone()),
                date: commit.map(|c| c.committed_date.0),
            })
        })
    }
}

// ---- branches ------------------------------------------------------------------------------

/// A repository's branches (by name) and tags (newest first), and how
/// many there are of each.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refs {
    pub branches: Capped<String>,
    pub tags: Capped<String>,
}

// ---- conversions ---------------------------------------------------------------------

pub(crate) fn issue_state(state: WireIssueState, reason: Option<StateReason>) -> IssueState {
    match (state, reason) {
        (WireIssueState::Open, _) => IssueState::Open,
        (WireIssueState::Closed, Some(StateReason::NotPlanned | StateReason::Duplicate)) => {
            IssueState::NotPlanned
        }
        (WireIssueState::Closed, _) => IssueState::Closed,
    }
}

impl WireCommit {
    pub(crate) fn into_detail(self) -> CommitDetail {
        let (headline, body) = self.message.split_once('\n').unwrap_or((&self.message, ""));
        let author = Person::from(self.author);
        let committer = Some(Person::from(self.committer)).filter(|c| {
            c.name() != author.name() && *c != Person::Unknown && c.login() != Some("web-flow")
        });
        CommitDetail {
            oid: self.oid.0,
            headline: headline.trim().to_owned(),
            body: body.trim().to_owned(),
            author,
            authored_at: self.authored_date.0,
            committer,
            committed_at: self.committed_date.0,
            parents: Capped::from_nodes(self.parents.total_count, self.parents.nodes, |p| p.oid.0),
            additions: count(self.additions),
            deletions: count(self.deletions),
            changed_files: self.changed_files_if_available.map(count),
            verified: self.signature.map(|s| s.is_valid),
        }
    }
}

impl CommitCard {
    pub(crate) fn into_info(self) -> CommitInfo {
        CommitInfo {
            author: self.author.into(),
            oid: self.oid.0,
            headline: self.message.lines().next().unwrap_or_default().to_owned(),
            date: self.committed_date.0,
        }
    }
}

impl RepoCard {
    pub fn into_summary(self) -> Option<RepoSummary> {
        Some(RepoSummary {
            repo: RepoId::parse(&self.name_with_owner)?,
            description: self.description.filter(|d| !d.trim().is_empty()),
            stars: count(self.stargazer_count),
            forks: count(self.fork_count),
            language: self.primary_language.as_ref().map(|l| l.name.clone()),
            language_color: self.primary_language.and_then(|l| l.color),
            pushed_at: self.pushed_at.map(|d| d.0),
            private: self.is_private,
            fork: self.is_fork,
            archived: self.is_archived,
        })
    }
}

pub(crate) fn entries(tree: Tree) -> Vec<TreeEntry> {
    let mut out: Vec<TreeEntry> = tree
        .entries
        .unwrap_or_default()
        .into_iter()
        .map(|e| {
            let kind = match (e.kind.as_str(), e.mode) {
                ("tree", _) => EntryKind::Dir,
                ("commit", _) => EntryKind::Submodule,
                (_, 0o120000) => EntryKind::Symlink,
                _ => EntryKind::File,
            };
            TreeEntry {
                path: e.path.unwrap_or_else(|| e.name.clone()),
                name: e.name,
                size: (kind == EntryKind::File).then(|| count(e.size)),
                kind,
            }
        })
        .collect();
    // Directories first, then by name (case-insensitively), like GitHub.
    out.sort_by_cached_key(|e| (e.kind != EntryKind::Dir, e.name.to_lowercase()));
    out
}

impl RepoFull {
    pub(crate) fn into_overview(self) -> Option<RepoOverview> {
        let (default_branch, head) = self.default_branch_ref.map(|r| (r.name, r.target)).unzip();
        let (last_commit, commits) = match head.flatten() {
            Some(RefTarget::Commit(c)) => {
                (Some(c.commit.into_info()), count(c.history.total_count))
            }
            _ => (None, 0),
        };
        let entries = match self.object {
            Some(GitObject::Tree(t)) => entries(t),
            _ => Vec::new(),
        };
        Some(RepoOverview {
            summary: self.card.into_summary()?,
            homepage: self.homepage_url.map(|u| u.0).filter(|u| !u.is_empty()),
            watchers: count(self.watchers.total_count),
            open_issues: count(self.issues.total_count),
            open_prs: count(self.pull_requests.total_count),
            closed_issues: count(self.closed_issues.total_count),
            closed_prs: count(self.closed_pull_requests.total_count),
            license: self
                .license_info
                .map(|l| l.spdx_id.filter(|s| s != "NOASSERTION").unwrap_or(l.name)),
            topics: Capped::from_nodes(
                self.repository_topics.total_count,
                self.repository_topics.nodes,
                |t| t.topic.name,
            ),
            default_branch,
            last_commit,
            commits,
            parent: self.parent.map(|p| p.name_with_owner),
            starred: self.viewer_has_starred,
            can_write: crate::api::queries::RepositoryPermission::writes(self.viewer_permission),
            can_admin: self.viewer_permission
                == Some(crate::api::queries::RepositoryPermission::Admin),
            id: self.id.into(),
            has_issues: self.has_issues_enabled,
            has_discussions: self.has_discussions_enabled,
            has_wiki: self.has_wiki_enabled,
            branches: self.branch_count.map_or(0, |c| count(c.total_count)),
            tags: self.tag_count.map_or(0, |c| count(c.total_count)),
            pinned_issues: (self.pinned_issues)
                .map_or_else(Capped::default, timeline::pinned_issues),
            entries,
        })
    }
}

impl BrowseItem {
    pub(crate) fn into_issue(self) -> Option<IssueSummary> {
        match self {
            BrowseItem::Issue(i) => Some(IssueSummary {
                repo: RepoId::parse(&i.repository.name_with_owner)?,
                number: count(i.number),
                title: i.title,
                kind: IssueKind::Issue(issue_state(i.state, i.state_reason)),
                author: author(i.author),
                updated_at: i.updated_at.0,
                comments: count(i.comments.total_count),
                labels: labels(i.labels),
                created_at: i.created_at.0,
            }),
            BrowseItem::PullRequest(p) => Some(IssueSummary {
                repo: RepoId::parse(&p.repository.name_with_owner)?,
                number: count(p.number),
                title: p.title,
                kind: IssueKind::Pr {
                    id: p.id.into(),
                    state: p.status.into(),
                    review: p.review_decision,
                    approvals: timeline::approvals(p.latest_opinionated_reviews),
                    checks: None,
                },
                author: author(p.author),
                updated_at: p.updated_at.0,
                comments: count(p.comments.total_count),
                labels: labels(p.labels),
                created_at: p.created_at.0,
            }),
            _ => None,
        }
    }

    pub(crate) fn into_user(self) -> Option<UserSummary> {
        match self {
            BrowseItem::User(u) => Some(UserSummary {
                login: u.login,
                name: u.name,
                bio: u.bio.filter(|b| !b.trim().is_empty()),
                is_org: false,
            }),
            BrowseItem::Organization(o) => Some(UserSummary {
                login: o.login,
                name: o.name,
                bio: o.description.filter(|b| !b.trim().is_empty()),
                is_org: true,
            }),
            _ => None,
        }
    }
}

impl IssueFull {
    pub(crate) fn into_detail(self) -> Option<IssueDetail> {
        Some(IssueDetail {
            repo: RepoId::parse(&self.repository.name_with_owner)?,
            number: count(self.number),
            title: self.title,
            body: self.body,
            state: issue_state(self.state, self.state_reason),
            author: author(self.author),
            created_at: self.created_at.0,
            labels: labels(self.labels),
            assignees: Capped::from_nodes(self.assignees.total_count, self.assignees.nodes, |u| {
                u.login
            }),
            milestone: self.milestone.map(MilestoneRef::from_wire),
            timeline: timeline::issue_timeline(self.timeline_items),
            comment_count: count(self.comments.total_count),
            closed_by: (self.closed_by_pull_requests_references)
                .map_or_else(Capped::default, timeline::linked_prs),
            branches: timeline::branches(self.linked_branches),
            parent: self.parent.and_then(timeline::LinkedIssue::into_linked),
            sub_issues: timeline::linked_issues(self.sub_issues),
            progress: timeline::progress(&self.sub_issues_summary),
            blocked_by: timeline::linked_issues(self.blocked_by),
            blocking: timeline::linked_issues(self.blocking),
            locked: (self.locked).then_some(crate::api::manage::Locked {
                reason: self.active_lock_reason,
            }),
            pinned: self.is_pinned.unwrap_or(false),
            can_write: crate::api::queries::RepositoryPermission::writes(
                self.repository.viewer_permission,
            ),
            issue_type: self.issue_type.map(|t| t.name),
            editable: Editable {
                id: self.id.clone().into(),
                can_update: self.viewer_can_update,
                can_delete: false,
                can_react: self.viewer_can_react,
                reactions: reactions(self.reaction_groups),
            },
            id: self.id.into(),
            can_close: self.viewer_can_close,
            can_reopen: self.viewer_can_reopen,
            sidebar: SidebarPermits {
                labels: self.viewer_can_label,
                assignees: self.viewer_can_assign,
                milestone: self.viewer_can_set_milestone,
                reviewers: false,
            },
        })
    }
}

pub(crate) fn comment(c: WireComment) -> Comment {
    Comment {
        id: c.full_database_id.and_then(|id| id.0.parse().ok()),
        author: author(c.author),
        body: c.body,
        created_at: c.created_at.0,
        editable: Some(Editable {
            id: c.id.into(),
            can_update: c.viewer_can_update,
            can_delete: c.viewer_can_delete,
            can_react: c.viewer_can_react,
            reactions: reactions(c.reaction_groups),
        }),
        hiding: crate::api::manage::Hiding {
            reason: c
                .is_minimized
                .then(|| c.minimized_reason.unwrap_or_default()),
            can_hide: c.viewer_can_minimize,
            can_unhide: c.viewer_can_unminimize,
        },
    }
}

impl WirePrActivity {
    /// The pull request's activity, its review threads apart (they're
    /// fetched apart, a page at a time).
    pub(crate) fn into_activity(self) -> PrActivity {
        PrActivity {
            id: self.id.into(),
            timeline: timeline::pr_timeline(self.timeline_items, self.pending_review),
            comment_count: count(self.comments.total_count),
            commits: Capped::from_nodes(self.commits.total_count, self.commits.nodes, |n| {
                n.commit.into_info()
            }),
            requested: timeline::requested(self.review_requests),
            latest: timeline::latest(self.latest_reviews),
            closing: (self.closing_issues_references)
                .map_or_else(Capped::default, timeline::linked_issues),
            merge_commit: self.merge_commit.map(|c| c.oid.0),
            threads: Vec::new(),
        }
    }
}

fn pinned(p: Pinned) -> Vec<RepoSummary> {
    nodes(p.nodes)
        .filter_map(|i| match i {
            PinnedItem::Repository(r) => r.into_summary(),
            PinnedItem::Other => None,
        })
        .collect()
}

pub(crate) fn repo_list(list: RepoList) -> Capped<RepoSummary> {
    let repos = nodes(list.nodes)
        .filter_map(RepoCard::into_summary)
        .collect();
    Capped::new(repos, count(list.total_count))
}

fn readme_text(object: Option<ReadmeObject>) -> Option<String> {
    match object? {
        ReadmeObject::Blob(b) => b.text.filter(|t| !t.trim().is_empty()),
        ReadmeObject::Other => None,
    }
}

impl ProfileQuery {
    pub(crate) fn into_profile(self) -> Option<Profile> {
        if let Some(u) = self.user {
            let status = u.status.and_then(|s| {
                let text = [s.emoji, s.message]
                    .into_iter()
                    .flatten()
                    .filter(|t| !t.trim().is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                (!text.is_empty()).then_some(text)
            });
            return Some(Profile {
                readme: readme_text(self.user_readme.and_then(|r| r.object)),
                status,
                pronouns: u.pronouns.filter(|p| !p.trim().is_empty()),
                socials: nodes(u.social_accounts.nodes)
                    .map(|s| (s.display_name, s.url.0))
                    .collect(),
                orgs: Capped::from_nodes(u.organizations.total_count, u.organizations.nodes, |o| {
                    o.login
                }),
                verified: false,
                people: Capped::default(),
                contributions: Some(u.contributions_collection.into_contributions()),
                login: u.login,
                name: u.name.filter(|n| !n.is_empty()),
                bio: u.bio.filter(|b| !b.trim().is_empty()),
                company: u.company.filter(|c| !c.is_empty()),
                location: u.location.filter(|l| !l.is_empty()),
                website: u.website_url.map(|w| w.0).filter(|w| !w.is_empty()),
                followers: Some(count(u.followers.total_count)),
                following: Some(count(u.following.total_count)),
                is_org: false,
                pinned: pinned(u.pinned_items),
                repos: repo_list(u.repositories),
                star_count: count(u.starred_repositories.total_count),
            });
        }
        let o = self.organization?;
        Some(Profile {
            login: o.login,
            name: o.name.filter(|n| !n.is_empty()),
            bio: o.description.filter(|b| !b.trim().is_empty()),
            company: None,
            location: o.location.filter(|l| !l.is_empty()),
            website: o.website_url.map(|w| w.0).filter(|w| !w.is_empty()),
            followers: None,
            following: None,
            is_org: true,
            pinned: pinned(o.pinned_items),
            repos: repo_list(o.repositories),
            star_count: 0,
            readme: readme_text(self.org_readme.and_then(|r| r.object)),
            status: None,
            pronouns: None,
            socials: Vec::new(),
            orgs: Vec::new().into(),
            verified: o.is_verified,
            people: Capped::from_nodes(
                o.members_with_role.total_count,
                o.members_with_role.nodes,
                |u| u.login,
            ),
            contributions: None,
        })
    }
}
