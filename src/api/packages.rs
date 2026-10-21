//! GitHub Packages, over REST: an owner's packages (a repository's are
//! its owner's that it published), a package's versions and how to pull
//! or install one, and deleting a version. Reading takes a classic
//! token's `read:packages` scope, deleting `delete:packages` too, which
//! gh doesn't ask for; a refusal says which is missing.

use serde::{Deserialize, Serialize};

use crate::api::browse::Results;
use crate::api::client::{Answer, ApiError, GitHub};
use crate::api::model::RepoId;

words! {
    /// A kind of package, as GitHub's API names them.
    #[derive(Hash, Serialize, Deserialize)]
    pub enum Kind {
        /// As GitHub's API and URLs say it (`container`).
        pub fn slug -> pub fn from_slug,
        /// As a person says it.
        pub fn name,
    }
    Container "container" "Container";
    Npm       "npm"       "npm";
    Maven     "maven"     "Maven";
    Rubygems  "rubygems"  "RubyGems";
    Nuget     "nuget"     "NuGet";
    /// Docker's old registry (`docker.pkg.github.com`).
    Docker    "docker"    "Docker";
}

/// Whose packages.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PackagesOf {
    /// A user's or an organization's.
    Owner(String),
    /// The ones a repository published.
    Repo(RepoId),
}

impl PackagesOf {
    pub fn owner(&self) -> &str {
        match self {
            PackagesOf::Owner(login) => login,
            PackagesOf::Repo(repo) => &repo.owner,
        }
    }
}

/// A package, by its owner, kind and name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PackageRef {
    pub owner: String,
    pub kind: Kind,
    pub name: String,
}

impl PackageRef {
    /// Its REST path, under `/orgs/` for an organization's.
    pub fn path(&self, org: bool) -> String {
        let name = crate::api::client::encode_path(&self.name);
        format!(
            "/{}/{}/packages/{}/{name}",
            if org { "orgs" } else { "users" },
            self.owner,
            self.kind.slug()
        )
    }
}

/// A package, as its list shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Package {
    pub at: PackageRef,
    /// The repository it's linked to.
    pub repo: Option<RepoId>,
    /// `public`, `private` or `internal`.
    pub visibility: String,
    pub versions: u64,
    pub updated_at: Option<String>,
}

/// An owner's packages, of every kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageList {
    pub items: Vec<Package>,
    /// Kinds with more than one page (100) of packages, only the first
    /// of which is listed.
    pub cut: Vec<Kind>,
}

/// One version of a package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    pub id: u64,
    /// Its name: a version number, or a container image's digest.
    pub name: String,
    /// A container image's tags.
    pub tags: Vec<String>,
    pub created_at: Option<String>,
}

/// A package, and a page of its versions, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageDetail {
    pub package: Package,
    pub versions: Results<Version>,
}

impl crate::api::browse::Page for PackageDetail {
    fn counts(&self) -> (usize, u64) {
        self.versions.counts()
    }
}

/// How to pull or install `version` of `p`, as GitHub's package page
/// says: the command, and what else it needs.
pub fn install(p: &PackageRef, repo: Option<&RepoId>, version: &Version) -> Vec<String> {
    let host = crate::host::current();
    let registry = |sub: &str, github: &str| {
        if host.is_enterprise() {
            format!("{sub}.{}", host.name())
        } else {
            github.to_owned()
        }
    };
    let owner = p.owner.to_lowercase();
    let (name, v) = (&p.name, &version.name);
    match p.kind {
        Kind::Container => {
            let at = match version.tags.first() {
                Some(tag) => format!(":{tag}"),
                None => format!("@{v}"),
            };
            let registry = registry("containers", "ghcr.io");
            vec![format!("docker pull {registry}/{owner}/{name}{at}")]
        }
        Kind::Docker => {
            let registry = registry("docker", "docker.pkg.github.com");
            let repo = repo.map_or_else(|| "REPOSITORY".to_owned(), |r| r.name.to_lowercase());
            vec![format!("docker pull {registry}/{owner}/{repo}/{name}:{v}")]
        }
        Kind::Npm => {
            let registry = registry("npm", "npm.pkg.github.com");
            vec![format!(
                "npm install @{owner}/{name}@{v} --registry=https://{registry}"
            )]
        }
        Kind::Rubygems => {
            let registry = registry("rubygems", "rubygems.pkg.github.com");
            vec![format!(
                "gem install {name} --version \"{v}\" --source \"https://{registry}/{owner}\""
            )]
        }
        Kind::Nuget => {
            let registry = registry("nuget", "nuget.pkg.github.com");
            vec![
                format!("dotnet add package {name} --version {v}"),
                format!("from the source https://{registry}/{owner}/index.json"),
            ]
        }
        Kind::Maven => {
            // GitHub names a Maven package `groupId.artifactId`.
            let (group, artifact) = name.rsplit_once('.').unwrap_or((name, name));
            let registry = registry("maven", "maven.pkg.github.com");
            vec![
                format!(
                    "<dependency><groupId>{group}</groupId><artifactId>{artifact}</artifactId><version>{v}</version></dependency>"
                ),
                format!("from the repository https://{registry}/{owner}/*"),
            ]
        }
    }
}

/// What a refusal of packages means, given the token's scopes (when
/// GitHub said them).
fn refusal(status: u16, message: &str, scopes: Option<&str>, deleting: bool) -> Option<String> {
    let has = |scope: &str| scopes.is_some_and(|s| crate::api::has_scope(s, scope));
    let reads = has("read:packages") || has("write:packages");
    let lacks = |scope: &str| crate::api::scope_missing("Packages", scope);
    match (status, scopes) {
        (401 | 403, Some(_)) if !reads => Some(lacks("read:packages")),
        (401 | 403, Some(_)) if deleting && !has("delete:packages") => {
            Some(lacks("delete:packages"))
        }
        (403, _) if message.to_lowercase().contains("scope") => {
            Some(format!("GitHub refused: {message}"))
        }
        _ => None,
    }
}

/// Versions on a page.
const VERSIONS: usize = 30;

impl GitHub {
    /// GitHub's answer for a packages path, or what its refusal means.
    async fn packages_answer(&self, path: &str, what: &str) -> Result<Answer, ApiError> {
        let answer = self.rest_refusable(path, what, |status, message, answer| {
            refusal(status, message, answer.header("x-oauth-scopes"), false)
        });
        answer.await?.map_err(ApiError::Denied)
    }

    /// Whether `login` is an organization.
    async fn is_org(&self, login: &str) -> Result<bool, ApiError> {
        #[derive(Deserialize)]
        struct Owner {
            #[serde(rename = "type")]
            kind: String,
        }
        let path = format!("/users/{login}");
        let owner: Owner = self.rest_json(&path, format!("@{login}")).await?;
        Ok(owner.kind == "Organization")
    }

    /// An owner's packages of every kind (or those a repository
    /// published), most recently updated first.
    pub async fn packages(&self, of: &PackagesOf) -> Result<PackageList, ApiError> {
        let owner = of.owner();
        let org = self.is_org(owner).await?;
        let base = format!("/{}/{owner}/packages", if org { "orgs" } else { "users" });
        let pages = futures::future::join_all(Kind::ALL.into_iter().map(|kind| {
            let path = format!("{base}?package_type={}&per_page=100", kind.slug());
            async move {
                let answer = self
                    .packages_answer(&path, &format!("@{owner}'s packages"))
                    .await?;
                let found: Vec<wire::Package> = serde_json::from_str(answer.body())?;
                Ok::<_, ApiError>((kind, found, answer.next_page().is_some()))
            }
        }))
        .await;
        let (mut items, mut cut) = (Vec::new(), Vec::new());
        for page in pages {
            let (kind, found, more) = page?;
            if more {
                cut.push(kind);
            }
            items.extend(found.into_iter().map(|p| p.into_package(owner, kind)));
        }
        if let PackagesOf::Repo(repo) = of {
            items.retain(|p| p.repo.as_ref() == Some(repo));
        }
        items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        let list = PackageList { items, cut };
        Ok(list)
    }

    /// A package and its versions, newest first, 30 at a time from
    /// `after`.
    pub async fn package(
        &self,
        p: &PackageRef,
        after: Option<String>,
    ) -> Result<PackageDetail, ApiError> {
        let what = format!("the package {}", p.name);
        let base = p.path(self.is_org(&p.owner).await?);
        let mut path = format!("{base}/versions?per_page={VERSIONS}");
        if let Some(after) = &after {
            path.extend(["&", after]);
        }
        let (package, answer) = tokio::try_join!(
            self.packages_answer(&base, &what),
            self.packages_answer(&path, &what),
        )?;
        let package: wire::Package = serde_json::from_str(package.body())?;
        let package = package.into_package(&p.owner, p.kind);
        let found: Vec<wire::Version> = serde_json::from_str(answer.body())?;
        let items: Vec<Version> = found.into_iter().map(wire::Version::into_version).collect();
        let next = answer.next_page().filter(|_| items.len() >= VERSIONS);
        let detail = PackageDetail {
            versions: Results {
                total: package.versions,
                items,
                next,
            },
            package,
        };
        Ok(self.checked(detail, format!("package {}", p.name)))
    }

    /// Deletes a package's version.
    pub async fn delete_version(&self, p: &PackageRef, id: u64) -> Result<(), ApiError> {
        let base = p.path(self.is_org(&p.owner).await?);
        let path = format!("{base}/versions/{id}");
        let result = self.rest_write(http::Method::DELETE, &path, None).await;
        match result {
            Err(ApiError::Http {
                status: 403,
                message,
            }) => {
                // The scopes GitHub said, from a read of the package.
                let scopes = match self.rest_answer(&base).await {
                    Ok(answer) => answer.header("x-oauth-scopes").map(str::to_owned),
                    Err(_) => None,
                };
                Err(ApiError::Denied(
                    refusal(403, &message, scopes.as_deref(), true)
                        .unwrap_or_else(|| format!("GitHub refused: {message}")),
                ))
            }
            other => other,
        }
    }
}

mod wire {
    use serde::Deserialize;

    use crate::api::model::RepoId;

    #[derive(Deserialize)]
    pub struct Package {
        name: String,
        #[serde(default)]
        visibility: String,
        #[serde(default)]
        version_count: u64,
        updated_at: Option<String>,
        repository: Option<Repo>,
    }

    #[derive(Deserialize)]
    struct Repo {
        full_name: String,
    }

    impl Package {
        pub fn into_package(self, owner: &str, kind: super::Kind) -> super::Package {
            super::Package {
                at: super::PackageRef {
                    owner: owner.to_owned(),
                    kind,
                    name: self.name,
                },
                repo: self.repository.and_then(|r| RepoId::parse(&r.full_name)),
                visibility: self.visibility,
                versions: self.version_count,
                updated_at: self.updated_at,
            }
        }
    }

    #[derive(Deserialize)]
    pub struct Version {
        id: u64,
        name: String,
        created_at: Option<String>,
        metadata: Option<Metadata>,
    }

    #[derive(Deserialize)]
    struct Metadata {
        container: Option<Container>,
    }

    #[derive(Deserialize)]
    struct Container {
        #[serde(default)]
        tags: Vec<String>,
    }

    impl Version {
        pub fn into_version(self) -> super::Version {
            super::Version {
                id: self.id,
                name: self.name,
                tags: (self.metadata.and_then(|m| m.container))
                    .map(|c| c.tags)
                    .unwrap_or_default(),
                created_at: self.created_at,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(kind: Kind, name: &str) -> PackageRef {
        PackageRef {
            owner: "Octo-Org".into(),
            kind,
            name: name.into(),
        }
    }

    fn version(name: &str, tags: &[&str]) -> Version {
        Version {
            id: 1,
            name: name.into(),
            tags: tags.iter().map(|t| (*t).to_owned()).collect(),
            created_at: None,
        }
    }

    #[test]
    fn each_kind_says_how_to_get_it() {
        let image = version("sha256:abc", &["latest"]);
        assert_eq!(
            install(&at(Kind::Container, "app"), None, &image),
            ["docker pull ghcr.io/octo-org/app:latest"]
        );
        let untagged = version("sha256:abc", &[]);
        assert_eq!(
            install(&at(Kind::Container, "app"), None, &untagged),
            ["docker pull ghcr.io/octo-org/app@sha256:abc"]
        );
        assert_eq!(
            install(&at(Kind::Npm, "lib"), None, &version("1.2.0", &[])),
            ["npm install @octo-org/lib@1.2.0 --registry=https://npm.pkg.github.com"]
        );
        let maven = install(
            &at(Kind::Maven, "com.example.app"),
            None,
            &version("2.0", &[]),
        );
        assert!(maven.first().is_some_and(|l| {
            l.contains("<groupId>com.example</groupId><artifactId>app</artifactId>")
        }));
    }

    #[test]
    fn a_missing_scope_is_named() {
        let why = refusal(
            403,
            "You need at least read:packages scope to list packages.",
            Some("repo, gist"),
            false,
        );
        assert!(why.is_some_and(|w| w.contains("`gh auth refresh -s read:packages`")));
        let why = refusal(403, "Forbidden", Some("repo, read:packages"), true);
        assert!(why.is_some_and(|w| w.contains("delete:packages")));
        assert_eq!(
            refusal(404, "Not Found", Some("read:packages"), false),
            None
        );
    }

    #[test]
    fn a_package_path_says_whose_it_is() {
        assert_eq!(
            at(Kind::Container, "my app").path(true),
            "/orgs/Octo-Org/packages/container/my%20app"
        );
    }
}
