//! Making a release, and the notes GitHub writes for one.

use serde::Deserialize;

use crate::api::client::{ApiError, GitHub};
use crate::api::model::RepoId;
use crate::schema::schema;

/// A release to make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRelease {
    pub repo: RepoId,
    /// The tag: one GitHub has, or a new one GitHub makes at `target`
    /// when the release is published.
    pub tag: String,
    /// A branch or a commit, for a new tag; `None` is the default branch.
    pub target: Option<String>,
    /// Its title: empty is the tag, as on GitHub.
    pub name: String,
    pub notes: String,
    pub draft: bool,
    pub prerelease: bool,
    /// Whether it becomes the repository's latest release.
    pub latest: bool,
}

/// What GitHub made.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Made {
    pub tag_name: String,
    pub draft: bool,
    pub html_url: String,
}

/// Release notes GitHub wrote from the pull requests since the last
/// release.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Notes {
    /// A title for the release.
    pub name: String,
    pub body: String,
}

impl GitHub {
    pub async fn create_release(&self, new: &NewRelease) -> Result<Made, ApiError> {
        let mut body = serde_json::json!({
            "tag_name": new.tag,
            "name": new.name,
            "body": new.notes,
            "draft": new.draft,
            "prerelease": new.prerelease,
            // A pre-release is never the latest.
            "make_latest": if new.latest && !new.prerelease { "true" } else { "false" },
        });
        if let (Some(target), Some(map)) = (&new.target, body.as_object_mut()) {
            map.insert("target_commitish".into(), target.clone().into());
        }
        let path = format!("/repos/{}/releases", new.repo);
        self.rest_write_json(http::Method::POST, &path, Some(&body))
            .await
    }

    /// The notes GitHub would write for a release of `tag` (made at
    /// `target`, if it's new), since the release before it.
    pub async fn release_notes(
        &self,
        repo: &RepoId,
        tag: &str,
        target: Option<&str>,
    ) -> Result<Notes, ApiError> {
        let mut body = serde_json::json!({ "tag_name": tag });
        if let (Some(target), Some(map)) = (target, body.as_object_mut()) {
            map.insert("target_commitish".into(), target.into());
        }
        let path = format!("/repos/{repo}/releases/generate-notes");
        self.rest_write_json(http::Method::POST, &path, Some(&body))
            .await
    }

    /// Whether `repo` has tag `tag`, and a release of it.
    pub async fn tag_look(&self, repo: &RepoId, tag: &str) -> Result<TagLook, ApiError> {
        use cynic::QueryBuilder;
        let op = TagQuery::build(TagVariables {
            owner: repo.owner.clone(),
            name: repo.name.clone(),
            qualified: format!("refs/tags/{tag}"),
            tag: tag.to_owned(),
        });
        self.find(op, repo, |q| {
            let r = q.repository?;
            Some(TagLook {
                exists: r.tag.is_some(),
                release: r.release.map(|r| (r.is_draft, r.url.0)),
            })
        })
        .await
    }
}

/// What GitHub has of a tag: whether it's there, and its release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagLook {
    pub exists: bool,
    /// Its release, if it has one: whether it's a draft, and its page.
    pub release: Option<(bool, String)>,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct TagVariables {
    pub owner: String,
    pub name: String,
    pub qualified: String,
    pub tag: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Query",
    schema_module = "schema",
    variables = "TagVariables"
)]
pub struct TagQuery {
    #[arguments(owner: $owner, name: $name)]
    pub repository: Option<TagRepo>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Repository",
    schema_module = "schema",
    variables = "TagVariables"
)]
pub struct TagRepo {
    #[arguments(qualifiedName: $qualified)]
    #[cynic(rename = "ref")]
    pub tag: Option<TagRef>,
    #[arguments(tagName: $tag)]
    pub release: Option<TagRelease>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Ref", schema_module = "schema")]
pub struct TagRef {
    pub name: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Release", schema_module = "schema")]
pub struct TagRelease {
    pub is_draft: bool,
    pub url: crate::api::queries::Uri,
}
