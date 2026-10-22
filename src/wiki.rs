//! Wikis: GitHub keeps each in a git repository beside the repository's
//! (`<repo>.wiki.git`), one file per page, which no API serves, so ghtui
//! reads them through git, cloned into its cache like diffs.

use crate::api::ApiError;
use crate::api::browse::{WikiDiff, WikiFileDiff, WikiHistory, WikiPage, WikiRevision};
use crate::api::model::Capped;
use crate::api::model::RepoId;
use crate::git::commit;
use crate::git::repo::Repo;

use crate::diff_job::GitContext;

pub(crate) mod form;

/// Where the wiki's latest pages are fetched to.
const HEAD: &str = "refs/ghtui/wiki";

/// The markups GitHub renders wiki pages from.
const MARKUPS: &[&str] = &[
    "md",
    "markdown",
    "mediawiki",
    "wiki",
    "textile",
    "rdoc",
    "org",
    "creole",
    "rst",
    "asciidoc",
    "adoc",
    "pod",
];

/// A page's file name split into its title and markup.
fn page_file(path: &str) -> Option<(&str, &str)> {
    let (title, ext) = path.rsplit_once('.')?;
    (MARKUPS.iter().any(|m| m.eq_ignore_ascii_case(ext))).then_some((title, ext))
}

/// How a title appears in a page's URL: spaces as dashes.
pub fn slug(title: &str) -> String {
    title.replace(' ', "-")
}

/// Whether GitHub said the wiki's repository isn't there (one without
/// pages, or a repository without a wiki), as git reports its 404.
fn absent(err: &crate::git::GitError) -> bool {
    matches!(err, crate::git::GitError::Failed { stderr, .. }
        if stderr.contains("fatal: repository '") && stderr.contains("' not found"))
}

/// The wiki's clone in the cache, cloned on first use.
async fn open(git: &GitContext, repo: &RepoId) -> Result<Repo, crate::git::GitError> {
    let name = format!("{}.wiki", repo.name);
    let url = crate::host::current().git_url(&repo.owner, &name);
    Repo::open_cache(
        &git.cache_root,
        &repo.owner,
        &name,
        &url,
        git.credentials.clone(),
        &|_| {},
    )
    .await
}

/// A wiki's page (its Home without one; its list of pages for `_pages`).
pub async fn page(
    git: &GitContext,
    repo: &RepoId,
    page: Option<&str>,
) -> Result<WikiPage, ApiError> {
    let wiki = fetched(git, repo).await?;
    read(&wiki, page).await.map_err(|e| unread(repo, e))
}

/// The wiki's clone, with GitHub's latest pages fetched (offline, those
/// fetched before).
async fn fetched(git: &GitContext, repo: &RepoId) -> Result<Repo, ApiError> {
    let failed = |err: crate::git::GitError| {
        if absent(&err) {
            // GitHub makes a wiki's repository only with its first page.
            ApiError::NotFound(format!(
                "{repo}'s wiki: it's turned off, or has no pages yet (GitHub makes the first on github.com)"
            ))
        } else {
            unread(repo, Read::Git(err))
        }
    };
    let wiki = open(git, repo).await.map_err(failed)?;
    if let Err(err) = wiki.fetch_head(HEAD, &|_| {}).await
        && !wiki.has(HEAD).await
    {
        return Err(failed(err));
    }
    Ok(wiki)
}

/// What a failed read of `repo`'s wiki says.
fn unread(repo: &RepoId, err: Read) -> ApiError {
    match err {
        Read::Git(err) => ApiError::Git(format!("{repo}'s wiki: {err}")),
        Read::NoPage(wanted) => ApiError::NotFound(format!("{repo}'s wiki has no page {wanted}")),
    }
}

/// The most revisions a history lists.
const REVISIONS: usize = 100;

/// The most lines of a diff shown.
const DIFF_LINES: usize = 5000;

/// How `git log` and `git show` print a revision for [`revisions`].
const REVISION: &str = "--format=%x1e%H%x1f%aN%x1f%aI%x1f%s";

/// The wiki's history, or a page's (by its title), newest first.
pub async fn history(
    git: &GitContext,
    repo: &RepoId,
    page: Option<&str>,
) -> Result<WikiHistory, ApiError> {
    history_in(&fetched(git, repo).await?, repo, page).await
}

/// [`history`], in the wiki's clone.
async fn history_in(
    wiki: &Repo,
    repo: &RepoId,
    page: Option<&str>,
) -> Result<WikiHistory, ApiError> {
    let file = file_of(wiki, page).await.map_err(|e| unread(repo, e))?;
    let failed = |e: crate::git::GitError| unread(repo, Read::Git(e));
    let max = format!("--max-count={REVISIONS}");
    let mut log = vec!["log", REVISION, "--name-only", max.as_str(), HEAD];
    let mut count = vec!["rev-list", "--count", HEAD];
    if let Some(file) = &file {
        log.extend(["--", file.as_str()]);
        count.extend(["--", file.as_str()]);
    }
    let revisions = revisions(&wiki.run(&log).await.map_err(failed)?);
    let total = wiki.run(&count).await.map_err(failed)?;
    Ok(WikiHistory {
        page: page.map(str::to_owned),
        revisions: Capped::new(revisions, total.trim().parse().unwrap_or(0)),
    })
}

/// The page's file (none without a page: the whole wiki).
async fn file_of(wiki: &Repo, page: Option<&str>) -> Result<Option<String>, Read> {
    let Some(page) = page else { return Ok(None) };
    match read(wiki, Some(page)).await?.file {
        Some(file) => Ok(Some(file)),
        None => Err(Read::NoPage(page.to_owned())),
    }
}

/// [`REVISION`]'s revisions, each `␞oid␟author␟date␟subject` and the
/// files it changed.
fn revisions(out: &str) -> Vec<WikiRevision> {
    out.split('\u{1e}')
        .filter_map(|record| {
            let mut lines = record.lines();
            let mut f = lines.next()?.split('\u{1f}');
            let (oid, author, at) = (f.next()?, f.next()?, f.next()?);
            let pages = lines
                .filter_map(|l| page_file(l.trim()))
                .map(|(title, _)| title_of(title).to_owned())
                .collect();
            Some(WikiRevision {
                oid: oid.to_owned(),
                author: author.to_owned(),
                at: at.to_owned(),
                message: f.next().unwrap_or_default().to_owned(),
                pages,
            })
        })
        .collect()
}

/// A wiki's revisions as its compare URL names them: one (what it
/// changed), or two (`a...b`: what changed from one to the other).
/// Only SHAs: anything else names no revision.
pub fn compared(spec: &str) -> Option<(Option<&str>, &str)> {
    let sha = |s: &str| (4..=40).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit());
    match spec.split_once("...").or_else(|| spec.split_once("..")) {
        Some((a, b)) => (sha(a) && sha(b)).then_some((Some(a), b)),
        None => sha(spec).then_some((None, spec)),
    }
}

/// What the wiki's revisions `spec` ([`compared`]) changed, of a page
/// (by its title) or of all of them.
pub async fn diff(
    git: &GitContext,
    repo: &RepoId,
    page: Option<&str>,
    spec: &str,
) -> Result<WikiDiff, ApiError> {
    if compared(spec).is_none() {
        return Err(ApiError::NotFound(format!("{repo}'s wiki revision {spec}")));
    }
    diff_in(&fetched(git, repo).await?, repo, page, spec).await
}

/// [`diff`], in the wiki's clone.
async fn diff_in(
    wiki: &Repo,
    repo: &RepoId,
    page: Option<&str>,
    spec: &str,
) -> Result<WikiDiff, ApiError> {
    let missing = |rev: &str| ApiError::NotFound(format!("{repo}'s wiki revision {rev}"));
    let (from, to) = compared(spec).ok_or_else(|| missing(spec))?;
    let file = file_of(wiki, page).await.map_err(|e| unread(repo, e))?;
    let failed = |e: crate::git::GitError| unread(repo, Read::Git(e));
    let commit = format!("{to}^{{commit}}");
    let shown = wiki.run(&["show", "--no-patch", REVISION, &commit]).await;
    let to = (revisions(&shown.map_err(|_| missing(to))?)
        .into_iter()
        .next())
    .ok_or_else(|| missing(to))?;
    let mut args = match from {
        // A first revision has no parent: `show` diffs it with nothing.
        None => vec![
            "show",
            "--format=",
            "--patch",
            "--no-color",
            "--no-ext-diff",
            &to.oid,
        ],
        Some(from) => vec!["diff", "--no-color", "--no-ext-diff", from, &to.oid],
    };
    if let Some(file) = &file {
        args.extend(["--", file.as_str()]);
    }
    let out = wiki.run(&args).await.map_err(failed)?;
    let (files, cut) = file_diffs(&out);
    Ok(WikiDiff {
        from: from.map(str::to_owned),
        to,
        files,
        cut,
    })
}

/// A patch's files, by their pages' titles, and whether it was cut
/// short ([`DIFF_LINES`]).
fn file_diffs(patch: &str) -> (Vec<WikiFileDiff>, bool) {
    let mut files: Vec<WikiFileDiff> = Vec::new();
    let mut shown = 0;
    let mut hunks = false;
    for line in patch.lines() {
        if let Some(paths) = line.strip_prefix("diff --git ") {
            let path = paths.rsplit_once(" b/").map_or(paths, |(_, b)| b);
            let title = page_file(path).map_or(path, |(t, _)| title_of(t));
            files.push(WikiFileDiff {
                title: title.to_owned(),
                lines: Vec::new(),
            });
            hunks = false;
            continue;
        }
        // The header's `index`, `---` and `+++` lines say nothing here.
        hunks |= line.starts_with("@@");
        if !hunks {
            continue;
        }
        if shown == DIFF_LINES {
            return (files, true);
        }
        if let Some(file) = files.last_mut() {
            file.lines.push(line.to_owned());
            shown += 1;
        }
    }
    (files, false)
}

/// Why a page couldn't be read.
enum Read {
    Git(crate::git::GitError),
    NoPage(String),
}

impl From<crate::git::GitError> for Read {
    fn from(err: crate::git::GitError) -> Self {
        Read::Git(err)
    }
}

/// A page of the wiki, as fetched last.
async fn read(wiki: &Repo, page: Option<&str>) -> Result<WikiPage, Read> {
    let commit = wiki.rev_parse(HEAD).await?.to_string();
    let files = wiki.file_names(&commit).await?;
    let pages: Vec<(&str, &str, &String)> = files
        .iter()
        .filter_map(|f| page_file(f).map(|(title, ext)| (title, ext, f)))
        .collect();
    let mut titles: Vec<String> = (pages.iter())
        .map(|(t, _, _)| title_of(t))
        .filter(|t| !t.starts_with('_'))
        .map(str::to_owned)
        .collect();
    titles.sort_by_cached_key(|t| t.to_lowercase());
    let find = |wanted: &str| {
        pages
            .iter()
            .find(|(t, _, _)| same_page(title_of(t), wanted))
    };
    let sidebar = match find("_Sidebar") {
        Some((_, _, path)) => Some(wiki.file_text(&commit, path).await?),
        None => None,
    };
    let wanted = page.unwrap_or("Home");
    let found = if wanted == "_pages" {
        None
    } else {
        find(wanted)
    };
    let (title, text, markdown, file) = match found {
        Some((t, ext, path)) => {
            let text = wiki.file_text(&commit, path).await?;
            let markdown = ["md", "markdown"]
                .iter()
                .any(|m| m.eq_ignore_ascii_case(ext));
            (
                Some(title_of(t).to_owned()),
                Some(text),
                markdown,
                Some((*path).clone()),
            )
        }
        // The list of pages, also what a wiki without a Home shows.
        None if page.is_none() || wanted == "_pages" => (None, None, false, None),
        None => return Err(Read::NoPage(wanted.to_owned())),
    };
    Ok(WikiPage {
        title,
        text,
        markdown,
        pages: titles,
        sidebar,
        commit: Some(commit),
        file,
    })
}

/// A page's title from its file's path without the extension: pages are
/// files at any depth, named by their file names.
fn title_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Whether two titles name one page, as its URL does: spaces as dashes,
/// in any case.
fn same_page(a: &str, b: &str) -> bool {
    let url = |c: u8| {
        if c == b' ' {
            b'-'
        } else {
            c.to_ascii_lowercase()
        }
    };
    a.len() == b.len() && a.bytes().zip(b.bytes()).all(|(x, y)| url(x) == url(y))
}

// ---- saving a page ------------------------------------------------------------

/// A page to save: a new one (without `file`), or the one at `file` as it
/// was at `base`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Save {
    pub repo: RepoId,
    pub title: String,
    pub file: Option<String>,
    pub base: Option<String>,
    pub text: String,
    /// Your GitHub login: the commit's author when git has no name for
    /// you.
    pub login: Option<String>,
}

/// Commits a page to the wiki and pushes it, with the git credentials
/// ghtui uses: its title, as its URL names it.
pub async fn save(git: &GitContext, save: &Save) -> Result<String, String> {
    let repo = &save.repo;
    let wiki = open(git, repo)
        .await
        .map_err(|err| unsaved(repo, "read", &err))?;
    save_in(&wiki, save).await
}

/// Why git couldn't `doing` the wiki to save a page: when GitHub has no
/// repository for it yet, that it makes one only with the first page.
fn unsaved(repo: &RepoId, doing: &str, err: &crate::git::GitError) -> String {
    if absent(err) {
        format!(
            "{repo}'s wiki has no pages yet, and GitHub makes its git repository only with the first page: create that on github.com"
        )
    } else {
        format!("Couldn't {doing} {repo}'s wiki: {err}")
    }
}

/// [`save`], in the wiki's clone.
async fn save_in(wiki: &Repo, save: &Save) -> Result<String, String> {
    let repo = &save.repo;
    let failed = |err: crate::git::GitError| format!("Couldn't save the page: {err}");
    // Once more if GitHub's wiki moves on between looking and pushing.
    for attempt in 0..2 {
        (wiki.fetch_head(HEAD, &|_| {}).await).map_err(|err| unsaved(repo, "fetch", &err))?;
        let head = wiki.rev_parse(HEAD).await.map_err(failed)?.to_string();
        let file = match &save.file {
            Some(file) => {
                let changed = match &save.base {
                    Some(base) if *base != head => {
                        commit::blob(wiki, base, file).await
                            != commit::blob(wiki, &head, file).await
                    }
                    _ => false,
                };
                if changed {
                    return Err(format!(
                        "{} changed on GitHub since you opened it: saving would overwrite that, so it isn't saved. Your text is kept here; open the page again to see GitHub's",
                        save.title
                    ));
                }
                file.clone()
            }
            None => {
                let files = wiki.file_names(&head).await.map_err(failed)?;
                let taken = (files.iter())
                    .filter_map(|f| page_file(f))
                    .any(|(t, _)| same_page(title_of(t), &save.title));
                if taken {
                    return Err(format!(
                        "The wiki has a page named {} already: choose another title",
                        save.title
                    ));
                }
                format!("{}.md", slug(&save.title))
            }
        };
        let made = make(wiki, &head, &file, save).await.map_err(failed)?;
        let branch = branch(wiki).await;
        let spec = format!("{made}:refs/heads/{branch}");
        let push = ["push", "--quiet", wiki.remote.as_str(), spec.as_str()];
        match wiki.git(&push, &[], None).await {
            Ok(_) => {
                wiki.git(&["update-ref", HEAD, &made], &[], None)
                    .await
                    .map_err(failed)?;
                return Ok(slug(&save.title));
            }
            Err(err) if attempt == 0 && commit::moved_on(&err) => {}
            Err(crate::git::GitError::Failed { stderr, .. }) => {
                return Err(format!("GitHub refused the page: {}", stderr.trim()));
            }
            Err(err) => return Err(failed(err)),
        }
    }
    Err(format!(
        "{repo}'s wiki kept changing while saving: save again"
    ))
}

/// The branch the wiki's pages are on: its remote's `HEAD`.
async fn branch(wiki: &Repo) -> String {
    let out = wiki
        .git(&["ls-remote", "--symref", &wiki.remote, "HEAD"], &[], None)
        .await
        .unwrap_or_default();
    out.lines()
        .find_map(|l| l.strip_prefix("ref: refs/heads/")?.split('\t').next())
        .map_or_else(|| "master".to_owned(), str::to_owned)
}

/// A commit on `parent` with `file` holding the page's text.
async fn make(
    wiki: &Repo,
    parent: &str,
    file: &str,
    save: &Save,
) -> Result<String, crate::git::GitError> {
    let mut text = save.text.clone();
    if !text.ends_with('\n') {
        text.push('\n');
    }
    let markup = match file.rsplit_once('.') {
        Some((_, "md" | "markdown")) | None => "markdown",
        Some((_, ext)) => ext,
    };
    let verb = if save.file.is_some() {
        "Updated"
    } else {
        "Created"
    };
    let change = commit::Change {
        additions: &[(file.to_owned(), text)],
        deletions: &[],
        message: &format!("{verb} {} ({markup})", save.title),
        login: save.login.as_deref(),
    };
    commit::make(wiki, parent, &change).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_are_files_in_wiki_markups() {
        assert_eq!(page_file("Home.md"), Some(("Home", "md")));
        assert_eq!(
            page_file("Doc FAQ.mediawiki"),
            Some(("Doc FAQ", "mediawiki"))
        );
        assert_eq!(page_file("logo.png"), None);
        assert_eq!(page_file(".gitignore"), None);
        assert_eq!(
            slug("Doc continuous integration"),
            "Doc-continuous-integration"
        );
        assert!(same_page("Doc FAQ", "doc-faq") && !same_page("Doc FAQ", "Doc FAQs"));
    }

    /// A wiki git can't read is a failure, not a wiki that isn't there.
    #[tokio::test]
    async fn a_wiki_git_cant_read_isnt_taken_for_none() {
        let dir = tempfile::tempdir().unwrap();
        let cache_root = dir.path().join("a-file");
        std::fs::write(&cache_root, "").unwrap();
        let git = GitContext {
            cache_root,
            credentials: crate::git::credentials::Credentials::Ambient,
            cwd: dir.path().to_owned(),
        };
        let err = page(&git, &RepoId::new("o", "r"), None).await.unwrap_err();
        assert!(matches!(err, ApiError::Git(_)), "{err:?}");
    }

    /// Only git's own word that GitHub has no such repository is a wiki
    /// that isn't there; any other failure stays a failure.
    #[test]
    fn only_a_missing_repository_is_no_wiki() {
        let failed = |stderr: &str| crate::git::GitError::Failed {
            args: "fetch".into(),
            stderr: stderr.into(),
        };
        // As git 2.5x says it for github.com/o/r.wiki.git.
        let missing = "remote: Repository not found.\nfatal: repository 'https://github.com/o/r.wiki.git/' not found\n";
        assert!(absent(&failed(missing)));
        // As a clone reports it, its lines joined.
        let missing = "Cloning into bare repository 'x'...; remote: Repository not found.; fatal: repository 'https://github.com/o/r.wiki.git/' not found";
        assert!(absent(&failed(missing)));
        let auth =
            "fatal: could not read Username for 'https://github.com': terminal prompts disabled";
        assert!(!absent(&failed(auth)));
        assert!(!absent(&failed(
            "fatal: unable to access 'https://github.com/o/r.wiki.git/': Could not resolve host: github.com"
        )));
        assert!(!absent(&crate::git::GitError::Spawn(
            std::io::ErrorKind::NotFound.into()
        )));
    }

    /// Reads a real wiki through git (network; run by hand with
    /// `GHTUI_WIKI_CACHE=<dir> cargo test -- --ignored real_wiki`).
    #[tokio::test]
    #[ignore = "clones a wiki from GitHub"]
    async fn real_wiki() {
        let cache_root = std::env::var_os("GHTUI_WIKI_CACHE")
            .expect("GHTUI_WIKI_CACHE")
            .into();
        let git = GitContext {
            cache_root,
            credentials: crate::git::credentials::Credentials::Ambient,
            cwd: std::env::current_dir().unwrap(),
        };
        let repo = RepoId::new("rust-lang", "rust");
        let home = page(&git, &repo, None).await.unwrap();
        assert_eq!(home.title.as_deref(), Some("Home"));
        assert!(home.pages.len() > 50, "{:?}", home.pages);
        let faq = page(&git, &repo, Some("Doc-language-faq")).await.unwrap();
        assert_eq!(faq.title.as_deref(), Some("Doc-language-faq"));
        let spaced = page(&git, &repo, Some("Doc-continuous-integration"))
            .await
            .unwrap();
        assert!(spaced.text.is_some());
        let list = page(&git, &repo, Some("_pages")).await.unwrap();
        assert!(list.text.is_none() && !list.pages.is_empty());
        page(&git, &repo, Some("No-such-page")).await.unwrap_err();
    }

    async fn run(dir: &std::path::Path, args: &[&str]) -> String {
        crate::git::run(Some(dir), args).await.unwrap()
    }

    /// A commit of `file` holding `text` in the working clone `dir`,
    /// pushed to "GitHub".
    async fn push_page(dir: &std::path::Path, file: &str, text: &str) {
        std::fs::write(dir.join(file), text).unwrap();
        run(dir, &["add", file]).await;
        run(dir, &["commit", "-q", "-m", file]).await;
        run(dir, &["push", "-q", "origin", "master"]).await;
    }

    /// "GitHub"'s wiki (a bare repository with a Home page), someone
    /// else's clone of it, and ghtui's clone in its cache.
    async fn wiki_setup() -> (tempfile::TempDir, Repo, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let github = dir.path().join("github.wiki.git");
        let theirs = dir.path().join("theirs");
        std::fs::create_dir_all(&theirs).unwrap();
        run(
            dir.path(),
            &["init", "-q", "--bare", "-b", "master", "github.wiki.git"],
        )
        .await;
        // As GitHub's, it serves partial clones: blobs come when read.
        run(&github, &["config", "uploadpack.allowFilter", "true"]).await;
        run(&theirs, &["init", "-q", "-b", "master"]).await;
        for (key, value) in [("user.name", "Them"), ("user.email", "them@example.com")] {
            run(&theirs, &["config", key, value]).await;
        }
        let url = format!("file://{}", github.display());
        run(&theirs, &["remote", "add", "origin", &url]).await;
        push_page(&theirs, "Home.md", "Welcome.\n").await;
        let quiet = |_: String| {};
        let cache = dir.path().join("cache");
        let ours = Repo::open_cache(
            &cache,
            "o",
            "r.wiki",
            &url,
            crate::git::credentials::Credentials::Ambient,
            &quiet,
        )
        .await
        .unwrap();
        assert!(ours.partial);
        ours.fetch_head(HEAD, &quiet).await.unwrap();
        (dir, ours, theirs)
    }

    fn edit(w: &WikiPage, text: &str) -> Save {
        Save {
            repo: RepoId::new("o", "r"),
            title: w.title.clone().unwrap(),
            file: w.file.clone(),
            base: w.commit.clone(),
            text: text.into(),
            login: Some("octocat".into()),
        }
    }

    /// A page edited from the cache's clone is committed on GitHub's
    /// latest and pushed, and reads back as saved; another page changed
    /// meanwhile doesn't stop it.
    #[tokio::test]
    async fn a_page_is_saved_on_top_of_githubs_latest() {
        let (_dir, ours, theirs) = wiki_setup().await;
        let home = read(&ours, None).await.ok().unwrap();
        assert_eq!(home.file.as_deref(), Some("Home.md"));
        push_page(&theirs, "FAQ.md", "Questions.\n").await;
        let saved = save_in(&ours, &edit(&home, "Welcome, all.")).await;
        assert_eq!(saved.as_deref(), Ok("Home"));
        let again = read(&ours, None).await.ok().unwrap();
        assert_eq!(again.text.as_deref(), Some("Welcome, all.\n"));
        assert_eq!(again.pages, ["FAQ", "Home"]);
        run(&theirs, &["pull", "-q", "origin", "master"]).await;
        let log = run(&theirs, &["log", "-1", "--format=%s|%an"]).await;
        assert!(log.starts_with("Updated Home (markdown)|"), "{log}");
        assert_eq!(
            std::fs::read_to_string(theirs.join("Home.md")).unwrap(),
            "Welcome, all.\n"
        );
    }

    /// A page changed on GitHub since it was opened isn't overwritten:
    /// saving says so, and GitHub keeps its text.
    #[tokio::test]
    async fn a_page_changed_meanwhile_is_never_overwritten() {
        let (_dir, ours, theirs) = wiki_setup().await;
        let home = read(&ours, None).await.ok().unwrap();
        push_page(&theirs, "Home.md", "Theirs.\n").await;
        let refused = save_in(&ours, &edit(&home, "Mine.")).await.unwrap_err();
        assert!(refused.contains("Home changed on GitHub"), "{refused}");
        let now = read(&ours, None).await.ok().unwrap();
        assert_eq!(now.text.as_deref(), Some("Theirs.\n"));
    }

    /// A new page is a new file named by its title; a title taken is
    /// refused.
    #[tokio::test]
    async fn a_new_page_is_made_once() {
        let (_dir, ours, _theirs) = wiki_setup().await;
        let new = Save {
            repo: RepoId::new("o", "r"),
            title: "Getting started".into(),
            file: None,
            base: None,
            text: "Steps.".into(),
            login: None,
        };
        assert_eq!(save_in(&ours, &new).await.as_deref(), Ok("Getting-started"));
        let page = read(&ours, Some("Getting-started")).await.ok().unwrap();
        assert_eq!(page.file.as_deref(), Some("Getting-started.md"));
        assert_eq!(page.text.as_deref(), Some("Steps.\n"));
        let again = save_in(&ours, &new).await.unwrap_err();
        assert!(again.contains("has a page named"), "{again}");
    }

    /// The history lists the wiki's changes newest first (a page's, only
    /// its own), and a change, or two revisions apart, show what changed
    /// page by page; a revision that isn't a SHA names nothing.
    #[tokio::test]
    async fn a_wikis_history_and_changes_come_from_git() {
        let (_dir, ours, theirs) = wiki_setup().await;
        push_page(&theirs, "FAQ.md", "Ask.\n").await;
        push_page(&theirs, "Home.md", "Welcome, all.\n").await;
        ours.fetch_head(HEAD, &|_| {}).await.unwrap();
        let repo = RepoId::new("o", "r");
        let all = history_in(&ours, &repo, None).await.unwrap();
        let titles: Vec<&str> = all.revisions.iter().map(|r| r.message.as_str()).collect();
        assert_eq!(titles, ["Home.md", "FAQ.md", "Home.md"]);
        assert_eq!(all.revisions[1].pages, ["FAQ"]);
        assert_eq!(all.revisions[0].author, "Them");
        let home = history_in(&ours, &repo, Some("Home")).await.unwrap();
        assert_eq!(home.revisions.len(), 2);
        let (last, first) = (&home.revisions[0].oid, &home.revisions[1].oid);
        let change = diff_in(&ours, &repo, None, last).await.unwrap();
        assert_eq!(change.files[0].title, "Home");
        assert_eq!(
            change.files[0].lines,
            ["@@ -1 +1 @@", "-Welcome.", "+Welcome, all."]
        );
        // The first revision, which has no parent.
        let made = diff_in(&ours, &repo, None, first).await.unwrap();
        assert_eq!(
            made.files[0].lines.last().map(String::as_str),
            Some("+Welcome.")
        );
        let spec = format!("{first}...{last}");
        let both = diff_in(&ours, &repo, None, &spec).await.unwrap();
        let pages: Vec<&str> = both.files.iter().map(|f| f.title.as_str()).collect();
        assert_eq!(pages, ["FAQ", "Home"]);
        assert_eq!(both.from.as_deref(), Some(first.as_str()));
        let one = diff_in(&ours, &repo, Some("FAQ"), &spec).await.unwrap();
        assert_eq!(one.files.len(), 1);
        diff_in(&ours, &repo, None, "--output=x").await.unwrap_err();
        diff_in(&ours, &repo, None, "abcdef0").await.unwrap_err();
    }
}
