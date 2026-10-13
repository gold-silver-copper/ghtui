//! An issue's or pull request's timeline: its comments, reviews and
//! events in GitHub's own order (`timelineItems`), and how it connects to
//! other issues and pull requests.
//!
//! The wire types are typed against GitHub's schema; the models before
//! them are what the app renders and caches.

use serde::{Deserialize, Serialize};

use crate::api::browse::{
    BotLogin, Comment, CommitCard, CommitInfo, RequestedReviewer, Reviewer, StateReason, UserLogin,
    WireComment, WireIssueState, comment,
};
use crate::api::edit::{Editable, WireReactionGroup, reactions};
use crate::api::model::{
    Capped, IssueState, Label, PrState, PrStatus, RepoId, ReviewState, Status, author, count,
};
use crate::api::queries::{
    Actor, BigInt, CommitOid, DateTime, PageVariablesFields, RepositoryName, fragments, nodes,
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

/// Events that are only who and when.
macro_rules! bare_events {
    ($($name:ident = $graphql:literal),* $(,)?) => {$(
        #[derive(cynic::QueryFragment, Debug)]
        #[cynic(graphql_type = $graphql, schema_module = "schema")]
        pub struct $name {
            pub actor: Option<Actor>,
            pub created_at: DateTime,
        }
    )*};
}

bare_events! {
    ReopenedEv = "ReopenedEvent",
    ReadyEv = "ReadyForReviewEvent",
    DraftEv = "ConvertToDraftEvent",
    RestoredEv = "HeadRefRestoredEvent",
    UnlockedEv = "UnlockedEvent",
    PinnedEv = "PinnedEvent",
    UnpinnedEv = "UnpinnedEvent",
    AutoMergeOnEv = "AutoMergeEnabledEvent",
    AutoMergeOffEv = "AutoMergeDisabledEvent",
    QueuedEv = "AddedToMergeQueueEvent",
    DiscussionEv = "ConvertedToDiscussionEvent",
    ProjectAddedEv = "AddedToProjectV2Event",
    ProjectRemovedEv = "RemovedFromProjectV2Event",
}

/// An issue a timeline names.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Issue", schema_module = "schema")]
pub struct LinkedIssue {
    pub number: i32,
    pub title: String,
    pub state: WireIssueState,
    pub state_reason: Option<StateReason>,
    pub repository: RepositoryName,
}

/// A pull request a timeline names.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequest", schema_module = "schema")]
pub struct LinkedPr {
    pub number: i32,
    pub title: String,
    #[cynic(spread)]
    pub status: PrStatus,
    pub repository: RepositoryName,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "ReferencedSubject", schema_module = "schema")]
pub enum Subject {
    Issue(LinkedIssue),
    PullRequest(LinkedPr),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "IssueOrPullRequest", schema_module = "schema")]
pub enum IssueOrPrLink {
    Issue(LinkedIssue),
    PullRequest(LinkedPr),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "Closer", schema_module = "schema")]
pub enum WireCloser {
    Commit(CommitOid),
    PullRequest(LinkedPr),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "Assignee", schema_module = "schema")]
pub enum WireAssignee {
    User(UserLogin),
    Bot(BotLogin),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::Enum, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cynic(graphql_type = "LockReason", schema_module = "schema")]
pub enum LockReason {
    OffTopic,
    Resolved,
    Spam,
    TooHeated,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "IssueType", schema_module = "schema")]
pub struct TypeName {
    pub name: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "ClosedEvent", schema_module = "schema")]
pub struct ClosedEv {
    pub actor: Option<Actor>,
    pub created_at: DateTime,
    pub closer: Option<WireCloser>,
    pub state_reason: Option<StateReason>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "MergedEvent", schema_module = "schema")]
pub struct MergedEv {
    pub actor: Option<Actor>,
    pub created_at: DateTime,
    pub commit: Option<CommitOid>,
    pub merge_ref_name: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "CrossReferencedEvent", schema_module = "schema")]
pub struct CrossReferencedEv {
    pub actor: Option<Actor>,
    pub created_at: DateTime,
    pub source: Subject,
    pub will_close_target: bool,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "ReferencedEvent", schema_module = "schema")]
pub struct ReferencedEv {
    pub actor: Option<Actor>,
    pub created_at: DateTime,
    pub commit: Option<CommitOid>,
    pub commit_repository: RepositoryName,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "LabeledEvent", schema_module = "schema")]
pub struct LabeledEv {
    pub actor: Option<Actor>,
    pub created_at: DateTime,
    pub label: crate::api::queries::Label,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "UnlabeledEvent", schema_module = "schema")]
pub struct UnlabeledEv {
    pub actor: Option<Actor>,
    pub created_at: DateTime,
    pub label: crate::api::queries::Label,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "AssignedEvent", schema_module = "schema")]
pub struct AssignedEv {
    pub actor: Option<Actor>,
    pub created_at: DateTime,
    pub assignee: Option<WireAssignee>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "UnassignedEvent", schema_module = "schema")]
pub struct UnassignedEv {
    pub actor: Option<Actor>,
    pub created_at: DateTime,
    pub assignee: Option<WireAssignee>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "MilestonedEvent", schema_module = "schema")]
pub struct MilestonedEv {
    pub actor: Option<Actor>,
    pub created_at: DateTime,
    pub milestone_title: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "DemilestonedEvent", schema_module = "schema")]
pub struct DemilestonedEv {
    pub actor: Option<Actor>,
    pub created_at: DateTime,
    pub milestone_title: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "ProjectV2ItemStatusChangedEvent",
    schema_module = "schema"
)]
pub struct StatusChangedEv {
    pub actor: Option<Actor>,
    pub created_at: DateTime,
    pub previous_status: String,
    pub status: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "ReviewRequestedEvent", schema_module = "schema")]
pub struct ReviewRequestedEv {
    pub actor: Option<Actor>,
    pub created_at: DateTime,
    pub requested_reviewer: Option<RequestedReviewer>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "ReviewRequestRemovedEvent", schema_module = "schema")]
pub struct ReviewUnrequestedEv {
    pub actor: Option<Actor>,
    pub created_at: DateTime,
    pub requested_reviewer: Option<RequestedReviewer>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequestReview", schema_module = "schema")]
pub struct ReviewAuthor {
    pub author: Option<Actor>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "ReviewDismissedEvent", schema_module = "schema")]
pub struct DismissedEv {
    pub actor: Option<Actor>,
    pub created_at: DateTime,
    pub dismissal_message: Option<String>,
    pub review: Option<ReviewAuthor>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RenamedTitleEvent", schema_module = "schema")]
pub struct RenamedEv {
    pub actor: Option<Actor>,
    pub created_at: DateTime,
    pub previous_title: String,
    pub current_title: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "HeadRefForcePushedEvent", schema_module = "schema")]
pub struct HeadPushedEv {
    pub actor: Option<Actor>,
    pub created_at: DateTime,
    pub before_commit: Option<CommitOid>,
    pub after_commit: Option<CommitOid>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "BaseRefForcePushedEvent", schema_module = "schema")]
pub struct BasePushedEv {
    pub actor: Option<Actor>,
    pub created_at: DateTime,
    pub before_commit: Option<CommitOid>,
    pub after_commit: Option<CommitOid>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "HeadRefDeletedEvent", schema_module = "schema")]
pub struct DeletedEv {
    pub actor: Option<Actor>,
    pub created_at: DateTime,
    pub head_ref_name: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "BaseRefChangedEvent", schema_module = "schema")]
pub struct BaseChangedEv {
    pub actor: Option<Actor>,
    pub created_at: DateTime,
    pub previous_ref_name: String,
    pub current_ref_name: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "LockedEvent", schema_module = "schema")]
pub struct LockedEv {
    pub actor: Option<Actor>,
    pub created_at: DateTime,
    pub lock_reason: Option<LockReason>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "TransferredEvent", schema_module = "schema")]
pub struct TransferredEv {
    pub actor: Option<Actor>,
    pub created_at: DateTime,
    pub from_repository: Option<RepositoryName>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "MarkedAsDuplicateEvent", schema_module = "schema")]
pub struct DuplicateEv {
    pub actor: Option<Actor>,
    pub created_at: DateTime,
    pub canonical: Option<IssueOrPrLink>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "UnmarkedAsDuplicateEvent", schema_module = "schema")]
pub struct NotDuplicateEv {
    pub actor: Option<Actor>,
    pub created_at: DateTime,
    pub canonical: Option<IssueOrPrLink>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RemovedFromMergeQueueEvent", schema_module = "schema")]
pub struct DequeuedEv {
    pub actor: Option<Actor>,
    pub created_at: DateTime,
    pub reason: Option<String>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "SubIssueAddedEvent", schema_module = "schema")]
pub struct SubAddedEv {
    pub actor: Option<Actor>,
    pub created_at: DateTime,
    pub sub_issue: Option<LinkedIssue>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "SubIssueRemovedEvent", schema_module = "schema")]
pub struct SubRemovedEv {
    pub actor: Option<Actor>,
    pub created_at: DateTime,
    pub sub_issue: Option<LinkedIssue>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "ParentIssueAddedEvent", schema_module = "schema")]
pub struct ParentAddedEv {
    pub actor: Option<Actor>,
    pub created_at: DateTime,
    pub parent: Option<LinkedIssue>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "ParentIssueRemovedEvent", schema_module = "schema")]
pub struct ParentRemovedEv {
    pub actor: Option<Actor>,
    pub created_at: DateTime,
    pub parent: Option<LinkedIssue>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "IssueTypeAddedEvent", schema_module = "schema")]
pub struct TypeAddedEv {
    pub actor: Option<Actor>,
    pub created_at: DateTime,
    pub issue_type: Option<TypeName>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "IssueTypeChangedEvent", schema_module = "schema")]
pub struct TypeChangedEv {
    pub actor: Option<Actor>,
    pub created_at: DateTime,
    pub issue_type: Option<TypeName>,
    pub prev_issue_type: Option<TypeName>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "IssueTypeRemovedEvent", schema_module = "schema")]
pub struct TypeRemovedEv {
    pub actor: Option<Actor>,
    pub created_at: DateTime,
    pub issue_type: Option<TypeName>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequestCommit", schema_module = "schema")]
pub struct CommitEv {
    pub commit: CommitCard,
}

/// A review, as the timeline and the list of reviews have it.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequestReview", schema_module = "schema")]
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
#[cynic(graphql_type = "PullRequestTimelineItems", schema_module = "schema")]
pub enum PrItem {
    IssueComment(Box<WireComment>),
    PullRequestReview(Box<WireReview>),
    PullRequestCommit(CommitEv),
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
#[cynic(graphql_type = "IssueTimelineItems", schema_module = "schema")]
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
    Closing = "IssueConnection" => LinkedIssue,
    ClosedBy = "PullRequestConnection" => LinkedPr,
    SubIssues = "IssueConnection" => LinkedIssue,
    Branches = "LinkedBranchConnection" => WireLinkedBranch,
    LatestReviews = "PullRequestReviewConnection" => WireLatest,
    Requests = "ReviewRequestConnection" => WireRequest,
    PendingReviews = "PullRequestReviewConnection" => WireReview,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "LinkedBranch", schema_module = "schema")]
pub struct WireLinkedBranch {
    #[cynic(rename = "ref")]
    pub branch: Option<RefName>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Ref", schema_module = "schema")]
pub struct RefName {
    pub name: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "SubIssuesSummary", schema_module = "schema")]
pub struct WireProgress {
    pub completed: i32,
    pub total: i32,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Team", schema_module = "schema")]
pub struct TeamName {
    /// `org/slug`.
    pub combined_slug: String,
}

fragments! {
    counted:
    OnBehalfOf = "TeamConnection" => TeamName,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequestReview", schema_module = "schema")]
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
#[cynic(graphql_type = "PullRequestReview", schema_module = "schema")]
pub struct Opinion {
    pub state: ReviewState,
}

fragments! {
    counted:
    Opinions = "PullRequestReviewConnection" => Opinion,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PinnedIssue", schema_module = "schema")]
pub struct PinnedIssue {
    pub issue: LinkedIssue,
}

fragments! {
    counted:
    PinnedIssues = "PinnedIssueConnection" => PinnedIssue,
}

/// A repository's pinned issues.
pub(crate) fn pinned_issues(p: PinnedIssues) -> Capped<Linked> {
    let items = nodes(p.nodes).filter_map(|p| p.issue.into_linked());
    Capped::new(items.collect(), count(p.total_count))
}

/// How many reviewers' latest word is an approval.
pub(crate) fn approvals(o: Option<Opinions>) -> u64 {
    let approved = o.map(|o| nodes(o.nodes).filter(|r| r.state == ReviewState::Approved));
    approved.map_or(0, |a| a.count() as u64)
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "ReviewRequest", schema_module = "schema")]
pub struct WireRequest {
    pub as_code_owner: bool,
    pub requested_reviewer: Option<RequestedReviewer>,
}

// ---- from the wire ---------------------------------------------------------------

fn actor(a: Option<Actor>) -> String {
    author(a)
}

fn event(actor_: Option<Actor>, at: DateTime, what: What) -> Item {
    Item::Event(Event {
        actor: actor(actor_),
        at: at.0,
        what,
    })
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

/// The events both kinds of timeline have, made into items.
macro_rules! shared_events {
    ($item:expr, $ty:ident, $($extra:tt)*) => {
        match $item {
            $ty::IssueComment(c) => Some(Item::Comment(comment(*c))),
            $ty::ClosedEvent(e) => {
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
                Some(event(e.actor, e.created_at, What::Closed { by, reason, duplicate }))
            }
            $ty::ReopenedEvent(e) => Some(event(e.actor, e.created_at, What::Reopened)),
            $ty::CrossReferencedEvent(e) => {
                let from = e.source.into_linked()?;
                let will_close = e.will_close_target;
                Some(event(e.actor, e.created_at, What::CrossReferenced { from, will_close }))
            }
            $ty::ReferencedEvent(e) => {
                let commit = oid(e.commit)?;
                let repo = RepoId::parse(&e.commit_repository.name_with_owner);
                Some(event(e.actor, e.created_at, What::Referenced { commit, repo }))
            }
            $ty::LabeledEvent(e) => {
                let label = Label { name: e.label.name, color: e.label.color };
                Some(event(e.actor, e.created_at, What::Labeled { label, added: true }))
            }
            $ty::UnlabeledEvent(e) => {
                let label = Label { name: e.label.name, color: e.label.color };
                Some(event(e.actor, e.created_at, What::Labeled { label, added: false }))
            }
            $ty::AssignedEvent(e) => {
                let who = assignee(e.assignee);
                Some(event(e.actor, e.created_at, What::Assigned { who, added: true }))
            }
            $ty::UnassignedEvent(e) => {
                let who = assignee(e.assignee);
                Some(event(e.actor, e.created_at, What::Assigned { who, added: false }))
            }
            $ty::MilestonedEvent(e) => {
                let title = e.milestone_title;
                Some(event(e.actor, e.created_at, What::Milestoned { title, added: true }))
            }
            $ty::DemilestonedEvent(e) => {
                let title = e.milestone_title;
                Some(event(e.actor, e.created_at, What::Milestoned { title, added: false }))
            }
            $ty::AddedToProjectV2Event(e) => {
                Some(event(e.actor, e.created_at, What::Project { added: true }))
            }
            $ty::RemovedFromProjectV2Event(e) => {
                Some(event(e.actor, e.created_at, What::Project { added: false }))
            }
            $ty::ProjectV2ItemStatusChangedEvent(e) => {
                let what = What::ProjectStatus { from: e.previous_status, to: e.status };
                Some(event(e.actor, e.created_at, what))
            }
            $ty::RenamedTitleEvent(e) => {
                let what = What::Renamed { from: e.previous_title, to: e.current_title };
                Some(event(e.actor, e.created_at, what))
            }
            $ty::LockedEvent(e) => {
                let reason = e.lock_reason.map(|r| crate::api::manage::lock_said(r).to_owned());
                Some(event(e.actor, e.created_at, What::Locked { reason }))
            }
            $ty::UnlockedEvent(e) => Some(event(e.actor, e.created_at, What::Unlocked)),
            $ty::MarkedAsDuplicateEvent(e) => {
                let of = e.canonical.and_then(IssueOrPrLink::into_linked);
                Some(event(e.actor, e.created_at, What::MarkedDuplicate { of, undone: false }))
            }
            $ty::UnmarkedAsDuplicateEvent(e) => {
                let of = e.canonical.and_then(IssueOrPrLink::into_linked);
                Some(event(e.actor, e.created_at, What::MarkedDuplicate { of, undone: true }))
            }
            $($extra)*
        }
    };
}

impl PrItem {
    pub(crate) fn into_item(self) -> Option<Item> {
        shared_events!(self, PrItem,
            PrItem::PullRequestReview(r) => Some(Item::Review(r.into_review())),
            PrItem::PullRequestCommit(c) => {
                let at = c.commit.committed_date.0.clone();
                let info = c.commit.into_info();
                let actor = info.author.login().unwrap_or_default().to_owned();
                Some(Item::Event(Event { actor, at, what: What::Committed(info) }))
            }
            PrItem::MergedEvent(e) => {
                let what = What::Merged { commit: oid(e.commit), into: e.merge_ref_name };
                Some(event(e.actor, e.created_at, what))
            }
            PrItem::ReviewRequestedEvent(e) => {
                let who = reviewer(e.requested_reviewer);
                Some(event(e.actor, e.created_at, What::ReviewRequested { who, added: true }))
            }
            PrItem::ReviewRequestRemovedEvent(e) => {
                let who = reviewer(e.requested_reviewer);
                Some(event(e.actor, e.created_at, What::ReviewRequested { who, added: false }))
            }
            PrItem::ReviewDismissedEvent(e) => {
                let of = e.review.map(|r| author(r.author));
                let message = e.dismissal_message.unwrap_or_default();
                Some(event(e.actor, e.created_at, What::ReviewDismissed { of, message }))
            }
            PrItem::ReadyForReviewEvent(e) => Some(event(e.actor, e.created_at, What::ReadyForReview)),
            PrItem::ConvertToDraftEvent(e) => Some(event(e.actor, e.created_at, What::ConvertedToDraft)),
            PrItem::HeadRefForcePushedEvent(e) => {
                let what = What::ForcePushed {
                    base: false,
                    before: oid(e.before_commit),
                    after: oid(e.after_commit),
                };
                Some(event(e.actor, e.created_at, what))
            }
            PrItem::BaseRefForcePushedEvent(e) => {
                let what = What::ForcePushed {
                    base: true,
                    before: oid(e.before_commit),
                    after: oid(e.after_commit),
                };
                Some(event(e.actor, e.created_at, what))
            }
            PrItem::HeadRefDeletedEvent(e) => {
                let what = What::BranchDeleted { name: e.head_ref_name };
                Some(event(e.actor, e.created_at, what))
            }
            PrItem::HeadRefRestoredEvent(e) => Some(event(e.actor, e.created_at, What::BranchRestored)),
            PrItem::BaseRefChangedEvent(e) => {
                let what = What::BaseChanged { from: e.previous_ref_name, to: e.current_ref_name };
                Some(event(e.actor, e.created_at, what))
            }
            PrItem::AutoMergeEnabledEvent(e) => Some(event(e.actor, e.created_at, What::AutoMerge { on: true })),
            PrItem::AutoMergeDisabledEvent(e) => Some(event(e.actor, e.created_at, What::AutoMerge { on: false })),
            PrItem::AddedToMergeQueueEvent(e) => {
                Some(event(e.actor, e.created_at, What::MergeQueue { added: true, reason: None }))
            }
            PrItem::RemovedFromMergeQueueEvent(e) => {
                let reason = e.reason.filter(|r| !r.trim().is_empty());
                Some(event(e.actor, e.created_at, What::MergeQueue { added: false, reason }))
            }
            PrItem::Other => None,
        )
    }
}

impl IssueItem {
    pub(crate) fn into_item(self) -> Option<Item> {
        shared_events!(self, IssueItem,
            IssueItem::PinnedEvent(e) => Some(event(e.actor, e.created_at, What::Pinned { pinned: true })),
            IssueItem::UnpinnedEvent(e) => Some(event(e.actor, e.created_at, What::Pinned { pinned: false })),
            IssueItem::TransferredEvent(e) => {
                let from = e.from_repository.map(|r| r.name_with_owner);
                Some(event(e.actor, e.created_at, What::Transferred { from }))
            }
            IssueItem::SubIssueAddedEvent(e) => {
                let issue = e.sub_issue.and_then(LinkedIssue::into_linked);
                Some(event(e.actor, e.created_at, What::SubIssue { issue, added: true }))
            }
            IssueItem::SubIssueRemovedEvent(e) => {
                let issue = e.sub_issue.and_then(LinkedIssue::into_linked);
                Some(event(e.actor, e.created_at, What::SubIssue { issue, added: false }))
            }
            IssueItem::ParentIssueAddedEvent(e) => {
                let issue = e.parent.and_then(LinkedIssue::into_linked);
                Some(event(e.actor, e.created_at, What::Parent { issue, added: true }))
            }
            IssueItem::ParentIssueRemovedEvent(e) => {
                let issue = e.parent.and_then(LinkedIssue::into_linked);
                Some(event(e.actor, e.created_at, What::Parent { issue, added: false }))
            }
            IssueItem::IssueTypeAddedEvent(e) => {
                let what = What::IssueType { from: None, to: e.issue_type.map(|t| t.name) };
                Some(event(e.actor, e.created_at, what))
            }
            IssueItem::IssueTypeChangedEvent(e) => {
                let what = What::IssueType {
                    from: e.prev_issue_type.map(|t| t.name),
                    to: e.issue_type.map(|t| t.name),
                };
                Some(event(e.actor, e.created_at, what))
            }
            IssueItem::IssueTypeRemovedEvent(e) => {
                let what = What::IssueType { from: e.issue_type.map(|t| t.name), to: None };
                Some(event(e.actor, e.created_at, what))
            }
            IssueItem::ConvertedToDiscussionEvent(e) => {
                Some(event(e.actor, e.created_at, What::ConvertedToDiscussion))
            }
            IssueItem::Other => None,
        )
    }
}

/// A timeline's items, oldest first, of how many there are in all.
/// Your pending review, which the timeline may not have, comes last.
pub(crate) fn pr_timeline(t: PrTimeline, pending: Option<PendingReviews>) -> Capped<Item> {
    let fetched = t.nodes.as_ref().map_or(0, Vec::len) as u64;
    let mut items: Vec<Item> = nodes(t.nodes).filter_map(PrItem::into_item).collect();
    // What ghtui doesn't show isn't left out.
    let unshown = fetched.saturating_sub(items.len() as u64);
    let total = count(t.total_count).saturating_sub(unshown);
    let has_pending = items
        .iter()
        .any(|i| matches!(i, Item::Review(r) if r.state == ReviewState::Pending));
    let pending = pending.and_then(|p| nodes(p.nodes).next());
    if let Some(p) = pending.filter(|_| !has_pending) {
        items.push(Item::Review(p.into_review()));
        return Capped::new(items, total + 1);
    }
    Capped::new(items, total)
}

pub(crate) fn issue_timeline(t: IssueTimeline) -> Capped<Item> {
    let fetched = t.nodes.as_ref().map_or(0, Vec::len) as u64;
    let items: Vec<Item> = nodes(t.nodes).filter_map(IssueItem::into_item).collect();
    let unshown = fetched.saturating_sub(items.len() as u64);
    Capped::new(items, count(t.total_count).saturating_sub(unshown))
}

pub(crate) fn linked_issues(c: Closing) -> Capped<Linked> {
    let items = nodes(c.nodes)
        .filter_map(LinkedIssue::into_linked)
        .collect();
    Capped::new(items, count(c.total_count))
}

pub(crate) fn sub_issues(c: SubIssues) -> Capped<Linked> {
    let items = nodes(c.nodes)
        .filter_map(LinkedIssue::into_linked)
        .collect();
    Capped::new(items, count(c.total_count))
}

pub(crate) fn linked_prs(c: ClosedBy) -> Capped<Linked> {
    let items = nodes(c.nodes).filter_map(LinkedPr::into_linked).collect();
    Capped::new(items, count(c.total_count))
}

pub(crate) fn branches(b: Branches) -> Capped<String> {
    let items = nodes(b.nodes)
        .filter_map(|b| Some(b.branch?.name))
        .collect();
    Capped::new(items, count(b.total_count))
}

pub(crate) fn progress(p: &WireProgress) -> Option<Progress> {
    (p.total > 0).then(|| Progress {
        done: count(p.completed),
        total: count(p.total),
    })
}

pub(crate) fn latest(l: Option<LatestReviews>) -> Capped<Latest> {
    let Some(l) = l else {
        return Capped::default();
    };
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
}

/// Who a pull request's review requests name; one GitHub's types don't
/// cover (an enterprise team) counts as left out.
pub(crate) fn requested(r: Option<Requests>) -> Capped<Requested> {
    let Some(r) = r else {
        return Capped::default();
    };
    let named = nodes(r.nodes).filter_map(|r| {
        Some(Requested {
            who: reviewer(r.requested_reviewer)?,
            code_owner: r.as_code_owner,
        })
    });
    Capped::new(named.collect(), count(r.total_count))
}

// ---- a pull request's reviews ----------------------------------------------------------

/// A page of a pull request's reviews, oldest first.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Query",
    schema_module = "schema",
    variables = "PageVariables"
)]
pub struct ReviewsQuery {
    #[arguments(owner: $owner, name: $name)]
    pub repository: Option<RepoReviews>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Repository",
    schema_module = "schema",
    variables = "PageVariables"
)]
pub struct RepoReviews {
    #[arguments(number: $number)]
    pub pull_request: Option<PrReviews>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "PullRequest",
    schema_module = "schema",
    variables = "PageVariables"
)]
pub struct PrReviews {
    #[arguments(first: 100, after: $after)]
    pub reviews: Option<ReviewsPage>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequestReviewConnection", schema_module = "schema")]
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
