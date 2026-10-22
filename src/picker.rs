//! One fuzzy picker for every list under a text field: the command
//! palette, go to file (in a repository and in a diff), branches and tags,
//! the diff's commits, and places in code (symbols, definitions).

use std::path::{Path, PathBuf};

use crate::api::browse::SearchKind;
use crate::api::model::RepoId;
use crate::api::watch::Watched;
use crate::route::{self, Dest, Route, Target, code, search};
use crate::theme::Bg;
use crate::ui::bars::Notice;
use crate::ui::diff_doc::Pos;
use crate::ui::overlays::PaletteItem;
use crate::ui::text::short_sha;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui_textarea::TextArea;

use crate::data::{Data, DataKey, Picked};
use crate::keymap::Action;
use crate::nav::new_input;
use crate::review::apply_commit_choice;
use crate::state::{Cmd, Cmds, Overlay, Remote, Screen, State, apply};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// The command palette.
    Commands,
    /// The repository's files at `rev`.
    Files {
        repo: RepoId,
        rev: String,
    },
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
    Commits {
        mark: Option<usize>,
    },
    /// Recent notices and errors, newest first, as they were on opening.
    Messages(Vec<(u64, Notice)>),
    /// A Home section's title, for the config file at the path: the field is all of it.
    SectionTitle(Titling, std::path::PathBuf),
    /// Editing what's on GitHub: a sidebar field, a reaction, a title.
    Edit(crate::edit::Choosing),
    /// One of a new issue's or pull request's choices.
    Form(crate::create::Pick),
    /// Places in code: a file's symbols, a name's definitions.
    Code {
        title: String,
        rows: Vec<(PaletteItem, Choice)>,
    },
    /// A project item's columns, fields or a field's values.
    Project(crate::projects::Pick),
    /// Why to dismiss a security alert; the field is the comment.
    Alert(crate::security::Dismissing),
    /// How you're notified of it ([`crate::watch`]).
    Watch(Watched),
    /// A release's tag, on its form's page ([`crate::release`]).
    Tag(Route),
    /// What a new tag is made at.
    Target(Route),
    /// Where to save a release's asset.
    SaveAsset(crate::release::assets::Of),
    /// A file to upload to a release.
    Upload(crate::release::assets::Of),
    /// The gist's description ([`crate::gist`]).
    Description(Route),
    /// A file's name (`None`: a new file's).
    FileName(Route, Option<usize>),
    /// Your starred lists ([`crate::site`]).
    Site(crate::site::Pick),
    Housekeeping(crate::housekeeping::Pick),
    /// Where to save an artifact; a workflow's branch or input.
    Workflows(crate::workflows::Pick),
    /// Locking, hiding, a base, sub-issues and the like ([`crate::manage`]).
    Manage(crate::manage::Pick),
    /// Someone to invite, a role, a repository's name ([`crate::settings`]).
    Settings(crate::settings::Pick),
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
    pub fn title(&self) -> &str {
        match self.kind {
            Kind::Commands => ":",
            Kind::Files { .. } | Kind::DiffFiles => "Go to file",
            Kind::Branches { .. } => "Switch branches/tags",
            Kind::Commits { .. } => "Commits",
            Kind::Messages(_) => "Messages",
            Kind::SectionTitle(Titling::New { .. }, _) => "Save to Home",
            Kind::SectionTitle(Titling::Rename { .. }, _) => "Rename the section",
            Kind::Edit(ref c) => c.title(),
            Kind::Form(pick) => pick.title(),
            Kind::Code { ref title, .. } => title,
            Kind::Project(ref pick) => pick.title(),
            Kind::Alert(ref d) => d.title(),
            Kind::Watch(Watched::Repo(_)) => "Watch the repository",
            Kind::Watch(_) => "Notifications",
            Kind::Tag(_) => "Choose a tag",
            Kind::Target(_) => "Target: a branch or commit",
            Kind::SaveAsset(_) => "Save the asset",
            Kind::Upload(_) => "Upload an asset",
            Kind::Description(_) => "Gist description",
            Kind::FileName(_, None) => "Add a file",
            Kind::FileName(..) => "Rename the file",
            Kind::Site(ref pick) => pick.title(),
            Kind::Housekeeping(ref pick) => pick.title(),
            Kind::Workflows(ref pick) => pick.title(),
            Kind::Manage(ref pick) => pick.title(),
            Kind::Settings(ref pick) => pick.title(),
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
    Go(Dest),
    Search(String),
    DiffFile(usize),
    Commits(PickItem),
    /// Copy this text.
    Copy(String),
    /// Go to open tab N (from 0).
    Tab(usize),
    /// Title a Home section this, in the config file at the path.
    Title(Titling, std::path::PathBuf, String),
    /// Handled while the picker is open ([`crate::edit::on_picker_key`]).
    Edit(crate::edit::Pick),
    /// Set one of a form's choices.
    Form(crate::create::Picked),
    /// Go to this line of the file on screen.
    Line(u32),
    /// A step of an action's plan ([`crate::act::take`]).
    Step(crate::act::Step),
}

pub type Rows = Vec<(PaletteItem, Option<Choice>)>;

pub(crate) fn item(label: impl Into<String>, hint: impl Into<String>) -> PaletteItem {
    PaletteItem {
        label: label.into(),
        hint: hint.into(),
    }
}

/// What `key` holds, or the row saying it's loading or why it failed
/// (opening the picker again tries again).
pub(crate) fn listed<'a, T: Picked + ?Sized, C>(
    state: &'a State,
    key: &DataKey,
    what: &str,
) -> Result<&'a T, Vec<(PaletteItem, Option<C>)>> {
    #[expect(clippy::disallowed_methods, reason = "said as the picker's row")]
    let listed = Remote::fetched(state.remote(key), "")
        .pick(T::pick)
        .text(what);
    listed.map_err(|why| vec![(item(why, ""), None)])
}

/// A picker's row: what it says, and what choosing it does.
pub(crate) fn entry<T>(
    label: impl Into<String>,
    hint: impl Into<String>,
    chosen: Option<T>,
) -> (PaletteItem, Option<T>) {
    (item(label, hint), chosen)
}

/// Where file `name` is saved for what's typed: in the current folder by
/// default, in a folder named, or as the file named.
fn save_path(typed: &str, name: &str) -> PathBuf {
    let typed = typed.trim();
    if typed.is_empty() {
        return Path::new(".").join(name);
    }
    let path = PathBuf::from(typed);
    if typed.ends_with('/') || path.is_dir() {
        path.join(name)
    } else {
        path
    }
}

/// The row saving file `name` where it's typed, with `save`'s job (told
/// the path and whether to replace what's there); where a file is,
/// replacing it, which choosing is the asking.
pub(crate) fn save_rows(
    q: &str,
    name: &str,
    hint: &str,
    save: impl FnOnce(PathBuf, bool) -> crate::work::Work,
) -> Rows {
    let path = save_path(q, name);
    let shown = path.display().to_string();
    if path.is_dir() {
        return vec![(item(format!("{shown} is a folder"), ""), None)];
    }
    let replace = path.exists();
    let row = if replace {
        item(format!("Replace {shown}"), "it exists")
    } else {
        item(format!("Save to {shown}"), hint)
    };
    let doing = format!("Saving {name} to {shown}…");
    let step = crate::act::Step::Start(doing, save(path, replace));
    vec![(row, Some(Choice::Step(step)))]
}

/// Places in code listed at once.
const CODE_ROWS: usize = 1000;

/// Places in code that match `q`, best first; all of them in order
/// before anything is typed.
fn code_rows(rows: &[(PaletteItem, Choice)], q: &str) -> Rows {
    let hits: Vec<_> = (rows.iter().enumerate())
        .filter_map(|(i, (item, choice))| {
            Some(((fuzzy_score(q, item.label.trim_start())?, i), item, choice))
        })
        .collect();
    // A repository's symbols can be many: the best few are what's read.
    let hits = best(hits, CODE_ROWS, |(key, ..)| *key);
    (hits.into_iter())
        .map(|(_, item, choice)| (item.clone(), Some(choice.clone())))
        .collect()
}

/// The first `n` of `hits` by `key` (which ties nothing: it ends with
/// each one's place), in order. Only those are sorted.
fn best<T, K: Ord>(mut hits: Vec<T>, n: usize, key: impl Fn(&T) -> K + Copy) -> Vec<T> {
    if let Some(last) = n.checked_sub(1).filter(|_| hits.len() > n) {
        hits.select_nth_unstable_by_key(last, key);
    }
    hits.truncate(n);
    hits.sort_unstable_by_key(key);
    hits
}

/// Notices that match `q`; choosing one copies it.
fn message_rows(kept: &[(u64, Notice)], q: &str, now: u64) -> Rows {
    let rows = kept.iter().filter_map(|(at, notice)| {
        let (text, kind) = match notice {
            Notice::Info(text) => (text, ""),
            Notice::Error(text) => (text, "error · "),
        };
        fuzzy_score(q, text)?;
        let hint = format!("{kind}{}", crate::ui::time::ago(*at, now));
        Some((item(text, hint), Some(Choice::Copy(text.to_owned()))))
    });
    rows.collect()
}

impl State {
    pub fn open_picker(&mut self, kind: Kind) -> Cmds {
        let placeholder = match kind {
            Kind::Commands => "A command, owner/repo, owner/repo#123, @user, a URL, or a search",
            Kind::Files { .. } | Kind::DiffFiles => "Type a file name",
            Kind::Branches { .. } => "Find a branch or tag",
            Kind::Commits { .. } => "space marks a range start · ↵ views · esc cancels",
            Kind::Messages(_) => "↵ copies a message",
            Kind::SectionTitle(..) => "The section's title",
            Kind::Edit(ref c) => c.placeholder(),
            Kind::Form(pick) if pick.several() => {
                "Type to filter · ↵ picks or unpicks · esc when done"
            }
            Kind::Form(_) | Kind::Watch(_) => "Type to filter",
            Kind::Code { .. } => "Type a name",
            Kind::Project(ref pick) => pick.placeholder(),
            Kind::Alert(ref d) => d.placeholder(),
            Kind::Tag(_) => "Find a tag, or type a new one",
            Kind::Target(_) => "Find a branch, or type a commit's SHA",
            Kind::SaveAsset(_) => "A file or folder (↵ saves in the current folder)",
            Kind::Upload(_) => "A file's path (a folder lists its files)",
            Kind::Description(_) => "Type the description",
            Kind::FileName(..) => "Type the file's name, with its extension",
            Kind::Site(ref pick) => pick.placeholder(),
            Kind::Housekeeping(ref pick) => pick.placeholder(),
            Kind::Workflows(ref pick) => pick.placeholder(),
            Kind::Manage(ref pick) => pick.placeholder(),
            Kind::Settings(ref pick) => pick.placeholder(),
        };
        let need = match &kind {
            Kind::Files { repo, rev } => Some(DataKey::Files(repo.clone(), rev.clone())),
            Kind::Branches { repo, .. } => Some(DataKey::Refs(repo.clone())),
            Kind::Edit(c) => c.need(),
            Kind::Watch(w) => Some(DataKey::Subscription(w.clone())),
            Kind::Tag(route) => route.repo().map(|r| DataKey::Tags(r.clone())),
            Kind::Target(route) => route.repo().map(|r| DataKey::Branches(r.clone())),
            Kind::Site(pick) => pick.need(),
            Kind::Housekeeping(pick) => pick.need(),
            Kind::Manage(pick) => pick.need(),
            Kind::Form(crate::create::Pick::Base | crate::create::Pick::Head) => self
                .route()
                .and_then(Route::repo)
                .map(|r| DataKey::Refs(r.clone())),
            _ => None,
        };
        // The field holds what's set now, to change.
        let start = match &kind {
            Kind::Edit(crate::edit::Choosing::Title { was, .. }) => Some(was.clone()),
            Kind::Description(route) => crate::gist::description(self, route),
            Kind::FileName(route, Some(i)) => crate::gist::file_name(self, route, *i),
            Kind::Housekeeping(pick) => pick.start(self),
            Kind::Manage(pick) => pick.start(),
            _ => None,
        };
        let mut input = new_input(&self.theme, Bg::ContainerHigh);
        input.set_placeholder_text(placeholder);
        input.set_placeholder_style(self.theme.meta(Bg::ContainerHigh));
        input.insert_str(start.unwrap_or_default());
        self.overlay = Some(Overlay::Picker(Box::new(Picker {
            kind,
            input,
            selected: 0,
        })));
        need.map_or_else(Cmds::new, |key| self.ensure(key, false))
    }

    /// Go to file, or switch branches, for the code on screen.
    pub fn open_finder(&mut self, branches: bool) -> Cmds {
        let (repo, rev, path, file) = match self.route() {
            Some(Route::Code(code::Tree { repo, rev, path })) => {
                (repo.clone(), rev.clone(), path.clone(), false)
            }
            Some(Route::Code(code::Blob {
                repo, rev, path, ..
            })) => (repo.clone(), rev.clone(), path.clone(), true),
            Some(route) if let Some(repo) = route.repo() => {
                let default = self.overview(repo).and_then(|o| o.default_branch.clone());
                let rev = default.unwrap_or_else(|| "HEAD".into());
                (repo.clone(), rev, String::new(), false)
            }
            _ => {
                return self.just_say("Open a repository first");
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
            Kind::Edit(c) => crate::edit::rows(self, c, q),
            Kind::Form(pick) => crate::create::picker_rows(self, *pick, q),
            Kind::Code { rows, .. } => code_rows(rows, q),
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
            Kind::Workflows(pick) => crate::workflows::rows(self, pick, q),
            Kind::Project(pick) => crate::projects::rows(self, pick, q),
            Kind::Alert(d) => crate::security::rows(d, q),
            Kind::Watch(w) => crate::watch::rows(self, w, q),
            Kind::Tag(route) => crate::release::tag_rows(self, route, q),
            Kind::Target(route) => crate::release::target_rows(self, route, q),
            Kind::SaveAsset(of) => crate::release::assets::save_rows(of, q),
            Kind::Upload(of) => crate::release::assets::upload_rows(self, of, q),
            Kind::Description(route) => crate::gist::description_rows(route, q),
            Kind::FileName(route, file) => crate::gist::name_rows(self, route, *file, q),
            Kind::Site(pick) => crate::site::rows(self, pick, q),
            Kind::Housekeeping(pick) => crate::housekeeping::rows(self, pick, q),
            Kind::Manage(pick) => (crate::manage::rows(self, pick, q).into_iter())
                .map(|(row, chosen)| (row, chosen.map(Choice::Step)))
                .collect(),
            Kind::Settings(pick) => (crate::settings::rows(pick, q).into_iter())
                .map(|(row, chosen)| (row, chosen.map(Choice::Step)))
                .collect(),
        }
    }

    /// What's on screen is numbered lines: a file, a blame, a job's log
    /// or a diff.
    fn shows_lines(&self) -> bool {
        self.screen().page().is_none_or(|p| {
            matches!(
                p.route,
                Route::Code(code::Blob { .. } | code::Blame { .. })
                    | Route::Runs(crate::route::runs::Job { .. })
            )
        })
    }

    /// Palette entries: somewhere to go, what you can do here (the menu's
    /// order first), and searching GitHub.
    pub fn commands(&self, input: &str) -> Rows {
        let mut out = Vec::new();
        // vim's way out, first.
        let quit_first = matches!(input.trim(), "q" | "q!" | "qa" | "qa!" | "quit");
        if quit_first {
            let key = self.key_here(Action::Quit).unwrap_or_default();
            out.push((item("Quit", key), Some(Choice::Action(Action::Quit))));
        }
        // A number, where lines are what's on screen: that line.
        if let Ok(line) = input.trim().parse::<u32>()
            && line > 0
            && self.shows_lines()
        {
            out.push((
                item(format!("Go to line {line}"), ""),
                Some(Choice::Line(line)),
            ));
        }
        if let Some(dest) = route::parse_input(input, self.context_repo()) {
            let label = match &dest.target {
                Target::Page(route) => format!("Go to {}", route.title()),
                Target::Files(of) => format!("Files changed in {of}"),
                Target::External(url) => format!("Open {url}"),
            };
            out.push((item(label, ""), Some(Choice::Go(dest))));
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
        let contexts = self.contexts();
        let doables = self.doables();
        let mut order: Vec<Action> = doables.iter().map(|d| d.action).collect();
        // Not moving around (those are keys, not commands to look for), and
        // only what does something here.
        order.extend((Action::ALL.iter()).filter(|a| {
            contexts.contains(&a.context())
                && !a.moves()
                && crate::nav::availability_in(&doables, **a).is_ok()
        }));
        let mut actions: Vec<(usize, Action)> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        // Names are snake_case: "go to" finds go_to_file, matched as typed
        // once rather than each name spelled with spaces.
        let underscored = input.replace(' ', "_");
        for a in order {
            if a == Action::CommandPalette || (quit_first && a == Action::Quit) || !seen.insert(a) {
                continue;
            }
            let by_name = fuzzy_score(&underscored, a.name());
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

    fn file_rows(&self, q: &str, repo: &RepoId, rev: &str) -> Rows {
        let key = DataKey::Files(repo.clone(), rev.into());
        let Data::Files(files, truncated) = or_return!(listed::<Data, _>(self, &key, "files"))
        else {
            return Vec::new();
        };
        let hits: Vec<((usize, usize, usize), &String)> = (files.iter().enumerate())
            .filter_map(|(i, p)| Some(((path_score(q, p)?, p.len(), i), p)))
            .collect();
        let mut rows: Rows = (best(hits, 200, |(key, _)| *key).into_iter())
            .map(|(_, path)| {
                let target = Target::Page(Route::blob(repo.clone(), rev.to_owned(), path.clone()));
                (item(path, ""), Some(Choice::Go(target.into())))
            })
            .collect();
        if *truncated && rows.len() < 200 {
            rows.push((item("GitHub listed only part of this repository", ""), None));
        }
        rows
    }

    fn branch_rows(&self, q: &str, repo: &RepoId, rev: &str, path: &str, file: bool) -> Rows {
        let key = DataKey::Refs(repo.clone());
        let refs = or_return!(listed::<crate::api::browse::Refs, _>(
            self, &key, "branches"
        ));
        let default = self
            .overview(repo)
            .and_then(|o| o.default_branch.as_deref());
        let mut rows = Vec::new();
        for (names, what) in [(&refs.branches, "branch"), (&refs.tags, "tag")] {
            for name in names {
                let Some(score) = fuzzy_score(q, name) else { continue };
                let is_default = default == Some(name.as_str());
                let hint = if is_default {
                    "default"
                } else if name == rev {
                    "current"
                } else {
                    what
                };
                let (repo, rev, path) = (repo.clone(), name.clone(), path.to_owned());
                let route = if file {
                    Route::blob(repo, rev, path)
                } else if path.is_empty() && is_default {
                    Route::code(repo)
                } else {
                    Route::Code(code::Tree { repo, rev, path })
                };
                let go = Choice::Go(Target::Page(route).into());
                rows.push((score, item(name, hint), go));
            }
        }
        rows.sort_by_key(|(s, ..)| *s);
        let mut rows: Rows = rows.into_iter().map(|(_, i, c)| (i, Some(c))).collect();
        // What wasn't fetched can't be picked: say so.
        for (list, what) in [(&refs.branches, "branch"), (&refs.tags, "tag")] {
            let rest = list.left_out();
            if rest > 0 {
                let what = crate::ui::text::noun(rest, what);
                rows.push((item(format!("{rest} more {what} on GitHub"), ""), None));
            }
        }
        rows
    }

    fn diff_file_rows(&self, q: &str) -> Rows {
        let Some(diff) = self.diff() else { return Vec::new() };
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
        let Some(diff) = self.diff() else { return Vec::new() };
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
/// rank before scattered ones, earlier before later. ASCII (most of what's
/// matched) is compared as it is, without lowercasing a copy.
pub fn fuzzy_score(needle: &str, haystack: &str) -> Option<usize> {
    /// Where the last of `needle` is found in `hay`, one after another.
    fn scattered<T: PartialEq>(
        needle: impl Iterator<Item = T>,
        mut hay: impl Iterator<Item = (usize, T)>,
    ) -> Option<usize> {
        let mut last = 0;
        for n in needle {
            last = hay.find(|(_, h)| *h == n)?.0;
        }
        Some(1000 + last)
    }
    if !(needle.is_ascii() && haystack.is_ascii()) {
        let (needle, hay) = (needle.to_lowercase(), haystack.to_lowercase());
        return hay
            .find(&needle)
            .or_else(|| scattered(needle.chars(), hay.char_indices()));
    }
    let lower = |b: u8| b.to_ascii_lowercase();
    find_ci(haystack, needle).or_else(|| {
        let hay = haystack.bytes().map(lower).enumerate();
        scattered(needle.bytes().map(lower), hay)
    })
}

/// Where `needle` is in `hay`, in any case.
pub(crate) fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    if !(hay.is_ascii() && needle.is_ascii()) {
        return hay.to_lowercase().find(&needle.to_lowercase());
    }
    if needle.is_empty() {
        return Some(0);
    }
    (hay.as_bytes().windows(needle.len())).position(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
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

pub fn on_key(state: &mut State, key: KeyEvent) -> Cmds {
    let Some(Overlay::Picker(p)) = &state.overlay else { return Cmds::new() };
    let mut rows = state.picker_rows(p);
    // Moving a sub-issue (alt-↑↓) before moving in the list.
    if let Some(cmds) = crate::manage::picker_key(state, key, &mut rows) {
        return cmds;
    }
    let Some(Overlay::Picker(p)) = &mut state.overlay else { return Cmds::new() };
    if move_in_list(key, &mut p.selected, rows.len()) {
        return Cmds::new();
    }
    if let Kind::Edit(_) = p.kind {
        return crate::edit::on_picker_key(state, key, rows);
    }
    match (key.code, &mut p.kind) {
        (KeyCode::Esc, _) => state.overlay = None,
        (KeyCode::Enter, kind) => {
            let mark = match kind {
                Kind::Commits { mark } => *mark,
                _ => None,
            };
            let selected = p.selected;
            // Labels and assignees: several, so the picker stays.
            let several = matches!(kind, Kind::Form(pick) if pick.several());
            if let Some(choice) = rows.get_mut(selected).and_then(|(_, c)| c.take()) {
                if !several {
                    state.overlay = None;
                }
                return choose(state, choice, mark);
            }
        }
        // Commits aren't filtered: space marks where a range starts.
        (KeyCode::Char(' '), Kind::Commits { mark }) => {
            if let Some((_, Some(Choice::Commits(PickItem::Commit(i))))) = rows.get(p.selected) {
                *mark = (*mark != Some(*i)).then_some(*i);
            }
        }
        (_, Kind::Commits { .. }) => {}
        _ => {
            crate::nav::type_into(&mut p.input, key);
            p.selected = 0;
        }
    }
    Cmds::new()
}

fn choose(state: &mut State, choice: Choice, mark: Option<usize>) -> Cmds {
    match choice {
        Choice::Action(action) => apply(state, action),
        Choice::Go(target) => state.go(target),
        Choice::Search(query) => state.push(search_route(&query)),
        Choice::DiffFile(file) => {
            if let Screen::Diff(screen) = state.screen_mut() {
                screen.cursor = Pos { file, row: 0 };
                screen.top = screen.cursor;
            }
            Cmds::new()
        }
        Choice::Commits(pick) => apply_commit_choice(state, pick, mark),
        Choice::Copy(text) => {
            state.info("Copied the message");
            Cmd::Copy(text).into()
        }
        Choice::Tab(i) => state.switch_to_tab(i),
        Choice::Line(line) => crate::code_nav::go_to_line(state, line),
        Choice::Title(Titling::New { kind, query }, path, title) => {
            let what = format!("Added “{title}” to Home");
            state.edit_home(path, crate::home::Edit::Add { title, kind, query }, what)
        }
        Choice::Form(picked) => crate::create::pick(state, picked),
        Choice::Title(Titling::Rename { at }, path, title) => {
            let what = format!("Renamed it “{title}”");
            state.edit_home(path, crate::home::Edit::Rename { at, title }, what)
        }
        Choice::Edit(_) => Cmds::new(),
        Choice::Step(step) => crate::act::take(state, Ok(step)),
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
    Route::Search(search::Search {
        kind: if issues {
            SearchKind::Issues
        } else {
            SearchKind::Repos
        },
        query: query.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file is saved by its own name in the current folder, in a
    /// folder named, or as the file named.
    #[test]
    fn a_file_is_saved_where_its_typed() {
        let name = "ghtui.tar.gz";
        assert_eq!(save_path("", name), Path::new(".").join(name));
        assert_eq!(save_path("out/", name), Path::new("out/").join(name));
        assert_eq!(save_path("x.tgz", name), PathBuf::from("x.tgz"));
    }

    #[test]
    fn file_matches_prefer_names() {
        assert!(path_score("main", "src/main.rs") < path_score("main", "maintainers/x.rs"));
        assert_eq!(path_score("zzz", "src/main.rs"), None);
    }
}
