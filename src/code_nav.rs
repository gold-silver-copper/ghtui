//! Code navigation without a language server. The symbols of the file on
//! screen come from its syntax tree, as the diff's scope names do (`gO`).
//! Beyond the file, a clone answers: the repository's definitions are
//! indexed at the revision on screen ([`crate::code_index`]) for `gd` and
//! the repository's symbols (`gS`), and `git grep` finds where a word is
//! used (`gR`). GitHub's code search is asked only when there's no clone
//! to read, or it can't be read in time.

use crate::route::code;
use std::time::Duration;

use crate::api::browse::Blob;
use crate::api::model::RepoId;
use crate::api::{ApiError, GitHub};
use crate::code_index::{Indexer, Snapshot, Stage, settle};
use crate::data::DataKey;
use crate::diff::Language;
use crate::diff::anchor::{LinePos, Side};
use crate::diff::scope::{self, Scope};
use crate::diff::text::Text;
use crate::git::index::grep_word;
use crate::picker::{Choice, Kind};
use crate::route::{Route, Target};
use crate::state::{Cmd, Git, Screen, State};
use crate::ui::overlays::PaletteItem;
use crate::ui::text;

/// Files code search finds that are read for a definition.
const READ: usize = 5;
/// How long `gd` waits for the index before asking GitHub instead.
const DEFINITION_WAIT: Duration = Duration::from_secs(3);
/// How long the symbols wait for the index before listing what's read.
const SYMBOLS_WAIT: Duration = Duration::from_secs(1);
/// How long references wait for the revision (fetched, if it's a branch).
const REFERENCES_WAIT: Duration = Duration::from_secs(20);
/// Symbols listed: the index holds them nearest the file first.
const SYMBOLS: usize = 50_000;
/// References listed, and how long `git grep` may look for them.
const REFERENCES: usize = 1000;
const GREP_TIME: Duration = Duration::from_secs(10);

/// What's wanted of the repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Want {
    /// Its index, built ahead of a question: nothing is answered.
    Index,
    /// Where this name is defined.
    Definition(String),
    /// Its symbols.
    Symbols,
    /// Where this word is used.
    References(String),
}

/// A question about `repo` at `rev`, asked from the file at `path`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    pub repo: RepoId,
    pub rev: String,
    pub path: String,
    pub want: Want,
}

/// A line of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub path: String,
    pub line: u32,
    /// As it's listed: `fn offset`, or the line's text.
    pub label: String,
}

/// What the repository answered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Answer {
    pub places: Vec<Place>,
    /// Said with them: what the index doesn't hold yet, or where they
    /// came from instead.
    pub note: Option<String>,
    /// Said when there are none.
    pub none: String,
}

/// The file on screen: where it is, its symbols and the word under the
/// cursor.
struct Here {
    repo: RepoId,
    /// The revision it's at; a diff's head may not be known yet.
    rev: Option<String>,
    path: String,
    scopes: Vec<Scope>,
    word: Option<String>,
}

impl Here {
    fn ask(&self, want: Want) -> Option<Ask> {
        Some(Ask {
            repo: self.repo.clone(),
            rev: self.rev.clone()?,
            path: self.path.clone(),
            want,
        })
    }
}

/// GitHub's name for a language whose symbols ghtui reads.
fn github_language(lang: Language) -> Option<&'static str> {
    Some(match lang {
        Language::Rust => "Rust",
        Language::TypeScript => "TypeScript",
        Language::Tsx => "TSX",
        Language::JavaScript => "JavaScript",
        Language::Python => "Python",
        Language::Go => "Go",
        _ => return None,
    })
}

/// Languages whose names are one another's: JavaScript's, TypeScript's
/// and TSX's.
fn family(path: &str) -> Option<&'static str> {
    match Language::from_path(path)? {
        Language::TypeScript | Language::Tsx | Language::JavaScript => Some("JavaScript"),
        lang => github_language(lang),
    }
}

/// The languages whose symbols ghtui reads.
const LANGUAGES: &str = "Rust, TypeScript, JavaScript, Python and Go";

/// What the index doesn't know, said when a name isn't in it.
const UNKNOWN: &str = "it knows functions, types, traits, classes and modules by name, not imports, re-exports, macros or types of values";

/// Why a file of `path` lists no symbols.
fn no_symbols(path: &str) -> String {
    match Language::from_path(path).and_then(github_language) {
        Some(_) => "No functions, types or modules in this file".to_owned(),
        None => format!("ghtui reads the symbols of {LANGUAGES} files only"),
    }
}

/// The file on screen, or why there's none to read.
fn here(state: &State) -> Result<Here, String> {
    match state.screen() {
        Screen::Page(p) => {
            let (Route::Code(code::Blob {
                repo, rev, path, ..
            })
            | Route::Code(code::Blame {
                repo, rev, path, ..
            })) = &p.route
            else {
                return Err("Symbols are a file's: open one, or Files changed".to_owned());
            };
            let key = DataKey::Blob(repo.clone(), rev.clone(), path.clone());
            let Some(blob) = state.picked::<Blob>(&key) else {
                return Err("The file hasn't loaded yet".to_owned());
            };
            let Some(source) = &blob.text else {
                return Err("A binary file has no symbols".to_owned());
            };
            let lang = Language::from_path(path);
            let word = p
                .word_cursor()
                .and_then(|(_, code, w)| Some(code.get(w.bytes)?.to_owned()));
            Ok(Here {
                repo: repo.clone(),
                rev: Some(rev.clone()),
                path: path.clone(),
                scopes: scope::scopes(lang, &Text::new(source.as_bytes())),
                word,
            })
        }
        Screen::Diff(screen) => {
            let Some(diff) = state.diff() else {
                return Err("The diff hasn't loaded yet".to_owned());
            };
            let Some(file) = diff.doc.files().get(screen.cursor.file) else {
                return Err("No file here".to_owned());
            };
            if file.diff.is_none() {
                return Err("The file's diff isn't ready yet".to_owned());
            }
            let word = diff
                .doc
                .code_at(screen.cursor, screen.half)
                .and_then(|code| {
                    let words = text::words(code);
                    let word = text::word_at(&words, screen.word.unwrap_or(0))?;
                    Some(code.get(word.bytes.clone())?.to_owned())
                });
            // The pull request's head: what its new definitions are in.
            Ok(Here {
                repo: screen.of.repo().clone(),
                rev: diff.head().map(|oid| oid.to_string()),
                path: file.meta.path().to_owned(),
                scopes: file.text().map(|t| t.scopes.clone()).unwrap_or_default(),
                word,
            })
        }
    }
}

/// The symbols as the picker lists them: nested as they are in the code,
/// each going to its first line.
fn symbol_rows(scopes: &[Scope]) -> Vec<(PaletteItem, Choice)> {
    let mut rows = Vec::new();
    for (i, s) in scopes.iter().enumerate() {
        let (start, end) = (*s.lines.start(), *s.lines.end());
        let around = scopes.iter().take(i);
        let depth = around
            .filter(|o| o.lines.contains(&start) && o.lines.contains(&end))
            .count();
        let item = PaletteItem {
            label: format!("{}{}", "  ".repeat(depth), s.name),
            hint: format!("line {start}"),
        };
        rows.push((item, Choice::Line(start)));
    }
    rows
}

/// The word under the cursor if it's a name, or says how to get one.
fn name_here(state: &mut State, word: Option<String>) -> Option<String> {
    let name = word.filter(|w| w.starts_with(|c: char| c.is_alphanumeric() || c == '_'));
    if name.is_none() {
        let next = state.first_key(crate::keymap::Action::WordNext);
        state.info(format!("No name under the cursor: {next} moves to one"));
    }
    name
}

/// Asks the repository, once the revision on screen is known.
fn ask(state: &mut State, here: &Here, want: Want) -> Vec<Cmd> {
    match here.ask(want) {
        Some(ask) => vec![Cmd::Git(Git::Code(ask))],
        None => {
            state.info("The diff's head commit isn't known yet");
            Vec::new()
        }
    }
}

/// `gO`: the symbols of the file on screen, to go to one. The
/// repository's index starts building, for what comes next.
#[must_use]
pub fn symbols(state: &mut State) -> Vec<Cmd> {
    match here(state) {
        Ok(here) if here.scopes.is_empty() => state.info(no_symbols(&here.path)),
        Ok(here) => {
            let rows = symbol_rows(&here.scopes);
            let mut cmds = state.open_picker(Kind::Code {
                title: "Symbols in this file".to_owned(),
                rows,
            });
            cmds.extend(here.ask(Want::Index).map(|a| Cmd::Git(Git::Code(a))));
            return cmds;
        }
        Err(why) => state.info(why),
    }
    Vec::new()
}

/// `gS`: the symbols of the whole repository, at the revision on screen.
#[must_use]
pub fn repo_symbols(state: &mut State) -> Vec<Cmd> {
    match here(state) {
        Ok(here) => ask(state, &here, Want::Symbols),
        Err(why) => {
            state.info(why);
            Vec::new()
        }
    }
}

/// `gR`: where the word under the cursor is used in the repository, at
/// the revision on screen.
#[must_use]
pub fn references(state: &mut State) -> Vec<Cmd> {
    let here = match here(state) {
        Ok(here) => here,
        Err(why) => {
            state.info(why);
            return Vec::new();
        }
    };
    let Some(word) = name_here(state, here.word.clone()) else {
        return Vec::new();
    };
    state.info(format!("Looking for “{word}” in {}", here.repo));
    ask(state, &here, Want::References(word))
}

/// Goes to line `line` of the file on screen. In the diff, a line the
/// hunks don't show shows the whole file.
#[must_use]
pub fn go_to_line(state: &mut State, line: u32) -> Vec<Cmd> {
    let Some((screen, diff)) = state.diff_parts() else {
        return crate::nav::go_to_line(state, line as usize);
    };
    let file = screen.cursor.file;
    let at = LinePos {
        side: Side::Right,
        line,
    };
    let row = |doc: &crate::ui::diff_doc::Doc| {
        let pos = doc.line_row(file, at)?;
        (doc.line_at(pos, Side::Right) == Some(at)).then_some(pos)
    };
    let mut pos = row(&diff.doc);
    if pos.is_none() && diff.doc.files().get(file).is_some_and(|f| !f.full) {
        diff.doc.toggle_full(file);
        pos = row(&diff.doc);
    }
    match pos {
        Some(pos) => {
            (screen.cursor, screen.half, screen.word) = (pos, Side::Right, None);
        }
        None => state.info(format!("There's no line {line} here")),
    }
    Vec::new()
}

/// `gd`: the definition of the word under the cursor. One in this file
/// is gone to (several are offered); else the repository is asked.
#[must_use]
pub fn definition(state: &mut State) -> Vec<Cmd> {
    let here = match here(state) {
        Ok(here) => here,
        Err(why) => {
            state.info(why);
            return Vec::new();
        }
    };
    let Some(word) = name_here(state, here.word.clone()) else {
        return Vec::new();
    };
    let defines = |s: &&Scope| s.defines.as_deref() == Some(word.as_str());
    let local: Vec<Scope> = here.scopes.iter().filter(defines).cloned().collect();
    match local.as_slice() {
        [one] => {
            state.info(format!("{} · line {}", one.name, one.lines.start()));
            let cmds = go_to_line(state, *one.lines.start());
            land_on(state, &word);
            return cmds;
        }
        [] => {}
        many => {
            let rows = symbol_rows(many);
            return state.open_picker(Kind::Code {
                title: format!("Definitions of “{word}”"),
                rows,
            });
        }
    }
    if family(&here.path).is_none() {
        state.info(format!(
            "Can't tell where “{word}” is defined: ghtui reads {LANGUAGES} only"
        ));
        return Vec::new();
    }
    state.info(format!(
        "“{word}” isn't defined in this file: looking in {}",
        here.repo
    ));
    ask(state, &here, Want::Definition(word))
}

/// Puts the word cursor on `name` where the page on screen settles: on
/// a definition, the name it defines, as vim's `gd` does.
fn land_on(state: &mut State, name: &str) {
    if let Screen::Page(p) = state.screen_mut() {
        p.land = Some(name.to_owned());
    }
    state.sync_page();
}

/// The repository's answer to `ask`: one definition opens, several
/// places are offered. It's dropped once the file it was asked from
/// isn't on screen.
#[must_use]
pub fn answered(state: &mut State, ask: &Ask, answer: Result<Answer, String>) -> Vec<Cmd> {
    let still = here(state).is_ok_and(|h| h.repo == ask.repo && h.path == ask.path);
    if !still {
        return Vec::new();
    }
    let answer = match answer {
        Ok(answer) => answer,
        Err(why) => {
            state.error(why);
            return Vec::new();
        }
    };
    let open = |p: &Place| {
        if p.path == ask.path {
            return Choice::Line(p.line);
        }
        let route = Route::Code(code::Blob {
            repo: ask.repo.clone(),
            rev: ask.rev.clone(),
            path: p.path.clone(),
            lines: Some((p.line, p.line)),
        });
        Choice::Go(Target::Page(route).into())
    };
    let title = match &ask.want {
        Want::Index => return Vec::new(),
        Want::Definition(word) => format!("Definitions of “{word}”"),
        Want::Symbols => "Symbols in the repository".to_owned(),
        Want::References(word) => format!("Where “{word}” is used"),
    };
    let note = answer.note.clone();
    let cmds = match (&ask.want, answer.places.as_slice()) {
        (_, []) => {
            state.info(note.map_or(answer.none.clone(), |n| format!("{} · {n}", answer.none)));
            return Vec::new();
        }
        (Want::Definition(word), [one]) => {
            let cmds = match open(one) {
                Choice::Line(line) => go_to_line(state, line),
                Choice::Go(dest) => match dest.target {
                    Target::Page(route) => state.push(route),
                    _ => Vec::new(),
                },
                _ => Vec::new(),
            };
            land_on(state, word);
            cmds
        }
        (_, places) => {
            let rows = places.iter().map(|p| {
                let item = PaletteItem {
                    label: p.label.clone(),
                    hint: format!("{}:{}", p.path, p.line),
                };
                (item, open(p))
            });
            let rows = rows.collect();
            state.open_picker(Kind::Code { title, rows })
        }
    };
    if let Some(note) = note {
        state.info(note);
    }
    cmds
}

/// Answers `ask` from the clone, or from GitHub's code search when the
/// clone can't say. Building the index is asked for nothing.
pub async fn answer(gh: &GitHub, index: &Indexer, ask: &Ask) -> Option<Result<Answer, String>> {
    let mut rx = index.get(&ask.repo, &ask.rev, &ask.path);
    Some(match &ask.want {
        Want::Index => return None,
        Want::Definition(word) => {
            let snap = settle(&mut rx, DEFINITION_WAIT, |_| false).await;
            definitions(gh, ask, word, &snap).await
        }
        Want::Symbols => {
            let snap = settle(&mut rx, SYMBOLS_WAIT, |_| false).await;
            repo_symbol_places(ask, &snap)
        }
        Want::References(word) => {
            // A clone of every file is grepped once the commit is known; a
            // partial one, once its files are fetched.
            let known =
                |s: &Snapshot| s.commit.is_some() && s.repo.as_ref().is_some_and(|r| !r.partial);
            let snap = settle(&mut rx, REFERENCES_WAIT, known).await;
            uses(index, ask, word, &snap).await
        }
    })
}

/// `gd` from the index, else from GitHub's code search.
async fn definitions(
    gh: &GitHub,
    ask: &Ask,
    word: &str,
    snap: &Snapshot,
) -> Result<Answer, String> {
    let same = family(&ask.path);
    let defs = snap
        .index
        .definitions(word, |p| p != ask.path && family(p) == same);
    let places: Vec<Place> = defs
        .map(|(f, d)| Place {
            path: f.path.clone(),
            line: d.line,
            label: d.name.clone(),
        })
        .collect();
    let repo = &ask.repo;
    let short = snap.short_of(repo);
    if !places.is_empty() {
        return Ok(Answer {
            places,
            note: short.filter(|_| !snap.done()).or(snap.stale.clone()),
            none: String::new(),
        });
    }
    // The whole index, and it isn't there.
    if snap.stage == Stage::Ready && short.is_none() {
        return Ok(Answer {
            none: format!(
                "No definition of “{word}” in {repo} at {}: {UNKNOWN}",
                snap.at()
            ),
            ..Answer::default()
        });
    }
    let why = short.unwrap_or_default();
    let Some(language) = Language::from_path(&ask.path).and_then(github_language) else {
        return Ok(Answer {
            none: why,
            ..Answer::default()
        });
    };
    let found = find(gh, ask, word, language).await;
    let found = found
        .map_err(|err| format!("{why} · couldn't search GitHub's code for “{word}”: {err}"))?;
    Ok(searched(
        repo,
        word,
        found,
        format!("{why}: asked GitHub's code search"),
    ))
}

/// The repository's symbols, as far as they're indexed.
fn repo_symbol_places(ask: &Ask, snap: &Snapshot) -> Result<Answer, String> {
    if let Stage::Failed(why) = &snap.stage {
        return Err(format!("Couldn't index {}: {why}", ask.repo));
    }
    let all = snap.index.symbols();
    let symbols = all.take(SYMBOLS).map(|(f, d)| Place {
        path: f.path.clone(),
        line: d.line,
        label: d.name.clone(),
    });
    let symbols: Vec<Place> = symbols.collect();
    let more = (symbols.len() == SYMBOLS && snap.index.symbols().nth(SYMBOLS).is_some())
        .then(|| format!("Listing the {SYMBOLS} symbols nearest this file"));
    let note = [snap.short_of(&ask.repo), more, snap.stale.clone()];
    let note: Vec<String> = note.into_iter().flatten().collect();
    let note = (!note.is_empty()).then(|| note.join(" · "));
    let none = match &note {
        Some(_) => "No symbols indexed yet".to_owned(),
        None => format!(
            "No functions, types or modules in {}'s {LANGUAGES} files",
            ask.repo
        ),
    };
    Ok(Answer {
        places: symbols,
        note,
        none,
    })
}

/// Where `word` is used, from `git grep` at the commit. A partial clone
/// greps the files fetched for the index, not ones it would fetch.
async fn uses(index: &Indexer, ask: &Ask, word: &str, snap: &Snapshot) -> Result<Answer, String> {
    let repo_id = &ask.repo;
    let (Some(repo), Some(commit)) = (&snap.repo, &snap.commit) else {
        let why = snap.short_of(repo_id).unwrap_or_default();
        return Err(format!("Can't look for “{word}” yet · {why}"));
    };
    let failed = |err: crate::git::GitError| format!("Couldn't look for “{word}”: {err}");
    let (tree, read) = if repo.partial {
        let files: Vec<(&str, &str)> = (snap.index.files.iter())
            .map(|f| (f.path.as_str(), f.blob.as_str()))
            .collect();
        let tree = crate::git::index::tree_of(repo, &files, &index.scratch())
            .await
            .map_err(failed)?;
        let mut read = format!(
            "Read in the {} code files fetched for the index",
            files.len()
        );
        if let Some(short) = snap.short_of(repo_id) {
            read = format!("{read} · {short}");
        }
        (tree, Some(read))
    } else {
        (commit.clone(), None)
    };
    let found = grep_word(repo, &tree, word, REFERENCES, GREP_TIME)
        .await
        .map_err(failed)?;
    let more = found
        .more
        .then(|| format!("Only the first {REFERENCES} are listed"));
    let note = [read, more, snap.stale.clone()].into_iter().flatten();
    let note: Vec<String> = note.collect();
    let places = found.hits.into_iter().map(|h| Place {
        label: h.text.trim().to_owned(),
        path: h.path,
        line: h.line,
    });
    Ok(Answer {
        places: places.collect(),
        note: (!note.is_empty()).then(|| note.join(" · ")),
        none: format!("“{word}” isn't used in {repo_id} at {}", snap.at()),
    })
}

/// What GitHub's code search is asked for `word`.
fn query(repo: &RepoId, word: &str, language: &str) -> String {
    format!("{word} repo:{repo} language:{language}")
}

/// What looking through code search's files found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Found {
    places: Vec<Place>,
    /// Files code search found the word in, and how many were read.
    total: u64,
    read: usize,
    /// The search timed out, finding only some.
    incomplete: bool,
}

/// Looks for `word`'s definition in the files GitHub's code search finds
/// it in, read at `ask`'s revision.
async fn find(gh: &GitHub, ask: &Ask, word: &str, language: &str) -> Result<Found, ApiError> {
    let hits = gh.code_search(&query(&ask.repo, word, language)).await?;
    let paths: Vec<String> = (hits.paths.into_iter())
        .filter(|path| *path != ask.path)
        .take(READ)
        .collect();
    let blobs = paths.iter().map(|p| gh.blob(&ask.repo, &ask.rev, p));
    let blobs = futures::future::join_all(blobs).await;
    let read = blobs.iter().filter(|b| b.is_ok()).count();
    let mut files = Vec::new();
    let mut failed = None;
    for (path, blob) in paths.into_iter().zip(blobs) {
        match blob {
            Ok(Blob {
                text: Some(text), ..
            }) => files.push((path, text)),
            Ok(_) => {}
            Err(err) => failed = Some(err),
        }
    }
    // Every file failing is GitHub failing, not the files missing.
    if let Some(err) = failed.filter(|_| read == 0) {
        return Err(err);
    }
    let word = word.to_owned();
    let parse = move || {
        let mut out = Vec::new();
        for (path, source) in &files {
            let scopes = scope::scopes(Language::from_path(path), &Text::new(source.as_bytes()));
            let defines = scopes
                .into_iter()
                .filter(|s| s.defines.as_ref() == Some(&word));
            out.extend(defines.map(|s| Place {
                path: path.clone(),
                line: *s.lines.start(),
                label: s.name,
            }));
        }
        out
    };
    let places = tokio::task::spawn_blocking(parse).await.unwrap_or_default();
    Ok(Found {
        places,
        total: hits.total,
        read,
        incomplete: hits.incomplete,
    })
}

/// Code search's answer, said as it is.
fn searched(repo: &RepoId, word: &str, found: Found, why: String) -> Answer {
    let none = match found {
        Found {
            total: 0,
            incomplete: true,
            ..
        } => format!(
            "Can't tell where “{word}” is defined: GitHub's code search gave up before finding it in {repo}"
        ),
        Found { total: 0, .. } => format!(
            "GitHub's code search finds no “{word}” in {repo} (it reads only the default branch)"
        ),
        Found { total, read, .. } => format!(
            "No definition of “{word}” in the {read} of {total} files GitHub's code search finds it in"
        ),
    };
    Answer {
        places: found.places,
        note: Some(why),
        none,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed(lang: Language, source: &str) -> Vec<(String, String)> {
        let scopes = scope::scopes(Some(lang), &Text::new(source.as_bytes()));
        let rows = symbol_rows(&scopes).into_iter();
        rows.map(|(item, _)| (item.label, item.hint)).collect()
    }

    /// A file's symbols nest as its code does, each at its first line.
    #[test]
    fn symbols_nest_as_the_code_does() {
        let rust = "use std::io;\n\nconst MAX: u32 = 3;\n\nstruct Doc;\n\nimpl Doc {\n    fn offset(&self) -> u32 {\n        MAX\n    }\n}\n\nmod tests {\n    fn it() {}\n}\n";
        let expected = [
            ("const MAX", "line 3"),
            ("struct Doc", "line 5"),
            ("impl Doc", "line 7"),
            ("  fn offset", "line 8"),
            ("mod tests", "line 13"),
            ("  fn it", "line 14"),
        ];
        let expected: Vec<(String, String)> = expected
            .iter()
            .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
            .collect();
        assert_eq!(listed(Language::Rust, rust), expected);
        let python =
            "class Shape:\n    def area(self):\n        return 1\n\ndef main():\n    pass\n";
        let labels: Vec<String> = (listed(Language::Python, python).into_iter())
            .map(|(label, _)| label)
            .collect();
        assert_eq!(labels, ["class Shape", "  def area", "def main"]);
    }

    #[test]
    fn code_search_is_asked_by_language() {
        let repo = RepoId::new("o", "r");
        assert_eq!(
            query(&repo, "other", "Rust"),
            "other repo:o/r language:Rust"
        );
        assert_eq!(family("a.tsx"), family("b.js"));
        assert_ne!(family("a.rs"), family("b.go"));
    }
}
