//! Starred lists: someone's lists, a list's repositories, and putting a
//! repository in your lists or taking it out (GitHub's star button's
//! menu), over GraphQL.

use serde::{Deserialize, Serialize};

use crate::api::browse::{RepoSummary, Results};
use crate::api::model::{Capped, NodeId, RepoId};
use crate::api::{ApiError, GitHub};

/// One of someone's starred lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StarList {
    pub id: NodeId,
    pub name: String,
    /// Its name in its URL (`/stars/octocat/lists/rust-tools`).
    pub slug: String,
    pub description: Option<String>,
    pub private: bool,
    /// How many repositories it has.
    pub count: u64,
    /// When a repository was last added (or it was made), ISO 8601.
    pub last_added: Option<String>,
}

/// A list's page: the list, and a page of its repositories.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListPage {
    pub list: StarList,
    pub repos: Results<RepoSummary>,
}

impl crate::api::browse::Page for ListPage {
    fn counts(&self) -> (usize, u64) {
        self.repos.counts()
    }
}

/// One of your lists, and whether a repository is in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mine {
    pub id: NodeId,
    pub name: String,
    pub private: bool,
    pub count: u64,
    /// Whether the repository is in it; `None` past what ghtui reads of
    /// a long list ([`MEMBERS_MAX`]).
    pub has: Option<bool>,
}

/// Your lists, as they stand for one repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MyLists {
    /// The repository's node ID.
    pub repo: NodeId,
    pub starred: bool,
    /// You, as GitHub knows you.
    pub viewer: String,
    pub lists: Capped<Mine>,
}

/// The most of one list's repositories read to say whether one is in
/// it.
pub const MEMBERS_MAX: usize = 1_000;

impl GitHub {
    /// Someone's starred lists, most recently added to first; GitHub
    /// shows you your private ones too.
    pub async fn star_lists(&self, login: &str) -> Result<Capped<StarList>, ApiError> {
        let vars = serde_json::json!({ "login": login });
        let found: wire::Counted<wire::List> = self
            .graphql_json(crate::api::raw::STAR_LISTS, vars, login, "/user/lists")
            .await?;
        let mut lists = found.into_capped(|l| Some(StarList::from(l)));
        lists.items.sort_by(|a, b| b.last_added.cmp(&a.last_added));
        Ok(lists)
    }

    /// A list, by its owner and slug, and its repositories 30 at a time
    /// from `after` (which names the list: `<id> <cursor>`).
    pub async fn star_list(
        &self,
        login: &str,
        slug: &str,
        after: Option<String>,
    ) -> Result<ListPage, ApiError> {
        let lists = self.star_lists(login).await?;
        let Some(list) = lists.iter().find(|l| l.slug == slug).cloned() else {
            return Err(ApiError::NotFound(format!("@{login}'s list {slug}")));
        };
        let cursor = after
            .as_deref()
            .and_then(|a| a.split_once(' '))
            .map(|(_, c)| c);
        let vars = serde_json::json!({ "id": list.id.as_str(), "after": cursor });
        let items: wire::Connection<wire::Repo> = self
            .graphql_json(
                &crate::api::raw::star_list_items(),
                vars,
                &list.name,
                "/node/items",
            )
            .await?;
        let mut repos = items.filter_results(wire::Repo::summary);
        repos.next = repos.next.map(|c| format!("{} {c}", list.id.as_str()));
        let page = ListPage { list, repos };
        Ok(self.checked(page, format!("@{login}'s list {slug}")))
    }

    /// Your lists, and which of them have `repo`.
    pub async fn my_lists(&self, repo: &RepoId) -> Result<MyLists, ApiError> {
        let vars = serde_json::json!({ "owner": repo.owner, "name": repo.name });
        let found: wire::Mine = self
            .graphql_data(crate::api::raw::MY_LISTS, vars, true)
            .await?;
        let Some(item) = found.repository else {
            return Err(ApiError::NotFound(repo.to_string()));
        };
        let mut lists = Vec::new();
        for list in found.viewer.lists.nodes.into_iter().flatten() {
            let has = self.has(&list.id, &item.id, list.items).await?;
            lists.push(Mine {
                id: list.id,
                name: list.name,
                private: list.is_private,
                count: has.1,
                has: has.0,
            });
        }
        let mine = MyLists {
            repo: item.id,
            starred: item.viewer_has_starred,
            viewer: found.viewer.login,
            lists: Capped::new(lists, found.viewer.lists.total_count),
        };
        Ok(mine)
    }

    /// Whether a list has repository `item`, reading past its first page
    /// up to [`MEMBERS_MAX`]; and how many it has.
    async fn has(
        &self,
        list: &NodeId,
        item: &NodeId,
        mut page: wire::Connection<wire::Member>,
    ) -> Result<(Option<bool>, u64), ApiError> {
        let total = page.total_count;
        let mut read = 0;
        loop {
            read += page.nodes.len();
            if page
                .nodes
                .iter()
                .flatten()
                .any(|n| n.id.as_ref() == Some(item))
            {
                return Ok((Some(true), total));
            }
            let next = page
                .page_info
                .end_cursor
                .filter(|_| page.page_info.has_next_page);
            let Some(after) = next else {
                return Ok((Some(false), total));
            };
            if read >= MEMBERS_MAX {
                return Ok((None, total));
            }
            let vars = serde_json::json!({ "id": list.as_str(), "after": after });
            page = self
                .graphql_json(
                    crate::api::raw::LIST_MEMBERS,
                    vars,
                    list.as_str(),
                    "/node/items",
                )
                .await?;
        }
    }

    /// Puts repository `item` in exactly `lists` of yours.
    pub async fn set_lists(&self, item: &NodeId, lists: &[NodeId]) -> Result<(), ApiError> {
        let lists: Vec<&str> = lists.iter().map(NodeId::as_str).collect();
        let vars = serde_json::json!({ "item": item.as_str(), "lists": lists });
        self.mutate_json(crate::api::raw::SET_LISTS, vars).await
    }

    /// Makes a list named `name`; its ID.
    pub async fn create_list(&self, name: &str) -> Result<NodeId, ApiError> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Answer {
            create_user_list: Made,
        }
        #[derive(Deserialize)]
        struct Made {
            list: Id,
        }
        #[derive(Deserialize)]
        struct Id {
            id: NodeId,
        }
        let vars = serde_json::json!({ "name": name });
        let made: Answer = (self.graphql_data(crate::api::raw::CREATE_LIST, vars, false)).await?;
        Ok(made.create_user_list.list.id)
    }

    /// Deletes one of your lists.
    pub async fn delete_list(&self, list: &NodeId) -> Result<(), ApiError> {
        let vars = serde_json::json!({ "list": list.as_str() });
        self.mutate_json(crate::api::raw::DELETE_LIST, vars).await
    }
}

mod wire {
    use serde::Deserialize;

    use crate::api::browse::RepoSummary;
    pub use crate::api::browse::wire::{Connection, Count, Counted};
    use crate::api::model::{NodeId, RepoId};

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct List {
        pub id: NodeId,
        pub name: String,
        pub slug: String,
        pub description: Option<String>,
        pub is_private: bool,
        pub last_added_at: Option<String>,
        pub items: Count,
    }

    impl From<List> for super::StarList {
        fn from(l: List) -> Self {
            super::StarList {
                id: l.id,
                name: l.name,
                slug: l.slug,
                description: l.description.filter(|d| !d.trim().is_empty()),
                private: l.is_private,
                count: l.items.total_count,
                last_added: l.last_added_at,
            }
        }
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Repo {
        name_with_owner: Option<String>,
        description: Option<String>,
        #[serde(default)]
        stargazer_count: u64,
        #[serde(default)]
        fork_count: u64,
        primary_language: Option<Language>,
        pushed_at: Option<String>,
        #[serde(default)]
        is_private: bool,
        #[serde(default)]
        is_fork: bool,
        #[serde(default)]
        is_archived: bool,
    }

    #[derive(Deserialize)]
    pub struct Language {
        name: String,
        color: Option<String>,
    }

    impl Repo {
        /// The row a repository's list shows, unless GitHub left out
        /// which repository it is.
        pub fn summary(self) -> Option<RepoSummary> {
            Some(RepoSummary {
                repo: RepoId::parse(self.name_with_owner.as_deref()?)?,
                description: self.description.filter(|d| !d.trim().is_empty()),
                stars: self.stargazer_count,
                forks: self.fork_count,
                language: self.primary_language.as_ref().map(|l| l.name.clone()),
                language_color: self.primary_language.and_then(|l| l.color),
                pushed_at: self.pushed_at,
                private: self.is_private,
                fork: self.is_fork,
                archived: self.is_archived,
            })
        }
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Mine {
        pub repository: Option<Item>,
        pub viewer: Viewer,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Item {
        pub id: NodeId,
        pub viewer_has_starred: bool,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Viewer {
        pub login: String,
        pub lists: Counted<MyList>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct MyList {
        pub id: NodeId,
        pub name: String,
        pub is_private: bool,
        pub items: Connection<Member>,
    }

    #[derive(Deserialize)]
    pub struct Member {
        pub id: Option<NodeId>,
    }
}
