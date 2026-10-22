//! Commits made from objects alone, through an index of their own, to
//! push by their IDs: a clone's branches, index and working tree stay as
//! they are. A wiki's pages ([`crate::wiki`]) and the files housekeeping
//! commits ([`crate::housekeeping`]) are saved this way.

use crate::git::GitError;
use crate::git::repo::Repo;

/// What a commit changes, and says.
pub struct Change<'a> {
    /// Paths and their new text.
    pub additions: &'a [(String, String)],
    pub deletions: &'a [String],
    pub message: &'a str,
    /// Your GitHub login: the commit's author when git has no name for
    /// you.
    pub login: Option<&'a str>,
}

/// The blob at `rev:path`, if there's one.
pub async fn blob(repo: &Repo, rev: &str, path: &str) -> Option<String> {
    let spec = format!("{rev}:{path}");
    let args = ["rev-parse", "--verify", "--quiet", &spec];
    let out = repo.run(&args).await;
    out.ok().map(|o| o.trim().to_owned())
}

/// Whether GitHub refused a push because the branch moved on since it
/// was read: worth reading it again and committing once more.
pub fn moved_on(err: &GitError) -> bool {
    matches!(err, GitError::Failed { stderr, .. }
        if stderr.contains("[rejected]") || stderr.contains("fetch first"))
}

/// The mode of `path` at `rev` (an executable stays one), else a plain
/// file's.
async fn mode(repo: &Repo, rev: &str, path: &str) -> String {
    let out = repo.run(&["ls-tree", rev, "--", path]).await;
    out.ok()
        .and_then(|o| o.split_whitespace().next().map(str::to_owned))
        .filter(|m| m == "100755")
        .unwrap_or_else(|| "100644".to_owned())
}

/// The commit of `change` on `parent`, by you as git knows you (else as
/// your login's GitHub address).
pub async fn make(repo: &Repo, parent: &str, change: &Change<'_>) -> Result<String, GitError> {
    let index = repo
        .run(&["rev-parse", "--git-path", "ghtui-edit-index"])
        .await?;
    let index = repo.path.join(index.trim());
    let index = index.with_file_name(format!("ghtui-edit-index-{}", std::process::id()));
    let index_path = index.to_string_lossy().into_owned();
    let env = [("GIT_INDEX_FILE", index_path.as_str())];
    let tree = async {
        repo.git(&["read-tree", parent], &env, None).await?;
        for path in change.deletions {
            let args = ["update-index", "--force-remove", "--", path];
            repo.git(&args, &env, None).await?;
        }
        for (path, text) in change.additions {
            // A renamed file keeps its mode.
            let was = change
                .deletions
                .first()
                .map_or(path.as_str(), String::as_str);
            let mode = mode(repo, parent, was).await;
            let blob = repo
                .git(&["hash-object", "-w", "--stdin"], &[], Some(text.clone()))
                .await?;
            let info = format!("{mode},{},{path}", blob.trim());
            repo.git(&["update-index", "--add", "--cacheinfo", &info], &env, None)
                .await?;
        }
        // A partial clone lacks the blobs it didn't need.
        repo.git(&["write-tree", "--missing-ok"], &env, None).await
    }
    .await;
    let _ = std::fs::remove_file(&index);
    let tree = tree?;
    let config = |key: &'static str| crate::git::local::config(&repo.path, key);
    let login = change.login.unwrap_or("ghtui");
    let name = (config("user.name").await).unwrap_or_else(|| login.to_owned());
    let email =
        (config("user.email").await).unwrap_or_else(|| format!("{login}@users.noreply.github.com"));
    let who = [
        ("GIT_AUTHOR_NAME", name.as_str()),
        ("GIT_AUTHOR_EMAIL", email.as_str()),
        ("GIT_COMMITTER_NAME", name.as_str()),
        ("GIT_COMMITTER_EMAIL", email.as_str()),
    ];
    let args = ["commit-tree", tree.trim(), "-p", parent, "-F", "-"];
    let made = repo
        .git(&args, &who, Some(change.message.to_owned()))
        .await?;
    Ok(made.trim().to_owned())
}
