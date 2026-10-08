//! Git access through the `git` CLI.
//!
//! All network operations and blob reads go through `git` itself so that
//! partial clones fetch missing objects transparently. See [`repo`] for
//! repository selection, fetching and prefetching, and [`blobs`] for the
//! long-lived `cat-file --batch` reader.

pub mod blobs;
pub mod credentials;
pub mod files;
pub mod repo;

use std::path::Path;
use std::process::Stdio;

use tokio::process::Command;

/// Oldest git we support. Partial clone, `--filter` and fetching objects by
/// ID are all mature by this release.
pub const MIN_VERSION: Version = Version(2, 36, 0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u32, pub u32, pub u32);

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("could not run git: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("git {args} failed: {stderr}")]
    Failed { args: String, stderr: String },
    #[error("unrecognized `git --version` output: {0}")]
    Version(String),
    #[error("unexpected git output: {0}")]
    Parse(String),
    #[error("git {what} stalled (no progress for {secs}s)")]
    Stalled { what: String, secs: u64 },
}

pub use oid::Oid;

mod oid {
    use crate::GitError;

    /// A git object ID: 40 hex digits, lowercased. Its own type so it can't
    /// be mixed up with a ref name, a short SHA, a path or a GraphQL node
    /// ID; reads as a `&str`. [`Oid::parse`] is the only way to make one.
    #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
    pub struct Oid(String);

    impl Oid {
        /// Exactly 40 hex digits, in either case.
        pub fn parse(s: &str) -> Result<Oid, GitError> {
            if s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit()) {
                Ok(Oid(s.to_ascii_lowercase()))
            } else {
                Err(GitError::Parse(format!("not a commit id: {s}")))
            }
        }

        /// The all-zero ID git uses for a side that doesn't exist.
        pub fn is_zero(&self) -> bool {
            self.0.bytes().all(|b| b == b'0')
        }
    }

    impl std::ops::Deref for Oid {
        type Target = str;
        fn deref(&self) -> &str {
            &self.0
        }
    }

    impl std::fmt::Display for Oid {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.0)
        }
    }
}

/// A configured remote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    pub name: String,
    pub url: String,
}

/// `owner/name` of a GitHub repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubRepo {
    pub owner: String,
    pub name: String,
}

/// A `git` command with a predictable environment: never prompts (a prompt
/// would hang behind the TUI, so HTTPS prompts are off and SSH runs in batch
/// mode unless the user configured their own SSH command), gives up on a
/// dead connection instead of waiting forever, and uses untranslated output.
pub(crate) fn git(dir: Option<&Path>) -> Command {
    let mut cmd = Command::new("git");
    if let Some(dir) = dir {
        cmd.arg("-C").arg(dir);
    }
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .kill_on_drop(true);
    // HTTPS: abort when under 1 KB/s for a minute.
    for (key, value) in [
        ("GIT_HTTP_LOW_SPEED_LIMIT", "1000"),
        ("GIT_HTTP_LOW_SPEED_TIME", "60"),
    ] {
        if std::env::var_os(key).is_none() {
            cmd.env(key, value);
        }
    }
    if std::env::var_os("GIT_SSH_COMMAND").is_none() && std::env::var_os("GIT_SSH").is_none() {
        cmd.env(
            "GIT_SSH_COMMAND",
            "ssh -o BatchMode=yes -o ServerAliveInterval=15 -o ServerAliveCountMax=4",
        );
    }
    cmd
}

/// A child's stdio handle that was set to `Stdio::piped()`.
pub(crate) fn piped<T>(handle: Option<T>, name: &str) -> Result<T, GitError> {
    handle.ok_or_else(|| std::io::Error::other(format!("git {name} not piped")).into())
}

pub(crate) async fn run(dir: Option<&Path>, args: &[&str]) -> Result<String, GitError> {
    stdout(&git(dir).args(args).output().await?, &args.join(" "))
}

/// What `git <what>` printed, or its complaint if it failed.
pub(crate) fn stdout(output: &std::process::Output, what: &str) -> Result<String, GitError> {
    if !output.status.success() {
        return Err(GitError::Failed {
            args: what.to_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub async fn version() -> Result<Version, GitError> {
    let out = run(None, &["--version"]).await?;
    parse_version(&out).ok_or(GitError::Version(out.trim().to_owned()))
}

/// Parses `git version 2.51.0` (also `2.39.3 (Apple Git-146)`, `2.45.windows.1`).
pub fn parse_version(s: &str) -> Option<Version> {
    let rest = s.trim().strip_prefix("git version ")?;
    let mut parts = rest
        .split(|c: char| !c.is_ascii_digit())
        .filter(|p| !p.is_empty())
        .map(|p| p.parse::<u32>().ok());
    Some(Version(
        parts.next()??,
        parts.next()??,
        parts.next().flatten().unwrap_or(0),
    ))
}

/// Remotes of the repository containing `dir`; empty if `dir` isn't in a
/// repository.
pub async fn remotes(dir: &Path) -> Result<Vec<Remote>, GitError> {
    let out = match run(Some(dir), &["config", "--get-regexp", r"^remote\..*\.url$"]).await {
        Ok(out) => out,
        // Exit status 1: no matches. 128: not a repository.
        Err(GitError::Failed { .. }) => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    Ok(parse_remotes(&out))
}

fn parse_remotes(config: &str) -> Vec<Remote> {
    config
        .lines()
        .filter_map(|line| {
            let (key, url) = line.split_once(' ')?;
            let name = key.strip_prefix("remote.")?.strip_suffix(".url")?;
            Some(Remote {
                name: name.to_owned(),
                url: url.trim().to_owned(),
            })
        })
        .collect()
}

/// Extracts `owner/name` from a github.com remote URL in any of the usual
/// forms (HTTPS, scp-style SSH, `ssh://`, `git://`).
pub fn github_repo_from_url(url: &str) -> Option<GitHubRepo> {
    let url = url.trim();
    let path = if let Some(rest) = url.strip_prefix("git@github.com:") {
        rest
    } else {
        let (_, rest) = url.split_once("://")?;
        let (authority, path) = rest.split_once('/')?;
        let host = authority.rsplit('@').next()?;
        let host = host.split(':').next()?;
        if !host.eq_ignore_ascii_case("github.com") && !host.eq_ignore_ascii_case("www.github.com")
        {
            return None;
        }
        path
    };
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, name) = path.split_once('/')?;
    if owner.is_empty() || name.is_empty() || name.contains('/') {
        return None;
    }
    Some(GitHubRepo {
        owner: owner.to_owned(),
        name: name.to_owned(),
    })
}

/// The repository a bare PR number most likely refers to. In a fork clone,
/// `upstream` is the base repository, so it wins over `origin`; otherwise the
/// first github.com remote.
pub fn infer_github_repo(remotes: &[Remote]) -> Option<GitHubRepo> {
    let by_name = |name: &str| {
        remotes
            .iter()
            .find(|r| r.name == name)
            .and_then(|r| github_repo_from_url(&r.url))
    };
    by_name("upstream")
        .or_else(|| by_name("origin"))
        .or_else(|| remotes.iter().find_map(|r| github_repo_from_url(&r.url)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(owner: &str, name: &str) -> Option<GitHubRepo> {
        Some(GitHubRepo {
            owner: owner.into(),
            name: name.into(),
        })
    }

    #[test]
    fn parses_versions() {
        assert_eq!(
            parse_version("git version 2.51.0\n"),
            Some(Version(2, 51, 0))
        );
        assert_eq!(
            parse_version("git version 2.39.3 (Apple Git-146)"),
            Some(Version(2, 39, 3))
        );
        assert_eq!(
            parse_version("git version 2.45.2.windows.1"),
            Some(Version(2, 45, 2))
        );
        assert_eq!(parse_version("git version 3.0"), Some(Version(3, 0, 0)));
        assert_eq!(parse_version("hg version 1"), None);
        assert!(Version(2, 51, 0) >= MIN_VERSION);
        assert!(Version(2, 35, 9) < MIN_VERSION);
    }

    #[test]
    fn parses_github_urls() {
        for url in [
            "https://github.com/o/r.git",
            "https://github.com/o/r",
            "https://github.com/o/r/",
            "https://user@github.com/o/r.git",
            "git@github.com:o/r.git",
            "git@github.com:o/r",
            "ssh://git@github.com/o/r.git",
            "ssh://git@github.com:22/o/r.git",
            "git://github.com/o/r.git",
        ] {
            assert_eq!(github_repo_from_url(url), repo("o", "r"), "{url}");
        }
        for url in [
            "https://gitlab.com/o/r.git",
            "git@gitlab.com:o/r.git",
            "/local/path/repo",
            "https://github.com/o",
            "https://github.com/o/r/extra",
        ] {
            assert_eq!(github_repo_from_url(url), None, "{url}");
        }
    }

    #[test]
    fn upstream_wins_over_origin() {
        let remotes = parse_remotes(
            "remote.origin.url git@github.com:me/r.git\nremote.upstream.url https://github.com/them/r.git\n",
        );
        assert_eq!(infer_github_repo(&remotes), repo("them", "r"));
    }

    #[test]
    fn falls_back_to_any_github_remote() {
        let remotes = parse_remotes(
            "remote.origin.url git@gitlab.com:me/r.git\nremote.gh.url https://github.com/them/r\n",
        );
        assert_eq!(infer_github_repo(&remotes), repo("them", "r"));
        assert_eq!(infer_github_repo(&[]), None);
    }

    #[tokio::test]
    async fn reads_remotes_from_a_real_repo() {
        let dir = tempfile::tempdir().unwrap();
        run(Some(dir.path()), &["init", "-q"]).await.unwrap();
        assert_eq!(remotes(dir.path()).await.unwrap(), Vec::new());
        run(
            Some(dir.path()),
            &["remote", "add", "origin", "git@github.com:o/r.git"],
        )
        .await
        .unwrap();
        let found = remotes(dir.path()).await.unwrap();
        assert_eq!(infer_github_repo(&found), repo("o", "r"));
    }

    #[tokio::test]
    async fn not_a_repo_has_no_remotes() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(remotes(dir.path()).await.unwrap(), Vec::new());
    }

    #[tokio::test]
    async fn installed_git_is_supported() {
        assert!(version().await.unwrap() >= MIN_VERSION);
    }
}
