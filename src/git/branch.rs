//! A pull request's head branch on GitHub, from the clone its diff is in
//! (yours or the cache's): where it's fetched from and pushed to, its
//! tip, and a commit pushed to it by its ID. The clone's own branches,
//! index and working tree stay as they are.

use crate::git::repo::Repo;
use crate::git::{GitError, Oid, github_repo_from_url, remotes};

/// Where `repo` reaches `owner/name`: the remote it fetches from when
/// that's the one, else another of its remotes that is, else its URL on
/// GitHub (a fork's, for a pull request from one).
pub async fn remote_for(repo: &Repo, base: (&str, &str), head: (&str, &str)) -> String {
    let same = |(o, n): (&str, &str), (p, m): (&str, &str)| {
        o.eq_ignore_ascii_case(p) && n.eq_ignore_ascii_case(m)
    };
    if same(base, head) {
        return repo.remote.clone();
    }
    let (owner, name) = head;
    let listed = remotes(&repo.path).await.unwrap_or_default();
    let found = listed
        .into_iter()
        .find(|r| github_repo_from_url(&r.url).is_some_and(|g| same((&g.owner, &g.name), head)));
    found.map_or_else(|| crate::host::current().git_url(owner, name), |r| r.name)
}

/// GitHub's tip of `branch` on `remote`, fetched to `into` (a ref name).
pub async fn tip(repo: &Repo, remote: &str, branch: &str, into: &str) -> Result<Oid, GitError> {
    let spec = format!("+refs/heads/{branch}:{into}");
    let args = ["fetch", "--no-tags", "--no-write-fetch-head", remote, &spec];
    repo.git(&args, &[], None).await?;
    repo.rev_parse(into).await
}

/// Pushes commit `oid` to `branch` on `remote`. It's no force push:
/// GitHub refuses it once the branch has moved past what `oid` is on.
pub async fn push(repo: &Repo, remote: &str, oid: &str, branch: &str) -> Result<(), GitError> {
    let spec = format!("{oid}:refs/heads/{branch}");
    let args = ["push", "--quiet", remote, &spec];
    repo.git(&args, &[], None).await.map(drop)
}

/// What git said when GitHub refused a push, in its own words.
pub fn refused(err: &GitError) -> String {
    match err {
        GitError::Failed { stderr, .. } => {
            let lines: Vec<&str> = (stderr.lines())
                .map(str::trim)
                .filter(|l| l.starts_with("remote:") || l.starts_with("! ") || l.contains("error"))
                .map(|l| l.trim_start_matches("remote:").trim())
                .filter(|l| !l.is_empty())
                .collect();
            if lines.is_empty() {
                stderr.trim().to_owned()
            } else {
                lines.join(" ")
            }
        }
        err => err.to_string(),
    }
}
