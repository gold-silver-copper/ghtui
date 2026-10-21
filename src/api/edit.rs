//! Editing what's already on GitHub: comments and descriptions, titles,
//! reactions, and an issue's or pull request's sidebar (labels,
//! assignees, reviewers, milestone). Each edit is an [`Edit`], sent as a
//! [`Change::Edit`](crate::api::change::Change::Edit) by [`GitHub::edit`].

use crate::schema::schema;
use serde::{Deserialize, Serialize};

use crate::api::client::{ApiError, GitHub};
use crate::api::model::{Capped, NodeId, RepoId, count};
use crate::api::queries::{NumberVariables, NumberVariablesFields, fragments, nodes};

words! {
    /// One of GitHub's eight reactions, in the order GitHub shows them.
    #[derive(cynic::Enum, Hash)]
    pub enum ReactionContent {
        pub fn emoji,
        /// Its name, as GitHub's reaction picker labels it.
        pub fn label,
    }
    ThumbsUp   "👍" "+1";
    ThumbsDown "👎" "-1";
    Laugh      "😄" "Laugh";
    Hooray     "🎉" "Hooray";
    Confused   "😕" "Confused";
    Heart      "❤"  "Heart";
    Rocket     "🚀" "Rocket";
    Eyes       "👀" "Eyes";
}

/// How many reacted with one reaction, and whether you're one of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reaction {
    pub content: ReactionContent,
    pub count: u64,
    pub mine: bool,
}

/// A comment or a description on GitHub: what edits name it by, what
/// GitHub lets you do to it, and the reactions under it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Editable {
    pub id: NodeId,
    pub can_update: bool,
    pub can_delete: bool,
    pub can_react: bool,
    /// The reactions anyone gave, in GitHub's order.
    pub reactions: Vec<Reaction>,
}

impl Editable {
    /// Whether you gave `content`.
    pub fn reacted(&self, content: ReactionContent) -> bool {
        self.reactions
            .iter()
            .any(|r| r.content == content && r.mine)
    }
}

/// What GitHub lets you change in an issue's or pull request's sidebar.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SidebarPermits {
    pub labels: bool,
    pub assignees: bool,
    pub milestone: bool,
    /// A pull request's: asking for reviews.
    pub reviewers: bool,
}

/// A label, a person, a team or a milestone: its node ID, and the name
/// it goes by (a label's name, a login, `org/team`, a title).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Named {
    pub id: NodeId,
    pub name: String,
}

/// What a comment or a description is, which decides how it's edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Written {
    /// An issue's description.
    Issue,
    /// A pull request's description.
    Pr,
    IssueComment,
    /// A comment in a review thread.
    ReviewComment,
    /// A review's summary; deleting one throws away your pending review.
    Review,
}

/// An edit to something already on GitHub.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Edit {
    /// New text for a comment or a description.
    Body {
        of: Written,
        id: NodeId,
        body: String,
    },
    /// Delete a comment (an issue comment or a review comment).
    Delete { of: Written, id: NodeId },
    /// A new title for an issue (`pr: false`) or a pull request.
    Title { pr: bool, id: NodeId, title: String },
    /// Give your reaction (`on`), or take it back.
    React {
        subject: NodeId,
        content: ReactionContent,
        on: bool,
    },
    Labels {
        subject: NodeId,
        add: Vec<Named>,
        remove: Vec<Named>,
    },
    Assignees {
        subject: NodeId,
        add: Vec<Named>,
        remove: Vec<Named>,
    },
    /// Ask exactly these for a review: anyone asked before and left out
    /// is no longer asked.
    Reviewers {
        pr: NodeId,
        users: Vec<Named>,
        teams: Vec<Named>,
        bots: Vec<Named>,
    },
    /// Set (or, with `None`, clear) an issue's or pull request's milestone.
    Milestone {
        pr: bool,
        id: NodeId,
        milestone: Option<Named>,
    },
}

/// An issue's or pull request's sidebar as it is, with the node IDs
/// edits need, and what its repository offers to put there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sidebar {
    pub labels: Capped<Named>,
    pub assignees: Capped<Named>,
    pub milestone: Option<Named>,
    /// Who a pull request asks for a review; empty for an issue.
    pub requested: Requested,
    /// Its author, whom GitHub won't ask to review their own.
    pub author: Option<String>,
    /// The repository's labels.
    pub all_labels: Capped<Named>,
    /// Who can be assigned (and asked for a review).
    pub users: Capped<Named>,
    /// Its open milestones.
    pub milestones: Capped<Named>,
    /// A pull request's organization's teams, or why GitHub didn't list
    /// them (a token that can't read the organization); `None` for an
    /// issue, or a user's repository.
    pub teams: Option<Result<Capped<Named>, String>>,
}

/// The reviewers a pull request asks, by kind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Requested {
    pub users: Vec<Named>,
    pub teams: Vec<Named>,
    pub bots: Vec<Named>,
    /// Requests GitHub has that aren't here.
    pub left_out: u64,
}

impl GitHub {
    /// Makes `edit` on GitHub.
    pub async fn edit(&self, edit: &Edit) -> Result<(), ApiError> {
        use cynic::MutationBuilder;
        let ids = |list: &[Named]| list.iter().map(|n| n.id.gql()).collect::<Vec<_>>();
        match edit {
            Edit::Body { of, id, body } => {
                let vars = BodyVariables {
                    id: id.gql(),
                    body: body.clone(),
                };
                match of {
                    Written::Issue => self.mutate(UpdateIssueBody::build(vars)).await.map(drop),
                    Written::Pr => self.mutate(UpdatePrBody::build(vars)).await.map(drop),
                    Written::IssueComment => {
                        self.mutate(UpdateIssueComment::build(vars)).await.map(drop)
                    }
                    Written::ReviewComment => self
                        .mutate(UpdateReviewComment::build(vars))
                        .await
                        .map(drop),
                    Written::Review => self.mutate(UpdateReview::build(vars)).await.map(drop),
                }
            }
            Edit::Delete { of, id } => {
                let vars = EditIdVariables { id: id.gql() };
                match of {
                    Written::ReviewComment => self
                        .mutate(DeleteReviewComment::build(vars))
                        .await
                        .map(drop),
                    Written::Review => self.mutate(DeleteReview::build(vars)).await.map(drop),
                    _ => self.mutate(DeleteIssueComment::build(vars)).await.map(drop),
                }
            }
            Edit::Title { pr, id, title } => {
                let vars = TitleVariables {
                    id: id.gql(),
                    title: title.clone(),
                };
                if *pr {
                    self.mutate(UpdatePrTitle::build(vars)).await.map(drop)
                } else {
                    self.mutate(UpdateIssueTitle::build(vars)).await.map(drop)
                }
            }
            Edit::React {
                subject,
                content,
                on,
            } => {
                let vars = ReactVariables {
                    subject: subject.gql(),
                    content: *content,
                };
                if *on {
                    self.mutate(AddReaction::build(vars)).await.map(drop)
                } else {
                    self.mutate(RemoveReaction::build(vars)).await.map(drop)
                }
            }
            Edit::Labels {
                subject,
                add,
                remove,
            } => {
                if !add.is_empty() {
                    let vars = AddVariables {
                        subject: subject.gql(),
                        ids: Some(ids(add)),
                    };
                    self.mutate(AddLabels::build(vars)).await?;
                }
                if !remove.is_empty() {
                    let vars = RemoveVariables {
                        subject: subject.gql(),
                        ids: ids(remove),
                    };
                    self.mutate(RemoveLabels::build(vars)).await?;
                }
                Ok(())
            }
            Edit::Assignees {
                subject,
                add,
                remove,
            } => {
                if !add.is_empty() {
                    let vars = AddVariables {
                        subject: subject.gql(),
                        ids: Some(ids(add)),
                    };
                    self.mutate(AddAssignees::build(vars)).await?;
                }
                if !remove.is_empty() {
                    let vars = RemoveVariables {
                        subject: subject.gql(),
                        ids: ids(remove),
                    };
                    self.mutate(RemoveAssignees::build(vars)).await?;
                }
                Ok(())
            }
            Edit::Reviewers {
                pr,
                users,
                teams,
                bots,
            } => {
                let op = RequestReviews::build(ReviewersVariables {
                    pr: pr.gql(),
                    users: Some(ids(users)),
                    teams: Some(ids(teams)),
                    bots: Some(ids(bots)),
                });
                self.mutate(op).await.map(drop)
            }
            Edit::Milestone { pr, id, milestone } => {
                let vars = MilestoneVariables {
                    id: id.gql(),
                    milestone: milestone.as_ref().map(|m| m.id.gql()),
                };
                if *pr {
                    self.mutate(SetPrMilestone::build(vars)).await.map(drop)
                } else {
                    self.mutate(SetIssueMilestone::build(vars)).await.map(drop)
                }
            }
        }
    }

    /// Issue or pull request `number`'s sidebar, and what its repository
    /// offers to put there. Not cached: it's read when a picker opens.
    pub async fn sidebar(&self, repo: &RepoId, number: u64) -> Result<Sidebar, ApiError> {
        use cynic::QueryBuilder;
        let subject = format!("{repo}#{number}");
        let op = SidebarQuery::build(NumberVariables {
            owner: repo.owner.clone(),
            name: repo.name.clone(),
            number: i32::try_from(number).map_err(|_| ApiError::NotFound(subject.clone()))?,
        });
        let (mut sidebar, org) = self
            .find(op, subject, |q| q.repository?.into_sidebar())
            .await?;
        // Asked apart: a token that can't read the organization fails
        // only this.
        if let Some(org) = org {
            let op = TeamsQuery::build(TeamsVariables { org: org.clone() });
            let teams = self.find(op, &org, |q| {
                let t = q.organization?.teams;
                Some(Capped::from_nodes(t.total_count, t.nodes, |t| {
                    named(t.id, t.combined_slug)
                }))
            });
            sidebar.teams = Some(teams.await.map_err(|err| err.to_string()));
        }
        Ok(sidebar)
    }
}

// ---- reactions and permissions, as pages read them ---------------------------

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "ReactionGroup")]
pub struct WireReactionGroup {
    pub content: ReactionContent,
    pub viewer_has_reacted: bool,
    pub reactors: ReactorCount,
}

fragments! {
    total_count: i32;
    ReactorCount = "ReactorConnection",
}

/// The reactions anyone gave, from GitHub's groups (which list all eight).
pub(crate) fn reactions(groups: Option<Vec<WireReactionGroup>>) -> Vec<Reaction> {
    (groups.into_iter().flatten())
        .map(|g| Reaction {
            content: g.content,
            count: count(g.reactors.total_count),
            mine: g.viewer_has_reacted,
        })
        .filter(|r| r.count > 0 || r.mine)
        .collect()
}

// ---- the sidebar --------------------------------------------------------------

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Query", variables = "NumberVariables")]
pub struct SidebarQuery {
    #[arguments(owner: $owner, name: $name)]
    pub repository: Option<SidebarRepo>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Repository", variables = "NumberVariables")]
pub struct SidebarRepo {
    #[arguments(number: $number)]
    pub issue_or_pull_request: Option<SidebarOf>,
    #[arguments(first: 100)]
    pub labels: Option<LabelNames>,
    #[arguments(first: 100)]
    pub assignable_users: UserNames,
    #[arguments(first: 100, states: [OPEN])]
    pub milestones: Option<MilestoneNames>,
    pub owner: SidebarOwner,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "IssueOrPullRequest")]
pub enum SidebarOf {
    Issue(SidebarIssue),
    PullRequest(SidebarPr),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Issue")]
pub struct SidebarIssue {
    #[arguments(first: 100)]
    pub labels: Option<LabelNames>,
    #[arguments(first: 100)]
    pub assignees: UserNames,
    pub milestone: Option<MilestoneNamed>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequest")]
pub struct SidebarPr {
    #[arguments(first: 100)]
    pub labels: Option<LabelNames>,
    #[arguments(first: 100)]
    pub assignees: UserNames,
    pub milestone: Option<MilestoneNamed>,
    #[arguments(first: 100)]
    pub review_requests: Option<RequestNames>,
    pub author: Option<crate::api::queries::Actor>,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "RepositoryOwner")]
pub enum SidebarOwner {
    Organization(OrgLogin),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Organization")]
pub struct OrgLogin {
    pub login: String,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct TeamsVariables {
    pub org: String,
}

/// An organization's teams, for asking them for reviews.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Query", variables = "TeamsVariables")]
pub struct TeamsQuery {
    #[arguments(login: $org)]
    pub organization: Option<OrgTeams>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Organization")]
pub struct OrgTeams {
    #[arguments(first: 100)]
    pub teams: TeamNames,
}

fragments! {
    counted:
    LabelNames = "LabelConnection" => LabelNamed,
    UserNames = "UserConnection" => UserNamed,
    MilestoneNames = "MilestoneConnection" => MilestoneNamed,
    TeamNames = "TeamConnection" => TeamNamed,
    RequestNames = "ReviewRequestConnection" => RequestNamed,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Label")]
pub struct LabelNamed {
    pub id: cynic::Id,
    pub name: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "User")]
pub struct UserNamed {
    pub id: cynic::Id,
    pub login: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Bot")]
pub struct BotNamed {
    pub id: cynic::Id,
    pub login: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Milestone")]
pub struct MilestoneNamed {
    pub id: cynic::Id,
    pub title: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Team")]
pub struct TeamNamed {
    pub id: cynic::Id,
    /// `org/slug`.
    pub combined_slug: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "ReviewRequest")]
pub struct RequestNamed {
    pub requested_reviewer: Option<RequestedNamed>,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "RequestedReviewer")]
pub enum RequestedNamed {
    User(UserNamed),
    Bot(BotNamed),
    Team(TeamNamed),
    #[cynic(fallback)]
    Other,
}

fn named(id: cynic::Id, name: String) -> Named {
    Named {
        id: id.into(),
        name,
    }
}

fn label_names(list: Option<LabelNames>) -> Capped<Named> {
    list.map_or_else(Capped::default, |l| {
        Capped::from_nodes(l.total_count, l.nodes, |l| named(l.id, l.name))
    })
}

fn user_names(list: UserNames) -> Capped<Named> {
    Capped::from_nodes(list.total_count, list.nodes, |u| named(u.id, u.login))
}

impl SidebarRepo {
    /// The sidebar, and for a pull request in an organization's
    /// repository, the organization, whose teams it may ask.
    fn into_sidebar(self) -> Option<(Sidebar, Option<String>)> {
        let (labels, assignees, milestone, requests, author) = match self.issue_or_pull_request? {
            SidebarOf::Issue(i) => (i.labels, i.assignees, i.milestone, None, None),
            SidebarOf::PullRequest(p) => (
                p.labels,
                p.assignees,
                p.milestone,
                p.review_requests,
                Some(crate::api::model::author(p.author)),
            ),
            SidebarOf::Other => return None,
        };
        let mut requested = Requested::default();
        if let Some(r) = requests {
            let mut shown = 0;
            for asked in nodes(r.nodes).filter_map(|r| r.requested_reviewer) {
                shown += 1;
                match asked {
                    RequestedNamed::User(u) => requested.users.push(named(u.id, u.login)),
                    RequestedNamed::Bot(b) => requested.bots.push(named(b.id, b.login)),
                    RequestedNamed::Team(t) => requested.teams.push(named(t.id, t.combined_slug)),
                    // One GitHub's types don't cover (an enterprise team).
                    RequestedNamed::Other => shown -= 1,
                }
            }
            requested.left_out = count(r.total_count).saturating_sub(shown);
        }
        let org = match self.owner {
            SidebarOwner::Organization(o) if author.is_some() => Some(o.login),
            _ => None,
        };
        let sidebar = Sidebar {
            labels: label_names(labels),
            assignees: user_names(assignees),
            milestone: milestone.map(|m| named(m.id, m.title)),
            requested,
            author,
            all_labels: label_names(self.labels),
            users: user_names(self.assignable_users),
            milestones: self.milestones.map_or_else(Capped::default, |m| {
                Capped::from_nodes(m.total_count, m.nodes, |m| named(m.id, m.title))
            }),
            teams: None,
        };
        Some((sidebar, org))
    }
}

// ---- mutations ------------------------------------------------------------------

// Results of mutations that only need to succeed.
fragments! {
    client_mutation_id: Option<String>;
    UpdateIssuePayload = "UpdateIssuePayload",
    UpdatePrPayload = "UpdatePullRequestPayload",
    UpdateIssueCommentPayload = "UpdateIssueCommentPayload",
    UpdateReviewCommentPayload = "UpdatePullRequestReviewCommentPayload",
    DeleteIssueCommentPayload = "DeleteIssueCommentPayload",
    DeleteReviewCommentPayload = "DeletePullRequestReviewCommentPayload",
    UpdateReviewPayload = "UpdatePullRequestReviewPayload",
    DeleteReviewPayload = "DeletePullRequestReviewPayload",
    AddReactionPayload = "AddReactionPayload",
    RemoveReactionPayload = "RemoveReactionPayload",
    AddLabelsPayload = "AddLabelsToLabelablePayload",
    RemoveLabelsPayload = "RemoveLabelsFromLabelablePayload",
    AddAssigneesPayload = "AddAssigneesToAssignablePayload",
    RemoveAssigneesPayload = "RemoveAssigneesFromAssignablePayload",
    RequestReviewsPayload = "RequestReviewsPayload",
}

#[derive(cynic::QueryVariables, Debug)]
pub struct EditIdVariables {
    pub id: cynic::Id,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct BodyVariables {
    pub id: cynic::Id,
    pub body: String,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct TitleVariables {
    pub id: cynic::Id,
    pub title: String,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct ReactVariables {
    pub subject: cynic::Id,
    pub content: ReactionContent,
}

/// Labels or assignees to add.
#[derive(cynic::QueryVariables, Debug)]
pub struct AddVariables {
    pub subject: cynic::Id,
    pub ids: Option<Vec<cynic::Id>>,
}

/// Labels or assignees to remove.
#[derive(cynic::QueryVariables, Debug)]
pub struct RemoveVariables {
    pub subject: cynic::Id,
    pub ids: Vec<cynic::Id>,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct ReviewersVariables {
    pub pr: cynic::Id,
    pub users: Option<Vec<cynic::Id>>,
    pub teams: Option<Vec<cynic::Id>>,
    pub bots: Option<Vec<cynic::Id>>,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct MilestoneVariables {
    pub id: cynic::Id,
    /// `None` clears it: sent as null.
    pub milestone: Option<cynic::Id>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "BodyVariables")]
pub struct UpdateIssueBody {
    #[arguments(input: { id: $id, body: $body })]
    pub update_issue: Option<UpdateIssuePayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "BodyVariables")]
pub struct UpdatePrBody {
    #[arguments(input: { pullRequestId: $id, body: $body })]
    pub update_pull_request: Option<UpdatePrPayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "BodyVariables")]
pub struct UpdateIssueComment {
    #[arguments(input: { id: $id, body: $body })]
    pub update_issue_comment: Option<UpdateIssueCommentPayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "BodyVariables")]
pub struct UpdateReviewComment {
    #[arguments(input: { pullRequestReviewCommentId: $id, body: $body })]
    pub update_pull_request_review_comment: Option<UpdateReviewCommentPayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "EditIdVariables")]
pub struct DeleteIssueComment {
    #[arguments(input: { id: $id })]
    pub delete_issue_comment: Option<DeleteIssueCommentPayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "EditIdVariables")]
pub struct DeleteReviewComment {
    #[arguments(input: { id: $id })]
    pub delete_pull_request_review_comment: Option<DeleteReviewCommentPayload>,
}

/// A review's summary.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "BodyVariables")]
pub struct UpdateReview {
    #[arguments(input: { pullRequestReviewId: $id, body: $body })]
    pub update_pull_request_review: Option<UpdateReviewPayload>,
}

/// Your pending review, thrown away with its comments.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "EditIdVariables")]
pub struct DeleteReview {
    #[arguments(input: { pullRequestReviewId: $id })]
    pub delete_pull_request_review: Option<DeleteReviewPayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "TitleVariables")]
pub struct UpdateIssueTitle {
    #[arguments(input: { id: $id, title: $title })]
    pub update_issue: Option<UpdateIssuePayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "TitleVariables")]
pub struct UpdatePrTitle {
    #[arguments(input: { pullRequestId: $id, title: $title })]
    pub update_pull_request: Option<UpdatePrPayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "ReactVariables")]
pub struct AddReaction {
    #[arguments(input: { subjectId: $subject, content: $content })]
    pub add_reaction: Option<AddReactionPayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "ReactVariables")]
pub struct RemoveReaction {
    #[arguments(input: { subjectId: $subject, content: $content })]
    pub remove_reaction: Option<RemoveReactionPayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "AddVariables")]
pub struct AddLabels {
    #[arguments(input: { labelableId: $subject, labelIds: $ids })]
    pub add_labels_to_labelable: Option<AddLabelsPayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "RemoveVariables")]
pub struct RemoveLabels {
    #[arguments(input: { labelableId: $subject, labelIds: $ids })]
    pub remove_labels_from_labelable: Option<RemoveLabelsPayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "AddVariables")]
pub struct AddAssignees {
    #[arguments(input: { assignableId: $subject, assigneeIds: $ids })]
    pub add_assignees_to_assignable: Option<AddAssigneesPayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "RemoveVariables")]
pub struct RemoveAssignees {
    #[arguments(input: { assignableId: $subject, assigneeIds: $ids })]
    pub remove_assignees_from_assignable: Option<RemoveAssigneesPayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "ReviewersVariables")]
pub struct RequestReviews {
    #[arguments(input: { pullRequestId: $pr, userIds: $users, teamIds: $teams, botIds: $bots })]
    pub request_reviews: Option<RequestReviewsPayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "MilestoneVariables")]
pub struct SetIssueMilestone {
    #[arguments(input: { id: $id, milestoneId: $milestone })]
    pub update_issue: Option<UpdateIssuePayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "MilestoneVariables")]
pub struct SetPrMilestone {
    #[arguments(input: { pullRequestId: $id, milestoneId: $milestone })]
    pub update_pull_request: Option<UpdatePrPayload>,
}
