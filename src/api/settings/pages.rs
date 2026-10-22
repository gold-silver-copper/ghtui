//! A repository's GitHub Pages site: where it's built from, its custom
//! domain, HTTPS, and its latest builds.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::Listed;
use crate::api::client::{ApiError, GitHub};
use crate::api::model::RepoId;
use crate::api::{has_scope, scope_missing};

/// What GitHub's Pages settings set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PagesForm {
    /// Built by an Actions workflow, not from a branch.
    pub workflow: bool,
    pub branch: String,
    /// `/` or `/docs`.
    pub path: String,
    /// Its custom domain; empty for none.
    pub domain: String,
    pub https: bool,
}

impl Default for PagesForm {
    fn default() -> Self {
        Self {
            workflow: false,
            branch: "main".into(),
            path: "/".into(),
            domain: String::new(),
            https: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Site {
    pub url: Option<String>,
    /// `built`, `building`, `errored`, or none yet.
    pub status: Option<String>,
    pub form: PagesForm,
    /// Its domain's certificate's state (`approved`, `new`…).
    pub certificate: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PagesBuild {
    pub status: String,
    pub error: Option<String>,
    pub pusher: Option<String>,
    pub commit: String,
    /// Milliseconds.
    pub duration: u64,
    /// ISO 8601.
    pub created_at: String,
}

/// The Pages page: the site, if there is one, and its builds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pages {
    pub site: Option<Site>,
    /// Its latest builds, or why GitHub won't list them.
    pub builds: Result<Listed<PagesBuild>, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PagesChange {
    /// Publish a site.
    Create { repo: RepoId, form: PagesForm },
    /// Only what differs is sent.
    Update {
        repo: RepoId,
        from: Box<PagesForm>,
        to: Box<PagesForm>,
    },
    /// Unpublish it.
    Delete { repo: RepoId },
}

/// How many builds the page lists.
const BUILDS: usize = 10;

/// The body of a PUT to `/pages`: what differs from `from` (or all of it).
fn update_body(from: Option<&PagesForm>, to: &PagesForm) -> Value {
    let mut body = Map::new();
    let changed = |f: fn(&PagesForm) -> Value| from.is_none_or(|from| f(from) != f(to));
    let source = |f: &PagesForm| json!({ "branch": f.branch, "path": f.path });
    if changed(|f| f.workflow.into()) {
        let kind = if to.workflow { "workflow" } else { "legacy" };
        body.insert("build_type".into(), kind.into());
    }
    if !to.workflow && (changed(source) || changed(|f| f.workflow.into())) {
        body.insert("source".into(), source(to));
    }
    if changed(|f| f.domain.clone().into()) {
        let domain = Some(to.domain.as_str()).filter(|d| !d.is_empty());
        body.insert("cname".into(), domain.into());
    }
    if changed(|f| f.https.into()) {
        body.insert("https_enforced".into(), to.https.into());
    }
    Value::Object(body)
}

mod wire {
    use serde::Deserialize;

    #[derive(Deserialize)]
    pub(super) struct Site {
        pub html_url: Option<String>,
        pub status: Option<String>,
        pub cname: Option<String>,
        #[serde(default)]
        pub https_enforced: bool,
        pub build_type: Option<String>,
        pub source: Option<Source>,
        pub https_certificate: Option<Certificate>,
    }

    #[derive(Deserialize)]
    pub(super) struct Source {
        pub branch: String,
        #[serde(default)]
        pub path: String,
    }

    #[derive(Deserialize)]
    pub(super) struct Certificate {
        pub state: String,
    }

    #[derive(Deserialize)]
    pub(super) struct Build {
        #[serde(default)]
        pub status: String,
        pub error: Option<Error>,
        pub pusher: Option<crate::api::browse::UserLogin>,
        #[serde(default)]
        pub commit: String,
        #[serde(default)]
        pub duration: u64,
        #[serde(default)]
        pub created_at: String,
    }

    #[derive(Deserialize)]
    pub(super) struct Error {
        pub message: Option<String>,
    }
}

impl GitHub {
    pub(super) async fn pages(&self, repo: &RepoId) -> Result<Pages, ApiError> {
        let path = format!("/repos/{repo}/pages");
        // No site is a 404; a token without `repo` reads none either.
        let answer = self.rest_refusable(&path, "its Pages site", |status, _, answer| {
            let scopes = answer.header("x-oauth-scopes");
            match status {
                404 if scopes.is_some_and(|s| !has_scope(s, "repo")) => {
                    Some(Err(scope_missing("Its Pages site", "repo")))
                }
                404 => Some(Ok(())),
                403 => Some(Err("GitHub doesn't show you its Pages site".to_owned())),
                _ => None,
            }
        });
        let site = match answer.await? {
            Ok(answer) => serde_json::from_str::<wire::Site>(answer.body())?,
            Err(Ok(())) => {
                return Ok(Pages {
                    site: None,
                    builds: Ok(Listed {
                        items: Vec::new(),
                        more: false,
                    }),
                });
            }
            Err(Err(why)) => return Err(ApiError::Denied(why)),
        };
        let builds = format!("/repos/{repo}/pages/builds?per_page={BUILDS}");
        let builds = self.rest_refusable(&builds, "its Pages builds", |status, _, _| {
            matches!(status, 403 | 404).then(|| "GitHub doesn't show you its builds".to_owned())
        });
        let builds = match builds.await? {
            Err(why) => Err(why),
            Ok(answer) => {
                let found: Vec<wire::Build> = serde_json::from_str(answer.body())?;
                let items = (found.into_iter())
                    .map(|b| PagesBuild {
                        status: b.status,
                        error: b.error.and_then(|e| e.message).filter(|m| !m.is_empty()),
                        pusher: b.pusher.map(|p| p.login),
                        commit: b.commit,
                        duration: b.duration,
                        created_at: b.created_at,
                    })
                    .collect();
                let more = answer.next_page().is_some();
                Ok(Listed { items, more })
            }
        };
        let workflow = site.build_type.as_deref() == Some("workflow");
        let source = site.source;
        Ok(Pages {
            site: Some(Site {
                url: site.html_url,
                status: site.status,
                form: PagesForm {
                    workflow,
                    branch: source
                        .as_ref()
                        .map_or_else(String::new, |s| s.branch.clone()),
                    path: source.map_or_else(|| "/".into(), |s| s.path),
                    domain: site.cname.unwrap_or_default(),
                    https: site.https_enforced,
                },
                certificate: site.https_certificate.map(|c| c.state),
            }),
            builds,
        })
    }

    pub(super) async fn change_pages(&self, change: &PagesChange) -> Result<(), ApiError> {
        use http::Method;
        match change {
            PagesChange::Create { repo, form } => {
                let path = format!("/repos/{repo}/pages");
                let kind = if form.workflow { "workflow" } else { "legacy" };
                let mut body = json!({ "build_type": kind });
                if !form.workflow
                    && let Value::Object(b) = &mut body
                {
                    b.insert(
                        "source".into(),
                        json!({"branch": form.branch, "path": form.path}),
                    );
                }
                self.rest_write(Method::POST, &path, Some(&body)).await?;
                // A domain is set once the site is there.
                if form.domain.is_empty() {
                    return Ok(());
                }
                let body = json!({ "cname": form.domain });
                self.rest_write(Method::PUT, &path, Some(&body)).await
            }
            PagesChange::Update { repo, from, to } => {
                let path = format!("/repos/{repo}/pages");
                let body = update_body(Some(from), to);
                self.rest_write(Method::PUT, &path, Some(&body)).await
            }
            PagesChange::Delete { repo } => {
                let path = format!("/repos/{repo}/pages");
                self.rest_write(Method::DELETE, &path, None).await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A change sends what changed; a branch source goes with a switch
    /// from a workflow, and an emptied domain is GitHub's null.
    #[test]
    fn a_pages_change_sends_what_changed() {
        let from = PagesForm {
            domain: "docs.example.com".into(),
            ..PagesForm::default()
        };
        let to = PagesForm {
            path: "/docs".into(),
            domain: String::new(),
            ..from.clone()
        };
        assert_eq!(
            update_body(Some(&from), &to),
            json!({"source": {"branch": "main", "path": "/docs"}, "cname": null})
        );
        let workflow = PagesForm {
            workflow: true,
            ..from.clone()
        };
        assert_eq!(
            update_body(Some(&workflow), &from),
            json!({"build_type": "legacy", "source": {"branch": "main", "path": "/"}})
        );
    }
}
