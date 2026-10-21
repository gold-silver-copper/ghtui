//! Making, editing and deleting a release, the notes GitHub writes for
//! one, and its assets: listed by REST's IDs, downloaded, uploaded and
//! deleted.

use serde::Deserialize;

use crate::api::client::{ApiError, GitHub};
use crate::api::model::RepoId;
use crate::api::queries::fragments;
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
        let path = format!("/repos/{}/releases", new.repo);
        let body = release_body(new, true);
        self.rest_write_json(http::Method::POST, &path, Some(&body))
            .await
    }

    /// Changes release `id` to what `new` says (all of it: its tag,
    /// title, notes, and whether it's a draft, a pre-release or the
    /// latest).
    pub async fn update_release(&self, id: u64, new: &NewRelease) -> Result<Made, ApiError> {
        // A draft is never the latest; GitHub refuses to be told so.
        let body = release_body(new, !new.draft);
        let path = format!("/repos/{}/releases/{id}", new.repo);
        self.rest_write_json(http::Method::PATCH, &path, Some(&body))
            .await
    }

    /// Deletes release `id` (its tag stays).
    pub async fn delete_release(&self, repo: &RepoId, id: u64) -> Result<(), ApiError> {
        let path = format!("/repos/{repo}/releases/{id}");
        self.rest_write(http::Method::DELETE, &path, None).await
    }

    /// Deletes tag `tag` on GitHub.
    pub async fn delete_tag(&self, repo: &RepoId, tag: &str) -> Result<(), ApiError> {
        let tag = crate::ui::pages::url::encode_path(tag);
        let path = format!("/repos/{repo}/git/refs/tags/{tag}");
        self.rest_write(http::Method::DELETE, &path, None).await
    }

    /// Release `id`'s asset named `name`, by REST's ID (GraphQL doesn't
    /// say it).
    async fn asset_id(&self, repo: &RepoId, id: u64, name: &str) -> Result<u64, ApiError> {
        #[derive(Deserialize)]
        struct Listed {
            id: u64,
            name: String,
        }
        // GitHub keeps at most 1,000 assets on a release.
        for page in 1..=10 {
            let path = format!("/repos/{repo}/releases/{id}/assets?per_page=100&page={page}");
            let listed = self.rest_get(&path, format!("{repo}'s release assets"));
            let listed: Vec<Listed> = serde_json::from_str(&listed.await?)?;
            if let Some(found) = listed.iter().find(|a| a.name == name) {
                return Ok(found.id);
            }
            if listed.len() < 100 {
                break;
            }
        }
        Err(ApiError::NotFound(format!("{name} on the release")))
    }

    /// Saves release `id`'s asset `name` to `dest`; an existing `dest` is
    /// replaced only if `replace`. Its size.
    pub async fn download_asset(
        &self,
        repo: &RepoId,
        id: u64,
        name: &str,
        dest: &std::path::Path,
        replace: bool,
    ) -> Result<u64, ApiError> {
        let asset = self.asset_id(repo, id, name).await?;
        let path = format!("/repos/{repo}/releases/assets/{asset}");
        self.download_as(&path, "application/octet-stream", dest, replace)
            .await
    }

    /// Deletes release `id`'s asset `name`.
    pub async fn delete_asset(&self, repo: &RepoId, id: u64, name: &str) -> Result<(), ApiError> {
        let asset = self.asset_id(repo, id, name).await?;
        let path = format!("/repos/{repo}/releases/assets/{asset}");
        self.rest_write(http::Method::DELETE, &path, None).await
    }

    /// Uploads the file at `file` to release `id` as asset `name`, to the
    /// upload URL GitHub gives the release (on its uploads host).
    pub async fn upload_asset(
        &self,
        repo: &RepoId,
        id: u64,
        file: &std::path::Path,
        name: &str,
    ) -> Result<(), ApiError> {
        #[derive(Deserialize)]
        struct Uploads {
            upload_url: String,
        }
        let path = format!("/repos/{repo}/releases/{id}");
        let at = self.rest_get(&path, format!("{repo}'s release"));
        let at: Uploads = serde_json::from_str(&at.await?)?;
        // `https://uploads.github.com/repos/o/r/releases/1/assets{?name,label}`
        let base = at.upload_url.split('{').next().unwrap_or_default();
        let encoded: String = url::form_urlencoded::byte_serialize(name.as_bytes()).collect();
        let url = format!("{base}?name={encoded}");
        self.upload(&url, content_type(name), file).await.map(drop)
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

/// A release's fields as REST takes them; whether it's the latest only
/// if `latest_said`.
fn release_body(new: &NewRelease, latest_said: bool) -> serde_json::Value {
    let mut body = serde_json::json!({
        "tag_name": new.tag,
        "name": new.name,
        "body": new.notes,
        "draft": new.draft,
        "prerelease": new.prerelease,
    });
    if let Some(map) = body.as_object_mut() {
        if latest_said {
            // A pre-release is never the latest.
            let latest = new.latest && !new.prerelease;
            map.insert("make_latest".into(), latest.to_string().into());
        }
        if let Some(target) = &new.target {
            map.insert("target_commitish".into(), target.clone().into());
        }
    }
    body
}

/// What an asset named `name` is sent as, by its extension, as GitHub's
/// website would send it.
pub fn content_type(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("gz" | "tgz") => "application/gzip",
        Some("zip") => "application/zip",
        Some("xz") => "application/x-xz",
        Some("zst") => "application/zstd",
        Some("bz2") => "application/x-bzip2",
        Some("tar") => "application/x-tar",
        Some("json") => "application/json",
        Some("txt" | "sha256" | "sha512" | "asc" | "sig") => "text/plain",
        Some("pdf") => "application/pdf",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("deb") => "application/vnd.debian.binary-package",
        Some("rpm") => "application/x-rpm",
        _ => "application/octet-stream",
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

fragments! {
    root "Query" repository(owner, name);
    TagQuery("TagVariables") => TagRepo,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Repository", variables = "TagVariables")]
pub struct TagRepo {
    #[arguments(qualifiedName: $qualified)]
    #[cynic(rename = "ref")]
    pub tag: Option<crate::api::browse::RefName>,
    #[arguments(tagName: $tag)]
    pub release: Option<TagRelease>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Release")]
pub struct TagRelease {
    pub is_draft: bool,
    pub url: crate::api::queries::Uri,
}
