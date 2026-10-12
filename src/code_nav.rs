//! Code navigation without a language server. The symbols of the file on
//! screen come from its syntax tree, as the diff's scope names do (`gO`).
//! The definition of the word under the cursor (`gd`) is looked for in
//! the file, then in the files GitHub's code search finds the word in.

use crate::api::browse::Blob;
use crate::api::model::RepoId;
use crate::api::{ApiError, GitHub};
use crate::browse::DataKey;
use crate::diff::Language;
use crate::diff::anchor::{LinePos, Side};
use crate::diff::scope::{self, Scope};
use crate::diff::text::Text;
use crate::picker::{Choice, Kind};
use crate::route::{Route, Target};
use crate::state::{Api, Cmd, Screen, State};
use crate::ui::overlays::PaletteItem;
use crate::ui::text;

/// Files code search finds that are read for a definition.
const READ: usize = 5;

/// A definition `gd` asks GitHub to look for: `word`, from `path` of
/// `repo` at `rev`, in files of the same language (GitHub's name for it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    pub repo: RepoId,
    pub rev: String,
    pub path: String,
    pub word: String,
    pub language: &'static str,
}

impl Ask {
    /// What GitHub's code search is asked.
    pub fn query(&self) -> String {
        format!(
            "{} repo:{} language:{}",
            self.word, self.repo, self.language
        )
    }
}

/// A name's definition in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Definition {
    pub path: String,
    pub line: u32,
    /// As the symbols list it: `fn offset`.
    pub name: String,
}

/// What looking through the repository found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Found {
    pub definitions: Vec<Definition>,
    /// Files code search found the word in, and how many were read.
    pub total: u64,
    pub read: usize,
    /// The search timed out, finding only some.
    pub incomplete: bool,
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

/// The languages whose symbols ghtui reads.
const LANGUAGES: &str = "Rust, TypeScript, JavaScript, Python and Go";

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
            let (Route::Blob {
                repo, rev, path, ..
            }
            | Route::Blame {
                repo, rev, path, ..
            }) = &p.route
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

/// `gO`: the symbols of the file on screen, to go to one.
#[must_use]
pub fn symbols(state: &mut State) -> Vec<Cmd> {
    match here(state) {
        Ok(here) if here.scopes.is_empty() => state.info(no_symbols(&here.path)),
        Ok(here) => {
            let rows = symbol_rows(&here.scopes);
            return state.open_picker(Kind::Code {
                title: "Symbols in this file",
                rows,
            });
        }
        Err(why) => state.info(why),
    }
    Vec::new()
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
/// is gone to (several are offered); else GitHub's code search is asked.
#[must_use]
pub fn definition(state: &mut State) -> Vec<Cmd> {
    let here = match here(state) {
        Ok(here) => here,
        Err(why) => {
            state.info(why);
            return Vec::new();
        }
    };
    let name = here
        .word
        .filter(|w| w.starts_with(|c: char| c.is_alphanumeric() || c == '_'));
    let Some(word) = name else {
        let next = state.first_key(crate::keymap::Action::WordNext);
        state.info(format!("No name under the cursor: {next} moves to one"));
        return Vec::new();
    };
    let defines = |s: &&Scope| s.defines.as_deref() == Some(word.as_str());
    let local: Vec<Scope> = here.scopes.iter().filter(defines).cloned().collect();
    match local.as_slice() {
        [one] => {
            state.info(format!("{} · line {}", one.name, one.lines.start()));
            return go_to_line(state, *one.lines.start());
        }
        [] => {}
        many => {
            let rows = symbol_rows(many);
            return state.open_picker(Kind::Code {
                title: "Definitions",
                rows,
            });
        }
    }
    let Some(language) = Language::from_path(&here.path).and_then(github_language) else {
        state.info(format!(
            "Can't tell where “{word}” is defined: ghtui reads {LANGUAGES} only"
        ));
        return Vec::new();
    };
    let Some(rev) = here.rev else {
        state.info("The diff's head commit isn't known yet");
        return Vec::new();
    };
    state.info(format!(
        "“{word}” isn't defined in this file: searching {} for it",
        here.repo
    ));
    vec![Cmd::Api(Api::FindDefinition(Ask {
        repo: here.repo,
        rev,
        path: here.path,
        word,
        language,
    }))]
}

/// Looks for `ask`'s definition in the files GitHub's code search finds
/// its word in, read at its revision.
pub async fn find(gh: &GitHub, ask: &Ask) -> Result<Found, ApiError> {
    let hits = gh.code_search(&ask.query()).await?;
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
    let word = ask.word.clone();
    let parse = move || {
        let mut out = Vec::new();
        for (path, source) in &files {
            let scopes = scope::scopes(Language::from_path(path), &Text::new(source.as_bytes()));
            let defines = scopes
                .into_iter()
                .filter(|s| s.defines.as_ref() == Some(&word));
            out.extend(defines.map(|s| Definition {
                path: path.clone(),
                line: *s.lines.start(),
                name: s.name,
            }));
        }
        out
    };
    let definitions = tokio::task::spawn_blocking(parse).await.unwrap_or_default();
    Ok(Found {
        definitions,
        total: hits.total,
        read,
        incomplete: hits.incomplete,
    })
}

/// GitHub's answer to `ask`: one definition opens, several are offered.
/// It's dropped once the file it was asked from isn't on screen.
#[must_use]
pub fn found(state: &mut State, ask: &Ask, result: Result<Found, ApiError>) -> Vec<Cmd> {
    let still = here(state).is_ok_and(|h| h.repo == ask.repo && h.path == ask.path);
    if !still {
        return Vec::new();
    }
    let Ask {
        repo, rev, word, ..
    } = ask;
    let found = match result {
        Ok(found) => found,
        Err(err) => {
            state.error(format!("Couldn't search the code for “{word}”: {err}"));
            return Vec::new();
        }
    };
    let open = |d: &Definition| Route::Blob {
        repo: repo.clone(),
        rev: rev.clone(),
        path: d.path.clone(),
        lines: Some((d.line, d.line)),
    };
    match found.definitions.as_slice() {
        [one] => return state.push(open(one)),
        [] if found.total == 0 && found.incomplete => state.info(format!(
            "Can't tell where “{word}” is defined: GitHub's code search gave up before finding it in {repo}"
        )),
        [] if found.total == 0 => state.info(format!(
            "GitHub's code search finds no “{word}” in {repo} (it reads only the default branch)"
        )),
        [] => state.info(format!(
            "No definition of “{word}” in the {} of {} files GitHub's code search finds it in",
            found.read, found.total
        )),
        many => {
            let rows = many.iter().map(|d| {
                let item = PaletteItem {
                    label: d.name.clone(),
                    hint: format!("{}:{}", d.path, d.line),
                };
                (item, Choice::Go(Target::Page(open(d)).into()))
            });
            let rows = rows.collect();
            return state.open_picker(Kind::Code {
                title: "Definitions",
                rows,
            });
        }
    }
    Vec::new()
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
}
