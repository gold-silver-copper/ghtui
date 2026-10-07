//! Wikis: GitHub keeps each in a git repository beside the repository's
//! (`<repo>.wiki.git`), one file per page, which no API serves, so ghtui
//! reads them through git, cloned into its cache like diffs.

use ghtui_api::ApiError;
use ghtui_api::browse::WikiPage;
use ghtui_api::model::RepoId;
use ghtui_git::repo::Repo;

use crate::diff_job::GitContext;

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
    MARKUPS
        .contains(&ext.to_ascii_lowercase().as_str())
        .then_some((title, ext))
}

/// How a title appears in a page's URL: spaces as dashes.
pub fn slug(title: &str) -> String {
    title.replace(' ', "-")
}

/// A wiki's page (its Home without one; its list of pages for `_pages`).
pub async fn page(
    git: &GitContext,
    repo: &RepoId,
    page: Option<&str>,
) -> Result<WikiPage, ApiError> {
    let unread = |err: &dyn std::fmt::Display| ApiError::Git(format!("{repo}'s wiki: {err}"));
    let name = format!("{}.wiki", repo.name);
    let url = format!("https://github.com/{}/{name}.git", repo.owner);
    let quiet = |_: String| {};
    let wiki = Repo::open_cache(
        &git.cache_root,
        &repo.owner,
        &name,
        &url,
        git.credentials.clone(),
        &quiet,
    )
    .await
    .map_err(|e| unread(&e))?;
    // Offline, the pages fetched before still read.
    if let Err(err) = wiki.fetch_head(HEAD).await
        && !wiki.has(HEAD).await
    {
        return Err(unread(&err));
    }
    let files = wiki.file_names(HEAD).await.map_err(|e| unread(&e))?;
    let pages: Vec<(&str, &str, &String)> = files
        .iter()
        .filter_map(|f| page_file(f).map(|(title, ext)| (title, ext, f)))
        .collect();
    // Pages are files at any depth, named by their file names.
    let title_of = |title: &str| title.rsplit('/').next().unwrap_or(title).to_owned();
    let mut titles: Vec<String> = pages
        .iter()
        .map(|(t, _, _)| title_of(t))
        .filter(|t| !t.starts_with('_'))
        .collect();
    titles.sort_by_key(|t| t.to_lowercase());
    let find = |wanted: &str| {
        pages
            .iter()
            .find(|(t, _, _)| slug(&title_of(t)).eq_ignore_ascii_case(&slug(wanted)))
    };
    let sidebar = match find("_Sidebar") {
        Some((_, _, path)) => Some(wiki.file_text(HEAD, path).await.map_err(|e| unread(&e))?),
        None => None,
    };
    let wanted = page.unwrap_or("Home");
    let found = if wanted == "_pages" {
        None
    } else {
        find(wanted)
    };
    let (title, text, markdown) = match found {
        Some((t, ext, path)) => {
            let text = wiki.file_text(HEAD, path).await.map_err(|e| unread(&e))?;
            let markdown = matches!(ext.to_ascii_lowercase().as_str(), "md" | "markdown");
            (Some(title_of(t)), Some(text), markdown)
        }
        // The list of pages, also what a wiki without a Home shows.
        None if page.is_none() || wanted == "_pages" => (None, None, false),
        None => {
            return Err(ApiError::NotFound(format!(
                "{repo}'s wiki has no page {wanted}"
            )));
        }
    };
    Ok(WikiPage {
        title,
        text,
        markdown,
        pages: titles,
        sidebar,
    })
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
    }

    /// A wiki git can't read is a failure, not a wiki that isn't there.
    #[tokio::test]
    async fn a_wiki_git_cant_read_isnt_taken_for_none() {
        let dir = tempfile::tempdir().unwrap();
        let cache_root = dir.path().join("a-file");
        std::fs::write(&cache_root, "").unwrap();
        let git = GitContext {
            cache_root,
            credentials: ghtui_git::credentials::Credentials::Ambient,
            cwd: dir.path().to_owned(),
        };
        let err = page(&git, &RepoId::new("o", "r"), None).await.unwrap_err();
        assert!(matches!(err, ApiError::Git(_)), "{err:?}");
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
            credentials: ghtui_git::credentials::Credentials::Ambient,
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
}
