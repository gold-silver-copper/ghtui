//! What you're subscribed to: an issue's, a pull request's or a
//! discussion's thread, and a repository's watch settings. GitHub calls
//! all of them subscribables, with one mutation for them all.

use crate::api::change::mutations;
use crate::api::client::{ApiError, GitHub};
use crate::api::model::{NodeId, RepoId};
use crate::api::queries::{NumberVariables, NumberVariablesFields, fragments};
use crate::schema::schema;

/// What a subscription is to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Watched {
    /// A repository's notifications: its watch settings.
    Repo(RepoId),
    /// An issue's or a pull request's thread, by its number.
    Thread(RepoId, u64),
    Discussion(RepoId, u64),
}

impl Watched {
    pub fn repo(&self) -> &RepoId {
        match self {
            Watched::Repo(repo) | Watched::Thread(repo, _) | Watched::Discussion(repo, _) => repo,
        }
    }
}

impl std::fmt::Display for Watched {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Watched::Repo(repo) => write!(f, "{repo}"),
            Watched::Thread(repo, n) | Watched::Discussion(repo, n) => write!(f, "{repo}#{n}"),
        }
    }
}

/// Whether you're notified, as GitHub says it.
#[derive(cynic::Enum, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubscriptionState {
    /// Never notified.
    Ignored,
    /// Notified of everything.
    Subscribed,
    /// Notified when you take part or are mentioned (for a repository,
    /// "Participating and @mentions").
    Unsubscribed,
}

/// Your subscription to something.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscription {
    /// What the mutation changes.
    pub id: NodeId,
    /// `None` when GitHub doesn't say (it can't for some).
    pub state: Option<SubscriptionState>,
    /// GitHub lets you change it.
    pub can: bool,
}

impl GitHub {
    /// Your subscription to `of`.
    pub async fn subscription(&self, of: &Watched) -> Result<Subscription, ApiError> {
        use cynic::QueryBuilder;
        let subject = of.to_string();
        let repo = of.repo();
        let number = match of {
            Watched::Repo(_) => 0,
            Watched::Thread(_, n) | Watched::Discussion(_, n) => {
                i32::try_from(*n).map_err(|_| ApiError::NotFound(subject.clone()))?
            }
        };
        let vars = NumberVariables {
            owner: repo.owner.to_string(),
            name: repo.name.to_string(),
            number,
        };
        match of {
            Watched::Repo(_) => {
                let op = RepoWatchQuery::build(RepoNameVariables {
                    owner: vars.owner,
                    name: vars.name,
                });
                self.find(op, subject, |q| q.repository.map(|r| r.watch.into()))
                    .await
            }
            Watched::Thread(..) => {
                let op = ThreadWatchQuery::build(vars);
                self.find(op, subject, |q| {
                    match q.repository?.issue_or_pull_request? {
                        ThreadWatchOf::Issue(w) => Some(w.into()),
                        ThreadWatchOf::PullRequest(w) => Some(w.into()),
                        ThreadWatchOf::Other => None,
                    }
                })
                .await
            }
            Watched::Discussion(..) => {
                let op = DiscussionWatchQuery::build(vars);
                let pick = |q: DiscussionWatchQuery| Some(q.repository?.discussion?.into());
                self.find(op, subject, pick).await
            }
        }
    }

    /// Sets your subscription to `of` (the subscribable `id`). A
    /// repository's goes through REST, which the repo scope allows; a
    /// thread's only through GraphQL, which takes the notifications scope.
    pub async fn subscribe(
        &self,
        of: &Watched,
        id: &NodeId,
        state: SubscriptionState,
    ) -> Result<(), ApiError> {
        if let Watched::Repo(repo) = of {
            let path = format!("/repos/{repo}/subscription");
            let (subscribed, ignored) = match state {
                // Participating and @mentions is not watching at all.
                SubscriptionState::Unsubscribed => {
                    return self.rest_write(http::Method::DELETE, &path, None).await;
                }
                SubscriptionState::Subscribed => (true, false),
                SubscriptionState::Ignored => (false, true),
            };
            let body = serde_json::json!({ "subscribed": subscribed, "ignored": ignored });
            return self.rest_write(http::Method::PUT, &path, Some(&body)).await;
        }
        let vars = SubscribeVariables {
            id: id.gql(),
            state,
        };
        self.run::<UpdateSubscription, _>(vars).await
    }
}

/// What a subscribable says of you.
macro_rules! watch_fragments {
    ($($name:ident = $graphql:literal),* $(,)?) => {$(
        #[derive(cynic::QueryFragment, Debug)]
        #[cynic(graphql_type = $graphql)]
        pub struct $name {
            pub id: cynic::Id,
            pub viewer_subscription: Option<SubscriptionState>,
            pub viewer_can_subscribe: bool,
        }

        impl From<$name> for Subscription {
            fn from(w: $name) -> Self {
                Subscription {
                    id: w.id.into(),
                    state: w.viewer_subscription,
                    can: w.viewer_can_subscribe,
                }
            }
        }
    )*};
}

watch_fragments!(
    RepoWatch = "Repository",
    IssueWatch = "Issue",
    PrWatch = "PullRequest",
    DiscussionWatch = "Discussion",
);

#[derive(cynic::QueryVariables, Debug)]
pub struct RepoNameVariables {
    pub owner: String,
    pub name: String,
}

fragments! {
    root "Query" repository(owner, name);
    RepoWatchQuery("RepoNameVariables") => RepoWatchRepo,
    ThreadWatchQuery("NumberVariables") => ThreadWatchRepo,
    DiscussionWatchQuery("NumberVariables") => DiscussionWatchRepo,
}
fragments! {
    root "Repository" issue_or_pull_request(number);
    ThreadWatchRepo("NumberVariables") => ThreadWatchOf,
}
fragments! {
    root "Repository" discussion(number);
    DiscussionWatchRepo("NumberVariables") => DiscussionWatch,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Repository")]
pub struct RepoWatchRepo {
    #[cynic(spread)]
    pub watch: RepoWatch,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "IssueOrPullRequest")]
pub enum ThreadWatchOf {
    Issue(IssueWatch),
    PullRequest(PrWatch),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct SubscribeVariables {
    pub id: cynic::Id,
    pub state: SubscriptionState,
}

fragments! {
    client_mutation_id: Option<String>;
    SubscribePayload = "UpdateSubscriptionPayload",
}

mutations! {
    UpdateSubscription("SubscribeVariables") update_subscription: SubscribePayload { subscribableId: $id, state: $state }
}
