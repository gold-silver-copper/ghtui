//! Who and what a comment can mention in a repository: the people it can
//! assign, and its recently updated issues and pull requests.

use serde::Deserialize;

use crate::api::browse::UserLogin;
use crate::api::model::RepoId;
use crate::api::{ApiError, GitHub};

/// What the comment box offers after `@` and `#` in a repository.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Mentionable {
    /// Its assignable users' logins.
    pub users: Vec<String>,
    /// Its issues and pull requests, most recently updated first: number
    /// and title.
    pub issues: Vec<(u64, String)>,
}

impl GitHub {
    /// Who and what `repo` lets a comment mention: one page of each,
    /// revalidated with ETags like any REST page.
    pub async fn mentionable(&self, repo: &RepoId) -> Result<Mentionable, ApiError> {
        #[derive(Deserialize)]
        struct Issue {
            number: u64,
            title: String,
        }
        let base = format!("/repos/{repo}");
        let users = format!("{base}/assignees?per_page=100");
        let issues = format!("{base}/issues?state=all&sort=updated&per_page=100");
        let (users, issues) = futures::join!(
            self.rest_json::<Vec<UserLogin>>(&users, format!("{repo}'s assignable users")),
            self.rest_json::<Vec<Issue>>(&issues, format!("{repo}'s issues")),
        );
        Ok(Mentionable {
            users: users?.into_iter().map(|u| u.login).collect(),
            issues: issues?.into_iter().map(|i| (i.number, i.title)).collect(),
        })
    }
}
