//! Who owns a file: the CODEOWNERS file GitHub reads (the first of
//! `.github/`, the root and `docs/`), read with git where there's a clone
//! ([`crate::source`]) or through the API, and the problems GitHub found
//! in it, which only GitHub knows.
//!
//! A rule is a pattern and its owners; the last rule that matches a path
//! owns it, and one with no owners leaves it unowned. Patterns are
//! gitignore's, as GitHub reads them: one with a slash before its end is
//! anchored at the root, one without matches at any depth; `*` stays
//! within a folder and `**` crosses them; a pattern that names a folder
//! owns what's in it, but `docs/*` only the files right in `docs/`.

use serde::{Deserialize, Serialize};

use crate::api::client::{ApiError, GitHub, repo_vars};
use crate::api::model::RepoId;
use crate::api::raw;

/// Where GitHub looks for the file, in its order.
pub const PLACES: [&str; 3] = [".github/CODEOWNERS", "CODEOWNERS", "docs/CODEOWNERS"];

/// A repository's code owners at a revision.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Codeowners {
    /// The file GitHub reads; `None` when there's none.
    pub path: Option<String>,
    pub rules: Vec<Rule>,
    /// What GitHub found wrong in it; `None` where GitHub couldn't be
    /// asked.
    pub errors: Option<Vec<OwnersError>>,
}

/// One line of the file: a pattern and who owns what it matches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub line: u32,
    pub pattern: String,
    /// `@login`, `@org/team` or an email; none leaves it unowned.
    pub owners: Vec<String>,
}

/// A problem GitHub found in the file, at a line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnersError {
    pub line: u32,
    pub column: u32,
    /// "Unknown owner", "Invalid pattern".
    pub kind: String,
    pub message: String,
    pub suggestion: Option<String>,
}

impl Codeowners {
    /// The rules of the file found at `path` (with its text), and what
    /// GitHub says of it.
    pub fn new(found: Option<(&str, &str)>, errors: Option<Vec<OwnersError>>) -> Self {
        let Some((path, text)) = found else { return Self::default() };
        Self {
            path: Some(path.to_owned()),
            rules: parse(text),
            errors,
        }
    }

    /// The rule that owns `path`: the last that matches it.
    pub fn owner(&self, path: &str) -> Option<&Rule> {
        self.rules.iter().rev().find(|r| matches(&r.pattern, path))
    }
}

/// The file's rules, in order: blank lines and comments left out.
pub fn parse(text: &str) -> Vec<Rule> {
    (text.lines().zip(1u32..))
        .filter_map(|(line, n)| {
            // A comment runs to the end of the line, unless escaped.
            let line = match line
                .find(" #")
                .or_else(|| line.starts_with('#').then_some(0))
            {
                Some(at) => line.get(..at).unwrap_or_default(),
                None => line,
            };
            let mut words = line.split_whitespace();
            let pattern = words.next()?.replace("\\#", "#");
            Some(Rule {
                line: n,
                pattern,
                owners: words.map(str::to_owned).collect(),
            })
        })
        .collect()
}

/// Whether `pattern` matches the file at `path`.
pub fn matches(pattern: &str, path: &str) -> bool {
    let folder_only = pattern.ends_with('/');
    let trimmed = pattern.trim_end_matches('/');
    let anchored = trimmed.contains('/');
    let pattern: Vec<&str> = trimmed.trim_start_matches('/').split('/').collect();
    if pattern.iter().all(|p| p.is_empty()) {
        return false;
    }
    // `docs/*` is the files right in `docs/`, not deeper.
    let one_level = pattern.len() > 1 && pattern.last() == Some(&"*");
    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let starts = if anchored { 0..1 } else { 0..parts.len() };
    starts.into_iter().any(|start| {
        let rest = parts.get(start..).unwrap_or_default();
        let file = !folder_only && segments(&pattern, rest);
        // A folder it names, above the file.
        let folder = !one_level
            && (1..rest.len()).any(|end| segments(&pattern, rest.get(..end).unwrap_or_default()));
        file || folder
    })
}

/// Whether the pattern's segments match the path's, `**` matching any
/// number of them.
fn segments(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => {
            (0..=path.len()).any(|skip| segments(rest, path.get(skip..).unwrap_or_default()))
        }
        Some((first, rest)) => match path.split_first() {
            Some((name, more)) => glob(first, name) && segments(rest, more),
            None => false,
        },
    }
}

/// Whether one segment's pattern (`*` any run, `?` any one) matches a
/// name.
fn glob(pattern: &str, name: &str) -> bool {
    fn go(p: &[char], n: &[char]) -> bool {
        match p.split_first() {
            None => n.is_empty(),
            Some(('*', rest)) => {
                (0..=n.len()).any(|skip| go(rest, n.get(skip..).unwrap_or_default()))
            }
            Some((c, rest)) => match n.split_first() {
                Some((m, more)) => (*c == '?' || c == m) && go(rest, more),
                None => false,
            },
        }
    }
    let (p, n): (Vec<char>, Vec<char>) = (pattern.chars().collect(), name.chars().collect());
    go(&p, &n)
}

#[derive(Deserialize)]
struct WireError {
    line: u32,
    column: u32,
    kind: String,
    message: String,
    suggestion: Option<String>,
}

#[derive(Deserialize)]
struct WireOwners {
    errors: Vec<WireError>,
}

fn errors(owners: Option<WireOwners>) -> Vec<OwnersError> {
    (owners.into_iter().flat_map(|o| o.errors))
        .map(|e| OwnersError {
            line: e.line,
            column: e.column,
            kind: e.kind,
            message: e.message,
            suggestion: e.suggestion.filter(|s| !s.is_empty()),
        })
        .collect()
}

/// The ref GitHub reads the file at: its default branch for `HEAD`.
fn ref_name(rev: &str) -> Option<&str> {
    (rev != "HEAD").then_some(rev)
}

impl GitHub {
    /// `repo`'s code owners at `rev`: the file and what GitHub found
    /// wrong in it.
    pub async fn codeowners(&self, repo: &RepoId, rev: &str) -> Result<Codeowners, ApiError> {
        #[derive(Deserialize)]
        struct Text {
            text: Option<String>,
        }
        #[derive(Deserialize)]
        struct Wire {
            github: Option<Text>,
            root: Option<Text>,
            docs: Option<Text>,
            codeowners: Option<WireOwners>,
        }
        let [github, root, docs] = PLACES.map(|p| format!("{rev}:{p}"));
        let vars =
            repo_vars!(repo, "github": github, "root": root, "docs": docs, "ref": ref_name(rev));
        let wire: Wire = (self.graphql_json(
            raw::CODEOWNERS,
            vars,
            format!("{repo}@{rev}"),
            "/repository",
        ))
        .await?;
        let texts = [wire.github, wire.root, wire.docs];
        let found = (PLACES.iter().zip(texts)).find_map(|(path, t)| Some((*path, t?.text?)));
        let found = found.as_ref().map(|(path, text)| (*path, text.as_str()));
        Ok(Codeowners::new(found, Some(errors(wire.codeowners))))
    }

    /// The problems GitHub found in `repo`'s CODEOWNERS file at `rev`.
    pub async fn codeowners_errors(
        &self,
        repo: &RepoId,
        rev: &str,
    ) -> Result<Vec<OwnersError>, ApiError> {
        let vars = repo_vars!(repo, "ref": ref_name(rev));
        let subject = format!("{repo}@{rev}");
        let owners: Option<WireOwners> = (self.graphql_json(
            raw::CODEOWNERS_ERRORS,
            vars,
            subject,
            "/repository/codeowners",
        ))
        .await
        .or_else(|err| match err {
            ApiError::NotFound(_) => Ok(None),
            err => Err(err),
        })?;
        Ok(errors(owners))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GitHub's own examples (docs: "About code owners").
    #[test]
    fn patterns_match_as_github_reads_them() {
        let yes = |p: &str, path: &str| assert!(matches(p, path), "{p} should match {path}");
        let no = |p: &str, path: &str| assert!(!matches(p, path), "{p} shouldn't match {path}");
        yes("*", "a/b/c.rs");
        yes("*.js", "src/app/main.js");
        no("*.js", "src/app/main.ts");
        yes("/build/logs/", "build/logs/a/b.log");
        no("/build/logs/", "src/build/logs/a.log");
        yes("docs/*", "docs/getting-started.md");
        no("docs/*", "docs/build-app/troubleshooting.md");
        yes("apps/", "web/apps/x/y.rs");
        no("apps/", "apps");
        yes("/docs/", "docs/a/b.md");
        yes("**/logs", "deep/down/logs/x.log");
        yes("/scripts/**", "scripts/a/b.sh");
        yes("src/ma?n.rs", "src/main.rs");
        yes("LICENSE", "LICENSE");
        yes("LICENSE", "vendor/LICENSE");
        no("/LICENSE", "vendor/LICENSE");
    }

    /// The last rule that matches owns a path; one with no owners leaves
    /// it unowned; comments and blank lines are left out.
    #[test]
    fn the_last_rule_owns() {
        let text = "# Owners\n* @octocat\n\n*.md @o/docs docs@example.com # prose\n/vendor/\n";
        let owners = Codeowners::new(Some(("CODEOWNERS", text)), None);
        let owner = |path: &str| owners.owner(path).map(|r| (r.line, r.owners.clone()));
        assert_eq!(owner("src/main.rs"), Some((2, vec!["@octocat".to_owned()])));
        assert_eq!(
            owner("README.md"),
            Some((4, vec!["@o/docs".to_owned(), "docs@example.com".to_owned()]))
        );
        assert_eq!(owner("vendor/x.md"), Some((5, Vec::new())));
        assert_eq!(Codeowners::new(None, None).owner("a"), None);
    }
}
