//! Application state and the single update function. Input events and async
//! results arrive as [`Msg`]s; [`update`] mutates [`State`] and returns
//! [`Cmd`]s for the runtime to execute in the background.

use std::collections::HashMap;
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ghtui_api::ApiError;
use ghtui_api::model::{Inbox, PrDetail, PrRef, ViewedFiles};
use ghtui_api::rate_limit::RateLimits;
use ghtui_diff::FileDiff;
use ghtui_store::ReviewState;
use ghtui_theme::{Bg, Theme};
use ghtui_ui::bars::Notice;
use ghtui_ui::diff_doc::{Doc, Pos, Viewed};
use ghtui_ui::overlays::{HelpEntry, PaletteItem};
use ghtui_ui::pr_view::PrView;
use ghtui_ui::{Ctx, Icons, PAD_Y, pr_list};
use ratatui::layout::Rect;
use ratatui_textarea::TextArea;

use crate::diff_job::DiffFiles;
use crate::diff_screen::{self, DiffScreen, DiffState};
use crate::keymap::{Action, Key, Keymap, Resolution};

#[derive(Debug)]
pub enum Msg {
    Key(KeyEvent),
    Resize(u16, u16),
    Viewer(Result<String, ApiError>),
    Inbox(Result<Inbox, ApiError>),
    Pr(PrRef, Box<Result<PrDetail, ApiError>>),
    RateLimits(RateLimits),
    Notice(Notice),
    DiffProgress(PrRef, String),
    DiffFiles(PrRef, Box<DiffFiles>),
    FileDiff(PrRef, usize, Arc<FileDiff>),
    DiffFailed(PrRef, String),
    ViewedLoaded(PrRef, Box<Result<ViewedFiles, ApiError>>),
    ViewedSaved {
        pr: PrRef,
        file: usize,
        previous: Viewed,
        result: Result<(), ApiError>,
    },
    ReviewLoaded(PrRef, ReviewState),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cmd {
    FetchViewer,
    FetchInbox,
    FetchPr(PrRef),
    OpenUrl(String),
    /// Start (or restart) the diff job for a PR.
    LoadDiff {
        pr: PrRef,
        base_ref: String,
    },
    /// Diff these files next.
    Prioritize(PrRef, Vec<usize>),
    FetchViewed(PrRef),
    SetViewed {
        pr: PrRef,
        pull_request_id: String,
        path: String,
        file: usize,
        viewed: bool,
        previous: Viewed,
    },
    LoadReview(PrRef),
    SaveReview(PrRef, ReviewState),
}

/// Data that comes from GitHub: what we have (possibly cached), whether a
/// refresh is in flight, and the last refresh error.
#[derive(Debug)]
pub struct Remote<T> {
    pub data: Option<T>,
    pub loading: bool,
    pub error: Option<String>,
}

impl<T> Default for Remote<T> {
    fn default() -> Self {
        Self {
            data: None,
            loading: false,
            error: None,
        }
    }
}

impl<T> Remote<T> {
    pub fn cached(data: Option<T>) -> Self {
        Self {
            data,
            ..Self::default()
        }
    }

    fn start(&mut self) {
        self.loading = true;
    }

    fn finish(&mut self, result: Result<T, ApiError>) {
        self.loading = false;
        match result {
            Ok(data) => {
                self.data = Some(data);
                self.error = None;
            }
            Err(err) => self.error = Some(err.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    Inbox { selected: usize, scroll: u16 },
    Pr { pr: PrRef, scroll: u16 },
    Diff(Box<DiffScreen>),
}

pub enum Overlay {
    Help,
    Palette(Box<Palette>),
    /// `/` prompt on the diff screen.
    Search(Box<TextArea<'static>>),
    /// `gf` file finder on the diff screen.
    FindFile(Box<Palette>),
}

pub struct Palette {
    pub input: TextArea<'static>,
    pub selected: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteCommand {
    Action(Action),
    OpenPr(PrRef),
}

pub struct State {
    /// Navigation stack; never empty, and `screens[0]` is the inbox.
    pub screens: Vec<Screen>,
    pub inbox: Remote<Inbox>,
    pub prs: HashMap<PrRef, Remote<PrDetail>>,
    pub diffs: HashMap<PrRef, DiffState>,
    pub viewer: Option<String>,
    pub rate_limits: RateLimits,
    pub overlay: Option<Overlay>,
    pub pending: Vec<Key>,
    pub notice: Option<Notice>,
    /// Terminal size.
    pub size: (u16, u16),
    pub keymap: Keymap,
    pub theme: Theme,
    pub icons: Icons,
    pub quit: bool,
}

impl State {
    pub fn new(theme: Theme, icons: Icons, keymap: Keymap, size: (u16, u16)) -> Self {
        Self {
            screens: vec![Screen::Inbox {
                selected: 0,
                scroll: 0,
            }],
            inbox: Remote::default(),
            prs: HashMap::new(),
            diffs: HashMap::new(),
            viewer: None,
            rate_limits: RateLimits::default(),
            overlay: None,
            pending: Vec::new(),
            notice: None,
            size,
            keymap,
            theme,
            icons,
            quit: false,
        }
    }

    pub fn screen(&self) -> &Screen {
        self.screens.last().expect("screen stack is never empty")
    }

    fn screen_mut(&mut self) -> &mut Screen {
        self.screens
            .last_mut()
            .expect("screen stack is never empty")
    }

    pub fn ctx(&self, now: u64) -> Ctx<'_> {
        Ctx {
            theme: &self.theme,
            icons: self.icons,
            now,
        }
    }

    /// The area between the top and status bars.
    pub fn content_area(&self) -> Rect {
        let (w, h) = self.size;
        Rect {
            x: 0,
            y: 1,
            width: w,
            height: h.saturating_sub(2),
        }
    }

    /// Height available to the inbox list.
    pub fn list_viewport(&self) -> u16 {
        let banner = u16::from(self.inbox.data.is_some() && self.inbox.error.is_some());
        self.content_area()
            .height
            .saturating_sub(2 * PAD_Y + banner)
    }

    /// Opens a PR on top of the stack (returns the fetch to run).
    pub fn open_pr(&mut self, pr: PrRef) -> Vec<Cmd> {
        self.screens.push(Screen::Pr {
            pr: pr.clone(),
            scroll: 0,
        });
        self.ensure_pr(&pr, true)
    }

    fn ensure_pr(&mut self, pr: &PrRef, force: bool) -> Vec<Cmd> {
        let remote = self.prs.entry(pr.clone()).or_default();
        if remote.loading || (!force && remote.data.is_some()) {
            return Vec::new();
        }
        remote.start();
        vec![Cmd::FetchPr(pr.clone())]
    }

    fn ensure_inbox(&mut self, force: bool) -> Vec<Cmd> {
        if self.inbox.loading || (!force && self.inbox.data.is_some() && self.inbox.error.is_none())
        {
            return Vec::new();
        }
        self.inbox.start();
        vec![Cmd::FetchInbox]
    }

    /// Fetches needed for the visible screen. `force` refreshes data we
    /// already have.
    pub fn load_visible(&mut self, force: bool) -> Vec<Cmd> {
        let mut cmds = Vec::new();
        if self.viewer.is_none() && force {
            cmds.push(Cmd::FetchViewer);
        }
        match self.screen().clone() {
            Screen::Inbox { .. } => cmds.extend(self.ensure_inbox(force)),
            Screen::Pr { pr, .. } => cmds.extend(self.ensure_pr(&pr, force)),
            Screen::Diff(screen) => {
                if force || !self.diffs.contains_key(&screen.pr) {
                    cmds.extend(self.start_diff(&screen.pr));
                }
            }
        }
        cmds
    }

    /// Opens the diff of a PR whose metadata we have.
    pub fn open_diff(&mut self, pr: PrRef) -> Vec<Cmd> {
        self.screens.push(Screen::Diff(Box::new(DiffScreen::new(
            pr.clone(),
            self.size.0,
        ))));
        let reuse = self.diffs.get(&pr).is_some_and(|d| d.error.is_none());
        if reuse {
            self.settle_diff()
        } else {
            self.start_diff(&pr)
        }
    }

    fn start_diff(&mut self, pr: &PrRef) -> Vec<Cmd> {
        let Some(base_ref) = self
            .prs
            .get(pr)
            .and_then(|r| r.data.as_ref())
            .map(|d| d.base_ref.clone())
        else {
            return Vec::new();
        };
        self.diffs.insert(pr.clone(), DiffState::loading());
        vec![
            Cmd::LoadDiff {
                pr: pr.clone(),
                base_ref,
            },
            Cmd::FetchViewed(pr.clone()),
            Cmd::LoadReview(pr.clone()),
        ]
    }

    /// Files matching the finder input, best first.
    pub fn finder_items(&self, input: &str) -> Vec<(usize, PaletteItem)> {
        let Screen::Diff(screen) = self.screen() else {
            return Vec::new();
        };
        let Some(diff) = self.diffs.get(&screen.pr) else {
            return Vec::new();
        };
        let mut items: Vec<(usize, usize, PaletteItem)> = diff
            .doc
            .files
            .iter()
            .enumerate()
            .filter_map(|(i, f)| {
                let path = f.meta.path();
                fuzzy_score(input.trim(), path).map(|score| {
                    let (adds, dels) = diff.doc.file_counts(f);
                    let hint = if f.diff.is_some() {
                        format!("+{adds} −{dels}")
                    } else {
                        String::new()
                    };
                    (
                        score,
                        i,
                        PaletteItem {
                            label: path.to_owned(),
                            hint,
                        },
                    )
                })
            })
            .collect();
        items.sort_by_key(|(score, i, _)| (*score, *i));
        items.into_iter().map(|(_, i, item)| (i, item)).collect()
    }

    /// Re-runs the diff screen's clamping and prioritization.
    fn settle_diff(&mut self) -> Vec<Cmd> {
        let content = self.content_area();
        let State { screens, diffs, .. } = self;
        let Some(Screen::Diff(screen)) = screens.last_mut() else {
            return Vec::new();
        };
        match diffs.get_mut(&screen.pr) {
            Some(diff) => diff_screen::settle(screen, diff, content),
            None => Vec::new(),
        }
    }

    /// What's loading on the visible screen, for the status bar.
    pub fn busy(&self) -> Option<String> {
        match self.screen() {
            Screen::Inbox { .. } if self.inbox.loading => Some(if self.inbox.data.is_some() {
                "Refreshing".to_owned()
            } else {
                "Loading pull requests".to_owned()
            }),
            Screen::Pr { pr, .. } if self.prs.get(pr).is_some_and(|r| r.loading) => {
                Some(format!("Loading {pr}"))
            }
            Screen::Diff(screen) => self.diffs.get(&screen.pr).and_then(DiffState::status),
            _ => None,
        }
    }

    pub fn tabs(&self) -> Vec<String> {
        self.screens
            .iter()
            .map(|s| match s {
                Screen::Inbox { .. } => "Inbox".to_owned(),
                Screen::Pr { pr, .. } => pr.to_string(),
                Screen::Diff(_) => "Files".to_owned(),
            })
            .collect()
    }

    pub fn help_entries(&self) -> Vec<HelpEntry> {
        Action::ALL
            .into_iter()
            .filter_map(|action| {
                let keys = self.keymap.keys_for(action);
                (!keys.is_empty()).then(|| HelpEntry {
                    keys: keys.join("  "),
                    description: action.description(),
                })
            })
            .collect()
    }

    /// Palette entries matching the current input.
    pub fn palette_commands(&self, input: &str) -> Vec<(PaletteCommand, PaletteItem)> {
        let input = input.trim();
        let mut out = Vec::new();
        let pr = PrRef::parse(input).or_else(|| {
            // A bare number opens a PR in the repo currently on screen.
            let number: u64 = input.trim_start_matches('#').parse().ok()?;
            match self.screen() {
                Screen::Pr { pr, .. } => Some(PrRef {
                    repo: pr.repo.clone(),
                    number,
                }),
                Screen::Diff(screen) => Some(PrRef {
                    repo: screen.pr.repo.clone(),
                    number,
                }),
                Screen::Inbox { .. } => None,
            }
        });
        if let Some(pr) = pr {
            out.push((
                PaletteCommand::OpenPr(pr.clone()),
                PaletteItem {
                    label: format!("Open {pr}"),
                    hint: String::new(),
                },
            ));
        }
        let mut actions: Vec<(usize, Action)> = Action::ALL
            .into_iter()
            .filter(|a| *a != Action::CommandPalette)
            .filter_map(|a| {
                let by_name = fuzzy_score(input, &a.name().replace('_', " "));
                let by_description = fuzzy_score(input, a.description());
                by_name
                    .into_iter()
                    .chain(by_description)
                    .min()
                    .map(|score| (score, a))
            })
            .collect();
        actions.sort_by_key(|(score, _)| *score);
        for (_, action) in actions {
            out.push((
                PaletteCommand::Action(action),
                PaletteItem {
                    label: action.description().to_owned(),
                    hint: self.keymap.keys_for(action).join(" "),
                },
            ));
        }
        out
    }
}

/// Case-insensitive subsequence match; lower is better. Substring matches
/// rank before scattered ones, earlier before later.
fn fuzzy_score(needle: &str, haystack: &str) -> Option<usize> {
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

pub fn update(state: &mut State, msg: Msg) -> Vec<Cmd> {
    match msg {
        Msg::Key(key) => on_key(state, key),
        Msg::Resize(w, h) => {
            state.size = (w, h);
            clamp_scroll(state)
        }
        Msg::Viewer(Ok(login)) => {
            state.viewer = Some(login);
            Vec::new()
        }
        Msg::Viewer(Err(err)) => {
            tracing::warn!(%err, "could not fetch viewer");
            if matches!(err, ApiError::Unauthorized) {
                state.notice = Some(Notice::Error(err.to_string()));
            }
            Vec::new()
        }
        Msg::Inbox(result) => {
            if let Err(err) = &result {
                tracing::warn!(%err, "inbox fetch failed");
            }
            state.inbox.finish(result);
            clamp_scroll(state)
        }
        Msg::Pr(pr, result) => {
            if let Err(err) = result.as_ref() {
                tracing::warn!(%pr, %err, "PR fetch failed");
            }
            state.prs.entry(pr).or_default().finish(*result);
            clamp_scroll(state)
        }
        Msg::RateLimits(limits) => {
            state.rate_limits = limits;
            Vec::new()
        }
        Msg::Notice(notice) => {
            state.notice = Some(notice);
            Vec::new()
        }
        Msg::DiffProgress(pr, line) => {
            if let Some(diff) = state.diffs.get_mut(&pr)
                && !diff.listed
            {
                diff.progress = Some(line);
            }
            Vec::new()
        }
        Msg::DiffFiles(pr, files) => {
            let Some(diff) = state.diffs.get_mut(&pr) else {
                return Vec::new();
            };
            let DiffFiles {
                refs,
                files,
                generated,
            } = *files;
            diff.set_files(refs, Doc::new(files, &generated));
            state.settle_diff()
        }
        Msg::FileDiff(pr, index, file) => {
            if let Some(diff) = state.diffs.get_mut(&pr) {
                diff.set_file(index, file);
            }
            state.settle_diff()
        }
        Msg::DiffFailed(pr, error) => {
            if let Some(diff) = state.diffs.get_mut(&pr) {
                diff.progress = None;
                diff.error = Some(error);
            }
            Vec::new()
        }
        Msg::ViewedLoaded(pr, result) => {
            match *result {
                Ok(viewed) => {
                    if let Some(diff) = state.diffs.get_mut(&pr) {
                        diff.set_viewed_states(viewed);
                    }
                }
                Err(err) => {
                    tracing::warn!(%pr, %err, "viewed state fetch failed");
                    state.notice =
                        Some(Notice::Error(format!("Couldn't load viewed files: {err}")));
                }
            }
            state.settle_diff()
        }
        Msg::ViewedSaved {
            pr,
            file,
            previous,
            result,
        } => {
            if let Err(err) = result {
                tracing::warn!(%pr, %err, "viewed state update failed");
                if let Some(diff) = state.diffs.get_mut(&pr) {
                    diff.doc.set_viewed(file, previous);
                }
                state.notice = Some(Notice::Error(format!("GitHub didn't save “viewed”: {err}")));
            }
            state.settle_diff()
        }
        Msg::ReviewLoaded(pr, review) => {
            if let Some(diff) = state.diffs.get_mut(&pr) {
                diff.set_review(review);
            }
            Vec::new()
        }
    }
}

fn on_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        state.quit = true;
        return Vec::new();
    }
    match &mut state.overlay {
        Some(Overlay::Help) => {
            if matches!(
                key.code,
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q' | '?')
            ) {
                state.overlay = None;
            }
            return Vec::new();
        }
        Some(Overlay::Palette(_)) => return on_palette_key(state, key),
        Some(Overlay::Search(_)) => return on_search_key(state, key),
        Some(Overlay::FindFile(_)) => return on_finder_key(state, key),
        None => {}
    }
    // Any keypress dismisses the last notice.
    state.notice = None;
    state.pending.push(Key::from(key));
    match state.keymap.resolve(&state.pending) {
        Resolution::Action(action) => {
            state.pending.clear();
            apply(state, action)
        }
        Resolution::Pending => Vec::new(),
        Resolution::Unbound => {
            state.pending.clear();
            Vec::new()
        }
    }
}

fn on_palette_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    let Some(Overlay::Palette(palette)) = &mut state.overlay else {
        return Vec::new();
    };
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => {
            state.overlay = None;
            Vec::new()
        }
        KeyCode::Down | KeyCode::Tab => {
            palette.selected += 1;
            clamp_palette(state);
            Vec::new()
        }
        KeyCode::Char('n') if ctrl => {
            palette.selected += 1;
            clamp_palette(state);
            Vec::new()
        }
        KeyCode::Up | KeyCode::BackTab => {
            palette.selected = palette.selected.saturating_sub(1);
            Vec::new()
        }
        KeyCode::Char('p') if ctrl => {
            palette.selected = palette.selected.saturating_sub(1);
            Vec::new()
        }
        KeyCode::Enter => {
            let input = palette.input.lines().join("");
            let selected = palette.selected;
            let command = state
                .palette_commands(&input)
                .into_iter()
                .nth(selected)
                .map(|(c, _)| c);
            state.overlay = None;
            match command {
                Some(PaletteCommand::Action(action)) => apply(state, action),
                Some(PaletteCommand::OpenPr(pr)) => state.open_pr(pr),
                None => Vec::new(),
            }
        }
        _ => {
            palette.input.input(key);
            palette.selected = 0;
            Vec::new()
        }
    }
}

fn on_search_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    let Some(Overlay::Search(input)) = &mut state.overlay else {
        return Vec::new();
    };
    match key.code {
        KeyCode::Esc => {
            state.overlay = None;
            Vec::new()
        }
        KeyCode::Enter => {
            let query = input.lines().join("");
            state.overlay = None;
            if query.is_empty() {
                return Vec::new();
            }
            let content = state.content_area();
            let State {
                screens,
                diffs,
                notice,
                ..
            } = state;
            let Some(Screen::Diff(screen)) = screens.last_mut() else {
                return Vec::new();
            };
            let Some(diff) = diffs.get_mut(&screen.pr) else {
                return Vec::new();
            };
            let (cmds, found) = diff_screen::search(screen, diff, query, content);
            *notice = Some(found);
            cmds
        }
        _ => {
            input.input(key);
            Vec::new()
        }
    }
}

fn on_finder_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    let Some(Overlay::FindFile(finder)) = &mut state.overlay else {
        return Vec::new();
    };
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => state.overlay = None,
        KeyCode::Down | KeyCode::Tab => finder.selected += 1,
        KeyCode::Char('n') if ctrl => finder.selected += 1,
        KeyCode::Up | KeyCode::BackTab => finder.selected = finder.selected.saturating_sub(1),
        KeyCode::Char('p') if ctrl => finder.selected = finder.selected.saturating_sub(1),
        KeyCode::Enter => {
            let input = finder.input.lines().join("");
            let selected = finder.selected;
            let target = state.finder_items(&input).get(selected).map(|(i, _)| *i);
            state.overlay = None;
            if let Some(file) = target
                && let Screen::Diff(screen) = state.screen_mut()
            {
                screen.cursor = Pos { file, row: 0 };
                screen.top = screen.cursor;
                return state.settle_diff();
            }
            return Vec::new();
        }
        _ => {
            finder.input.input(key);
            finder.selected = 0;
        }
    }
    // Keep the selection on an existing item.
    let count = match &state.overlay {
        Some(Overlay::FindFile(finder)) => state.finder_items(&finder.input.lines().join("")).len(),
        _ => 0,
    };
    if let Some(Overlay::FindFile(finder)) = &mut state.overlay {
        finder.selected = finder.selected.min(count.saturating_sub(1));
    }
    Vec::new()
}

pub fn new_search_input(theme: &Theme) -> TextArea<'static> {
    let mut input = TextArea::default();
    input.set_style(theme.body(Bg::Container));
    input.set_cursor_line_style(theme.body(Bg::Container));
    input.set_cursor_style(theme.fill(Bg::Primary));
    input
}

fn clamp_palette(state: &mut State) {
    let Some(Overlay::Palette(palette)) = &state.overlay else {
        return;
    };
    let input = palette.input.lines().join("");
    let count = state.palette_commands(&input).len();
    if let Some(Overlay::Palette(palette)) = &mut state.overlay {
        palette.selected = palette.selected.min(count.saturating_sub(1));
    }
}

pub fn new_palette(theme: &Theme) -> Palette {
    let mut input = TextArea::default();
    input.set_style(theme.body(Bg::ContainerHigh));
    input.set_cursor_line_style(theme.body(Bg::ContainerHigh));
    input.set_cursor_style(theme.fill(Bg::Primary));
    input.set_placeholder_text("Type a command or owner/repo#123");
    input.set_placeholder_style(theme.meta(Bg::ContainerHigh));
    Palette { input, selected: 0 }
}

fn apply(state: &mut State, action: Action) -> Vec<Cmd> {
    let half_page = (state.content_area().height / 2).max(1);
    let content = state.content_area();
    {
        let State {
            screens,
            diffs,
            notice,
            ..
        } = &mut *state;
        if let Some(Screen::Diff(screen)) = screens.last_mut()
            && let Some(diff) = diffs.get_mut(&screen.pr)
            && let Some(cmds) = diff_screen::apply(screen, diff, action, content, notice)
        {
            return cmds;
        }
    }
    match action {
        Action::Quit => state.quit = true,
        Action::Close => {
            if state.screens.len() > 1 {
                state.screens.pop();
                return state.load_visible(false);
            }
            state.quit = true;
        }
        Action::Back => {
            if state.screens.len() > 1 {
                state.screens.pop();
                return state.load_visible(false);
            }
        }
        Action::Help => state.overlay = Some(Overlay::Help),
        Action::CommandPalette => {
            state.overlay = Some(Overlay::Palette(Box::new(new_palette(&state.theme))));
        }
        Action::Refresh => return state.load_visible(true),
        Action::OpenInBrowser => {
            if let Some(pr) = current_pr(state) {
                state.notice = Some(Notice::Info(format!("Opened {pr} in the browser")));
                let url = match state.screen() {
                    Screen::Diff(_) => format!("{}/files", pr.url()),
                    _ => pr.url(),
                };
                return vec![Cmd::OpenUrl(url)];
            }
        }
        Action::Open => match state.screen().clone() {
            Screen::Inbox { .. } => {
                if let Some(pr) = current_pr(state) {
                    return state.open_pr(pr);
                }
            }
            Screen::Pr { pr, .. } => {
                if state.prs.get(&pr).is_some_and(|r| r.data.is_some()) {
                    return state.open_diff(pr);
                }
            }
            Screen::Diff(_) => {}
        },
        Action::Search => {
            if let Screen::Diff(_) = state.screen() {
                state.overlay = Some(Overlay::Search(Box::new(new_search_input(&state.theme))));
            }
        }
        Action::FindFile => {
            if let Screen::Diff(_) = state.screen() {
                let mut finder = new_palette(&state.theme);
                finder.input.set_placeholder_text("File path");
                state.overlay = Some(Overlay::FindFile(Box::new(finder)));
            }
        }
        // Diff-only actions elsewhere do nothing.
        Action::NextHunk
        | Action::PrevHunk
        | Action::NextFile
        | Action::PrevFile
        | Action::ToggleTree
        | Action::SwitchPane
        | Action::ToggleSplit
        | Action::IgnoreWhitespace
        | Action::ExpandContext
        | Action::FullFile
        | Action::ToggleViewed
        | Action::NextUnviewed
        | Action::MarkReviewed
        | Action::SearchNext
        | Action::SearchPrev => {}
        Action::Down => move_by(state, 1, 1),
        Action::Up => move_by(state, -1, -1),
        Action::HalfPageDown => move_by(state, i64::from(half_page / 2), i64::from(half_page)),
        Action::HalfPageUp => move_by(state, -i64::from(half_page / 2), -i64::from(half_page)),
        Action::Top => move_by(state, i64::MIN / 2, i64::MIN / 2),
        Action::Bottom => move_by(state, i64::MAX / 2, i64::MAX / 2),
    }
    Vec::new()
}

/// The PR under the cursor (inbox) or on screen.
fn current_pr(state: &State) -> Option<PrRef> {
    match state.screen() {
        Screen::Inbox { selected, .. } => {
            let inbox = state.inbox.data.as_ref()?;
            pr_list::flat_prs(inbox)
                .get(*selected)
                .map(|p| p.pr.clone())
        }
        Screen::Pr { pr, .. } => Some(pr.clone()),
        Screen::Diff(screen) => Some(screen.pr.clone()),
    }
}

/// Moves the inbox selection by `rows` or scrolls the PR view by `lines`.
fn move_by(state: &mut State, rows: i64, lines: i64) {
    match state.screen_mut() {
        Screen::Inbox { selected, .. } => {
            *selected = (*selected as i64).saturating_add(rows).max(0) as usize;
        }
        Screen::Pr { scroll, .. } => {
            *scroll = (i64::from(*scroll))
                .saturating_add(lines)
                .clamp(0, i64::from(u16::MAX)) as u16;
        }
        Screen::Diff(_) => {}
    }
    clamp_scroll(state);
}

/// Keeps the selection in range and visible, and scrolling in bounds.
fn clamp_scroll(state: &mut State) -> Vec<Cmd> {
    let viewport = state.list_viewport();
    let content = state.content_area();
    match state.screen().clone() {
        Screen::Inbox { selected, scroll } => {
            let (selected, scroll) = match &state.inbox.data {
                Some(inbox) => {
                    let count = pr_list::flat_prs(inbox).len();
                    let selected = selected.min(count.saturating_sub(1));
                    let rows = pr_list::rows(inbox);
                    (
                        selected,
                        pr_list::scroll_to_selection(&rows, selected, scroll, viewport),
                    )
                }
                None => (0, 0),
            };
            *state.screen_mut() = Screen::Inbox { selected, scroll };
        }
        Screen::Pr { pr, scroll } => {
            let remote = state.prs.get(&pr);
            let view = PrView {
                ctx: state.ctx(0),
                pr: &pr,
                detail: remote.and_then(|r| r.data.as_ref()),
                loading: remote.is_some_and(|r| r.loading),
                error: remote.and_then(|r| r.error.as_deref()),
                scroll,
                bg: Bg::ContainerLow,
                diff_key: "",
                browser_key: "",
            };
            let scroll = scroll.min(view.max_scroll(content));
            *state.screen_mut() = Screen::Pr { pr, scroll };
        }
        Screen::Diff(_) => return state.settle_diff(),
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghtui_api::model::{PrState, PrSummary};
    use ghtui_theme::{ColorDepth, DEFAULT_SEED, Mode};
    use ghtui_ui::overlays::PALETTE_ROWS;

    fn state() -> State {
        State::new(
            Theme::new(DEFAULT_SEED, Mode::Dark, ColorDepth::TrueColor),
            Icons::default(),
            Keymap::default(),
            (100, 30),
        )
    }

    fn summary(n: u64) -> PrSummary {
        PrSummary {
            pr: PrRef::parse(&format!("o/r#{n}")).unwrap(),
            title: format!("PR {n}"),
            author: "a".into(),
            state: PrState::Open,
            updated_at: "2026-01-01T00:00:00Z".into(),
            additions: 0,
            deletions: 0,
            comments: 0,
            review: None,
            checks: None,
        }
    }

    fn with_inbox(n: u64) -> State {
        let mut state = state();
        update(
            &mut state,
            Msg::Inbox(Ok(Inbox {
                review_requested: Vec::new(),
                authored: (1..=n).map(summary).collect(),
                ..Inbox::default()
            })),
        );
        state
    }

    fn press(state: &mut State, keys: &str) -> Vec<Cmd> {
        let mut cmds = Vec::new();
        for key in crate::keymap::parse_sequence(keys).unwrap() {
            cmds.extend(update(state, Msg::Key(KeyEvent::new(key.code, key.mods))));
        }
        cmds
    }

    fn selected(state: &State) -> usize {
        match state.screen() {
            Screen::Inbox { selected, .. } => *selected,
            _ => panic!("not on inbox"),
        }
    }

    #[test]
    fn navigates_the_inbox() {
        let mut state = with_inbox(5);
        press(&mut state, "jj");
        assert_eq!(selected(&state), 2);
        press(&mut state, "G");
        assert_eq!(selected(&state), 4);
        press(&mut state, "j");
        assert_eq!(selected(&state), 4, "clamped at the end");
        press(&mut state, "gg");
        assert_eq!(selected(&state), 0);
        press(&mut state, "k");
        assert_eq!(selected(&state), 0);
    }

    #[test]
    fn opening_a_pr_fetches_it_and_back_returns() {
        let mut state = with_inbox(3);
        press(&mut state, "j");
        let cmds = press(&mut state, "<Enter>");
        let pr = PrRef::parse("o/r#2").unwrap();
        assert_eq!(cmds, vec![Cmd::FetchPr(pr.clone())]);
        assert_eq!(state.tabs(), ["Inbox", "o/r#2"]);
        assert_eq!(state.busy().as_deref(), Some("Loading o/r#2"));

        update(
            &mut state,
            Msg::Pr(pr.clone(), Box::new(Err(ApiError::Network("down".into())))),
        );
        assert_eq!(state.prs[&pr].error.as_deref(), Some("network error: down"));
        assert_eq!(
            press(&mut state, "r"),
            vec![Cmd::FetchViewer, Cmd::FetchPr(pr)]
        );

        press(&mut state, "<Esc>");
        assert_eq!(state.tabs(), ["Inbox"]);
        assert_eq!(selected(&state), 1, "selection survives");
        assert!(!state.quit);
        press(&mut state, "q");
        assert!(state.quit);
    }

    #[test]
    fn duplicate_fetches_are_suppressed() {
        let mut state = state();
        assert_eq!(state.load_visible(false), vec![Cmd::FetchInbox]);
        assert_eq!(state.load_visible(true), vec![Cmd::FetchViewer]);
    }

    #[test]
    fn failed_refresh_keeps_cached_data() {
        let mut state = with_inbox(2);
        state.load_visible(true);
        update(&mut state, Msg::Inbox(Err(ApiError::RateLimited(30))));
        assert_eq!(state.inbox.data.as_ref().unwrap().authored.len(), 2);
        assert!(
            state
                .inbox
                .error
                .as_deref()
                .unwrap()
                .contains("rate limited")
        );
    }

    #[test]
    fn opens_in_browser() {
        let mut state = with_inbox(1);
        assert_eq!(
            press(&mut state, "o"),
            vec![Cmd::OpenUrl("https://github.com/o/r/pull/1".into())]
        );
    }

    #[test]
    fn palette_opens_prs_and_runs_actions() {
        let mut state = with_inbox(1);
        press(&mut state, ":");
        assert!(matches!(state.overlay, Some(Overlay::Palette(_))));
        let cmds = press(&mut state, "a/b#9<Enter>");
        assert_eq!(cmds, vec![Cmd::FetchPr(PrRef::parse("a/b#9").unwrap())]);
        assert!(state.overlay.is_none());

        // A bare number resolves against the repo on screen.
        press(&mut state, ":12<Enter>");
        assert_eq!(state.tabs().last().unwrap(), "a/b#12");

        press(&mut state, ":help<Enter>");
        assert!(matches!(state.overlay, Some(Overlay::Help)));
        press(&mut state, "<Esc>");
        assert!(state.overlay.is_none());
    }

    #[test]
    fn palette_fuzzy_matches() {
        let state = state();
        let labels: Vec<String> = state
            .palette_commands("brows")
            .into_iter()
            .map(|(_, item)| item.label)
            .collect();
        assert_eq!(labels.first().map(String::as_str), Some("Open on GitHub"));
        assert!(fuzzy_score("xyz", "Refresh").is_none());
        assert!(fuzzy_score("rfr", "Refresh").is_some());
    }

    #[test]
    fn pending_keys_show_and_clear() {
        let mut state = with_inbox(1);
        press(&mut state, "g");
        assert_eq!(state.pending.len(), 1);
        press(&mut state, "x");
        assert!(state.pending.is_empty());
    }

    #[test]
    fn pr_scroll_is_bounded() {
        let mut state = state();
        let pr = PrRef::parse("o/r#1").unwrap();
        state.open_pr(pr.clone());
        press(&mut state, "<C-d><C-d><C-d>");
        let Screen::Pr { scroll, .. } = state.screen() else {
            panic!()
        };
        assert_eq!(*scroll, 0, "nothing to scroll while loading");
    }

    #[test]
    fn palette_list_stays_in_bounds() {
        let mut state = state();
        press(&mut state, ":");
        for _ in 0..(PALETTE_ROWS * 3) {
            press(&mut state, "<Down>");
        }
        let Some(Overlay::Palette(palette)) = &state.overlay else {
            panic!()
        };
        assert_eq!(palette.selected, state.palette_commands("").len() - 1);
    }

    mod diff {
        use super::*;
        use crate::diff_screen::DiffScreen;
        use ghtui_api::model::{ViewedFiles, ViewedState};
        use ghtui_ui::diff_doc::Viewed;

        /// A diff screen on o/r#7 built from the snapshot fixture.
        fn diff_state(width: u16) -> (State, PrRef) {
            let mut s = state();
            s.size = (width, 40);
            let pr = PrRef::parse("o/r#7").unwrap();
            s.diffs
                .insert(pr.clone(), crate::snapshot_tests::diff_fixture());
            s.screens
                .push(Screen::Diff(Box::new(DiffScreen::new(pr.clone(), width))));
            let _ = s.settle_diff();
            (s, pr)
        }

        fn screen(s: &State) -> &DiffScreen {
            match s.screen() {
                Screen::Diff(screen) => screen,
                _ => panic!("not on the diff"),
            }
        }

        fn viewed_files(pr_id: &str) -> ViewedFiles {
            ViewedFiles {
                pull_request_id: pr_id.into(),
                states: [("src/point.rs".to_owned(), ViewedState::Viewed)].into(),
            }
        }

        #[test]
        fn split_is_automatic_by_width_and_toggles() {
            let (mut narrow, _) = diff_state(120);
            assert!(!narrow.diffs.values().next().unwrap().doc.opts.split);
            press(&mut narrow, "s");
            assert!(narrow.diffs.values().next().unwrap().doc.opts.split);
            let (wide, _) = diff_state(220);
            assert!(wide.diffs.values().next().unwrap().doc.opts.split);
        }

        #[test]
        fn viewed_state_applies_and_rolls_back_on_failure() {
            let (mut s, pr) = diff_state(120);
            // Before GitHub's state arrives, toggling explains why not.
            assert!(press(&mut s, "v").is_empty());
            assert!(matches!(s.notice, Some(Notice::Error(_))));

            update(
                &mut s,
                Msg::ViewedLoaded(pr.clone(), Box::new(Ok(viewed_files("PR_1")))),
            );
            assert_eq!(s.diffs[&pr].doc.files[0].viewed, Viewed::Viewed);
            assert!(
                s.diffs[&pr].doc.files[0].collapsed(),
                "viewed files collapse"
            );

            // ]u skips the viewed file.
            press(&mut s, "gg]u");
            assert_eq!(screen(&s).cursor.file, 1);

            // Toggle file 1 viewed: optimistic, then GitHub refuses.
            let cmds = press(&mut s, "v");
            assert!(cmds.iter().any(|c| matches!(
                c,
                Cmd::SetViewed { file: 1, viewed: true, previous: Viewed::Unviewed, pull_request_id, .. }
                    if pull_request_id == "PR_1"
            )));
            assert_eq!(s.diffs[&pr].doc.files[1].viewed, Viewed::Viewed);
            update(
                &mut s,
                Msg::ViewedSaved {
                    pr: pr.clone(),
                    file: 1,
                    previous: Viewed::Unviewed,
                    result: Err(ApiError::Network("offline".into())),
                },
            );
            assert_eq!(s.diffs[&pr].doc.files[1].viewed, Viewed::Unviewed);
            assert!(matches!(s.notice, Some(Notice::Error(_))));
        }

        #[test]
        fn review_marks_toggle_and_persist() {
            let (mut s, pr) = diff_state(120);
            // Off a changed line: nothing to mark.
            assert!(press(&mut s, "gg").is_empty());
            assert!(press(&mut s, "m").is_empty());
            // Onto the first change.
            press(&mut s, "]hjjj");
            let cmds = press(&mut s, "m");
            let Some(Cmd::SaveReview(saved_pr, review)) = cmds.first() else {
                panic!("{cmds:?}")
            };
            assert_eq!(saved_pr, &pr);
            assert_eq!(review.reviewed_hunks.len(), 1);
            assert_eq!(s.diffs[&pr].doc.reviewed.len(), 1);
            let cmds = press(&mut s, "m");
            let Some(Cmd::SaveReview(_, review)) = cmds.first() else {
                panic!()
            };
            assert!(review.reviewed_hunks.is_empty());
        }

        #[test]
        fn search_and_repeat() {
            let (mut s, pr) = diff_state(120);
            press(&mut s, "/");
            assert!(matches!(s.overlay, Some(Overlay::Search(_))));
            press(&mut s, "origin<Enter>");
            assert!(s.overlay.is_none());
            let cursor = screen(&s).cursor;
            assert!(s.diffs[&pr].doc.row_text(cursor).contains("origin"));
            assert!(matches!(s.notice, Some(Notice::Info(ref m)) if m.contains("1 line")));
            press(&mut s, "n");
            assert_eq!(screen(&s).cursor, cursor, "single match wraps to itself");
            press(&mut s, "/");
            press(&mut s, "zzzz<Enter>");
            assert!(matches!(s.notice, Some(Notice::Error(_))));
        }

        #[test]
        fn file_finder_jumps_to_files() {
            let (mut s, _) = diff_state(120);
            press(&mut s, "gf");
            assert!(matches!(s.overlay, Some(Overlay::FindFile(_))));
            assert_eq!(s.finder_items("gone")[0].1.label, "gone.py");
            press(&mut s, "gone<Enter>");
            assert!(s.overlay.is_none());
            assert_eq!(screen(&s).cursor.file, 3);
            assert_eq!(screen(&s).cursor.row, 0);
        }

        #[test]
        fn whitespace_and_full_file_keep_the_cursor_line() {
            let (mut s, pr) = diff_state(120);
            press(&mut s, "]hjjj");
            let text = s.diffs[&pr].doc.row_text(screen(&s).cursor);
            press(&mut s, "F");
            assert!(s.diffs[&pr].doc.files[0].full);
            assert_eq!(s.diffs[&pr].doc.row_text(screen(&s).cursor), text);
            press(&mut s, "w");
            assert_eq!(
                s.diffs[&pr].doc.opts.whitespace,
                ghtui_diff::Whitespace::Ignore
            );
            press(&mut s, "x");
        }
    }
}
