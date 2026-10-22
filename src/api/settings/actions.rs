//! Actions' secrets and variables (the repository's, its environments'
//! and those its organization shares with it) and its caches. A secret's
//! value is sealed with the repository's (or environment's) public key
//! before it leaves the form ([`seal`]): GitHub, and ghtui, never show it.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Listed, PER_PAGE};
use crate::api::client::{ApiError, GitHub, encode_path};
use crate::api::model::RepoId;
use crate::api::{has_scope, scope_missing};

/// A secret: its name and dates, never its value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Secret {
    pub name: String,
    /// ISO 8601.
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    /// An organization's: `all`, `private` or `selected`.
    #[serde(default)]
    pub visibility: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Variable {
    pub name: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub visibility: Option<String>,
}

/// What a secret or a variable is, for listing them.
pub trait Kept: DeserializeOwned {
    /// Their REST path's word, and the field their list is in.
    const KIND: &'static str;
    /// How many a page lists at most (variables: 30).
    const PER_PAGE: usize;
    fn name(&self) -> &str;
    fn updated_at(&self) -> &str;
    fn visibility(&self) -> Option<&str>;
}

impl Kept for Secret {
    const KIND: &'static str = "secrets";
    const PER_PAGE: usize = PER_PAGE;
    fn name(&self) -> &str {
        &self.name
    }
    fn updated_at(&self) -> &str {
        &self.updated_at
    }
    fn visibility(&self) -> Option<&str> {
        self.visibility.as_deref()
    }
}

impl Kept for Variable {
    const KIND: &'static str = "variables";
    const PER_PAGE: usize = 30;
    fn name(&self) -> &str {
        &self.name
    }
    fn updated_at(&self) -> &str {
        &self.updated_at
    }
    fn visibility(&self) -> Option<&str> {
        self.visibility.as_deref()
    }
}

/// The public key a secret's value is sealed with, as GitHub gives it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealKey {
    pub key_id: String,
    /// Base64.
    pub key: String,
}

/// An environment's secrets or variables.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvStore<T> {
    pub name: String,
    pub items: Listed<T>,
    pub key: Option<SealKey>,
}

/// The Secrets or Variables page: the repository's, its environments',
/// and its organization's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Store<T> {
    pub repo: Listed<T>,
    /// What the repository's secrets are sealed with (secrets only).
    pub key: Option<SealKey>,
    pub environments: Vec<EnvStore<T>>,
    /// It has more environments than are listed.
    pub more_environments: bool,
    /// Its organization's that it may use, or why GitHub won't list them.
    pub org: Result<Listed<T>, String>,
}

impl<T> Store<T> {
    /// The environment named `name`.
    pub fn env(&self, name: &str) -> Option<&EnvStore<T>> {
        self.environments.iter().find(|e| e.name == name)
    }

    /// The list `env` names: an environment's, else the repository's.
    pub fn list(&self, env: Option<&str>) -> Option<&Listed<T>> {
        match env {
            Some(env) => self.env(env).map(|e| &e.items),
            None => Some(&self.repo),
        }
    }

    /// The key a secret for `env` is sealed with.
    pub fn key(&self, env: Option<&str>) -> Option<&SealKey> {
        match env {
            Some(env) => self.env(env)?.key.as_ref(),
            None => self.key.as_ref(),
        }
    }
}

/// A cache an Actions job saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cache {
    pub id: u64,
    #[serde(default)]
    pub key: String,
    #[serde(default, rename = "ref")]
    pub git_ref: String,
    #[serde(default, rename = "size_in_bytes")]
    pub size: u64,
    #[serde(default, rename = "last_accessed_at")]
    pub last_used: String,
    #[serde(default)]
    pub created_at: String,
}

/// The Caches page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Caches {
    /// The most recently used first.
    pub items: Listed<Cache>,
    /// How many it has, and their size, in bytes.
    pub count: u64,
    pub used: u64,
    /// Its limit in bytes, when GitHub says (it tells admins).
    pub limit: Option<u64>,
}

/// How many environments the page lists, each one more request or two.
const ENVIRONMENTS: usize = 10;

/// GitHub's limit on a repository's caches unless it's been changed.
pub const DEFAULT_LIMIT: u64 = 10 << 30;

/// A change to a repository's secrets, variables or caches. A secret is
/// already sealed: the change never holds its value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionsChange {
    SetSecret {
        repo: RepoId,
        env: Option<String>,
        name: String,
        /// The value sealed with `key_id`'s key, in base64.
        sealed: String,
        key_id: String,
        /// When it was last set, if it was: GitHub's date then changes.
        was: Option<String>,
    },
    DeleteSecret {
        repo: RepoId,
        env: Option<String>,
        name: String,
    },
    /// A new variable (`from: None`), or one changed (and maybe renamed).
    SetVariable {
        repo: RepoId,
        env: Option<String>,
        from: Option<String>,
        name: String,
        value: String,
    },
    DeleteVariable {
        repo: RepoId,
        env: Option<String>,
        name: String,
    },
    DeleteCache {
        repo: RepoId,
        id: u64,
        key: String,
    },
    /// Every cache with this key.
    DeleteCaches {
        repo: RepoId,
        key: String,
        count: usize,
    },
}

/// Seals `value` for `key`, as GitHub asks for a secret: a libsodium
/// sealed box, in base64.
pub fn seal(key: &SealKey, value: &str) -> Result<String, String> {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD;
    let bad = || "GitHub's public key for secrets isn't one: reload the page".to_owned();
    let bytes = b64.decode(&key.key).map_err(|_| bad())?;
    let bytes: [u8; 32] = bytes.try_into().map_err(|_| bad())?;
    let public = crypto_box::PublicKey::from(bytes);
    let sealed = public
        .seal(&mut crypto_box::aead::OsRng, value.as_bytes())
        .map_err(|_| "The secret couldn't be sealed".to_owned())?;
    Ok(b64.encode(sealed))
}

/// Where a scope's secrets or variables are: `/repos/o/r/actions` or an
/// environment's.
fn base(repo: &RepoId, env: Option<&str>) -> String {
    match env {
        Some(env) => format!("/repos/{repo}/environments/{}", encode_path(env)),
        None => format!("/repos/{repo}/actions"),
    }
}

/// A refusal of a list anyone who can read the repository may see.
fn unread(status: u16, scopes: Option<&str>, what: &str) -> Option<String> {
    if !matches!(status, 403 | 404) {
        return None;
    }
    Some(if scopes.is_some_and(|s| !has_scope(s, "repo")) {
        scope_missing(what, "repo")
    } else {
        format!("GitHub doesn't show you {what}")
    })
}

impl GitHub {
    /// A REST list whose items are in a field of its answer (`secrets`),
    /// as Actions lists them.
    async fn listed_in<T: DeserializeOwned>(
        &self,
        path: &str,
        field: &str,
        what: &str,
    ) -> Result<Listed<T>, ApiError> {
        let answer = self.admin_answer(path, what).await?;
        let mut answer_json: Value = serde_json::from_str(answer.body())?;
        let total = answer_json.get("total_count").and_then(Value::as_u64);
        let items = answer_json.get_mut(field).map_or(json!([]), Value::take);
        let items: Vec<T> = serde_json::from_value(items)?;
        let more = answer.next_page().is_some()
            || total.is_some_and(|t| usize::try_from(t).is_ok_and(|t| t > items.len()));
        Ok(Listed { items, more })
    }

    /// A repository's secrets (`T` = [`Secret`]) or variables.
    pub(super) async fn kept<T: Kept>(&self, repo: &RepoId) -> Result<Store<T>, ApiError> {
        #[derive(Deserialize)]
        struct Env {
            name: String,
        }
        let (kind, per) = (T::KIND, T::PER_PAGE);
        let secrets = kind == "secrets";
        let list = |base: String| async move {
            let path = format!("{base}/{kind}?per_page={per}");
            self.listed_in::<T>(&path, kind, kind).await
        };
        let key = |base: String| async move {
            if !secrets {
                return Ok(None);
            }
            let path = format!("{base}/secrets/public-key");
            let answer = self.admin_answer(&path, kind).await?;
            Ok::<_, ApiError>(serde_json::from_str(answer.body()).ok())
        };
        let envs = format!("/repos/{repo}/environments?per_page=100");
        let org = format!("/repos/{repo}/actions/organization-{kind}?per_page={per}");
        let org = async {
            let what = format!("{}'s organization {kind}", repo.owner);
            let answer = self.rest_refusable(&org, &what, |status, _, answer| {
                unread(status, answer.header("x-oauth-scopes"), &what)
            });
            match answer.await? {
                Err(why) => Ok(Err(why)),
                Ok(answer) => {
                    let mut found: Value = serde_json::from_str(answer.body())?;
                    let items = found.get_mut(kind).map_or(json!([]), Value::take);
                    let items = serde_json::from_value(items)?;
                    let more = answer.next_page().is_some();
                    Ok::<_, ApiError>(Ok(Listed { items, more }))
                }
            }
        };
        let (items, repo_key, envs, org) = tokio::try_join!(
            list(base(repo, None)),
            key(base(repo, None)),
            self.listed_in::<Env>(&envs, "environments", "environments"),
            org,
        )?;
        let more_environments = envs.more || envs.items.len() > ENVIRONMENTS;
        let environments = envs
            .items
            .into_iter()
            .take(ENVIRONMENTS)
            .map(|e| async move {
                let at = base(repo, Some(&e.name));
                let (items, key) = tokio::try_join!(list(at.clone()), key(at))?;
                Ok::<_, ApiError>(EnvStore {
                    name: e.name,
                    items,
                    key,
                })
            });
        let environments = futures::future::try_join_all(environments).await?;
        Ok(Store {
            repo: items,
            key: repo_key,
            environments,
            more_environments,
            org,
        })
    }

    pub(super) async fn caches(&self, repo: &RepoId) -> Result<Caches, ApiError> {
        #[derive(Deserialize)]
        struct Usage {
            #[serde(default)]
            active_caches_size_in_bytes: u64,
            #[serde(default)]
            active_caches_count: u64,
        }
        #[derive(Deserialize)]
        struct Limit {
            max_cache_size_gb: u64,
        }
        let what = "its caches";
        let read = |path: String| async move {
            let answer = self.rest_refusable(&path, what, |status, _, answer| {
                unread(status, answer.header("x-oauth-scopes"), what)
            });
            answer.await?.map_err(ApiError::Denied)
        };
        let list = format!(
            "/repos/{repo}/actions/caches?per_page={PER_PAGE}&sort=last_accessed_at&direction=desc"
        );
        let limit = async {
            // Only admins may read it.
            let path = format!("/repos/{repo}/actions/cache/storage-limit");
            let answer = self.rest_answer(&path).await.ok()?;
            let limit: Limit = serde_json::from_str(answer.body()).ok()?;
            Some(limit.max_cache_size_gb << 30)
        };
        let (list, usage, limit) = tokio::join!(
            read(list),
            read(format!("/repos/{repo}/actions/cache/usage")),
            limit
        );
        let (list, usage) = (list?, usage?);
        let mut found: Value = serde_json::from_str(list.body())?;
        let total = found.get("total_count").and_then(Value::as_u64);
        let items = found
            .get_mut("actions_caches")
            .map_or(json!([]), Value::take);
        let items: Vec<Cache> = serde_json::from_value(items)?;
        let usage: Usage = serde_json::from_str(usage.body())?;
        let more = list.next_page().is_some();
        Ok(Caches {
            count: total.unwrap_or(usage.active_caches_count),
            used: usage.active_caches_size_in_bytes,
            limit,
            items: Listed { items, more },
        })
    }

    pub(super) async fn change_actions(&self, change: &ActionsChange) -> Result<(), ApiError> {
        use ActionsChange as C;
        use http::Method;
        let at = |repo, env: &Option<String>, kind, name: &str| {
            format!(
                "{}/{kind}/{}",
                base(repo, env.as_deref()),
                encode_path(name)
            )
        };
        match change {
            C::SetSecret {
                repo,
                env,
                name,
                sealed,
                key_id,
                ..
            } => {
                let body = json!({ "encrypted_value": sealed, "key_id": key_id });
                let path = at(repo, env, "secrets", name);
                self.rest_write(Method::PUT, &path, Some(&body)).await
            }
            C::DeleteSecret { repo, env, name } => {
                let path = at(repo, env, "secrets", name);
                self.rest_write(Method::DELETE, &path, None).await
            }
            C::SetVariable {
                repo,
                env,
                from: None,
                name,
                value,
            } => {
                let body = json!({ "name": name, "value": value });
                let path = format!("{}/variables", base(repo, env.as_deref()));
                self.rest_write(Method::POST, &path, Some(&body)).await
            }
            C::SetVariable {
                repo,
                env,
                from: Some(from),
                name,
                value,
            } => {
                let body = json!({ "name": name, "value": value });
                let path = at(repo, env, "variables", from);
                self.rest_write(Method::PATCH, &path, Some(&body)).await
            }
            C::DeleteVariable { repo, env, name } => {
                let path = at(repo, env, "variables", name);
                self.rest_write(Method::DELETE, &path, None).await
            }
            C::DeleteCache { repo, id, .. } => {
                let path = format!("/repos/{repo}/actions/caches/{id}");
                self.rest_write(Method::DELETE, &path, None).await
            }
            C::DeleteCaches { repo, key, .. } => {
                let path = format!("/repos/{repo}/actions/caches?key={}", encode_path(key));
                self.rest_write(Method::DELETE, &path, None).await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sealed value opens with the key's secret half, and only with it.
    #[test]
    fn a_secret_is_sealed_for_its_key() {
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD;
        let secret = crypto_box::SecretKey::generate(&mut crypto_box::aead::OsRng);
        let key = SealKey {
            key_id: "568250167242549743".into(),
            key: b64.encode(secret.public_key().as_bytes()),
        };
        let sealed = b64.decode(seal(&key, "hunter2").unwrap()).unwrap();
        assert_eq!(secret.unseal(&sealed).unwrap(), b"hunter2");
        let bad = SealKey {
            key: "AAAA".into(),
            ..key
        };
        seal(&bad, "x").unwrap_err();
    }
}
