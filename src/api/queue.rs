//! A branch's merge queue: the pull requests waiting to merge into it,
//! in order, as GitHub's `/queue/<branch>` page lists them.

use serde::{Deserialize, Serialize};

use crate::api::model::{Capped, QueueState, RepoId};
use crate::api::{ApiError, GitHub};

/// A pull request in a merge queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueEntry {
    /// 1 is next.
    pub position: u64,
    pub state: QueueState,
    /// ISO 8601.
    pub enqueued_at: String,
    /// Who put it in the queue.
    pub by: Option<String>,
    /// GitHub's estimate of the seconds until it merges.
    pub eta: Option<u64>,
    /// Merged on its own, not with the entries around it.
    pub solo: bool,
    /// Put at the front of the queue.
    pub jump: bool,
    /// The pull request: its number, title and author (none if it can't
    /// be seen).
    pub number: Option<u64>,
    pub title: String,
    pub author: Option<String>,
}

/// A branch's merge queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeQueue {
    pub url: String,
    /// How it merges (`SQUASH`), where GitHub says.
    pub method: Option<String>,
    pub entries: Capped<QueueEntry>,
}

impl GitHub {
    /// `branch`'s merge queue in `repo`; not found where it has none.
    pub async fn merge_queue(&self, repo: &RepoId, branch: &str) -> Result<MergeQueue, ApiError> {
        let vars = crate::api::client::repo_vars!(repo, "branch": branch);
        let what = format!("a merge queue for {repo}'s {branch}");
        let q: wire::Queue = self
            .graphql_json(
                crate::api::raw::MERGE_QUEUE,
                vars,
                what,
                "/repository/mergeQueue",
            )
            .await?;
        let entries = q.entries.into_capped(|e| {
            let pr = e.pull_request;
            Some(QueueEntry {
                position: u64::try_from(e.position).unwrap_or(0),
                state: e.state,
                enqueued_at: e.enqueued_at,
                by: e.enqueuer.map(|a| a.login),
                eta: e
                    .estimated_time_to_merge
                    .and_then(|s| u64::try_from(s).ok()),
                solo: e.solo,
                jump: e.jump,
                number: pr.as_ref().and_then(|p| u64::try_from(p.number).ok()),
                title: pr.as_ref().map(|p| p.title.clone()).unwrap_or_default(),
                author: pr.and_then(|p| p.author).map(|a| a.login),
            })
        });
        Ok(MergeQueue {
            url: q.url,
            method: q.configuration.and_then(|c| c.merge_method),
            entries,
        })
    }
}

mod wire {
    use serde::Deserialize;

    use crate::api::browse::UserLogin as Login;
    use crate::api::browse::wire::Counted;
    use crate::api::model::QueueState;

    #[derive(Deserialize)]
    pub struct Queue {
        pub url: String,
        pub configuration: Option<Configuration>,
        pub entries: Counted<Entry>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Configuration {
        pub merge_method: Option<String>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Entry {
        pub position: i64,
        pub state: QueueState,
        pub enqueued_at: String,
        pub enqueuer: Option<Login>,
        pub estimated_time_to_merge: Option<i64>,
        pub solo: bool,
        pub jump: bool,
        pub pull_request: Option<Pr>,
    }

    #[derive(Deserialize)]
    pub struct Pr {
        pub number: i64,
        pub title: String,
        pub author: Option<Login>,
    }
}
