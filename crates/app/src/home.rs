//! Home: your own saved searches, each a section showing the first rows
//! of a GitHub search, with a row that opens the rest. Sections come from
//! `[[home]]` tables in `config.toml` (or the defaults, today's GitHub
//! dashboard), and changing them in ghtui writes them back there,
//! keeping the file's comments and layout.
//!
//! A section is the search page's own list (the same [`DataKey`]), so
//! its count, its order and the list its last row opens can't disagree:
//! they're one fetch.

use std::path::{Path, PathBuf};

use ghtui_api::browse::SearchKind;
use serde::Deserialize;
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, value};

use crate::browse::{DataKey, Need};
use crate::keymap::Action;
use crate::picker::{self, Titling};
use crate::route::Route;
use crate::state::{Cmd, Overlay, Screen, State};

/// Rows a section shows unless it says.
pub const DEFAULT_ROWS: usize = 25;
/// The most a section shows: one page of GitHub's search, which is also
/// the first page of the list its last row opens.
pub const MAX_ROWS: usize = 30;
/// Sections on screen are fetched again when this much older.
pub const FRESH_SECS: u64 = 120;

/// A `[[home]]` table as written. Each value is taken as whatever it is,
/// so a mistake in one section is that section's to show, not a reason
/// not to start; an unknown key is still an error, as everywhere.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SectionConfig {
    pub title: Option<Loose>,
    pub pulls: Option<Loose>,
    pub issues: Option<Loose>,
    pub repos: Option<Loose>,
    pub rows: Option<Loose>,
}

/// A config value, before it's checked.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Loose {
    Text(String),
    Number(i64),
    Other(serde::de::IgnoredAny),
}

/// A Home section: its title and the search it shows the start of, or
/// what's wrong with how it's written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub title: String,
    pub search: Result<Search, String>,
}

/// What a section lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Search {
    pub kind: SearchKind,
    pub query: String,
    pub rows: usize,
}

impl Search {
    /// The list it shows the start of, shared with the search page.
    pub fn key(&self) -> DataKey {
        DataKey::Search(self.kind, self.query.clone())
    }

    /// The page with all of it.
    pub fn route(&self) -> Route {
        Route::Search {
            kind: self.kind,
            query: self.query.clone(),
        }
    }
}

/// The config key for a kind of list, and what it calls one.
fn kind_key(kind: SearchKind) -> Option<&'static str> {
    match kind {
        SearchKind::Pulls => Some("pulls"),
        SearchKind::Issues => Some("issues"),
        SearchKind::Repos => Some("repos"),
        _ => None,
    }
}

/// Review requests: also the header's "N to review".
pub const REVIEW_REQUESTS: &str = "is:open review-requested:@me archived:false sort:updated-desc";

/// Home without `[[home]]` tables: GitHub's dashboard. Repositories are
/// the ones you own, as your profile lists them; GitHub's search can't
/// say "and the ones I collaborate on", only name owners (`org:`).
pub fn defaults() -> Vec<Section> {
    let section = |title: &str, kind, query: &str, rows| Section {
        title: title.to_owned(),
        search: Ok(Search {
            kind,
            query: query.to_owned(),
            rows,
        }),
    };
    vec![
        section(
            "Review requests",
            SearchKind::Pulls,
            REVIEW_REQUESTS,
            DEFAULT_ROWS,
        ),
        section(
            "Your pull requests",
            SearchKind::Pulls,
            "is:open author:@me archived:false sort:updated-desc",
            DEFAULT_ROWS,
        ),
        section(
            "Your repositories",
            SearchKind::Repos,
            "user:@me fork:true sort:updated",
            20,
        ),
    ]
}

/// The sections `[[home]]` tables describe; without any (not even
/// `home = []`), the defaults.
pub fn sections(tables: Option<&[SectionConfig]>) -> Vec<Section> {
    tables.map_or_else(defaults, |t| t.iter().map(section).collect())
}

fn section(t: &SectionConfig) -> Section {
    let title = match &t.title {
        Some(Loose::Text(title)) if !title.trim().is_empty() => Ok(title.trim().to_owned()),
        Some(_) => Err("its title must be some text".to_owned()),
        None => Err("it needs a title".to_owned()),
    };
    let lists: Vec<(SearchKind, &Loose)> = [
        (SearchKind::Pulls, &t.pulls),
        (SearchKind::Issues, &t.issues),
        (SearchKind::Repos, &t.repos),
    ]
    .into_iter()
    .filter_map(|(kind, q)| Some((kind, q.as_ref()?)))
    .collect();
    let search = match lists.as_slice() {
        [(kind, Loose::Text(query))] if !query.trim().is_empty() => Ok((*kind, query.trim())),
        [(kind, _)] => Err(format!(
            "its {} must be a GitHub search, like \"is:open author:@me\"",
            kind_key(*kind).unwrap_or_default()
        )),
        [] => Err("it needs pulls, issues or repos: a GitHub search".to_owned()),
        _ => Err("it has more than one of pulls, issues and repos: a section lists one".to_owned()),
    };
    let rows = match &t.rows {
        None => Ok(DEFAULT_ROWS),
        Some(Loose::Number(n)) => usize::try_from(*n)
            .ok()
            .filter(|n| (1..=MAX_ROWS).contains(n))
            .ok_or(()),
        Some(_) => Err(()),
    }
    .map_err(|()| format!("rows must be from 1 to {MAX_ROWS} (one page of GitHub's search)"));
    let search = match (&title, search, rows) {
        (Err(why), ..) => Err(why.clone()),
        (_, Err(why), _) | (_, _, Err(why)) => Err(why),
        (Ok(_), Ok((kind, query)), Ok(rows)) => Ok(Search {
            kind,
            query: query.to_owned(),
            rows,
        }),
    };
    Section {
        title: title.unwrap_or_else(|_| "Untitled section".to_owned()),
        search,
    }
}

// ---- writing them back ---------------------------------------------------------------------

/// A change to Home's sections, made to `config.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    Add {
        title: String,
        kind: SearchKind,
        query: String,
    },
    Rename {
        at: usize,
        title: String,
    },
    /// Swaps the section with the one before (`up`) or after it.
    Move {
        at: usize,
        up: bool,
    },
    Remove {
        at: usize,
    },
    /// Puts back a removed section's table, as [`Edited::removed`] kept it.
    Restore {
        at: usize,
        table: String,
    },
}

/// A change made: the file's new text, and the table taken out by a
/// removal (as TOML, comments and all).
#[derive(Debug)]
pub struct Edited {
    pub text: String,
    pub removed: Option<String>,
}

/// `edit` made to a config file's `text`, if Home there is still `shown`
/// (else it changed outside ghtui, and the edit would land on the wrong
/// section). Without `[[home]]` tables, the defaults are written out
/// first. Comments and layout stay as they were.
pub fn edit_text(text: &str, shown: &[Section], edit: &Edit) -> Result<Edited, String> {
    let config = crate::config::Config::parse(text)
        .map_err(|err| format!("config.toml doesn't read: {err:#}. Fix it there first"))?;
    if sections(config.home.as_deref()) != shown {
        return Err(
            "config.toml changed since ghtui read it: Home now shows what it says. Try again"
                .into(),
        );
    }
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|err| format!("config.toml doesn't read: {err}"))?;
    match doc.get("home") {
        None => {
            let mut tables = ArrayOfTables::new();
            for s in shown {
                tables.push(table(s)?);
            }
            doc.insert("home", Item::ArrayOfTables(tables));
        }
        // No sections, as written after removing the last.
        Some(Item::Value(toml_edit::Value::Array(a))) if a.is_empty() => {
            doc.insert("home", Item::ArrayOfTables(ArrayOfTables::new()));
        }
        Some(_) => {}
    }
    let Some(Item::ArrayOfTables(tables)) = doc.get_mut("home") else {
        return Err(
            "Home's sections in config.toml aren't [[home]] tables, so ghtui won't rewrite them: change them there"
                .into(),
        );
    };
    let n = tables.len();
    let missing = || "That section isn't there any more".to_owned();
    let mut removed = None;
    match edit {
        Edit::Add { title, kind, query } => {
            let mut t = Table::new();
            t.insert("title", value(title.as_str()));
            t.insert(kind_key(*kind).ok_or_else(missing)?, value(query.as_str()));
            tables.push(t);
        }
        Edit::Rename { at, title } => {
            let t = tables.get_mut(*at).ok_or_else(missing)?;
            match t.get_mut("title").and_then(Item::as_value_mut) {
                // In place, keeping what's around it (a comment after it).
                Some(v) => {
                    let decor = v.decor().clone();
                    *v = title.as_str().into();
                    *v.decor_mut() = decor;
                }
                None => {
                    t.insert("title", value(title.as_str()));
                }
            }
        }
        Edit::Move { at, up } => {
            let other = if *up { at.checked_sub(1) } else { Some(at + 1) };
            let other = other.filter(|&o| o < n).ok_or_else(missing)?;
            if *at >= n {
                return Err(missing());
            }
            let (lo, hi) = ((*at).min(other), (*at).max(other));
            let mut first = tables.remove(lo);
            // `hi` moved down one with `lo` gone.
            let mut second = tables.remove(hi - 1);
            // Each table keeps its comments; the places stay where they were.
            let (p1, p2) = (first.position(), second.position());
            first.set_position(p2);
            second.set_position(p1);
            tables.insert(lo, second);
            tables.insert(hi, first);
        }
        Edit::Remove { at } => {
            if *at >= n {
                return Err(missing());
            }
            let t = tables.remove(*at);
            let mut alone = DocumentMut::new();
            let mut one = ArrayOfTables::new();
            one.push(t);
            alone.insert("home", Item::ArrayOfTables(one));
            removed = Some(alone.to_string());
        }
        Edit::Restore { at, table } => {
            let doc: DocumentMut = table.parse().map_err(|_| missing())?;
            let mut t = doc
                .get("home")
                .and_then(Item::as_array_of_tables)
                .and_then(|a| a.get(0))
                .cloned()
                .ok_or_else(missing)?;
            // Where it was: just before the one now in its place (a tie
            // keeps their order), or after the last.
            let at = (*at).min(n);
            t.set_position(tables.get(at).and_then(Table::position));
            tables.insert(at, t);
        }
    }
    // No tables would read as the defaults: none is `home = []`.
    if tables.is_empty() {
        doc.insert("home", value(toml_edit::Array::new()));
    }
    Ok(Edited {
        text: doc.to_string(),
        removed,
    })
}

/// A section as a `[[home]]` table.
fn table(s: &Section) -> Result<Table, String> {
    let search = s.search.as_ref().map_err(Clone::clone)?;
    let mut t = Table::new();
    t.insert("title", value(s.title.as_str()));
    let key = kind_key(search.kind).ok_or("not a kind of list Home shows")?;
    t.insert(key, value(search.query.as_str()));
    if search.rows != DEFAULT_ROWS {
        t.insert("rows", value(i64::try_from(search.rows).unwrap_or(25)));
    }
    Ok(t)
}

/// Makes `edit` to the config file at `path` (through a symlink, to
/// what it points at), written whole or not at all. What Home shows then
/// (the file read again), and the table a removal took out.
pub fn edit_file(
    path: &Path,
    shown: &[Section],
    edit: &Edit,
) -> Result<(Vec<Section>, Option<String>), String> {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(format!("Couldn't read {}: {err}", path.display())),
    };
    let edited = edit_text(&text, shown, edit)?;
    let dir = path.parent().unwrap_or(Path::new("."));
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let mut file = tempfile::NamedTempFile::new_in(dir)?;
        std::io::Write::write_all(&mut file, edited.text.as_bytes())?;
        if let Ok(meta) = std::fs::metadata(&path) {
            file.as_file().set_permissions(meta.permissions())?;
        }
        file.persist(&path).map_err(|e| e.error)?;
        Ok(())
    };
    write().map_err(|err| format!("Couldn't write {}: {err}", path.display()))?;
    let config = crate::config::Config::parse(&edited.text).map_err(|err| format!("{err:#}"))?;
    Ok((sections(config.home.as_deref()), edited.removed))
}

// ---- Home on screen ---------------------------------------------------------------------------

impl State {
    /// What Home fetches: each section's search, once.
    pub fn home_needs(&self) -> Vec<Need> {
        let mut keys: Vec<DataKey> = Vec::new();
        for s in self.home.iter().filter_map(|s| s.search.as_ref().ok()) {
            let key = s.key();
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        keys.into_iter().map(Need::Data).collect()
    }

    /// The repositories Home's sections list, for the search box.
    pub fn home_repos(&self) -> impl Iterator<Item = &ghtui_api::browse::RepoSummary> {
        let lists = self.home.iter().filter_map(|s| {
            let s = s.search.as_ref().ok()?;
            match self.picked::<ghtui_api::browse::SearchResults>(&s.key())? {
                ghtui_api::browse::SearchResults::Repos(r) => Some(r.items.iter().take(s.rows)),
                _ => None,
            }
        });
        lists.flatten()
    }

    /// Whether a section's search was asked for long enough ago that Home
    /// on screen asks again.
    pub(crate) fn aged(&self, need: &Need) -> bool {
        let Need::Data(key) = need else {
            return false;
        };
        let now = (self.clock)();
        (self.data.get(key))
            .and_then(|r| r.asked_at)
            .is_some_and(|at| now.saturating_sub(at) >= FRESH_SECS)
    }

    /// The section the selection on Home is in.
    pub fn section_here(&self) -> Option<(usize, &Section)> {
        let Screen::Page(p) = self.screen() else {
            return None;
        };
        if p.route != Route::Home {
            return None;
        }
        let page = p.page();
        let start = page.items.get(p.selected?)?.start;
        (self.home.iter().enumerate().rev())
            .find(|(i, _)| page.anchors.get(&anchor(*i)).is_some_and(|&l| l <= start))
    }
}

/// Where section `i` starts on Home's page.
pub fn anchor(i: usize) -> String {
    format!("section-{i}")
}

/// The actions on Home's sections (and saving a list as one).
pub const ACTIONS: [Action; 6] = [
    Action::SaveSection,
    Action::RenameSection,
    Action::MoveSectionUp,
    Action::MoveSectionDown,
    Action::Delete,
    Action::UndoDelete,
];

/// What one of Home's actions does here, everything it needs checked.
enum Plan<'a> {
    /// Asks for a section's title, starting as this.
    Title(Titling, String),
    /// Moves the section at this index, titled this, up (or down).
    Move(usize, bool, &'a str),
    Remove(usize, &'a str),
    /// Puts back the section removed, where it was.
    Restore(&'a (usize, String)),
}

/// What `action` does to Home's sections here, and the config file it
/// writes, or why it can't.
fn plan(state: &State, action: Action) -> Result<(&Path, Plan<'_>), String> {
    let plan = match action {
        Action::SaveSection => {
            let elsewhere =
                "Saving to Home works on a list of issues, pull requests or repositories";
            let (kind, query) = state.route().and_then(Route::search).ok_or(elsewhere)?;
            let kind = match kind {
                // A repository's pull request list is an issue search for `is:pr`.
                SearchKind::Issues if query.split_whitespace().any(|w| w == "is:pr") => {
                    SearchKind::Pulls
                }
                SearchKind::Issues | SearchKind::Pulls | SearchKind::Repos => kind,
                _ => return Err("Home sections list issues, pull requests or repositories".into()),
            };
            if query.trim().is_empty() {
                return Err("Type a search first: an empty one would list all of GitHub".into());
            }
            let name = match state.route() {
                Some(Route::Search { query, .. }) => query.clone(),
                Some(Route::Issues { repo, .. }) => format!("Issues in {repo}"),
                Some(Route::Pulls { repo, .. }) => format!("Pull requests in {repo}"),
                Some(route) => route.title(),
                None => String::new(),
            };
            Plan::Title(Titling::New { kind, query }, name)
        }
        Action::Delete | Action::UndoDelete if state.route() != Some(&Route::Home) => {
            return Err(
                "Deleting works on a draft comment in Files changed, or on a Home section".into(),
            );
        }
        _ if state.route() != Some(&Route::Home) => {
            return Err("That works on Home's sections".into());
        }
        Action::UndoDelete => {
            Plan::Restore((state.removed.as_ref()).ok_or("No section was removed just now")?)
        }
        _ => {
            let (at, section) = (state.section_here()).ok_or("Select a row in a section first")?;
            let title = section.title.as_str();
            match action {
                Action::RenameSection => Plan::Title(Titling::Rename { at }, title.to_owned()),
                Action::MoveSectionUp if at == 0 => return Err("It's the first section".into()),
                Action::MoveSectionDown if at + 1 >= state.home.len() => {
                    return Err("It's the last section".into());
                }
                Action::MoveSectionUp | Action::MoveSectionDown => {
                    Plan::Move(at, action == Action::MoveSectionUp, title)
                }
                Action::Delete => Plan::Remove(at, title),
                _ => return Err("That works on Home's sections".into()),
            }
        }
    };
    let path = (state.config_path.as_deref())
        .ok_or("There's no config file to keep it in (no config directory)")?;
    if state.home_editing {
        return Err("Still saving the last change".into());
    }
    Ok((path, plan))
}

/// Why `action` can't change Home's sections here, if it can't.
pub fn unavailable(state: &State, action: Action) -> Option<String> {
    plan(state, action).err()
}

/// Runs one of the actions on Home's sections.
#[must_use]
pub fn act(state: &mut State, action: Action) -> Vec<Cmd> {
    let (path, plan) = match plan(state, action) {
        Ok((path, plan)) => (path.to_owned(), plan),
        Err(why) => {
            state.info(why);
            return Vec::new();
        }
    };
    let (edit, what) = match plan {
        Plan::Title(titling, name) => {
            let cmds = state.open_picker(picker::Kind::SectionTitle(titling, path));
            if let Some(Overlay::Picker(p)) = &mut state.overlay {
                p.input.insert_str(&name);
            }
            return cmds;
        }
        Plan::Move(at, up, title) => {
            let what = format!("Moved “{title}” {}", if up { "up" } else { "down" });
            (Edit::Move { at, up }, what)
        }
        Plan::Remove(at, title) => {
            let undo = state.first_key(Action::UndoDelete);
            let what = format!("Removed “{title}” · {undo} brings it back");
            (Edit::Remove { at }, what)
        }
        Plan::Restore(&(at, ref table)) => {
            let (table, what) = (table.clone(), "Brought the section back".to_owned());
            (Edit::Restore { at, table }, what)
        }
    };
    state.edit_home(path, edit, what)
}

impl State {
    /// Sends `edit` to the config file at `path`; `what` says it once it's made.
    #[must_use]
    pub fn edit_home(&mut self, path: PathBuf, edit: Edit, what: String) -> Vec<Cmd> {
        self.home_editing = true;
        vec![Cmd::EditHome {
            path,
            shown: self.home.clone(),
            edit,
            what,
        }]
    }

    /// `config.toml` was changed (or not, and why): Home shows what it
    /// says now, the selection staying on the section moved.
    #[must_use]
    pub fn home_edited(
        &mut self,
        edit: &Edit,
        result: Result<(Vec<Section>, Option<String>), String>,
        what: String,
    ) -> Vec<Cmd> {
        self.home_editing = false;
        match result {
            Ok((sections, removed)) => {
                self.home = sections;
                match (edit, removed) {
                    (Edit::Remove { at }, Some(table)) => self.removed = Some((*at, table)),
                    (Edit::Restore { .. }, _) => self.removed = None,
                    _ => {}
                }
                self.info(what);
            }
            Err(why) => {
                // What the file says now, if it reads.
                if let Some(sections) = self
                    .config_path
                    .as_deref()
                    .and_then(|p| crate::config::Config::load(p).ok())
                    .map(|c| sections(c.home.as_deref()))
                {
                    self.home = sections;
                }
                self.error(why);
            }
        }
        self.data_gen += 1;
        if self.route() == Some(&Route::Home) {
            self.ensure_route(&Route::Home, false)
        } else {
            Vec::new()
        }
    }

    /// Follows a section's row that isn't a row of its list: its empty
    /// state, its error, or what's wrong with how it's written.
    #[must_use]
    pub fn open_section(&mut self, i: usize) -> Vec<Cmd> {
        match self.home.get(i).map(|s| s.search.clone()) {
            Some(Ok(search)) => self.push(search.route()),
            Some(Err(why)) => {
                let path = self
                    .config_path
                    .as_deref()
                    .map_or_else(|| "config.toml".to_owned(), |p| p.display().to_string());
                self.info(format!(
                    "This section can't be shown: {why}. Fix it in {path}"
                ));
                Vec::new()
            }
            None => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn parsed(text: &str) -> Vec<Section> {
        sections(Config::parse(text).unwrap().home.as_deref())
    }

    const WRITTEN: &str = r#"# my config
[ui]
density = "compact"

# What I review first.
[[home]]
title = "Needs me"   # the important one
pulls = "is:open review-requested:@me"

[[home]]
title = "Bugs"
issues = "is:open label:bug repo:o/r"
rows = 5

[keys]
down = ["j"]
"#;

    /// A mistake in one section is that section's to show; the others
    /// load, and the config still loads.
    #[test]
    fn a_mistake_stays_in_its_section() {
        let shown = parsed(
            r#"
            [[home]]
            title = "Fine"
            pulls = "author:@me"

            [[home]]
            title = "Two lists"
            pulls = "a"
            issues = "b"

            [[home]]
            title = "Too many"
            repos = "user:@me"
            rows = 500

            [[home]]
            pulls = "no title"
            "#,
        );
        assert_eq!(shown.len(), 4);
        assert_eq!(
            shown[0].search,
            Ok(Search {
                kind: SearchKind::Pulls,
                query: "author:@me".into(),
                rows: DEFAULT_ROWS
            })
        );
        let why: Vec<String> = shown[1..]
            .iter()
            .map(|s| s.search.clone().unwrap_err())
            .collect();
        assert!(why[0].contains("more than one"), "{why:?}");
        assert!(why[1].contains("from 1 to 30"), "{why:?}");
        assert!(why[2].contains("needs a title"), "{why:?}");
        // An unknown key is still an error, as everywhere in the config.
        Config::parse("[[home]]\ntitle = \"x\"\npull = \"a\"").unwrap_err();
        // No sections are the defaults: GitHub's dashboard.
        assert_eq!(parsed(""), defaults());
    }

    /// Edits keep the file's comments and layout; a move takes a
    /// section's comments with it; a removal can be put back as it was.
    #[test]
    fn edits_keep_the_file_as_written() {
        let shown = parsed(WRITTEN);
        let rename = Edit::Rename {
            at: 0,
            title: "Reviews".into(),
        };
        let renamed = edit_text(WRITTEN, &shown, &rename).unwrap().text;
        assert_eq!(
            renamed,
            WRITTEN.replace("title = \"Needs me\"", "title = \"Reviews\"")
        );
        let moved = edit_text(WRITTEN, &shown, &Edit::Move { at: 0, up: false })
            .unwrap()
            .text;
        let titles: Vec<String> = parsed(&moved).into_iter().map(|s| s.title).collect();
        assert_eq!(titles, ["Bugs", "Needs me"]);
        assert!(moved.starts_with("# my config\n[ui]"), "{moved}");
        let comment = moved.find("# What I review first.").unwrap();
        assert!(moved.find("title = \"Bugs\"").unwrap() < comment, "{moved}");
        assert!(moved.contains("# the important one"), "{moved}");
        assert!(moved.trim_end().ends_with("down = [\"j\"]"), "{moved}");
        let removed = edit_text(WRITTEN, &shown, &Edit::Remove { at: 0 }).unwrap();
        assert_eq!(parsed(&removed.text).len(), 1);
        let back = Edit::Restore {
            at: 0,
            table: removed.removed.unwrap(),
        };
        let restored = edit_text(&removed.text, &parsed(&removed.text), &back).unwrap();
        assert_eq!(restored.text, WRITTEN);
        // From the middle too.
        let removed = edit_text(WRITTEN, &shown, &Edit::Remove { at: 1 }).unwrap();
        let back = Edit::Restore {
            at: 1,
            table: removed.removed.unwrap(),
        };
        let restored = edit_text(&removed.text, &parsed(&removed.text), &back).unwrap();
        assert_eq!(restored.text, WRITTEN);
    }

    /// The first edit of a Home that has no tables writes the defaults
    /// out, then changes them; removing every section leaves none, not
    /// the defaults. A file changed since it was read isn't edited.
    #[test]
    fn defaults_are_written_out_and_changes_elsewhere_win() {
        let add = Edit::Add {
            title: "Mine".into(),
            kind: SearchKind::Issues,
            query: "assignee:@me".into(),
        };
        let text = edit_text("[ui]\nnerd_font = true\n", &defaults(), &add)
            .unwrap()
            .text;
        let shown = parsed(&text);
        assert_eq!(shown.len(), 4);
        assert_eq!(shown[..3], defaults()[..]);
        assert!(text.starts_with("[ui]\nnerd_font = true\n"), "{text}");
        let mut text = text;
        for _ in 0..4 {
            text = edit_text(&text, &parsed(&text), &Edit::Remove { at: 0 })
                .unwrap()
                .text;
        }
        assert!(parsed(&text).is_empty(), "{text}");
        let stale = edit_text(WRITTEN, &defaults(), &Edit::Remove { at: 0 }).unwrap_err();
        assert!(stale.contains("changed since"), "{stale}");
    }
}
