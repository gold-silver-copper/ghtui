//! An issue's or pull request's timeline: its comments, reviews and
//! events in GitHub's own order (`timelineItems`), and how it connects to
//! other issues and pull requests.
//!
//! The wire types are typed against GitHub's schema; the models before
//! them are what the app renders and caches.

use serde::{Deserialize, Serialize};

use crate::api::browse::{
    BotLogin, Comment, CommitInfo, PrCommitNode, RefName, RequestedReviewer, Reviewer, StateReason,
    TeamSlug, TypeName, UserLogin, WireComment, WireIssueState, comment,
};
use crate::api::edit::{Editable, WireReactionGroup, reactions};
use crate::api::model::{
    Capped, IssueState, Label, PrState, PrStatus, RepoId, ReviewState, Status, author, count,
};
use crate::api::queries::{
    Actor, BigInt, CommitOid, DateTime, Label as QLabel, PageVariablesFields, RepositoryName,
    fragments, nodes,
};
use crate::schema::schema;

// ---- models ------------------------------------------------------------------

/// One entry of a conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Item {
    Comment(Comment),
    Review(Review),
    Event(Event),
}

impl Item {
    /// When it happened (ISO 8601).
    pub fn at(&self) -> &str {
        match self {
            Item::Comment(c) => &c.created_at,
            Item::Review(r) => &r.at,
            Item::Event(e) => &e.at,
        }
    }
}

/// A pull request's review: what its author decided and said.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Review {
    /// GitHub's number for it, which its link's anchor uses
    /// (`#pullrequestreview-…`) and its comments name.
    pub id: Option<u64>,
    pub author: String,
    pub state: ReviewState,
    /// Its summary, as Markdown.
    pub body: String,
    /// When it was submitted (ISO 8601); a pending review's, when it was
    /// started.
    pub at: String,
    /// The head it reviewed.
    pub commit: Option<String>,
    /// How many comments it made on lines and files.
    pub comments: u64,
    /// What edits name it by (its summary), what GitHub lets you do to
    /// it, and its reactions.
    pub editable: Editable,
}

impl Review {
    /// Its anchor in the conversation.
    pub fn anchor(&self) -> Option<String> {
        self.id.map(|id| format!("pullrequestreview-{id}"))
    }
}

/// Something that happened to an issue or pull request, by someone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub actor: String,
    /// ISO 8601.
    pub at: String,
    pub what: What,
}

/// An issue or pull request named by an event or a sidebar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Linked {
    pub repo: RepoId,
    pub number: u64,
    pub title: String,
    pub state: LinkedState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkedState {
    Issue(IssueState),
    Pr(PrState),
}

impl Linked {
    pub fn status(&self) -> Status {
        match self.state {
            LinkedState::Issue(s) => Status::Issue(s),
            LinkedState::Pr(s) => Status::Pr(s),
        }
    }

    pub fn is_pr(&self) -> bool {
        matches!(self.state, LinkedState::Pr(_))
    }

    /// `#12`, or `owner/name#12` from elsewhere than `here`.
    pub fn name(&self, here: &RepoId) -> String {
        if self.repo == *here {
            format!("#{}", self.number)
        } else {
            format!("{}#{}", self.repo, self.number)
        }
    }

    /// Its page on GitHub.
    pub fn url(&self) -> String {
        let kind = if self.is_pr() { "pull" } else { "issues" };
        format!(
            "{}/{}/{kind}/{}",
            crate::host::current().web(),
            self.repo,
            self.number
        )
    }
}

/// What closed an issue or pull request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Closer {
    Commit(String),
    Pr(Linked),
}

/// What an event was.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum What {
    Closed {
        by: Option<Closer>,
        /// An issue's: completed, not planned, a duplicate.
        reason: Option<IssueState>,
        duplicate: bool,
    },
    Reopened,
    Merged {
        commit: Option<String>,
        into: String,
    },
    /// Mentioned from another issue or pull request.
    CrossReferenced {
        from: Linked,
        will_close: bool,
    },
    /// Mentioned by a commit's message.
    Referenced {
        commit: String,
        repo: Option<RepoId>,
    },
    Labeled {
        label: Label,
        added: bool,
    },
    Assigned {
        who: String,
        added: bool,
    },
    Milestoned {
        title: String,
        added: bool,
    },
    /// Added to or removed from a project.
    Project {
        added: bool,
    },
    /// Moved between a project's statuses.
    ProjectStatus {
        from: String,
        to: String,
    },
    ReviewRequested {
        who: Option<Reviewer>,
        added: bool,
    },
    ReviewDismissed {
        /// Whose review.
        of: Option<String>,
        message: String,
    },
    Renamed {
        from: String,
        to: String,
    },
    ReadyForReview,
    ConvertedToDraft,
    /// A commit added to a pull request.
    Committed(CommitInfo),
    /// The head (or base) branch force-pushed, old head to new.
    ForcePushed {
        base: bool,
        before: Option<String>,
        after: Option<String>,
    },
    BranchDeleted {
        name: String,
    },
    BranchRestored,
    BaseChanged {
        from: String,
        to: String,
    },
    Locked {
        reason: Option<String>,
    },
    Unlocked,
    Pinned {
        pinned: bool,
    },
    Transferred {
        from: Option<String>,
    },
    MarkedDuplicate {
        of: Option<Linked>,
        undone: bool,
    },
    AutoMerge {
        on: bool,
    },
    MergeQueue {
        added: bool,
        reason: Option<String>,
    },
    SubIssue {
        issue: Option<Linked>,
        added: bool,
    },
    Parent {
        issue: Option<Linked>,
        added: bool,
    },
    IssueType {
        from: Option<String>,
        to: Option<String>,
    },
    ConvertedToDiscussion,
}

impl What {
    /// Which run of events it folds into: events of one kind by one
    /// person, one after another, read as one line.
    pub fn kind(&self) -> &'static str {
        match self {
            What::Closed { .. } => "closed",
            What::Reopened => "reopened",
            What::Merged { .. } => "merged",
            What::CrossReferenced { .. } => "referenced",
            What::Referenced { .. } => "commit",
            What::Labeled { .. } => "labels",
            What::Assigned { .. } => "assignees",
            What::Milestoned { .. } => "milestone",
            What::Project { .. } | What::ProjectStatus { .. } => "project",
            What::ReviewRequested { .. } => "reviewers",
            What::ReviewDismissed { .. } => "dismissed",
            What::Renamed { .. } => "title",
            What::ReadyForReview | What::ConvertedToDraft => "draft",
            What::Committed(_) => "commits",
            What::ForcePushed { .. } => "force",
            What::BranchDeleted { .. } | What::BranchRestored => "branch",
            What::BaseChanged { .. } => "base",
            What::Locked { .. } | What::Unlocked => "lock",
            What::Pinned { .. } => "pin",
            What::Transferred { .. } => "transfer",
            What::MarkedDuplicate { .. } => "duplicate",
            What::AutoMerge { .. } => "auto-merge",
            What::MergeQueue { .. } => "queue",
            What::SubIssue { .. } => "sub-issues",
            What::Parent { .. } => "parent",
            What::IssueType { .. } => "type",
            What::ConvertedToDiscussion => "discussion",
        }
    }
}

/// How an issue's sub-issues are coming along.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
}

/// The latest review of one reviewer, as the sidebar lists them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Latest {
    pub author: String,
    pub state: ReviewState,
    pub id: Option<u64>,
    /// ISO 8601.
    pub at: String,
    /// The head it reviewed: an approval of an older one came before the
    /// last push.
    pub commit: Option<String>,
    /// Teams it was given for (`org/slug`).
    pub for_teams: Vec<String>,
}

/// Someone asked for a review who hasn't given one since.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Requested {
    pub who: Reviewer,
    /// Asked because they own code it changes (CODEOWNERS).
    pub code_owner: bool,
}

// ---- wire: the items -------------------------------------------------------------

/// The events: each is who and when, and what else it carries, `?` marking
/// what may be null. cynic's derive names its locals with the span of each
/// field type's first token, so this macro writes that token itself: a type
/// is `Option<T>` or `self::T`, never the caller's own tokens.
macro_rules! events {
    ($($name:ident = $graphql:literal { $($field:ident: $(?$opt:ident)? $($ty:ident)?),* $(,)? }),* $(,)?) => {$(
        #[derive(cynic::QueryFragment, Debug)]
        #[cynic(graphql_type = $graphql)]
        pub struct $name {
            pub actor: Option<Actor>,
            pub created_at: DateTime,
            $(pub $field: $(Option<$opt>)? $(self::$ty)?,)*
        }
    )*};
}

// events! writes `self::String`, and the prelude's String isn't in `self`.
use std::string::String;

events! {
    ReopenedEv = "ReopenedEvent" {},
    ReadyEv = "ReadyForReviewEvent" {},
    DraftEv = "ConvertToDraftEvent" {},
    RestoredEv = "HeadRefRestoredEvent" {},
    UnlockedEv = "UnlockedEvent" {},
    PinnedEv = "PinnedEvent" {},
    UnpinnedEv = "UnpinnedEvent" {},
    AutoMergeOnEv = "AutoMergeEnabledEvent" {},
    AutoMergeOffEv = "AutoMergeDisabledEvent" {},
    QueuedEv = "AddedToMergeQueueEvent" {},
    DiscussionEv = "ConvertedToDiscussionEvent" {},
    ProjectAddedEv = "AddedToProjectV2Event" {},
    ProjectRemovedEv = "RemovedFromProjectV2Event" {},
    ClosedEv = "ClosedEvent" { closer: ?WireCloser, state_reason: ?StateReason },
    MergedEv = "MergedEvent" { commit: ?CommitOid, merge_ref_name: String },
    ReferencedEv = "ReferencedEvent" { commit: ?CommitOid, commit_repository: RepositoryName },
    LabeledEv = "LabeledEvent" { label: QLabel },
    UnlabeledEv = "UnlabeledEvent" { label: QLabel },
    AssignedEv = "AssignedEvent" { assignee: ?WireAssignee },
    UnassignedEv = "UnassignedEvent" { assignee: ?WireAssignee },
    MilestonedEv = "MilestonedEvent" { milestone_title: String },
    DemilestonedEv = "DemilestonedEvent" { milestone_title: String },
    StatusChangedEv = "ProjectV2ItemStatusChangedEvent" { previous_status: String, status: String },
    ReviewRequestedEv = "ReviewRequestedEvent" { requested_reviewer: ?RequestedReviewer },
    ReviewUnrequestedEv = "ReviewRequestRemovedEvent" { requested_reviewer: ?RequestedReviewer },
    DismissedEv = "ReviewDismissedEvent" { dismissal_message: ?String, review: ?ReviewAuthor },
    RenamedEv = "RenamedTitleEvent" { previous_title: String, current_title: String },
    HeadPushedEv = "HeadRefForcePushedEvent" { before_commit: ?CommitOid, after_commit: ?CommitOid },
    BasePushedEv = "BaseRefForcePushedEvent" { before_commit: ?CommitOid, after_commit: ?CommitOid },
    DeletedEv = "HeadRefDeletedEvent" { head_ref_name: String },
    BaseChangedEv = "BaseRefChangedEvent" { previous_ref_name: String, current_ref_name: String },
    LockedEv = "LockedEvent" { lock_reason: ?LockReason },
    TransferredEv = "TransferredEvent" { from_repository: ?RepositoryName },
    DuplicateEv = "MarkedAsDuplicateEvent" { canonical: ?IssueOrPrLink },
    NotDuplicateEv = "UnmarkedAsDuplicateEvent" { canonical: ?IssueOrPrLink },
    DequeuedEv = "RemovedFromMergeQueueEvent" { reason: ?String },
    SubAddedEv = "SubIssueAddedEvent" { sub_issue: ?LinkedIssue },
    SubRemovedEv = "SubIssueRemovedEvent" { sub_issue: ?LinkedIssue },
    ParentAddedEv = "ParentIssueAddedEvent" { parent: ?LinkedIssue },
    ParentRemovedEv = "ParentIssueRemovedEvent" { parent: ?LinkedIssue },
    TypeAddedEv = "IssueTypeAddedEvent" { issue_type: ?TypeName },
    TypeChangedEv = "IssueTypeChangedEvent" { issue_type: ?TypeName, prev_issue_type: ?TypeName },
    TypeRemovedEv = "IssueTypeRemovedEvent" { issue_type: ?TypeName },
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "CrossReferencedEvent")]
pub struct CrossReferencedEv {
    pub actor: Option<Actor>,
    pub created_at: DateTime,
    pub source: Subject,
    pub will_close_target: bool,
}

/// An issue a timeline names.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Issue")]
pub struct LinkedIssue {
    pub number: i32,
    pub title: String,
    pub state: WireIssueState,
    pub state_reason: Option<StateReason>,
    pub repository: RepositoryName,
}

/// A pull request a timeline names.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequest")]
pub struct LinkedPr {
    pub number: i32,
    pub title: String,
    #[cynic(spread)]
    pub status: PrStatus,
    pub repository: RepositoryName,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "ReferencedSubject")]
pub enum Subject {
    Issue(LinkedIssue),
    PullRequest(LinkedPr),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "IssueOrPullRequest")]
pub enum IssueOrPrLink {
    Issue(LinkedIssue),
    PullRequest(LinkedPr),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "Closer")]
pub enum WireCloser {
    Commit(CommitOid),
    PullRequest(LinkedPr),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "Assignee")]
pub enum WireAssignee {
    User(UserLogin),
    Bot(BotLogin),
    #[cynic(fallback)]
    Other,
}

words! {
    #[derive(cynic::Enum, Hash)]
    pub enum LockReason {
        pub(crate) fn wire,
        /// "off-topic", "too heated".
        pub fn said,
    }
    OffTopic  "OFF_TOPIC"  "off-topic";
    Resolved  "RESOLVED"   "resolved";
    Spam      "SPAM"       "spam";
    TooHeated "TOO_HEATED" "too heated";
}

fragments! { author: Option<Actor>; ReviewAuthor = "PullRequestReview" }

/// A review, as the timeline and the list of reviews have it.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequestReview")]
pub struct WireReview {
    pub id: cynic::Id,
    /// `databaseId` overflows GraphQL's `Int` on newer reviews.
    pub full_database_id: Option<BigInt>,
    pub author: Option<Actor>,
    pub state: ReviewState,
    pub body: String,
    pub submitted_at: Option<DateTime>,
    pub created_at: DateTime,
    pub commit: Option<CommitOid>,
    #[arguments(first: 0)]
    pub comments: crate::api::queries::ReviewCommentCount,
    pub viewer_can_update: bool,
    pub viewer_can_delete: bool,
    pub viewer_can_react: bool,
    pub reaction_groups: Option<Vec<WireReactionGroup>>,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "PullRequestTimelineItems")]
pub enum PrItem {
    IssueComment(Box<WireComment>),
    PullRequestReview(Box<WireReview>),
    PullRequestCommit(PrCommitNode),
    ClosedEvent(ClosedEv),
    ReopenedEvent(ReopenedEv),
    MergedEvent(MergedEv),
    CrossReferencedEvent(CrossReferencedEv),
    ReferencedEvent(ReferencedEv),
    LabeledEvent(LabeledEv),
    UnlabeledEvent(UnlabeledEv),
    AssignedEvent(AssignedEv),
    UnassignedEvent(UnassignedEv),
    MilestonedEvent(MilestonedEv),
    DemilestonedEvent(DemilestonedEv),
    AddedToProjectV2Event(ProjectAddedEv),
    RemovedFromProjectV2Event(ProjectRemovedEv),
    ProjectV2ItemStatusChangedEvent(StatusChangedEv),
    ReviewRequestedEvent(ReviewRequestedEv),
    ReviewRequestRemovedEvent(ReviewUnrequestedEv),
    ReviewDismissedEvent(DismissedEv),
    RenamedTitleEvent(RenamedEv),
    ReadyForReviewEvent(ReadyEv),
    ConvertToDraftEvent(DraftEv),
    HeadRefForcePushedEvent(HeadPushedEv),
    BaseRefForcePushedEvent(BasePushedEv),
    HeadRefDeletedEvent(DeletedEv),
    HeadRefRestoredEvent(RestoredEv),
    BaseRefChangedEvent(BaseChangedEv),
    LockedEvent(LockedEv),
    UnlockedEvent(UnlockedEv),
    MarkedAsDuplicateEvent(DuplicateEv),
    UnmarkedAsDuplicateEvent(NotDuplicateEv),
    AutoMergeEnabledEvent(AutoMergeOnEv),
    AutoMergeDisabledEvent(AutoMergeOffEv),
    AddedToMergeQueueEvent(QueuedEv),
    RemovedFromMergeQueueEvent(DequeuedEv),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "IssueTimelineItems")]
pub enum IssueItem {
    IssueComment(Box<WireComment>),
    ClosedEvent(ClosedEv),
    ReopenedEvent(ReopenedEv),
    CrossReferencedEvent(CrossReferencedEv),
    ReferencedEvent(ReferencedEv),
    LabeledEvent(LabeledEv),
    UnlabeledEvent(UnlabeledEv),
    AssignedEvent(AssignedEv),
    UnassignedEvent(UnassignedEv),
    MilestonedEvent(MilestonedEv),
    DemilestonedEvent(DemilestonedEv),
    AddedToProjectV2Event(ProjectAddedEv),
    RemovedFromProjectV2Event(ProjectRemovedEv),
    ProjectV2ItemStatusChangedEvent(StatusChangedEv),
    RenamedTitleEvent(RenamedEv),
    LockedEvent(LockedEv),
    UnlockedEvent(UnlockedEv),
    PinnedEvent(PinnedEv),
    UnpinnedEvent(UnpinnedEv),
    TransferredEvent(TransferredEv),
    MarkedAsDuplicateEvent(DuplicateEv),
    UnmarkedAsDuplicateEvent(NotDuplicateEv),
    SubIssueAddedEvent(SubAddedEv),
    SubIssueRemovedEvent(SubRemovedEv),
    ParentIssueAddedEvent(ParentAddedEv),
    ParentIssueRemovedEvent(ParentRemovedEv),
    IssueTypeAddedEvent(TypeAddedEv),
    IssueTypeChangedEvent(TypeChangedEv),
    IssueTypeRemovedEvent(TypeRemovedEv),
    ConvertedToDiscussionEvent(DiscussionEv),
    #[cynic(fallback)]
    Other,
}

fragments! {
    counted:
    PrTimeline = "PullRequestTimelineItemsConnection" => PrItem,
    IssueTimeline = "IssueTimelineItemsConnection" => IssueItem,
    Issues = "IssueConnection" => LinkedIssue,
    ClosedBy = "PullRequestConnection" => LinkedPr,
    Branches = "LinkedBranchConnection" => WireLinkedBranch,
    LatestReviews = "PullRequestReviewConnection" => WireLatest,
    Requests = "ReviewRequestConnection" => WireRequest,
    PendingReviews = "PullRequestReviewConnection" => WireReview,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "LinkedBranch")]
pub struct WireLinkedBranch {
    #[cynic(rename = "ref")]
    pub branch: Option<RefName>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "SubIssuesSummary")]
pub struct WireProgress {
    pub completed: i32,
    pub total: i32,
}

fragments! {
    counted:
    OnBehalfOf = "TeamConnection" => TeamSlug,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequestReview")]
pub struct WireLatest {
    pub full_database_id: Option<BigInt>,
    pub author: Option<Actor>,
    pub state: ReviewState,
    pub submitted_at: Option<DateTime>,
    pub commit: Option<CommitOid>,
    #[arguments(first: 10)]
    pub on_behalf_of: OnBehalfOf,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequestReview")]
pub struct Opinion {
    pub state: ReviewState,
}

fragments! {
    counted:
    Opinions = "PullRequestReviewConnection" => Opinion,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct PinnedIssue {
    pub issue: LinkedIssue,
}

fragments! {
    counted:
    PinnedIssues = "PinnedIssueConnection" => PinnedIssue,
}

/// A repository's pinned issues.
pub(crate) fn pinned_issues(p: PinnedIssues) -> Capped<Linked> {
    capped(p.total_count, p.nodes, |p| p.issue.into_linked())
}

/// How many reviewers' latest word is an approval.
pub(crate) fn approvals(o: Option<Opinions>) -> u64 {
    let approved = o.map(|o| nodes(o.nodes).filter(|r| r.state == ReviewState::Approved));
    approved.map_or(0, |a| a.count() as u64)
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "ReviewRequest")]
pub struct WireRequest {
    pub as_code_owner: bool,
    pub requested_reviewer: Option<RequestedReviewer>,
}

// ---- from the wire ---------------------------------------------------------------

fn event(actor_: Option<Actor>, at: DateTime, what: What) -> Item {
    Item::Event(Event {
        actor: author(actor_),
        at: at.0,
        what,
    })
}

fn label(l: QLabel) -> Label {
    Label {
        name: l.name,
        color: l.color,
    }
}

fn linked(issue: Option<LinkedIssue>) -> Option<Linked> {
    issue.and_then(LinkedIssue::into_linked)
}

fn oid(c: Option<CommitOid>) -> Option<String> {
    c.map(|c| c.oid.0)
}

fn database_id(id: Option<BigInt>) -> Option<u64> {
    id.and_then(|id| id.0.parse().ok())
}

impl LinkedIssue {
    pub(crate) fn into_linked(self) -> Option<Linked> {
        Some(Linked {
            repo: RepoId::parse(&self.repository.name_with_owner)?,
            number: count(self.number),
            title: self.title,
            state: LinkedState::Issue(crate::api::browse::issue_state(
                self.state,
                self.state_reason,
            )),
        })
    }
}

impl LinkedPr {
    pub(crate) fn into_linked(self) -> Option<Linked> {
        Some(Linked {
            repo: RepoId::parse(&self.repository.name_with_owner)?,
            number: count(self.number),
            title: self.title,
            state: LinkedState::Pr(self.status.into()),
        })
    }
}

impl Subject {
    fn into_linked(self) -> Option<Linked> {
        match self {
            Subject::Issue(i) => i.into_linked(),
            Subject::PullRequest(p) => p.into_linked(),
            Subject::Other => None,
        }
    }
}

impl IssueOrPrLink {
    fn into_linked(self) -> Option<Linked> {
        match self {
            IssueOrPrLink::Issue(i) => i.into_linked(),
            IssueOrPrLink::PullRequest(p) => p.into_linked(),
            IssueOrPrLink::Other => None,
        }
    }
}

fn assignee(a: Option<WireAssignee>) -> String {
    match a {
        Some(WireAssignee::User(u)) => u.login,
        Some(WireAssignee::Bot(b)) => b.login,
        _ => "ghost".to_owned(),
    }
}

pub(crate) fn reviewer(r: Option<RequestedReviewer>) -> Option<Reviewer> {
    match r? {
        RequestedReviewer::User(u) => Some(Reviewer::User(u.login)),
        RequestedReviewer::Bot(b) => Some(Reviewer::User(b.login)),
        RequestedReviewer::Team(t) => Some(Reviewer::Team(t.combined_slug)),
        RequestedReviewer::Other => None,
    }
}

impl WireReview {
    pub(crate) fn into_review(self) -> Review {
        Review {
            id: database_id(self.full_database_id),
            author: author(self.author),
            state: self.state,
            body: self.body,
            at: self.submitted_at.unwrap_or(self.created_at).0,
            commit: oid(self.commit),
            comments: count(self.comments.total_count),
            editable: Editable {
                id: self.id.into(),
                can_update: self.viewer_can_update,
                can_delete: self.viewer_can_delete,
                can_react: self.viewer_can_react,
                reactions: reactions(self.reaction_groups),
            },
        }
    }
}

/// A timeline's items: its comments, the `$arm`s it has besides, and its
/// events, each `Variant(e) => what` (who and when come from `e`; a `?`
/// in `what` leaves the event out). The events both kinds of timeline
/// have are written here once.
macro_rules! timeline_items {
    ($item:expr, $ty:ident, { $($arm:tt)* } $($v:ident($e:ident) => $what:expr,)*) => {
        timeline_items!(@all $item, $ty, { $($arm)* }
            ClosedEvent(e) => {
                let by = match e.closer {
                    Some(WireCloser::Commit(c)) => Some(Closer::Commit(c.oid.0)),
                    Some(WireCloser::PullRequest(p)) => p.into_linked().map(Closer::Pr),
                    _ => None,
                };
                let reason = e.state_reason.map(|r| match r {
                    StateReason::NotPlanned => IssueState::NotPlanned,
                    _ => IssueState::Closed,
                });
                let duplicate = e.state_reason == Some(StateReason::Duplicate);
                What::Closed { by, reason, duplicate }
            },
            ReopenedEvent(e) => What::Reopened,
            CrossReferencedEvent(e) => What::CrossReferenced {
                from: e.source.into_linked()?,
                will_close: e.will_close_target,
            },
            ReferencedEvent(e) => {
                let repo = RepoId::parse(&e.commit_repository.name_with_owner);
                What::Referenced { commit: oid(e.commit)?, repo }
            },
            LabeledEvent(e) => What::Labeled { label: label(e.label), added: true },
            UnlabeledEvent(e) => What::Labeled { label: label(e.label), added: false },
            AssignedEvent(e) => What::Assigned { who: assignee(e.assignee), added: true },
            UnassignedEvent(e) => What::Assigned { who: assignee(e.assignee), added: false },
            MilestonedEvent(e) => What::Milestoned { title: e.milestone_title, added: true },
            DemilestonedEvent(e) => What::Milestoned { title: e.milestone_title, added: false },
            AddedToProjectV2Event(e) => What::Project { added: true },
            RemovedFromProjectV2Event(e) => What::Project { added: false },
            ProjectV2ItemStatusChangedEvent(e) => {
                What::ProjectStatus { from: e.previous_status, to: e.status }
            },
            RenamedTitleEvent(e) => What::Renamed { from: e.previous_title, to: e.current_title },
            LockedEvent(e) => What::Locked { reason: e.lock_reason.map(|r| r.said().to_owned()) },
            UnlockedEvent(e) => What::Unlocked,
            MarkedAsDuplicateEvent(e) => What::MarkedDuplicate {
                of: e.canonical.and_then(IssueOrPrLink::into_linked),
                undone: false,
            },
            UnmarkedAsDuplicateEvent(e) => What::MarkedDuplicate {
                of: e.canonical.and_then(IssueOrPrLink::into_linked),
                undone: true,
            },
            $($v($e) => $what,)*
        )
    };
    (@all $item:expr, $ty:ident, { $($arm:tt)* } $($v:ident($e:ident) => $what:expr,)*) => {
        match $item {
            $ty::IssueComment(c) => Some(Item::Comment(comment(*c))),
            $($arm)*
            $($ty::$v($e) => Some(event($e.actor, $e.created_at, $what)),)*
            $ty::Other => None,
        }
    };
}

impl PrItem {
    pub(crate) fn into_item(self) -> Option<Item> {
        timeline_items!(self, PrItem, {
            PrItem::PullRequestReview(r) => Some(Item::Review(r.into_review())),
            PrItem::PullRequestCommit(c) => {
                let at = c.commit.committed_date.0.clone();
                let info = c.commit.into_info();
                let actor = info.author.login().unwrap_or_default().to_owned();
                Some(Item::Event(Event { actor, at, what: What::Committed(info) }))
            }
        }
            MergedEvent(e) => What::Merged { commit: oid(e.commit), into: e.merge_ref_name },
            ReviewRequestedEvent(e) => {
                What::ReviewRequested { who: reviewer(e.requested_reviewer), added: true }
            },
            ReviewRequestRemovedEvent(e) => {
                What::ReviewRequested { who: reviewer(e.requested_reviewer), added: false }
            },
            ReviewDismissedEvent(e) => What::ReviewDismissed {
                of: e.review.map(|r| author(r.author)),
                message: e.dismissal_message.unwrap_or_default(),
            },
            ReadyForReviewEvent(e) => What::ReadyForReview,
            ConvertToDraftEvent(e) => What::ConvertedToDraft,
            HeadRefForcePushedEvent(e) => What::ForcePushed {
                base: false,
                before: oid(e.before_commit),
                after: oid(e.after_commit),
            },
            BaseRefForcePushedEvent(e) => What::ForcePushed {
                base: true,
                before: oid(e.before_commit),
                after: oid(e.after_commit),
            },
            HeadRefDeletedEvent(e) => What::BranchDeleted { name: e.head_ref_name },
            HeadRefRestoredEvent(e) => What::BranchRestored,
            BaseRefChangedEvent(e) => {
                What::BaseChanged { from: e.previous_ref_name, to: e.current_ref_name }
            },
            AutoMergeEnabledEvent(e) => What::AutoMerge { on: true },
            AutoMergeDisabledEvent(e) => What::AutoMerge { on: false },
            AddedToMergeQueueEvent(e) => What::MergeQueue { added: true, reason: None },
            RemovedFromMergeQueueEvent(e) => What::MergeQueue {
                added: false,
                reason: e.reason.filter(|r| !r.trim().is_empty()),
            },
        )
    }
}

impl IssueItem {
    pub(crate) fn into_item(self) -> Option<Item> {
        timeline_items!(self, IssueItem, {}
            PinnedEvent(e) => What::Pinned { pinned: true },
            UnpinnedEvent(e) => What::Pinned { pinned: false },
            TransferredEvent(e) => {
                What::Transferred { from: e.from_repository.map(|r| r.name_with_owner) }
            },
            SubIssueAddedEvent(e) => What::SubIssue { issue: linked(e.sub_issue), added: true },
            SubIssueRemovedEvent(e) => What::SubIssue { issue: linked(e.sub_issue), added: false },
            ParentIssueAddedEvent(e) => What::Parent { issue: linked(e.parent), added: true },
            ParentIssueRemovedEvent(e) => What::Parent { issue: linked(e.parent), added: false },
            IssueTypeAddedEvent(e) => {
                What::IssueType { from: None, to: e.issue_type.map(|t| t.name) }
            },
            IssueTypeChangedEvent(e) => What::IssueType {
                from: e.prev_issue_type.map(|t| t.name),
                to: e.issue_type.map(|t| t.name),
            },
            IssueTypeRemovedEvent(e) => {
                What::IssueType { from: e.issue_type.map(|t| t.name), to: None }
            },
            ConvertedToDiscussionEvent(e) => What::ConvertedToDiscussion,
        )
    }
}

/// The nodes `f` makes items, of a connection's `total`.
fn capped<N, T>(
    total: i32,
    list: Option<Vec<Option<N>>>,
    f: impl FnMut(N) -> Option<T>,
) -> Capped<T> {
    Capped::new(nodes(list).filter_map(f).collect(), count(total))
}

/// A timeline's items, oldest first, of how many there are in all: what
/// ghtui doesn't show isn't left out.
fn timeline<N>(
    total: i32,
    list: Option<Vec<Option<N>>>,
    item: fn(N) -> Option<Item>,
) -> Capped<Item> {
    let fetched = list.as_ref().map_or(0, Vec::len) as u64;
    let items: Vec<Item> = nodes(list).filter_map(item).collect();
    let unshown = fetched.saturating_sub(items.len() as u64);
    Capped::new(items, count(total).saturating_sub(unshown))
}

/// A pull request's timeline. Your pending review, which the timeline may
/// not have, comes last.
pub(crate) fn pr_timeline(t: PrTimeline, pending: Option<PendingReviews>) -> Capped<Item> {
    let timeline = timeline(t.total_count, t.nodes, PrItem::into_item);
    let has_pending =
        (timeline.iter()).any(|i| matches!(i, Item::Review(r) if r.state == ReviewState::Pending));
    let pending = pending.and_then(|p| nodes(p.nodes).next());
    let Some(p) = pending.filter(|_| !has_pending) else {
        return timeline;
    };
    let total = timeline.total() + 1;
    let mut items = timeline.items;
    items.push(Item::Review(p.into_review()));
    Capped::new(items, total)
}

pub(crate) fn issue_timeline(t: IssueTimeline) -> Capped<Item> {
    timeline(t.total_count, t.nodes, IssueItem::into_item)
}

pub(crate) fn linked_issues(c: Issues) -> Capped<Linked> {
    capped(c.total_count, c.nodes, LinkedIssue::into_linked)
}

pub(crate) fn linked_prs(c: ClosedBy) -> Capped<Linked> {
    capped(c.total_count, c.nodes, LinkedPr::into_linked)
}

pub(crate) fn branches(b: Branches) -> Capped<String> {
    capped(b.total_count, b.nodes, |b| Some(b.branch?.name))
}

pub(crate) fn progress(p: &WireProgress) -> Option<Progress> {
    (p.total > 0).then(|| Progress {
        done: count(p.completed),
        total: count(p.total),
    })
}

pub(crate) fn latest(l: Option<LatestReviews>) -> Capped<Latest> {
    l.map_or_else(Capped::default, |l| {
        Capped::from_nodes(l.total_count, l.nodes, |r| Latest {
            author: author(r.author),
            state: r.state,
            id: database_id(r.full_database_id),
            at: r.submitted_at.map(|d| d.0).unwrap_or_default(),
            commit: oid(r.commit),
            for_teams: nodes(r.on_behalf_of.nodes)
                .map(|t| t.combined_slug)
                .collect(),
        })
    })
}

/// Who a pull request's review requests name; one GitHub's types don't
/// cover (an enterprise team) counts as left out.
pub(crate) fn requested(r: Option<Requests>) -> Capped<Requested> {
    r.map_or_else(Capped::default, |r| {
        capped(r.total_count, r.nodes, |r| {
            Some(Requested {
                who: reviewer(r.requested_reviewer)?,
                code_owner: r.as_code_owner,
            })
        })
    })
}

// ---- a pull request's reviews ----------------------------------------------------------

fragments! {
    root "Query" repository(owner, name);
    /// A page of a pull request's reviews, oldest first.
    ReviewsQuery("PageVariables") => RepoReviews,
}
fragments! {
    root "Repository" pull_request(number);
    RepoReviews("PageVariables") => PrReviews,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequest", variables = "PageVariables")]
pub struct PrReviews {
    #[arguments(first: 100, after: $after)]
    pub reviews: Option<ReviewsPage>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequestReviewConnection")]
pub struct ReviewsPage {
    pub page_info: crate::api::queries::PageInfo,
    pub nodes: Option<Vec<Option<WireReview>>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode<T: serde::de::DeserializeOwned>(json: serde_json::Value) -> T {
        serde_json::from_value(json).unwrap_or_else(|e| panic!("{e}"))
    }

    /// A pull request's timeline keeps GitHub's order and states (a
    /// dismissed review reads as dismissed, as the conversation shows it),
    /// and what ghtui doesn't show isn't counted as left out.
    #[test]
    fn a_timeline_keeps_order_and_counts_only_what_it_shows() {
        let review = |state: &str| {
            serde_json::json!({"__typename": "PullRequestReview", "id": "PRR_1",
                "fullDatabaseId": "7", "author": {"login": "hubot"}, "state": state,
                "body": "", "submittedAt": "2026-10-01T00:00:00Z",
                "createdAt": "2026-10-01T00:00:00Z", "commit": {"oid": "abc"},
                "comments": {"totalCount": 2}, "viewerCanUpdate": false,
                "viewerCanDelete": false, "viewerCanReact": true, "reactionGroups": []})
        };
        let wire: crate::api::browse::WirePrActivity = decode(serde_json::json!({
            "id": "PR_1",
            "comments": {"totalCount": 0},
            "timelineItems": {"totalCount": 5, "nodes": [
                review("DISMISSED"),
                {"__typename": "SomethingNew"},
                {"__typename": "ReadyForReviewEvent", "actor": null,
                 "createdAt": "2026-10-02T00:00:00Z"},
            ]},
            "commits": {"totalCount": 0, "nodes": []},
            "reviewRequests": {"totalCount": 1, "nodes": [
                {"asCodeOwner": true, "requestedReviewer": {"__typename": "Team", "combinedSlug": "o/core"}}
            ]},
            "latestReviews": null,
            "closingIssuesReferences": null,
            "mergeCommit": null,
            "pending_review": {"totalCount": 1, "nodes": [review("PENDING")]},
        }));
        let a = wire.into_activity();
        let [Item::Review(r), Item::Event(e), Item::Review(mine)] = a.timeline.items.as_slice()
        else {
            panic!("{:?}", a.timeline);
        };
        assert_eq!(
            (r.state, r.id, r.comments),
            (ReviewState::Dismissed, Some(7), 2)
        );
        assert_eq!(
            (e.actor.as_str(), &e.what),
            ("ghost", &What::ReadyForReview)
        );
        assert_eq!(
            mine.state,
            ReviewState::Pending,
            "your pending review comes last"
        );
        assert_eq!(a.timeline.left_out(), 2);
        assert!(a.requested.first().is_some_and(|r| r.code_owner));
    }

    #[test]
    fn a_closing_commit_and_a_will_close_reference_read_as_github_says_them() {
        let items: Vec<PrItem> = decode(serde_json::json!([
            {"__typename": "CrossReferencedEvent", "actor": {"login": "bob"},
             "createdAt": "2026-01-01T00:00:00Z", "willCloseTarget": true,
             "source": {"__typename": "PullRequest", "number": 5, "title": "Fix",
                        "isDraft": false, "state": "OPEN",
                        "repository": {"nameWithOwner": "o/r"}}},
            {"__typename": "ClosedEvent", "actor": {"login": "alice"},
             "createdAt": "2026-01-02T00:00:00Z", "stateReason": null,
             "closer": {"__typename": "Commit", "oid": "abc"}},
            {"__typename": "SomethingNew"}
        ]));
        let items: Vec<Item> = items.into_iter().filter_map(PrItem::into_item).collect();
        let [Item::Event(refd), Item::Event(closed)] = items.as_slice() else {
            panic!("{items:?}");
        };
        let What::CrossReferenced { from, will_close } = &refd.what else {
            panic!("{refd:?}");
        };
        assert!(*will_close && from.is_pr() && from.name(&RepoId::new("o", "r")) == "#5");
        assert_eq!(closed.actor, "alice");
        assert!(matches!(
            &closed.what,
            What::Closed { by: Some(Closer::Commit(c)), .. } if c == "abc"
        ));
    }
}
