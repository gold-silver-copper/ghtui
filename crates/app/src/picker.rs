//! One fuzzy picker for every list under a text field: the command
//! palette, go to file (in a repository and in a diff), branches and tags,
//! and the diff's commits.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ghtui_api::browse::SearchKind;
use ghtui_api::model::RepoId;
use ghtui_theme::Bg;
use ghtui_ui::bars::Notice;
use ghtui_ui::diff_doc::Pos;
use ghtui_ui::overlays::PaletteItem;
use ghtui_ui::text::short_sha;
use ratatui_textarea::TextArea;

use crate::browse::{Data, DataKey, Need};
use crate::keymap::Action;
use crate::nav::new_input;
use crate::review::apply_commit_choice;
use crate::route::{self, Route, Target};
use crate::state::{Cmd, Overlay, Remote, Screen, State, apply};

pub enum Kind {
    /// The command palette.
    Commands,
    /// The repository's files at `rev`.
    Files { repo: RepoId, rev: String },
    /// Branches and tags; you land on `path` (a file if `file`) there.
    Branches {
        repo: RepoId,
        rev: String,
        path: String,
        file: bool,
    },
    /// The diff's files.
    DiffFiles,
    /// The diff's commits; `mark` is where a range starts.
    Commits { mark: Option<usize> },
    /// Recent notices and errors, newest first, as they were on opening.
    Messages(Vec<(u64, Notice)>),
    /// A Home section's title, for the config file at the path: the field is all of it.
    SectionTitle(Titling, std::path::PathBuf),
}

/// What a section's title is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Titling {
    /// A list saved to Home.
    New { kind: SearchKind, query: String },
    /// Home's section at `at`.
    Rename { at: usize },
}

pub struct Picker {
    pub kind: Kind,
    pub input: TextArea<'static>,
    pub selected: usize,
}

impl Picker {
    pub fn title(&self) -> &'static str {
        match self.kind {
            Kind::Commands => ":",
            Kind::Files { .. } | Kind::DiffFiles => "Go to file",
            Kind::Branches { .. } => "Switch branches/tags",
            Kind::Commits { .. } => "Commits",
            Kind::Messages(_) => "Messages",
            Kind::SectionTitle(Titling::New { .. }, _) => "Save to Home",
            Kind::SectionTitle(Titling::Rename { .. }, _) => "Rename the section",
        }
    }
}

/// Which changes the diff shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickItem {
    All,
    SinceReview,
    Commit(usize),
}

/// What choosing a row does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    Action(Action),
    Go(Target),
    Search(String),
    DiffFile(usize),
    Commits(PickItem),
    /// Copy this text.
    Copy(String),
    /// Go to open tab N (from 0).
    Tab(usize),
    /// Title a Home section this, in the config file at the path.
    Title(Titling, std::path::PathBuf, String),
}

type Rows = Vec<(PaletteItem, Option<Choice>)>;

fn item(label: impl Into<String>, hint: impl Into<String>) -> PaletteItem {
    PaletteItem {
        label: label.into(),
        hint: hint.into(),
    }
}

/// Notices that match `q`; choosing one copies it.
fn message_rows(kept: &[(u64, Notice)], q: &str, now: u64) -> Rows {
    let rows = kept.iter().filter_map(|(at, notice)| {
        let (text, kind) = match notice {
            Notice::Info(text) => (text, ""),
            Notice::Error(text) => (text, "error · "),
        };
        fuzzy_score(q, text)?;
        let hint = format!("{kind}{}", ghtui_ui::time::ago(*at, now));
        Some((item(text, hint), Some(Choice::Copy(text.to_owned()))))
    });
    rows.collect()
}

impl State {
    #[must_use]
    pub fn open_picker(&mut self, kind: Kind) -> Vec<Cmd> {
        let placeholder = match kind {
            Kind::Commands => "A command, owner/repo, owner/repo#123, @user, a URL, or a search",
            Kind::Files { .. } | Kind::DiffFiles => "Type a file name",
            Kind::Branches { .. } => "Find a branch or tag",
            Kind::Commits { .. } => "space marks a range start · ↵ views · esc cancels",
            Kind::Messages(_) => "↵ copies a message",
            Kind::SectionTitle(..) => "The section's title",
        };
        let need = match &kind {
            Kind::Files { repo, rev } => Some(DataKey::Files(repo.clone(), rev.clone())),
            Kind::Branches { repo, .. } => Some(DataKey::Refs(repo.clone())),
            _ => None,
        };
        let mut input = new_input(&self.theme, Bg::ContainerHigh);
        input.set_placeholder_text(placeholder);
        input.set_placeholder_style(self.theme.meta(Bg::ContainerHigh));
        self.overlay = Some(Overlay::Picker(Box::new(Picker {
            kind,
            input,
            selected: 0,
        })));
        need.map_or_else(Vec::new, |key| self.ensure(Need::Data(key), false))
    }

    /// Go to file, or switch branches, for the code on screen.
    #[must_use]
    pub fn open_finder(&mut self, branches: bool) -> Vec<Cmd> {
        let (repo, rev, path, file) = match self.route() {
            Some(Route::Tree { repo, rev, path }) => {
                (repo.clone(), rev.clone(), path.clone(), false)
            }
            Some(Route::Blob {
                repo, rev, path, ..
            }) => (repo.clone(), rev.clone(), path.clone(), true),
            Some(route) if let Some(repo) = route.repo() => {
                let default = self.overview(repo).and_then(|o| o.default_branch.clone());
                let rev = default.unwrap_or_else(|| "HEAD".into());
                (repo.clone(), rev, String::new(), false)
            }
            _ => {
                self.info("Open a repository first");
                return Vec::new();
            }
        };
        self.open_picker(if branches {
            Kind::Branches {
                repo,
                rev,
                path,
                file,
            }
        } else {
            Kind::Files { repo, rev }
        })
    }

    /// The picker's rows: what to show, and what choosing each does.
    pub fn picker_rows(&self, p: &Picker) -> Rows {
        let q = p.input.lines().join("");
        let q = q.trim();
        match &p.kind {
            Kind::Commands => self.commands(q),
            Kind::Files { repo, rev } => self.file_rows(q, repo, rev),
            Kind::Branches {
                repo,
                rev,
                path,
                file,
            } => self.branch_rows(q, repo, rev, path, *file),
            Kind::DiffFiles => self.diff_file_rows(q),
            Kind::Commits { mark } => self.commit_rows(*mark),
            Kind::Messages(kept) => message_rows(kept, q, (self.clock)()),
            Kind::SectionTitle(titling, path) => {
                if q.is_empty() {
                    return vec![(item("Type a title", ""), None)];
                }
                let label = match titling {
                    Titling::New { query, .. } => format!("Add “{q}” to Home: {query}"),
                    Titling::Rename { .. } => format!("Rename it “{q}”"),
                };
                let choice = Choice::Title(titling.clone(), path.clone(), q.to_owned());
                vec![(item(label, "↵"), Some(choice))]
            }
        }
    }

    /// Palette entries: somewhere to go, what you can do here (the menu's
    /// order first), and searching GitHub.
    pub fn commands(&self, input: &str) -> Rows {
        let mut out = Vec::new();
        if let Some(target) = route::parse_input(input, self.context_repo()) {
            let label = match &target {
                Target::Page(route) => format!("Go to {}", route.title()),
                Target::Files(of) => format!("Files changed in {of}"),
                Target::External(url) => format!("Open {url}"),
            };
            out.push((item(label, ""), Some(Choice::Go(target))));
        }
        // Open tabs, by title.
        if self.tab_count() > 1 {
            let mut tabs: Vec<(usize, usize, String)> = self
                .tab_titles()
                .into_iter()
                .enumerate()
                .filter_map(|(i, title)| Some((fuzzy_score(input, &title)?, i, title)))
                .collect();
            tabs.sort_by_key(|(score, ..)| *score);
            for (_, i, title) in tabs {
                let key = Action::OPEN_TABS
                    .get(i)
                    .and_then(|a| self.key_here(*a))
                    .unwrap_or_default();
                out.push((
                    item(format!("Tab {}: {title}", i + 1), key),
                    Some(Choice::Tab(i)),
                ));
            }
        }
        let scope = self.scope();
        let doables = self.doables();
        let mut order: Vec<Action> = doables.iter().map(|d| d.action).collect();
        order.extend(Action::ALL.iter().filter(|a| a.scope().overlaps(scope)));
        let mut actions: Vec<(usize, Action)> = Vec::new();
        for a in order {
            if a == Action::CommandPalette || actions.iter().any(|(_, b)| *b == a) {
                continue;
            }
            let by_name = fuzzy_score(input, &a.name().replace('_', " "));
            let by_description = fuzzy_score(input, a.description());
            if let Some(score) = by_name.into_iter().chain(by_description).min() {
                actions.push((score, a));
            }
        }
        // Stable: equal scores keep the menu's order.
        actions.sort_by_key(|(score, _)| *score);
        // Good action matches first, then searching; scattered matches after.
        let (close, far): (Vec<_>, Vec<_>) = actions.into_iter().partition(|(s, _)| *s < 1000);
        // What doesn't apply here says why, as in the menu.
        let action = |(_, a): (usize, Action)| {
            let why = match doables.iter().find(|d| d.action == a) {
                Some(d) => d.unavailable.clone(),
                // Not offered here at all: why, for what has a reason.
                None if crate::home::ACTIONS.contains(&a) => crate::home::unavailable(self, a),
                None => None,
            };
            let label = match why {
                Some(why) => format!("{} ({why})", a.description()),
                None => a.description().to_owned(),
            };
            let row = item(label, self.key_here(a).unwrap_or_default());
            (row, Some(Choice::Action(a)))
        };
        out.extend(close.into_iter().map(action));
        if !input.is_empty() {
            out.push((
                item(format!("Search GitHub for “{input}”"), ""),
                Some(Choice::Search(input.to_owned())),
            ));
        }
        out.extend(far.into_iter().map(action));
        out
    }

    /// What `key` holds, or the row saying it's loading or why it failed
    /// (opening the picker again tries again).
    fn listed(&self, key: &DataKey, what: &str) -> Result<&Data, Rows> {
        #[expect(clippy::disallowed_methods, reason = "said as the picker's row")]
        let listed = Remote::fetched(self.data.get(key), "").text(what);
        listed.map_err(|why| vec![(item(why, ""), None)])
    }

    fn file_rows(&self, q: &str, repo: &RepoId, rev: &str) -> Rows {
        let (files, truncated) =
            match self.listed(&DataKey::Files(repo.clone(), rev.into()), "files") {
                Ok(Data::Files(files, truncated)) => (files, truncated),
                other => return other.err().unwrap_or_default(),
            };
        let mut hits: Vec<(usize, &String)> = files
            .iter()
            .filter_map(|p| path_score(q, p).map(|s| (s, p)))
            .collect();
        hits.sort_by_key(|(s, p)| (*s, p.len()));
        let mut rows: Rows = hits
            .into_iter()
            .take(200)
            .map(|(_, path)| {
                let target = Target::Page(Route::blob(repo.clone(), rev.to_owned(), path.clone()));
                (item(path, ""), Some(Choice::Go(target)))
            })
            .collect();
        if *truncated && rows.len() < 200 {
            rows.push((item("GitHub listed only part of this repository", ""), None));
        }
        rows
    }

    fn branch_rows(&self, q: &str, repo: &RepoId, rev: &str, path: &str, file: bool) -> Rows {
        let refs = match self.listed(&DataKey::Refs(repo.clone()), "branches") {
            Ok(Data::Refs(refs)) => refs,
            other => return other.err().unwrap_or_default(),
        };
        let default = self
            .overview(repo)
            .and_then(|o| o.default_branch.as_deref());
        let mut rows = Vec::new();
        for (names, what) in [(&refs.branches, "branch"), (&refs.tags, "tag")] {
            for name in names {
                let Some(score) = fuzzy_score(q, name) else {
                    continue;
                };
                let is_default = default == Some(name.as_str());
                let hint = if is_default {
                    "default"
                } else if name == rev {
                    "current"
                } else {
                    what
                };
                let (repo, rev, path) = (repo.clone(), name.clone(), path.to_owned());
                let target = if file {
                    Route::blob(repo, rev, path)
                } else if path.is_empty() && is_default {
                    Route::Repo(repo)
                } else {
                    Route::Tree { repo, rev, path }
                };
                rows.push((score, item(name, hint), Choice::Go(Target::Page(target))));
            }
        }
        rows.sort_by_key(|(s, ..)| *s);
        let mut rows: Rows = rows.into_iter().map(|(_, i, c)| (i, Some(c))).collect();
        // What wasn't fetched can't be picked: say so.
        for (list, what) in [(&refs.branches, "branches"), (&refs.tags, "tags")] {
            let rest = list.left_out();
            if rest > 0 {
                rows.push((item(format!("{rest} more {what} on GitHub"), ""), None));
            }
        }
        rows
    }

    fn diff_file_rows(&self, q: &str) -> Rows {
        let Some(diff) = self.diff() else {
            return Vec::new();
        };
        let mut hits: Vec<(usize, usize, _)> = diff
            .doc
            .files()
            .iter()
            .enumerate()
            .filter_map(|(i, f)| fuzzy_score(q, f.meta.path()).map(|s| (s, i, f)))
            .collect();
        hits.sort_by_key(|&(score, i, _)| (score, i));
        hits.into_iter()
            .map(|(_, i, f)| {
                let (adds, dels) = diff.doc.file_counts(f);
                let hint = match f.diff {
                    Some(_) => format!("+{adds} −{dels}"),
                    None => String::new(),
                };
                (item(f.meta.path(), hint), Some(Choice::DiffFile(i)))
            })
            .collect()
    }

    fn commit_rows(&self, mark: Option<usize>) -> Rows {
        let Some(diff) = self.diff() else {
            return Vec::new();
        };
        let current = |on: bool| if on { "current" } else { "" };
        let since = diff.doc.since_active();
        let mut rows = vec![
            (
                item("All changes", current(diff.range().is_none() && !since)),
                Some(Choice::Commits(PickItem::All)),
            ),
            (
                item(
                    "Changes since your last review",
                    current(diff.range().is_none() && since),
                ),
                Some(Choice::Commits(PickItem::SinceReview)),
            ),
        ];
        for (i, c) in diff.inputs().commits.iter().enumerate() {
            let hint = if mark == Some(i) { "range start" } else { "" };
            rows.push((
                item(format!("{} {}", short_sha(&c.oid), c.subject), hint),
                Some(Choice::Commits(PickItem::Commit(i))),
            ));
        }
        rows
    }
}

/// Case-insensitive subsequence match; lower is better. Substring matches
/// rank before scattered ones, earlier before later.
pub fn fuzzy_score(needle: &str, haystack: &str) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    let needle = needle.to_lowercase();
    let hay = haystack.to_lowercase();
    if let Some(pos) = hay.find(&needle) {
        return Some(pos);
    }
    let mut chars = hay.char_indices();
    let mut last = 0;
    for n in needle.chars() {
        let (i, _) = chars.find(|(_, h)| *h == n)?;
        last = i;
    }
    Some(1000 + last)
}

/// Fuzzy file matching that prefers the file name.
fn path_score(q: &str, path: &str) -> Option<usize> {
    if q.is_empty() {
        return Some(path.matches('/').count());
    }
    let name = path.rsplit('/').next().unwrap_or(path);
    fuzzy_score(q, name).or_else(|| fuzzy_score(q, path).map(|s| s + 2000))
}

/// Up and down a list: arrows, tab, ctrl-n and ctrl-p. False for other keys.
pub fn move_in_list(key: KeyEvent, selected: &mut usize, count: usize) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Down | KeyCode::Tab => *selected += 1,
        KeyCode::Char('n') if ctrl => *selected += 1,
        KeyCode::Up | KeyCode::BackTab => *selected = selected.saturating_sub(1),
        KeyCode::Char('p') if ctrl => *selected = selected.saturating_sub(1),
        _ => return false,
    }
    *selected = (*selected).min(count.saturating_sub(1));
    true
}

#[must_use]
pub fn on_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    let Some(Overlay::Picker(p)) = &state.overlay else {
        return Vec::new();
    };
    let mut rows = state.picker_rows(p);
    let Some(Overlay::Picker(p)) = &mut state.overlay else {
        return Vec::new();
    };
    if move_in_list(key, &mut p.selected, rows.len()) {
        return Vec::new();
    }
    match (key.code, &mut p.kind) {
        (KeyCode::Esc, _) => state.overlay = None,
        (KeyCode::Enter, kind) => {
            let mark = match kind {
                Kind::Commits { mark } => *mark,
                _ => None,
            };
            let selected = p.selected;
            if let Some(choice) = rows.get_mut(selected).and_then(|(_, c)| c.take()) {
                state.overlay = None;
                return choose(state, choice, mark);
            }
        }
        // Commits aren't filtered: space marks where a range starts.
        (KeyCode::Char(' '), Kind::Commits { mark }) => {
            if let Some((_, Some(Choice::Commits(PickItem::Commit(i))))) = rows.get(p.selected) {
                *mark = (*mark != Some(*i)).then_some(*i);
            }
        }
        (KeyCode::Char('j'), Kind::Commits { .. }) => {
            p.selected = (p.selected + 1).min(rows.len().saturating_sub(1));
        }
        (KeyCode::Char('k'), Kind::Commits { .. }) => p.selected = p.selected.saturating_sub(1),
        (_, Kind::Commits { .. }) => {}
        _ => {
            crate::nav::type_into(&mut p.input, key);
            p.selected = 0;
        }
    }
    Vec::new()
}

#[must_use]
fn choose(state: &mut State, choice: Choice, mark: Option<usize>) -> Vec<Cmd> {
    match choice {
        Choice::Action(action) => apply(state, action),
        Choice::Go(target) => state.go(target),
        Choice::Search(query) => state.push(search_route(&query)),
        Choice::DiffFile(file) => {
            if let Screen::Diff(screen) = state.screen_mut() {
                screen.cursor = Pos { file, row: 0 };
                screen.top = screen.cursor;
            }
            Vec::new()
        }
        Choice::Commits(pick) => apply_commit_choice(state, pick, mark),
        Choice::Copy(text) => {
            state.info("Copied the message");
            vec![Cmd::Copy(text)]
        }
        Choice::Tab(i) => state.switch_to_tab(i),
        Choice::Title(Titling::New { kind, query }, path, title) => {
            let what = format!("Added “{title}” to Home");
            state.edit_home(path, crate::home::Edit::Add { title, kind, query }, what)
        }
        Choice::Title(Titling::Rename { at }, path, title) => {
            let what = format!("Renamed it “{title}”");
            state.edit_home(path, crate::home::Edit::Rename { at, title }, what)
        }
    }
}

/// A GitHub search: issues when the query looks like it's about them,
/// otherwise repositories (GitHub's default).
fn search_route(query: &str) -> Route {
    const ISSUE_QUALIFIERS: [&str; 7] = [
        "repo:",
        "is:",
        "author:",
        "assignee:",
        "label:",
        "involves:",
        "mentions:",
    ];
    let issues = query
        .split_whitespace()
        .any(|w| ISSUE_QUALIFIERS.iter().any(|p| w.starts_with(p)));
    Route::Search {
        kind: if issues {
            SearchKind::Issues
        } else {
            SearchKind::Repos
        },
        query: query.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_matches_prefer_names() {
        assert!(path_score("main", "src/main.rs") < path_score("main", "maintainers/x.rs"));
        assert_eq!(path_score("zzz", "src/main.rs"), None);
    }
}
