//! Editing what's already on GitHub: comments and descriptions, titles,
//! reactions, and an issue's or pull request's sidebar (labels,
//! assignees, reviewers, milestone, an issue's type). Each edit is an [`Edit`], sent as a
//! [`Change::Edit`](crate::api::change::Change::Edit) by [`GitHub::edit`].

use crate::schema::schema;
use serde::{Deserialize, Serialize};

use crate::api::change::{IdVariablesFields, mutations};
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
    /// Set (or, with `None`, clear) an issue's type.
    IssueType {
        issue: NodeId,
        issue_type: Option<Named>,
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
    /// An issue's type.
    pub issue_type: Option<Named>,
    /// The types its repository's issues may have (those turned on): none
    /// for a pull request, or where the owner has none.
    pub issue_types: Capped<Named>,
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

/// The node IDs of `list`.
fn ids(list: &[Named]) -> Vec<cynic::Id> {
    list.iter().map(|n| n.id.gql()).collect()
}

impl GitHub {
    /// Makes `edit` on GitHub.
    pub async fn edit(&self, edit: &Edit) -> Result<(), ApiError> {
        match edit {
            Edit::Body { of, id, body } => {
                let vars = BodyVariables {
                    id: id.gql(),
                    body: body.clone(),
                };
                match of {
                    Written::Issue => self.run::<UpdateIssueBody, _>(vars).await,
                    Written::Pr => self.run::<UpdatePrBody, _>(vars).await,
                    Written::IssueComment => self.run::<UpdateIssueComment, _>(vars).await,
                    Written::ReviewComment => self.run::<UpdateReviewComment, _>(vars).await,
                    Written::Review => self.run::<UpdateReview, _>(vars).await,
                }
            }
            Edit::Delete { of, id } => match of {
                Written::ReviewComment => self.run_on::<DeleteReviewComment>(id).await,
                Written::Review => self.run_on::<DeleteReview>(id).await,
                _ => self.run_on::<DeleteIssueComment>(id).await,
            },
            Edit::Title { pr, id, title } => {
                let vars = TitleVariables {
                    id: id.gql(),
                    title: title.clone(),
                };
                if *pr {
                    self.run::<UpdatePrTitle, _>(vars).await
                } else {
                    self.run::<UpdateIssueTitle, _>(vars).await
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
                    self.run::<AddReaction, _>(vars).await
                } else {
                    self.run::<RemoveReaction, _>(vars).await
                }
            }
            Edit::Labels {
                subject,
                add,
                remove,
            } => {
                self.add_remove::<AddLabels, RemoveLabels>(subject, add, remove)
                    .await
            }
            Edit::Assignees {
                subject,
                add,
                remove,
            } => {
                self.add_remove::<AddAssignees, RemoveAssignees>(subject, add, remove)
                    .await
            }
            Edit::Reviewers {
                pr,
                users,
                teams,
                bots,
            } => {
                self.run::<RequestReviews, _>(ReviewersVariables {
                    pr: pr.gql(),
                    users: Some(ids(users)),
                    teams: Some(ids(teams)),
                    bots: Some(ids(bots)),
                })
                .await
            }
            Edit::Milestone { pr, id, milestone } => {
                let vars = MilestoneVariables {
                    id: id.gql(),
                    milestone: milestone.as_ref().map(|m| m.id.gql()),
                };
                if *pr {
                    self.run::<SetPrMilestone, _>(vars).await
                } else {
                    self.run::<SetIssueMilestone, _>(vars).await
                }
            }
            Edit::IssueType { issue, issue_type } => {
                self.run::<SetIssueType, _>(IssueTypeVariables {
                    issue: issue.gql(),
                    issue_type: issue_type.as_ref().map(|t| t.id.gql()),
                })
                .await
            }
        }
    }

    /// Adds `add` to what `subject` has with `A`, and takes `remove` off
    /// with `R`; an empty list sends nothing.
    async fn add_remove<A, R>(
        &self,
        subject: &NodeId,
        add: &[Named],
        remove: &[Named],
    ) -> Result<(), ApiError>
    where
        A: cynic::MutationBuilder<AddVariables> + serde::de::DeserializeOwned,
        R: cynic::MutationBuilder<RemoveVariables> + serde::de::DeserializeOwned,
    {
        if !add.is_empty() {
            let ids = Some(ids(add));
            self.run::<A, _>(AddVariables {
                subject: subject.gql(),
                ids,
            })
            .await?;
        }
        if !remove.is_empty() {
            let ids = ids(remove);
            self.run::<R, _>(RemoveVariables {
                subject: subject.gql(),
                ids,
            })
            .await?;
        }
        Ok(())
    }

    /// Issue or pull request `number`'s sidebar, and what its repository
    /// offers to put there. Not cached: it's read when a picker opens.
    pub async fn sidebar(&self, repo: &RepoId, number: u64) -> Result<Sidebar, ApiError> {
        use cynic::QueryBuilder;
        let subject = format!("{repo}#{number}");
        let op = SidebarQuery::build(NumberVariables {
            owner: repo.owner.to_string(),
            name: repo.name.to_string(),
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
                Some(q.organization?.teams.into_capped(Named::from))
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

fragments! {
    root "Query" repository(owner, name);
    SidebarQuery("NumberVariables") => SidebarRepo,
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
    #[arguments(first: 100)]
    pub issue_types: Option<IssueTypeNames>,
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
    pub issue_type: Option<IssueTypeNamed>,
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
    Organization(crate::api::browse::OrgLogin),
    #[cynic(fallback)]
    Other,
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
    IssueTypeNames = "IssueTypeConnection" => IssueTypeOption,
}

/// One of a repository's issue types, and whether it's turned on.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "IssueType")]
pub struct IssueTypeOption {
    pub id: cynic::Id,
    pub name: String,
    pub is_enabled: bool,
}

/// The types turned on, of all a repository has.
fn issue_types(list: Option<IssueTypeNames>) -> Capped<Named> {
    let Some(list) = list else { return Capped::default() };
    let all = list.into_capped(|t| t);
    let off = all.items.iter().filter(|t| !t.is_enabled).count() as u64;
    let total = all.total().saturating_sub(off);
    let on = (all.items.into_iter())
        .filter(|t| t.is_enabled)
        .map(|t| Named {
            id: t.id.into(),
            name: t.name,
        });
    Capped::new(on.collect(), total)
}

/// Node IDs and the names they go by, each a [`Named`].
macro_rules! named {
    ($($name:ident = $graphql:literal.$field:ident),* $(,)?) => {$(
        #[derive(cynic::QueryFragment, Debug)]
        #[cynic(graphql_type = $graphql)]
        pub struct $name {
            pub id: cynic::Id,
            pub $field: String,
        }

        impl From<$name> for Named {
            fn from(n: $name) -> Named {
                Named {
                    id: n.id.into(),
                    name: n.$field,
                }
            }
        }
    )*};
}

named! {
    LabelNamed = "Label".name,
    UserNamed = "User".login,
    BotNamed = "Bot".login,
    MilestoneNamed = "Milestone".title,
    IssueTypeNamed = "IssueType".name,
    // `org/slug`.
    TeamNamed = "Team".combined_slug,
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

fn label_names(list: Option<LabelNames>) -> Capped<Named> {
    list.map_or_else(Capped::default, |l| l.into_capped(Named::from))
}

impl SidebarRepo {
    /// The sidebar, and for a pull request in an organization's
    /// repository, the organization, whose teams it may ask.
    fn into_sidebar(self) -> Option<(Sidebar, Option<String>)> {
        let (labels, assignees, milestone, requests, author, issue_type) = match self
            .issue_or_pull_request?
        {
            SidebarOf::Issue(i) => (i.labels, i.assignees, i.milestone, None, None, i.issue_type),
            SidebarOf::PullRequest(p) => (
                p.labels,
                p.assignees,
                p.milestone,
                p.review_requests,
                Some(crate::api::model::author(p.author)),
                None,
            ),
            SidebarOf::Other => return None,
        };
        let mut requested = Requested::default();
        if let Some(r) = requests {
            let mut shown = 0;
            for asked in nodes(r.nodes).filter_map(|r| r.requested_reviewer) {
                shown += 1;
                match asked {
                    RequestedNamed::User(u) => requested.users.push(u.into()),
                    RequestedNamed::Bot(b) => requested.bots.push(b.into()),
                    RequestedNamed::Team(t) => requested.teams.push(t.into()),
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
        // A pull request has an author here, and no type.
        let pr = author.is_some();
        let sidebar = Sidebar {
            labels: label_names(labels),
            assignees: assignees.into_capped(Named::from),
            milestone: milestone.map(Named::from),
            requested,
            author,
            all_labels: label_names(self.labels),
            users: self.assignable_users.into_capped(Named::from),
            milestones: (self.milestones)
                .map_or_else(Capped::default, |m| m.into_capped(Named::from)),
            teams: None,
            issue_types: if pr {
                Capped::default()
            } else {
                issue_types(self.issue_types)
            },
            issue_type: issue_type.map(Named::from),
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
    UpdateIssueTypePayload = "UpdateIssueIssueTypePayload",
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

#[derive(cynic::QueryVariables, Debug)]
pub struct IssueTypeVariables {
    pub issue: cynic::Id,
    /// `None` clears it: sent as null.
    pub issue_type: Option<cynic::Id>,
}

mutations! {
    UpdateIssueBody("BodyVariables") update_issue: UpdateIssuePayload { id: $id, body: $body }
    UpdatePrBody("BodyVariables") update_pull_request: UpdatePrPayload { pullRequestId: $id, body: $body }
    UpdateIssueComment("BodyVariables") update_issue_comment: UpdateIssueCommentPayload { id: $id, body: $body }
    UpdateReviewComment("BodyVariables") update_pull_request_review_comment: UpdateReviewCommentPayload { pullRequestReviewCommentId: $id, body: $body }
    DeleteIssueComment("IdVariables") delete_issue_comment: DeleteIssueCommentPayload { id: $id }
    DeleteReviewComment("IdVariables") delete_pull_request_review_comment: DeleteReviewCommentPayload { id: $id }
    /// A review's summary.
    UpdateReview("BodyVariables") update_pull_request_review: UpdateReviewPayload { pullRequestReviewId: $id, body: $body }
    /// Your pending review, thrown away with its comments.
    DeleteReview("IdVariables") delete_pull_request_review: DeleteReviewPayload { pullRequestReviewId: $id }
    UpdateIssueTitle("TitleVariables") update_issue: UpdateIssuePayload { id: $id, title: $title }
    UpdatePrTitle("TitleVariables") update_pull_request: UpdatePrPayload { pullRequestId: $id, title: $title }
    AddReaction("ReactVariables") add_reaction: AddReactionPayload { subjectId: $subject, content: $content }
    RemoveReaction("ReactVariables") remove_reaction: RemoveReactionPayload { subjectId: $subject, content: $content }
    AddLabels("AddVariables") add_labels_to_labelable: AddLabelsPayload { labelableId: $subject, labelIds: $ids }
    RemoveLabels("RemoveVariables") remove_labels_from_labelable: RemoveLabelsPayload { labelableId: $subject, labelIds: $ids }
    AddAssignees("AddVariables") add_assignees_to_assignable: AddAssigneesPayload { assignableId: $subject, assigneeIds: $ids }
    RemoveAssignees("RemoveVariables") remove_assignees_from_assignable: RemoveAssigneesPayload { assignableId: $subject, assigneeIds: $ids }
    RequestReviews("ReviewersVariables") request_reviews: RequestReviewsPayload { pullRequestId: $pr, userIds: $users, teamIds: $teams, botIds: $bots }
    SetIssueMilestone("MilestoneVariables") update_issue: UpdateIssuePayload { id: $id, milestoneId: $milestone }
    SetPrMilestone("MilestoneVariables") update_pull_request: UpdatePrPayload { pullRequestId: $id, milestoneId: $milestone }
    SetIssueType("IssueTypeVariables") update_issue_issue_type: UpdateIssueTypePayload { issueId: $issue, issueTypeId: $issue_type }
}
