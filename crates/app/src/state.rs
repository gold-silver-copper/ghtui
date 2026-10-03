//! Application state and the single update function. Input events and async
//! results arrive as [`Msg`]s; [`update`] mutates [`State`] and returns
//! [`Cmd`]s for the runtime to execute in the background.

use std::collections::HashMap;
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ghtui_api::ApiError;
use ghtui_api::model::{Inbox, PrDetail, PrRef, RepoId, ViewedFiles};
use ghtui_api::rate_limit::RateLimits;
use ghtui_diff::FileDiff;
use ghtui_store::ReviewState;
use ghtui_theme::{Bg, Theme};
use ghtui_ui::bars::Notice;
use ghtui_ui::diff_doc::{Doc, Pos, Viewed};
use ghtui_ui::overlays::{HelpEntry, PaletteItem};
use ghtui_ui::pages::{self, PrTab};
use ghtui_ui::{Ctx, Icons, PAD_X, PAD_Y};
use ratatui::layout::Rect;
use ratatui_textarea::TextArea;

use crate::browse::{self, Data, DataKey, Need, PageScreen};
use crate::diff_job::DiffFiles;
use crate::diff_screen::{self, DiffScreen, DiffState};
use crate::keymap::{Action, Key, Keymap, Resolution, Scope};
use crate::review::{
    self, Compose, ComposeTarget, EditPurpose, Preview, SubmitDialog, SubmitOutcome,
};
use crate::route::{self, OPEN, Route, Target};
use ghtui_api::model::{PatchFile, ReviewEvent, ReviewThread};
use ghtui_store::DraftComment;
use ghtui_ui::annotations::AnnotationKey;

#[derive(Debug)]
pub enum Msg {
    Key(KeyEvent),
    Resize(u16, u16),
    Viewer(Result<String, ApiError>),
    Inbox(Result<Inbox, ApiError>),
    Pr(PrRef, Box<Result<PrDetail, ApiError>>),
    /// Page data; `fresh: false` is the cached copy shown while fetching.
    Fetched {
        key: DataKey,
        result: Result<Data, ApiError>,
        fresh: bool,
    },
    /// The next page of a list.
    FetchedMore(DataKey, Result<Data, ApiError>),
    Commented(DataKey, Result<(), ApiError>),
    Starred {
        repo: RepoId,
        starred: bool,
        result: Result<(), ApiError>,
    },
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
    ThreadsLoaded(PrRef, Result<Vec<ReviewThread>, ApiError>),
    PatchesLoaded(PrRef, Result<Vec<PatchFile>, ApiError>),
    OutdatedMapped(PrRef, Vec<(String, Option<u32>)>),
    Replied(PrRef, Result<(), ApiError>),
    ResolvedSet {
        pr: PrRef,
        thread_id: String,
        resolved: bool,
        result: Result<(), ApiError>,
    },
    ReviewSubmitted(PrRef, SubmitOutcome),
    /// `$EDITOR` finished (or failed to start).
    Edited(EditPurpose, Result<String, String>),
    MovesDetected(PrRef, Vec<ghtui_diff::moves::Move>),
    LastReview(PrRef, Result<Option<String>, ApiError>),
    SinceReady(
        PrRef,
        String,
        Result<std::collections::HashSet<String>, String>,
    ),
    CommitsListed(PrRef, Result<Vec<(String, String)>, String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cmd {
    FetchViewer,
    FetchInbox,
    FetchPr(PrRef),
    /// Fetch page data; with `cached`, first send what the cache has.
    Fetch {
        key: DataKey,
        cached: bool,
    },
    /// The next page of a search, after the cursor.
    FetchMore {
        key: DataKey,
        after: String,
    },
    /// Comment on an issue or pull request, then refresh `refresh`.
    AddComment {
        subject_id: String,
        body: String,
        refresh: DataKey,
    },
    SetStarred {
        repo: RepoId,
        id: String,
        starred: bool,
    },
    OpenUrl(String),
    /// Start (or restart) the diff job for a PR.
    /// With `range`, diff `(from, to)` instead of the whole PR.
    LoadDiff {
        pr: PrRef,
        base_ref: String,
        range: Option<(String, String)>,
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
    FetchThreads(PrRef),
    FetchPatches(PrRef),
    MapOutdated {
        pr: PrRef,
        head: String,
        /// `(thread, path, original commit, original line)`.
        items: Vec<(String, String, String, u32)>,
    },
    Reply {
        pr: PrRef,
        thread_id: String,
        body: String,
    },
    SetResolved {
        pr: PrRef,
        thread_id: String,
        resolved: bool,
    },
    SubmitReview {
        pr: PrRef,
        head: String,
        drafts: Vec<DraftComment>,
        event: ReviewEvent,
        body: String,
    },
    /// Suspend the TUI and edit `text` in `$EDITOR`.
    Edit {
        purpose: EditPurpose,
        text: String,
    },
    DetectMoves(PrRef, Vec<(usize, Arc<FileDiff>)>),
    FetchLastReview {
        pr: PrRef,
        login: String,
    },
    /// Block hashes of the diff at `old_head`, for "since my last review".
    SinceReview {
        pr: PrRef,
        old_head: String,
    },
    ListCommits(PrRef),
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
    /// A GitHub page: home, repository, file, list, issue, PR, profile,
    /// search.
    Page(Box<PageScreen>),
    /// A pull request's files.
    Diff(Box<DiffScreen>),
}

pub enum Overlay {
    Help,
    Palette(Box<Palette>),
    /// `/` prompt on the diff screen.
    Search(Box<TextArea<'static>>),
    /// A one-line prompt on a page (GitHub search, list filter).
    Prompt(Box<Prompt>),
    /// Choosing among the links on a line.
    Links(Box<LinkPicker>),
    /// `gf` file finder on the diff screen.
    FindFile(Box<Palette>),
    /// Writing a comment, reply or suggestion.
    Compose(Box<Compose>),
    /// Submitting the review.
    Submit(Box<SubmitDialog>),
    /// Choosing commits to view.
    Commits(Box<CommitPicker>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    Search,
    Filter,
}

pub struct Prompt {
    pub kind: PromptKind,
    pub input: TextArea<'static>,
}

impl Prompt {
    pub fn label(&self) -> &'static str {
        match self.kind {
            PromptKind::Search => "Search GitHub ",
            PromptKind::Filter => "Filter ",
        }
    }
}

pub struct LinkPicker {
    /// `(url, item)`.
    pub items: Vec<(String, PaletteItem)>,
    pub selected: usize,
    /// Shown as the picker's (read-only) prompt line.
    pub input: TextArea<'static>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickItem {
    All,
    SinceReview,
    Commit(usize),
}

pub struct CommitPicker {
    pub items: Vec<(PickItem, PaletteItem)>,
    pub selected: usize,
    /// Start of a range (index into `items`).
    pub mark: Option<usize>,
    /// Shown as the picker's (read-only) prompt line.
    pub input: TextArea<'static>,
}

pub struct Palette {
    pub input: TextArea<'static>,
    pub selected: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteCommand {
    Action(Action),
    Go(Target),
    Search(String),
}

pub struct State {
    /// Navigation history, like a browser's; never empty.
    pub screens: Vec<Screen>,
    pub inbox: Remote<Inbox>,
    pub prs: HashMap<PrRef, Remote<PrDetail>>,
    /// Everything else pages show.
    pub data: HashMap<DataKey, Remote<Data>>,
    /// Bumped whenever data changes, so pages know to rebuild.
    pub data_gen: u64,
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
    /// Unix seconds; pages show relative times.
    pub clock: fn() -> u64,
    pub quit: bool,
}

impl State {
    pub fn new(theme: Theme, icons: Icons, keymap: Keymap, size: (u16, u16)) -> Self {
        let mut state = Self {
            screens: vec![Screen::Page(Box::new(PageScreen::new(Route::Home)))],
            inbox: Remote::default(),
            prs: HashMap::new(),
            data: HashMap::new(),
            data_gen: 0,
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
            clock: ghtui_store::now,
            quit: false,
        };
        state.sync_page();
        state
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

    /// Which keymap applies.
    pub fn scope(&self) -> Scope {
        match self.screen() {
            Screen::Page(_) => Scope::Page,
            Screen::Diff(_) => Scope::Diff,
        }
    }

    /// The page on screen, if it is one.
    pub fn route(&self) -> Option<&Route> {
        match self.screen() {
            Screen::Page(p) => Some(&p.route),
            Screen::Diff(_) => None,
        }
    }

    /// The repository on screen.
    fn context_repo(&self) -> Option<&RepoId> {
        match self.screen() {
            Screen::Page(p) => p.route.repo(),
            Screen::Diff(d) => Some(&d.pr.repo),
        }
    }

    /// Columns pages wrap to.
    pub fn page_width(&self) -> u16 {
        self.size.0.saturating_sub(2 * PAD_X).min(browse::MAX_WIDTH)
    }

    /// Rows a page shows.
    pub fn page_height(&self) -> usize {
        usize::from(self.content_area().height.saturating_sub(2 * PAD_Y)).max(1)
    }

    /// Rebuilds the page on screen if its data or the width changed.
    pub fn sync_page(&mut self) {
        let width = self.page_width();
        let generation = self.data_gen;
        let route = match self.screen() {
            Screen::Page(p) if p.built != Some((generation, width)) => p.route.clone(),
            _ => return,
        };
        let page = self.build_page(&route, width, (self.clock)());
        let height = self.page_height();
        if let Screen::Page(p) = self.screen_mut() {
            p.page = Arc::new(page);
            p.built = Some((generation, width));
            clamp_page(p, height);
        }
    }

    /// Navigates to a page, keeping the current one in history.
    pub fn push(&mut self, route: Route) -> Vec<Cmd> {
        self.screens
            .push(Screen::Page(Box::new(PageScreen::new(route.clone()))));
        self.ensure_route(&route, true)
    }

    /// Replaces the page on screen (switching tabs, changing a filter).
    fn replace(&mut self, route: Route, force: bool) -> Vec<Cmd> {
        match self.screen_mut() {
            Screen::Page(p) => **p = PageScreen::new(route.clone()),
            Screen::Diff(_) => return self.push(route),
        }
        self.ensure_route(&route, force)
    }

    /// Follows a link.
    pub fn go(&mut self, target: Target) -> Vec<Cmd> {
        match target {
            Target::Page(route) => self.push(route),
            Target::Files(pr) => {
                let mut cmds = self.ensure_pr(&pr, false);
                cmds.extend(self.open_diff(pr));
                cmds
            }
            Target::External(url) => {
                self.notice = Some(Notice::Info(format!("Opened {url} in the browser")));
                vec![Cmd::OpenUrl(url)]
            }
        }
    }

    fn follow(&mut self, url: &str) -> Vec<Cmd> {
        if url == pages::MORE {
            return self.load_more();
        }
        self.go(Target::from_url(url))
    }

    fn load_more(&mut self) -> Vec<Cmd> {
        let Some((kind, query)) = self.route().and_then(Route::search) else {
            return Vec::new();
        };
        let key = DataKey::Search(kind, query);
        let Some(remote) = self.data.get_mut(&key) else {
            return Vec::new();
        };
        let after = match &remote.data {
            Some(Data::Search(results)) if !remote.loading => browse::next_cursor(results),
            _ => None,
        };
        let Some(after) = after.map(str::to_owned) else {
            return Vec::new();
        };
        remote.start();
        vec![Cmd::FetchMore { key, after }]
    }

    /// Fetches a page's data. The repository header shared by a repo's
    /// pages is only fetched when missing.
    fn ensure_route(&mut self, route: &Route, force: bool) -> Vec<Cmd> {
        let mut cmds = Vec::new();
        for need in browse::needs(route) {
            let header =
                matches!(need, Need::Data(DataKey::Repo(_))) && !matches!(route, Route::Repo(_));
            cmds.extend(self.ensure(need, force && !header));
        }
        cmds
    }

    fn ensure(&mut self, need: Need, force: bool) -> Vec<Cmd> {
        match need {
            Need::Inbox => self.ensure_inbox(force),
            Need::Pr(pr) => self.ensure_pr(&pr, force),
            Need::Data(key) => {
                let remote = self.data.entry(key.clone()).or_default();
                if remote.loading || (!force && remote.data.is_some() && remote.error.is_none()) {
                    return Vec::new();
                }
                remote.start();
                vec![Cmd::Fetch {
                    cached: remote.data.is_none(),
                    key,
                }]
            }
        }
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
            Screen::Page(p) => cmds.extend(self.ensure_route(&p.route, force)),
            Screen::Diff(screen) => {
                cmds.extend(self.ensure_pr(&screen.pr, false));
                if force || !self.diffs.contains_key(&screen.pr) {
                    cmds.extend(self.start_diff(&screen.pr));
                }
            }
        }
        cmds
    }

    /// Opens the diff of a PR (it starts once the PR's metadata is in).
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
                range: None,
            },
            Cmd::FetchViewed(pr.clone()),
            Cmd::LoadReview(pr.clone()),
            Cmd::FetchThreads(pr.clone()),
            Cmd::FetchPatches(pr.clone()),
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
            Screen::Page(p) if self.page_loading(&p.route) => Some("Loading".to_owned()),
            Screen::Page(_) => None,
            Screen::Diff(screen) => match self.diffs.get(&screen.pr) {
                Some(diff) => diff.status(),
                None => Some(format!("Loading {}", screen.pr)),
            },
        }
    }

    /// Top-bar titles of the latest screens (older ones fold into "…").
    pub fn tabs(&self) -> Vec<String> {
        const SHOWN: usize = 4;
        let skip = self.screens.len().saturating_sub(SHOWN);
        let mut tabs: Vec<String> = self.screens[skip..]
            .iter()
            .map(|s| match s {
                Screen::Page(p) => p.route.title(),
                Screen::Diff(screen) => {
                    let diff = self.diffs.get(&screen.pr);
                    match diff.and_then(|d| d.range.as_ref()) {
                        Some(range) => format!("Files · {}", range.label),
                        None if diff.is_some_and(|d| d.doc.since_active) => {
                            "Files · since your review".to_owned()
                        }
                        None => "Files".to_owned(),
                    }
                }
            })
            .collect();
        if skip > 0 {
            tabs.insert(0, "…".to_owned());
        }
        tabs
    }

    pub fn help_entries(&self) -> Vec<HelpEntry> {
        let scope = self.scope();
        Action::ALL
            .into_iter()
            .filter(|a| a.scope() == Scope::Global || a.scope() == scope)
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
        if let Some(target) = route::parse_input(input, self.context_repo()) {
            let label = match &target {
                Target::Page(route) => format!("Go to {}", route.title()),
                Target::Files(pr) => format!("Files changed in {pr}"),
                Target::External(url) => format!("Open {url}"),
            };
            out.push((
                PaletteCommand::Go(target),
                PaletteItem {
                    label,
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
        let search = (!input.is_empty()).then(|| {
            (
                PaletteCommand::Search(input.to_owned()),
                PaletteItem {
                    label: format!("Search GitHub for “{input}”"),
                    hint: String::new(),
                },
            )
        });
        // Good action matches first, then searching; scattered matches after.
        let (close, far): (Vec<_>, Vec<_>) = actions.into_iter().partition(|(s, _)| *s < 1000);
        let item = |action: Action| {
            (
                PaletteCommand::Action(action),
                PaletteItem {
                    label: action.description().to_owned(),
                    hint: self.keymap.keys_for(action).join(" "),
                },
            )
        };
        out.extend(close.into_iter().map(|(_, a)| item(a)));
        out.extend(search);
        out.extend(far.into_iter().map(|(_, a)| item(a)));
        out
    }
}

/// Keeps a page's cursor on the page and in view.
fn clamp_page(p: &mut PageScreen, height: usize) {
    let total = p.page.lines.len();
    p.cursor = p.cursor.min(total.saturating_sub(1));
    p.scroll = ghtui_ui::page::scroll_to(p.cursor, p.scroll, height, total);
}

/// Moves a page's cursor by `delta` lines, skipping blank lines.
fn move_cursor(p: &mut PageScreen, delta: i64, height: usize) {
    let lines = &p.page.lines;
    if lines.is_empty() {
        return;
    }
    let last = lines.len() as i64 - 1;
    let step = if delta >= 0 { 1 } else { -1 };
    let at = (p.cursor as i64).saturating_add(delta).clamp(0, last);
    // The nearest non-blank line onward, else back toward the cursor.
    let blank = |i: i64| lines[i as usize].is_blank();
    let mut ahead = at;
    while blank(ahead) && (0..=last).contains(&(ahead + step)) {
        ahead += step;
    }
    let mut behind = at;
    while blank(behind) && (0..=last).contains(&(behind - step)) {
        behind -= step;
    }
    if !blank(ahead) {
        p.cursor = ahead as usize;
    } else if !blank(behind) {
        p.cursor = behind as usize;
    }
    clamp_page(p, height);
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
    if !matches!(msg, Msg::Key(_)) {
        state.data_gen += 1;
    }
    let cmds = handle(state, msg);
    state.sync_page();
    cmds
}

fn handle(state: &mut State, msg: Msg) -> Vec<Cmd> {
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
            state.prs.entry(pr.clone()).or_default().finish(*result);
            // The diff was opened before the PR's metadata arrived.
            let mut cmds = Vec::new();
            if let Screen::Diff(screen) = state.screen()
                && screen.pr == pr
                && !state.diffs.contains_key(&pr)
            {
                cmds = state.start_diff(&pr);
            }
            cmds.extend(clamp_scroll(state));
            cmds
        }
        Msg::Fetched { key, result, fresh } => {
            if let Err(err) = &result {
                tracing::warn!(?key, %err, "fetch failed");
            }
            let is_pr = matches!(result, Ok(Data::Issue(None)));
            let remote = state.data.entry(key.clone()).or_default();
            if fresh {
                remote.finish(result);
            } else if remote.data.is_none()
                && let Ok(data) = result
            {
                remote.data = Some(data);
            }
            // GitHub redirects an issue number that's a pull request.
            if is_pr
                && let DataKey::Issue(repo, number) = key
                && state.route()
                    == Some(&Route::Issue {
                        repo: repo.clone(),
                        number,
                    })
            {
                return state.replace(
                    Route::Pr {
                        pr: PrRef { repo, number },
                        tab: PrTab::Conversation,
                    },
                    true,
                );
            }
            Vec::new()
        }
        Msg::FetchedMore(key, result) => {
            let Some(remote) = state.data.get_mut(&key) else {
                return Vec::new();
            };
            remote.loading = false;
            match result {
                Ok(Data::Search(more)) => {
                    if let Some(Data::Search(results)) = &mut remote.data {
                        browse::append(results, *more);
                    }
                }
                Ok(_) => {}
                Err(err) => {
                    state.notice = Some(Notice::Error(format!("Couldn't load more: {err}")));
                }
            }
            Vec::new()
        }
        Msg::Commented(key, result) => match result {
            Ok(()) => {
                state.overlay = None;
                state.notice = Some(Notice::Info("Comment posted".into()));
                state.ensure(Need::Data(key), true)
            }
            Err(err) => {
                if let Some(Overlay::Compose(compose)) = &mut state.overlay {
                    compose.sending = false;
                    compose.error = Some(err.to_string());
                }
                Vec::new()
            }
        },
        Msg::Starred {
            repo,
            starred,
            result,
        } => {
            match result {
                Ok(()) => {
                    let verb = if starred { "Starred" } else { "Unstarred" };
                    state.notice = Some(Notice::Info(format!("{verb} {repo}")));
                }
                Err(err) => {
                    set_starred(state, &repo, !starred);
                    state.notice = Some(Notice::Error(format!("GitHub didn't save that: {err}")));
                }
            }
            Vec::new()
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
            // "Since my last review" chosen while switching ranges.
            let resume_since = diff.since_requested;
            let mut cmds = state.settle_diff();
            if resume_since {
                if let Some((_, diff)) = state.diff_parts() {
                    diff.since_requested = false;
                }
                cmds.extend(review_action(state, Action::ToggleSinceReview).unwrap_or_default());
            }
            cmds
        }
        Msg::FileDiff(pr, index, file) => {
            let mut cmds = Vec::new();
            if let Some(diff) = state.diffs.get_mut(&pr) {
                diff.set_file(index, file);
                if let Some(inputs) = diff.take_move_inputs() {
                    cmds.push(Cmd::DetectMoves(pr.clone(), inputs));
                }
            }
            cmds.extend(state.settle_diff());
            cmds
        }
        Msg::MovesDetected(pr, moves) => {
            match state.diff_parts() {
                Some((screen, diff)) if screen.pr == pr => {
                    let (cursor, top) =
                        (diff.doc.anchor(screen.cursor), diff.doc.anchor(screen.top));
                    diff.doc.set_moves(moves);
                    screen.cursor = diff.doc.locate(cursor);
                    screen.top = diff.doc.locate(top);
                }
                _ => {
                    if let Some(diff) = state.diffs.get_mut(&pr) {
                        diff.doc.set_moves(moves);
                    }
                }
            }
            state.settle_diff()
        }
        Msg::LastReview(pr, result) => {
            let commit = result.unwrap_or_else(|err| {
                tracing::warn!(%pr, %err, "last review lookup failed");
                None
            });
            let Some(diff) = state.diffs.get_mut(&pr) else {
                return Vec::new();
            };
            diff.last_review = Some(commit);
            if diff.since_requested {
                return start_since_review(state);
            }
            Vec::new()
        }
        Msg::SinceReady(pr, old_head, result) => {
            match result {
                Ok(hashes) => {
                    if let Some((screen, diff)) = state.diff_parts()
                        && screen.pr == pr
                    {
                        diff.since_requested = false;
                        let (cursor, top) =
                            (diff.doc.anchor(screen.cursor), diff.doc.anchor(screen.top));
                        diff.doc.set_since(Some(hashes), true);
                        screen.cursor = diff.doc.locate(cursor);
                        screen.top = diff.doc.locate(top);
                        state.notice = Some(Notice::Info(format!(
                            "Showing changes since your review of {}",
                            &old_head[..7.min(old_head.len())]
                        )));
                    }
                }
                Err(err) => {
                    if let Some(diff) = state.diffs.get_mut(&pr) {
                        diff.since_requested = false;
                    }
                    state.notice = Some(Notice::Error(format!(
                        "Couldn't compare with your last review: {err}"
                    )));
                }
            }
            state.settle_diff()
        }
        Msg::CommitsListed(pr, result) => match result {
            Ok(commits) => {
                if let Some(diff) = state.diffs.get_mut(&pr) {
                    diff.commits = commits;
                }
                state.notice = None;
                open_commit_picker(state);
                Vec::new()
            }
            Err(err) => {
                state.notice = Some(Notice::Error(format!("Couldn't list commits: {err}")));
                Vec::new()
            }
        },
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
            state.settle_diff()
        }
        Msg::ThreadsLoaded(pr, result) => {
            match result {
                Ok(threads) => {
                    if let Some(diff) = state.diffs.get_mut(&pr) {
                        diff.set_threads(threads);
                    }
                }
                Err(err) => {
                    tracing::warn!(%pr, %err, "review threads fetch failed");
                    state.notice = Some(Notice::Error(format!(
                        "Couldn't load review threads: {err}"
                    )));
                }
            }
            let mut cmds = state.map_outdated(&pr);
            cmds.extend(state.settle_diff());
            cmds
        }
        Msg::PatchesLoaded(pr, result) => {
            match result {
                Ok(patches) => {
                    if let Some(diff) = state.diffs.get_mut(&pr) {
                        diff.set_patches(patches);
                    }
                }
                // The local fallback covers commenting; just note it.
                Err(err) => {
                    tracing::warn!(%pr, %err, "GitHub patches unavailable; using local hunks")
                }
            }
            Vec::new()
        }
        Msg::OutdatedMapped(pr, mapped) => {
            if let Some(diff) = state.diffs.get_mut(&pr) {
                diff.mapped.extend(mapped);
                diff.refresh_annotations();
            }
            state.settle_diff()
        }
        Msg::Replied(pr, result) => {
            match result {
                Ok(()) => {
                    state.overlay = None;
                    state.notice = Some(Notice::Info("Reply posted".into()));
                    return vec![Cmd::FetchThreads(pr)];
                }
                Err(err) => {
                    if let Some(Overlay::Compose(compose)) = &mut state.overlay {
                        compose.sending = false;
                        compose.error = Some(err.to_string());
                    }
                }
            }
            Vec::new()
        }
        Msg::ResolvedSet {
            pr,
            thread_id,
            resolved,
            result,
        } => {
            if let Err(err) = result {
                if let Some(diff) = state.diffs.get_mut(&pr) {
                    if let Some(t) = diff.threads.iter_mut().find(|t| t.id == thread_id) {
                        t.resolved = !resolved;
                    }
                    diff.refresh_annotations();
                }
                state.notice = Some(Notice::Error(format!("GitHub didn't save that: {err}")));
            }
            state.settle_diff()
        }
        Msg::ReviewSubmitted(pr, outcome) => on_submitted(state, pr, outcome),
        Msg::Edited(purpose, result) => on_edited(state, purpose, result),
    }
}

/// Applies the result of submitting a review: accepted drafts leave the
/// local queue, rejected ones keep their text and GitHub's reason.
fn on_submitted(state: &mut State, pr: PrRef, outcome: SubmitOutcome) -> Vec<Cmd> {
    let mut cmds = vec![Cmd::FetchThreads(pr.clone())];
    if let Some(diff) = state.diffs.get_mut(&pr) {
        diff.review
            .pending
            .retain(|d| !outcome.accepted.contains(&d.id));
        for draft in &mut diff.review.pending {
            draft.error = outcome
                .rejected
                .iter()
                .find(|(id, _)| *id == draft.id)
                .map(|(_, reason)| reason.clone());
        }
        if outcome.submitted {
            diff.review.last_reviewed_head = diff.refs.as_ref().map(|r| r.head.clone());
        }
        diff.refresh_annotations();
        cmds.push(Cmd::SaveReview(pr.clone(), diff.review.clone()));
    }
    if outcome.submitted {
        state.overlay = None;
        state.notice = Some(Notice::Info("Review submitted".into()));
    } else if let Some(Overlay::Submit(dialog)) = &mut state.overlay {
        dialog.sending = false;
        dialog.error = Some(match (&outcome.error, outcome.rejected.len()) {
            (Some(err), _) => err.clone(),
            (None, n) => format!(
                "GitHub rejected {n} comment{}; they're marked in the diff. The review stays pending on GitHub until you submit again.",
                if n == 1 { "" } else { "s" }
            ),
        });
    }
    cmds.extend(state.settle_diff());
    cmds
}

fn on_edited(state: &mut State, purpose: EditPurpose, result: Result<String, String>) -> Vec<Cmd> {
    let text = match result {
        Ok(text) => text,
        Err(err) => {
            state.notice = Some(Notice::Error(format!("Couldn't run the editor: {err}")));
            return Vec::new();
        }
    };
    let theme = state.theme.clone();
    match purpose {
        EditPurpose::Compose => {
            if let Some(Overlay::Compose(compose)) = &mut state.overlay {
                let target = compose.target.clone();
                let preview = compose.preview.clone();
                **compose = Compose::new(&theme, target, &text);
                compose.preview = preview;
            }
        }
        EditPurpose::Summary => {
            if let Some(Overlay::Submit(dialog)) = &mut state.overlay {
                let event = dialog.event;
                **dialog = SubmitDialog::new(&theme);
                dialog.event = event;
                dialog.input.insert_str(text.trim_end());
            }
        }
        EditPurpose::Suggest {
            path,
            start,
            end,
            original,
        } => {
            let suggested: Vec<String> = text
                .trim_end_matches('\n')
                .lines()
                .map(str::to_owned)
                .collect();
            if suggested == original {
                state.notice = Some(Notice::Info("No change suggested".into()));
                return Vec::new();
            }
            let body = review::suggestion_body(&suggested.join("\n"));
            let mut compose = Compose::new(&theme, ComposeTarget::Line { path, start, end }, &body);
            compose.preview = Some(Preview {
                start_line: start.line,
                original,
                suggested,
            });
            state.overlay = Some(Overlay::Compose(Box::new(compose)));
        }
    }
    Vec::new()
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
        Some(Overlay::Prompt(_)) => return on_prompt_key(state, key),
        Some(Overlay::Links(_)) => return on_links_key(state, key),
        Some(Overlay::FindFile(_)) => return on_finder_key(state, key),
        Some(Overlay::Compose(_)) => return on_compose_key(state, key),
        Some(Overlay::Submit(_)) => return on_submit_key(state, key),
        Some(Overlay::Commits(_)) => return on_commits_key(state, key),
        None => {}
    }
    // Any keypress dismisses the last notice.
    state.notice = None;
    state.pending.push(Key::from(key));
    match state.keymap.resolve(&state.pending, state.scope()) {
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
                Some(PaletteCommand::Go(target)) => state.go(target),
                Some(PaletteCommand::Search(query)) => state.push(search_route(&query)),
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

fn on_prompt_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    let Some(Overlay::Prompt(prompt)) = &mut state.overlay else {
        return Vec::new();
    };
    match key.code {
        KeyCode::Esc => {
            state.overlay = None;
            Vec::new()
        }
        KeyCode::Enter => {
            let query = prompt.input.lines().join("").trim().to_owned();
            let kind = prompt.kind;
            state.overlay = None;
            if query.is_empty() {
                return Vec::new();
            }
            match (kind, state.route().cloned()) {
                (PromptKind::Search, _) => state.push(search_route(&query)),
                (PromptKind::Filter, Some(Route::Issues { repo, .. })) => {
                    state.replace(Route::Issues { repo, query }, true)
                }
                (PromptKind::Filter, Some(Route::Pulls { repo, .. })) => {
                    state.replace(Route::Pulls { repo, query }, true)
                }
                (PromptKind::Filter, Some(Route::Search { kind, .. })) => {
                    state.replace(Route::Search { kind, query }, true)
                }
                (PromptKind::Filter, _) => Vec::new(),
            }
        }
        _ => {
            prompt.input.input(key);
            Vec::new()
        }
    }
}

fn on_links_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    let Some(Overlay::Links(picker)) = &mut state.overlay else {
        return Vec::new();
    };
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let last = picker.items.len().saturating_sub(1);
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => state.overlay = None,
        KeyCode::Down | KeyCode::Tab | KeyCode::Char('j') => {
            picker.selected = (picker.selected + 1).min(last)
        }
        KeyCode::Char('n') if ctrl => picker.selected = (picker.selected + 1).min(last),
        KeyCode::Up | KeyCode::BackTab | KeyCode::Char('k') => {
            picker.selected = picker.selected.saturating_sub(1)
        }
        KeyCode::Char('p') if ctrl => picker.selected = picker.selected.saturating_sub(1),
        KeyCode::Enter => {
            let url = picker.items.get(picker.selected).map(|(u, _)| u.clone());
            state.overlay = None;
            if let Some(url) = url {
                return state.follow(&url);
            }
        }
        _ => {}
    }
    Vec::new()
}

/// A GitHub search: issues when the query looks like it's about them,
/// otherwise repositories (GitHub's default).
fn search_route(query: &str) -> Route {
    let issues = query.split_whitespace().any(|w| {
        [
            "repo:",
            "is:",
            "author:",
            "assignee:",
            "label:",
            "involves:",
            "mentions:",
        ]
        .iter()
        .any(|p| w.starts_with(p))
    });
    Route::Search {
        kind: if issues {
            ghtui_api::browse::SearchKind::Issues
        } else {
            ghtui_api::browse::SearchKind::Repos
        },
        query: query.to_owned(),
    }
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
    input.set_placeholder_text("A command, owner/repo, owner/repo#123, @user, a URL, or a search");
    input.set_placeholder_style(theme.meta(Bg::ContainerHigh));
    Palette { input, selected: 0 }
}

fn apply(state: &mut State, action: Action) -> Vec<Cmd> {
    let content = state.content_area();
    if let Some(cmds) = review_action(state, action) {
        return cmds;
    }
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
    if let Some(cmds) = page_action(state, action) {
        return cmds;
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
            let url = match state.screen() {
                Screen::Page(p) => p.route.url(),
                Screen::Diff(screen) => format!("{}/files", screen.pr.url()),
            };
            state.notice = Some(Notice::Info(format!("Opened {url} in the browser")));
            return vec![Cmd::OpenUrl(url)];
        }
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
        Action::GoHome => {
            if state.route() != Some(&Route::Home) {
                return state.push(Route::Home);
            }
        }
        Action::GoIssues | Action::GoPulls => {
            let Some(repo) = state.context_repo().cloned() else {
                state.notice = Some(Notice::Info("Open a repository first".into()));
                return Vec::new();
            };
            let query = OPEN.to_owned();
            let route = if action == Action::GoIssues {
                Route::Issues { repo, query }
            } else {
                Route::Pulls { repo, query }
            };
            if state.route() != Some(&route) {
                return state.push(route);
            }
        }
        Action::Tab1 => return switch_tab(state, 1),
        Action::Tab2 => return switch_tab(state, 2),
        Action::Tab3 => return switch_tab(state, 3),
        // Diff and page actions elsewhere do nothing.
        _ => {}
    }
    Vec::new()
}

/// Actions on a page screen.
fn page_action(state: &mut State, action: Action) -> Option<Vec<Cmd>> {
    let height = state.page_height();
    let half = (height / 2).max(1) as i64;
    let Screen::Page(p) = state.screen_mut() else {
        return None;
    };
    let delta = match action {
        Action::Down => Some(1),
        Action::Up => Some(-1),
        Action::HalfPageDown => Some(half),
        Action::HalfPageUp => Some(-half),
        Action::Top => Some(i64::MIN / 2),
        Action::Bottom => Some(i64::MAX / 2),
        _ => None,
    };
    if let Some(delta) = delta {
        move_cursor(p, delta, height);
        return Some(Vec::new());
    }
    if matches!(action, Action::NextLink | Action::PrevLink) {
        let target = if action == Action::NextLink {
            p.page.link_line_from(p.cursor + 1, true)
        } else {
            p.cursor
                .checked_sub(1)
                .and_then(|from| p.page.link_line_from(from, false))
        };
        if let Some(line) = target {
            p.cursor = line;
            clamp_page(p, height);
        }
        return Some(Vec::new());
    }
    let route = p.route.clone();
    let links = if action == Action::Open {
        p.links_at_cursor()
    } else {
        Vec::new()
    };
    let cmds = match action {
        Action::Open => match links.as_slice() {
            [] => Vec::new(),
            [(url, _)] => {
                let url = url.clone();
                state.follow(&url)
            }
            _ => {
                state.overlay = Some(Overlay::Links(Box::new(link_picker(&state.theme, links))));
                Vec::new()
            }
        },
        Action::Search => {
            let mut input = new_search_input(&state.theme);
            if let Some(repo) = route.repo() {
                input.insert_str(format!("repo:{repo} "));
            }
            state.overlay = Some(Overlay::Prompt(Box::new(Prompt {
                kind: PromptKind::Search,
                input,
            })));
            Vec::new()
        }
        Action::Filter => {
            let (Route::Issues { query, .. }
            | Route::Pulls { query, .. }
            | Route::Search { query, .. }) = &route
            else {
                state.notice = Some(Notice::Info("Only lists can be filtered".into()));
                return Some(Vec::new());
            };
            let mut input = new_search_input(&state.theme);
            input.insert_str(query);
            state.overlay = Some(Overlay::Prompt(Box::new(Prompt {
                kind: PromptKind::Filter,
                input,
            })));
            Vec::new()
        }
        Action::Star => star(state, &route),
        Action::Comment => comment(state, &route),
        _ => return None,
    };
    Some(cmds)
}

fn link_picker(theme: &Theme, links: Vec<(String, String)>) -> LinkPicker {
    let mut input = new_palette(theme).input;
    input.set_placeholder_text("Which link?");
    let items = links
        .into_iter()
        .map(|(url, text)| {
            let hint = url
                .strip_prefix("https://github.com/")
                .unwrap_or(&url)
                .to_owned();
            (url, PaletteItem { label: text, hint })
        })
        .collect();
    LinkPicker {
        items,
        selected: 0,
        input,
    }
}

/// Switches to the page's numbered tab. From the diff, 1 and 2 return to
/// the pull request's conversation and commits.
fn switch_tab(state: &mut State, n: usize) -> Vec<Cmd> {
    match state.screen().clone() {
        Screen::Page(p) => match state.tab_target(&p.route, n) {
            Some(Target::Page(route)) if route != p.route => state.replace(route, false),
            Some(Target::Files(pr)) => state.go(Target::Files(pr)),
            _ => Vec::new(),
        },
        Screen::Diff(screen) => {
            let tab = match n {
                1 => PrTab::Conversation,
                2 => PrTab::Commits,
                _ => return Vec::new(),
            };
            state.screens.pop();
            let route = Route::Pr {
                pr: screen.pr.clone(),
                tab,
            };
            if state.screens.is_empty() {
                state
                    .screens
                    .push(Screen::Page(Box::new(PageScreen::new(route.clone()))));
                return state.ensure_route(&route, false);
            }
            match state.route() {
                Some(Route::Pr { pr, .. }) if *pr == screen.pr => state.replace(route, false),
                _ => state.push(route),
            }
        }
    }
}

fn star(state: &mut State, route: &Route) -> Vec<Cmd> {
    let Some(repo) = route.repo().cloned() else {
        state.notice = Some(Notice::Info("Open a repository to star it".into()));
        return Vec::new();
    };
    let Some(overview) = state.overview(&repo) else {
        state.notice = Some(Notice::Info("The repository hasn't loaded yet".into()));
        return Vec::new();
    };
    let (id, starred) = (overview.id.clone(), !overview.starred);
    set_starred(state, &repo, starred);
    vec![Cmd::SetStarred { repo, id, starred }]
}

/// Shows a repository as starred or not (optimistically, before GitHub
/// confirms).
fn set_starred(state: &mut State, repo: &RepoId, starred: bool) {
    if let Some(Remote {
        data: Some(Data::Repo(o)),
        ..
    }) = state.data.get_mut(&DataKey::Repo(repo.clone()))
        && o.starred != starred
    {
        o.starred = starred;
        o.summary.stars = if starred {
            o.summary.stars + 1
        } else {
            o.summary.stars.saturating_sub(1)
        };
    }
    state.data_gen += 1;
}

fn comment(state: &mut State, route: &Route) -> Vec<Cmd> {
    let target = match route {
        Route::Issue { repo, number } => {
            let key = DataKey::Issue(repo.clone(), *number);
            match state.remote(&key).and_then(|r| r.data.as_ref()) {
                Some(Data::Issue(Some(issue))) => {
                    Some((issue.id.clone(), format!("{repo}#{number}"), key))
                }
                _ => None,
            }
        }
        Route::Pr { pr, .. } => {
            let key = DataKey::PrActivity(pr.clone());
            match state.remote(&key).and_then(|r| r.data.as_ref()) {
                Some(Data::PrActivity(a)) => Some((a.id.clone(), pr.to_string(), key)),
                _ => None,
            }
        }
        _ => {
            state.notice = Some(Notice::Info(
                "Comments go on issues and pull requests".into(),
            ));
            return Vec::new();
        }
    };
    let Some((subject_id, name, refresh)) = target else {
        state.notice = Some(Notice::Info("Still loading".into()));
        return Vec::new();
    };
    let compose = Compose::new(
        &state.theme,
        ComposeTarget::Conversation {
            subject_id,
            name,
            refresh,
        },
        "",
    );
    state.overlay = Some(Overlay::Compose(Box::new(compose)));
    Vec::new()
}

/// Keeps the page cursor (or the diff) in bounds after a change.
fn clamp_scroll(state: &mut State) -> Vec<Cmd> {
    let height = state.page_height();
    match state.screen_mut() {
        Screen::Page(p) => {
            clamp_page(p, height);
            Vec::new()
        }
        Screen::Diff(_) => state.settle_diff(),
    }
}

// ---- reviewing ---------------------------------------------------------------

impl State {
    /// Asks the diff job to map outdated threads, once both the threads and
    /// the diff's head are known.
    fn map_outdated(&mut self, pr: &PrRef) -> Vec<Cmd> {
        let Some(diff) = self.diffs.get_mut(pr) else {
            return Vec::new();
        };
        let Some(head) = diff.refs.as_ref().map(|r| r.head.clone()) else {
            return Vec::new();
        };
        if diff.mapping_requested {
            return Vec::new();
        }
        let items = review::outdated_to_map(&diff.threads);
        if items.is_empty() {
            return Vec::new();
        }
        diff.mapping_requested = true;
        vec![Cmd::MapOutdated {
            pr: pr.clone(),
            head,
            items,
        }]
    }

    fn diff_parts(&mut self) -> Option<(&mut DiffScreen, &mut DiffState)> {
        let State { screens, diffs, .. } = self;
        let Some(Screen::Diff(screen)) = screens.last_mut() else {
            return None;
        };
        let diff = diffs.get_mut(&screen.pr)?;
        Some((screen, diff))
    }
}

/// Review actions on the diff screen. `None` lets other handlers try.
fn review_action(state: &mut State, action: Action) -> Option<Vec<Cmd>> {
    let theme = state.theme.clone();
    let (screen, diff) = state.diff_parts()?;
    let pr = screen.pr.clone();
    let cursor = screen.cursor;
    let annotation = diff
        .doc
        .annotation_at(cursor)
        .and_then(|i| diff.doc.annotations.get(i as usize).cloned());
    let notice = |state: &mut State, n: Notice| {
        state.notice = Some(n);
        Some(Vec::new())
    };
    let in_range = diff.range.is_some();
    if in_range
        && matches!(
            action,
            Action::Comment | Action::Suggest | Action::FileComment | Action::SubmitReview
        )
    {
        return notice(
            state,
            Notice::Error(
                "Comments anchor to the whole PR: pick “All changes” (gc) to comment".into(),
            ),
        );
    }
    match action {
        Action::ToggleSinceReview => {
            if in_range {
                return notice(state, Notice::Error("Pick “All changes” (gc) first".into()));
            }
            if diff.doc.since.is_some() {
                let active = !diff.doc.since_active;
                let (c, t) = (diff.doc.anchor(screen.cursor), diff.doc.anchor(screen.top));
                let hashes = diff.doc.since.clone();
                diff.doc.set_since(hashes, active);
                screen.cursor = diff.doc.locate(c);
                screen.top = diff.doc.locate(t);
                state.notice = Some(Notice::Info(
                    if active {
                        "Showing changes since your last review"
                    } else {
                        "Showing all changes"
                    }
                    .into(),
                ));
                return Some(state.settle_diff());
            }
            diff.since_requested = true;
            if diff.last_review.is_none() {
                let Some(login) = state.viewer.clone() else {
                    return Some(start_since_review_local(state));
                };
                state.notice = Some(Notice::Info("Looking up your last review…".into()));
                return Some(vec![Cmd::FetchLastReview { pr, login }]);
            }
            Some(start_since_review(state))
        }
        Action::PickCommits => {
            if diff.commits.is_empty() {
                state.notice = Some(Notice::Info("Listing commits…".into()));
                return Some(vec![Cmd::ListCommits(pr)]);
            }
            open_commit_picker(state);
            Some(Vec::new())
        }
        Action::Back if screen.selection.is_some() => {
            screen.selection = None;
            Some(Vec::new())
        }
        Action::Comment => {
            let selection = screen.selection.take();
            match review::target(&diff.doc, cursor, selection) {
                Ok(target) => {
                    let reason = match &target {
                        ComposeTarget::File { reason, .. } => reason.clone(),
                        _ => None,
                    };
                    state.overlay =
                        Some(Overlay::Compose(Box::new(Compose::new(&theme, target, ""))));
                    if let Some(reason) = reason {
                        state.notice = Some(Notice::Info(reason));
                    }
                    Some(Vec::new())
                }
                Err(err) => notice(state, Notice::Error(err)),
            }
        }
        Action::FileComment => {
            // On a draft: turn it into a file comment (e.g. after GitHub
            // rejected its line). Elsewhere: comment on the file.
            if let Some(ann) = &annotation
                && let AnnotationKey::Draft(id) = ann.key
            {
                if let Some(d) = diff.review.pending.iter_mut().find(|d| d.id == id) {
                    d.line = None;
                    d.start_line = None;
                    d.start_side = None;
                    d.error = None;
                }
                diff.refresh_annotations();
                let save = Cmd::SaveReview(pr, diff.review.clone());
                state.notice = Some(Notice::Info("Now a comment on the file".into()));
                return Some(vec![save]);
            }
            let path = diff.doc.files.get(cursor.file)?.meta.path().to_owned();
            state.overlay = Some(Overlay::Compose(Box::new(Compose::new(
                &theme,
                ComposeTarget::File { path, reason: None },
                "",
            ))));
            Some(Vec::new())
        }
        Action::Suggest => {
            let selection = screen.selection.take();
            let target = match review::target(&diff.doc, cursor, selection) {
                Ok(target) => target,
                Err(err) => return notice(state, Notice::Error(err)),
            };
            let ComposeTarget::Line { path, start, end } = target else {
                return notice(
                    state,
                    Notice::Error("Suggestions need lines GitHub can comment on".into()),
                );
            };
            if start.side != ghtui_diff::anchor::Side::Right
                || end.side != ghtui_diff::anchor::Side::Right
            {
                return notice(
                    state,
                    Notice::Error("Suggestions apply to new lines (not deleted ones)".into()),
                );
            }
            let text = diff.doc.files.get(cursor.file)?.text()?;
            let original: Vec<String> = (start.line..=end.line)
                .map(|n| text.new.line(n as usize - 1).to_owned())
                .collect();
            Some(vec![Cmd::Edit {
                text: original.join("\n") + "\n",
                purpose: EditPurpose::Suggest {
                    path,
                    start,
                    end,
                    original,
                },
            }])
        }
        Action::ReplyThread => match annotation {
            Some(ann) if ann.can_reply => {
                let AnnotationKey::Thread(thread_id) = ann.key else {
                    return Some(Vec::new());
                };
                state.overlay = Some(Overlay::Compose(Box::new(Compose::new(
                    &theme,
                    ComposeTarget::Reply { thread_id },
                    "",
                ))));
                Some(Vec::new())
            }
            Some(_) => notice(state, Notice::Info("You can't reply to this one".into())),
            None => notice(state, Notice::Info("Move to a thread to reply".into())),
        },
        Action::ResolveThread => {
            let Some(ann) = annotation else {
                return notice(state, Notice::Info("Move to a thread to resolve it".into()));
            };
            let AnnotationKey::Thread(thread_id) = ann.key else {
                return notice(state, Notice::Info("Drafts can't be resolved".into()));
            };
            let resolved = !ann.resolved;
            if (resolved && !ann.can_resolve) || (!resolved && !ann.can_unresolve) {
                return notice(state, Notice::Info("You can't change this thread".into()));
            }
            // Optimistic; rolled back if GitHub refuses.
            if let Some(t) = diff.threads.iter_mut().find(|t| t.id == thread_id) {
                t.resolved = resolved;
            }
            diff.refresh_annotations();
            Some(vec![Cmd::SetResolved {
                pr,
                thread_id,
                resolved,
            }])
        }
        Action::DeleteDraft => {
            let Some(AnnotationKey::Draft(id)) = annotation.map(|a| a.key) else {
                return notice(
                    state,
                    Notice::Info("Move to a draft comment to delete it".into()),
                );
            };
            diff.review.pending.retain(|d| d.id != id);
            diff.refresh_annotations();
            let save = Cmd::SaveReview(pr, diff.review.clone());
            state.notice = Some(Notice::Info("Draft deleted".into()));
            Some(vec![save])
        }
        Action::Open => {
            let ann_index = diff.doc.annotation_at(cursor)?;
            let ann = diff.doc.annotations.get(ann_index as usize)?.clone();
            if let AnnotationKey::Draft(id) = ann.key {
                let body = diff
                    .review
                    .pending
                    .iter()
                    .find(|d| d.id == id)?
                    .body
                    .clone();
                state.overlay = Some(Overlay::Compose(Box::new(Compose::new(
                    &theme,
                    ComposeTarget::Draft { id },
                    &body,
                ))));
                return Some(Vec::new());
            }
            diff.doc.toggle_thread(ann_index);
            Some(state.settle_diff())
        }
        Action::SubmitReview => {
            if diff.refs.is_none() {
                return notice(state, Notice::Error("The diff hasn't loaded yet".into()));
            }
            state.overlay = Some(Overlay::Submit(Box::new(SubmitDialog::new(&theme))));
            Some(Vec::new())
        }
        _ => None,
    }
}

/// Compares with the head of your last review, once known.
fn start_since_review(state: &mut State) -> Vec<Cmd> {
    let Some((screen, diff)) = state.diff_parts() else {
        return Vec::new();
    };
    let pr = screen.pr.clone();
    let old = diff
        .last_review
        .clone()
        .flatten()
        .or_else(|| diff.review.last_reviewed_head.clone());
    let head = diff.refs.as_ref().map(|r| r.head.clone());
    let problem = match (&old, &head) {
        (None, _) => Some("You haven't reviewed this PR yet"),
        (_, None) => Some("The diff hasn't loaded yet"),
        (Some(old), Some(head)) if old == head => {
            Some("Nothing new: you reviewed the current head")
        }
        _ => None,
    };
    if let Some(message) = problem {
        diff.since_requested = false;
        state.notice = Some(Notice::Info(message.into()));
        return Vec::new();
    }
    state.notice = Some(Notice::Info("Comparing with your last review…".into()));
    vec![Cmd::SinceReview {
        pr,
        old_head: old.unwrap_or_default(),
    }]
}

/// Without a GitHub login, only the locally remembered review is known.
fn start_since_review_local(state: &mut State) -> Vec<Cmd> {
    if let Some((_, diff)) = state.diff_parts() {
        diff.last_review = Some(None);
    }
    start_since_review(state)
}

fn open_commit_picker(state: &mut State) {
    let theme = state.theme.clone();
    let Some((_, diff)) = state.diff_parts() else {
        return;
    };
    let current = |on: bool| {
        if on {
            "current".to_owned()
        } else {
            String::new()
        }
    };
    let mut items = vec![
        (
            PickItem::All,
            PaletteItem {
                label: "All changes".into(),
                hint: current(diff.range.is_none() && !diff.doc.since_active),
            },
        ),
        (
            PickItem::SinceReview,
            PaletteItem {
                label: "Changes since your last review".into(),
                hint: current(diff.range.is_none() && diff.doc.since_active),
            },
        ),
    ];
    for (i, (sha, subject)) in diff.commits.iter().enumerate() {
        items.push((
            PickItem::Commit(i),
            PaletteItem {
                label: format!("{} {subject}", &sha[..7.min(sha.len())]),
                hint: String::new(),
            },
        ));
    }
    let mut input = TextArea::default();
    input.set_style(theme.body(Bg::ContainerHigh));
    input.set_cursor_style(theme.body(Bg::ContainerHigh));
    input.set_placeholder_text("Space marks a range start · Enter views · Esc cancels");
    input.set_placeholder_style(theme.meta(Bg::ContainerHigh));
    state.overlay = Some(Overlay::Commits(Box::new(CommitPicker {
        items,
        selected: 0,
        mark: None,
        input,
    })));
}

fn on_commits_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    let Some(Overlay::Commits(picker)) = &mut state.overlay else {
        return Vec::new();
    };
    match key.code {
        KeyCode::Esc => {
            state.overlay = None;
            Vec::new()
        }
        KeyCode::Down | KeyCode::Char('j') => {
            picker.selected = (picker.selected + 1).min(picker.items.len() - 1);
            Vec::new()
        }
        KeyCode::Up | KeyCode::Char('k') => {
            picker.selected = picker.selected.saturating_sub(1);
            Vec::new()
        }
        KeyCode::Char(' ') => {
            let i = picker.selected;
            if matches!(picker.items[i].0, PickItem::Commit(_)) {
                picker.mark = if picker.mark == Some(i) {
                    None
                } else {
                    Some(i)
                };
                for (j, (_, item)) in picker.items.iter_mut().enumerate() {
                    if Some(j) == picker.mark {
                        item.hint = "range start".into();
                    } else if item.hint == "range start" {
                        item.hint.clear();
                    }
                }
            }
            Vec::new()
        }
        KeyCode::Enter => {
            let choice = picker.items[picker.selected].0.clone();
            let mark = picker.mark.map(|m| picker.items[m].0.clone());
            state.overlay = None;
            apply_commit_choice(state, choice, mark)
        }
        _ => Vec::new(),
    }
}

fn apply_commit_choice(state: &mut State, choice: PickItem, mark: Option<PickItem>) -> Vec<Cmd> {
    let width = state.size.0;
    let pr = match state.screen() {
        Screen::Diff(screen) => screen.pr.clone(),
        _ => return Vec::new(),
    };
    let Some(base_ref) = state
        .prs
        .get(&pr)
        .and_then(|r| r.data.as_ref())
        .map(|d| d.base_ref.clone())
    else {
        return Vec::new();
    };
    let Some((screen, diff)) = state.diff_parts() else {
        return Vec::new();
    };
    let range = match choice {
        PickItem::SinceReview if diff.range.is_none() => {
            if diff.doc.since_active {
                return Vec::new();
            }
            return review_action(state, Action::ToggleSinceReview).unwrap_or_default();
        }
        PickItem::SinceReview => {
            // Back to the whole PR first, then compare once it's loaded.
            diff.restart(None);
            diff.since_requested = true;
            *screen = DiffScreen::new(pr.clone(), width);
            return vec![Cmd::LoadDiff {
                pr,
                base_ref,
                range: None,
            }];
        }
        PickItem::All if diff.range.is_none() => {
            if diff.doc.since_active {
                return review_action(state, Action::ToggleSinceReview).unwrap_or_default();
            }
            return Vec::new();
        }
        PickItem::All => None,
        PickItem::Commit(i) => {
            let j = match mark {
                Some(PickItem::Commit(m)) => m,
                _ => i,
            };
            let (first, last) = (i.min(j), i.max(j));
            let (from, _) = &diff.commits[first];
            let (to, _) = &diff.commits[last];
            let short = |sha: &str| sha[..7.min(sha.len())].to_owned();
            let label = if first == last {
                short(to)
            } else {
                format!("{}..{}", short(from), short(to))
            };
            Some(diff_screen::RangeView {
                label,
                from: format!("{from}^"),
                to: to.clone(),
            })
        }
    };
    let cmd_range = range.as_ref().map(|r| (r.from.clone(), r.to.clone()));
    diff.restart(range);
    *screen = DiffScreen::new(pr.clone(), width);
    vec![Cmd::LoadDiff {
        pr,
        base_ref,
        range: cmd_range,
    }]
}

fn on_compose_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    let Some(Overlay::Compose(compose)) = &mut state.overlay else {
        return Vec::new();
    };
    if compose.sending {
        return Vec::new();
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc if compose.text().trim().is_empty() || compose.confirm_discard => {
            state.overlay = None;
            Vec::new()
        }
        KeyCode::Esc => {
            compose.confirm_discard = true;
            compose.error = Some("Press Esc again to discard this comment".into());
            Vec::new()
        }
        KeyCode::Char('e') if ctrl => vec![Cmd::Edit {
            purpose: EditPurpose::Compose,
            text: compose.text(),
        }],
        KeyCode::Char('s') if ctrl => save_compose(state),
        _ => {
            compose.confirm_discard = false;
            compose.error = None;
            compose.input.input(key);
            Vec::new()
        }
    }
}

/// `<C-s>` in the composer: drafts join the pending review (and are saved);
/// replies post right away.
fn save_compose(state: &mut State) -> Vec<Cmd> {
    let Some(Overlay::Compose(compose)) = &state.overlay else {
        return Vec::new();
    };
    let body = compose.text();
    if body.trim().is_empty() {
        if let Some(Overlay::Compose(compose)) = &mut state.overlay {
            compose.error = Some("Write something first".into());
        }
        return Vec::new();
    }
    let target = compose.target.clone();
    if let ComposeTarget::Conversation {
        subject_id,
        refresh,
        ..
    } = target
    {
        if let Some(Overlay::Compose(compose)) = &mut state.overlay {
            compose.sending = true;
        }
        return vec![Cmd::AddComment {
            subject_id,
            body,
            refresh,
        }];
    }
    let Some((screen, diff)) = state.diff_parts() else {
        return Vec::new();
    };
    let pr = screen.pr.clone();
    match target {
        ComposeTarget::Reply { thread_id } => {
            if let Some(Overlay::Compose(compose)) = &mut state.overlay {
                compose.sending = true;
            }
            vec![Cmd::Reply {
                pr,
                thread_id,
                body,
            }]
        }
        ComposeTarget::Draft { id } => {
            if let Some(d) = diff.review.pending.iter_mut().find(|d| d.id == id) {
                d.body = body;
                d.error = None;
            }
            diff.refresh_annotations();
            let save = Cmd::SaveReview(pr, diff.review.clone());
            state.overlay = None;
            vec![save]
        }
        ComposeTarget::Conversation { .. } => Vec::new(),
        target @ (ComposeTarget::Line { .. } | ComposeTarget::File { .. }) => {
            let head = diff
                .refs
                .as_ref()
                .map(|r| r.head.clone())
                .unwrap_or_default();
            let id = diff.review.next_draft_id();
            let Some(draft) = review::draft(&target, body, id, &head) else {
                return Vec::new();
            };
            diff.review.pending.push(draft);
            diff.refresh_annotations();
            let save = Cmd::SaveReview(pr, diff.review.clone());
            let count = diff.review.pending.len();
            state.overlay = None;
            state.notice = Some(Notice::Info(format!(
                "Added to your review ({count} pending). Submit with {}",
                state
                    .keymap
                    .keys_for(Action::SubmitReview)
                    .first()
                    .cloned()
                    .unwrap_or_default()
            )));
            let mut cmds = vec![save];
            cmds.extend(state.settle_diff());
            cmds
        }
    }
}

fn on_submit_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    let Some(Overlay::Submit(dialog)) = &mut state.overlay else {
        return Vec::new();
    };
    if dialog.sending {
        return Vec::new();
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => {
            state.overlay = None;
            Vec::new()
        }
        KeyCode::Tab => {
            dialog.cycle(true);
            Vec::new()
        }
        KeyCode::BackTab => {
            dialog.cycle(false);
            Vec::new()
        }
        KeyCode::Char('e') if ctrl => vec![Cmd::Edit {
            purpose: EditPurpose::Summary,
            text: dialog.input.lines().join("\n"),
        }],
        KeyCode::Char('s') if ctrl => {
            let event = dialog.event;
            let body = dialog.input.lines().join("\n");
            if event == ReviewEvent::RequestChanges && body.trim().is_empty() {
                dialog.error = Some("Requesting changes needs a summary".into());
                return Vec::new();
            }
            dialog.sending = true;
            dialog.error = None;
            let Some((screen, diff)) = state.diff_parts() else {
                return Vec::new();
            };
            let head = diff
                .refs
                .as_ref()
                .map(|r| r.head.clone())
                .unwrap_or_default();
            vec![Cmd::SubmitReview {
                pr: screen.pr.clone(),
                head,
                drafts: diff.review.pending.clone(),
                event,
                body,
            }]
        }
        _ => {
            dialog.error = None;
            dialog.input.input(key);
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghtui_api::browse::SearchResults;
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

    fn page(state: &State) -> &PageScreen {
        match state.screen() {
            Screen::Page(p) => p,
            Screen::Diff(_) => panic!("not on a page"),
        }
    }

    fn route(state: &State) -> Route {
        page(state).route.clone()
    }

    fn cursor_text(state: &State) -> String {
        let p = page(state);
        p.page.lines[p.cursor].text()
    }

    fn fetched(state: &mut State, key: DataKey, data: Data) -> Vec<Cmd> {
        update(
            state,
            Msg::Fetched {
                key,
                result: Ok(data),
                fresh: true,
            },
        )
    }

    fn repo() -> RepoId {
        RepoId::new("gold-silver-copper", "ghtui")
    }

    fn with_repo() -> State {
        let mut state = state();
        state.push(Route::Repo(repo()));
        fetched(
            &mut state,
            DataKey::Repo(repo()),
            Data::Repo(Box::new(crate::fixtures::overview())),
        );
        state
    }

    #[test]
    fn home_moves_and_follows_links() {
        let mut state = with_inbox(3);
        assert_eq!(route(&state), Route::Home);
        // Past the section headers' "see all" links to the first PR.
        press(&mut state, "<Tab><Tab>");
        assert!(
            cursor_text(&state).contains("PR 1"),
            "{}",
            cursor_text(&state)
        );
        press(&mut state, "j");
        assert!(
            cursor_text(&state).contains("o/r#1"),
            "the PR's second line"
        );
        let cmds = press(&mut state, "<Enter>");
        let pr = PrRef::parse("o/r#1").unwrap();
        assert_eq!(
            cmds,
            vec![
                Cmd::FetchPr(pr.clone()),
                Cmd::Fetch {
                    key: DataKey::PrActivity(pr.clone()),
                    cached: true
                }
            ],
            "Enter on a card's second line follows its title"
        );
        assert_eq!(state.tabs(), ["Home", "o/r#1"]);
        assert_eq!(state.busy().as_deref(), Some("Loading"));

        update(
            &mut state,
            Msg::Pr(pr.clone(), Box::new(Err(ApiError::Network("down".into())))),
        );
        let text: Vec<String> = page(&state).page.lines.iter().map(|l| l.text()).collect();
        assert!(
            text.iter().any(|l| l.contains("network error: down")),
            "{text:?}"
        );
        assert_eq!(
            press(&mut state, "r"),
            vec![Cmd::FetchViewer, Cmd::FetchPr(pr)],
            "the activity is still loading"
        );

        press(&mut state, "<Esc>");
        assert_eq!(state.tabs(), ["Home"]);
        assert!(cursor_text(&state).contains("o/r#1"), "the cursor survives");
        assert!(!state.quit);
        press(&mut state, "q");
        assert!(state.quit);
    }

    #[test]
    fn the_cursor_skips_blank_lines_and_stays_on_the_page() {
        let mut state = with_inbox(3);
        press(&mut state, "G");
        let p = page(&state);
        assert!(!p.page.lines[p.cursor].is_blank());
        assert_eq!(
            p.cursor,
            p.page.lines.iter().rposition(|l| !l.is_blank()).unwrap()
        );
        press(&mut state, "gg");
        assert_eq!(page(&state).cursor, 0);
        press(&mut state, "k<C-u>");
        assert_eq!(page(&state).cursor, 0);
        press(&mut state, "<C-d><C-d><C-d><C-d>");
        let p = page(&state);
        assert!(p.cursor < p.page.lines.len());
    }

    #[test]
    fn duplicate_fetches_are_suppressed() {
        let mut state = state();
        assert_eq!(
            state.load_visible(false),
            vec![
                Cmd::FetchInbox,
                Cmd::Fetch {
                    key: DataKey::ViewerRepos,
                    cached: true
                }
            ]
        );
        assert_eq!(state.load_visible(true), vec![Cmd::FetchViewer]);
    }

    #[test]
    fn cached_data_shows_while_fetching() {
        let mut state = state();
        state.push(Route::User("octocat".into()));
        update(
            &mut state,
            Msg::Fetched {
                key: DataKey::Profile("octocat".into()),
                result: Ok(Data::Profile(Box::new(crate::fixtures::profile()))),
                fresh: false,
            },
        );
        assert!(cursor_text(&state).contains("The Octocat"));
        assert_eq!(state.busy().as_deref(), Some("Loading"), "still refreshing");
    }

    #[test]
    fn failed_refresh_keeps_cached_data() {
        let mut state = with_inbox(2);
        state.load_visible(true);
        update(&mut state, Msg::Inbox(Err(ApiError::RateLimited(30))));
        assert_eq!(state.inbox.data.as_ref().unwrap().authored.len(), 2);
        let first = page(&state).page.lines[0].text();
        assert!(
            first.contains("Couldn't refresh") && first.contains("rate limited"),
            "{first}"
        );
    }

    #[test]
    fn opens_in_browser() {
        let mut state = with_inbox(1);
        assert_eq!(
            press(&mut state, "o"),
            vec![Cmd::OpenUrl("https://github.com/".into())]
        );
        state.push(Route::Issues {
            repo: repo(),
            query: "is:closed".into(),
        });
        assert_eq!(
            press(&mut state, "o"),
            vec![Cmd::OpenUrl(
                "https://github.com/gold-silver-copper/ghtui/issues?q=is:closed".into()
            )]
        );
    }

    #[test]
    fn palette_goes_places_and_runs_actions() {
        let mut state = with_inbox(1);
        press(&mut state, ":");
        assert!(matches!(state.overlay, Some(Overlay::Palette(_))));
        let cmds = press(&mut state, "a/b#9<Enter>");
        let key = DataKey::Issue(RepoId::new("a", "b"), 9);
        assert_eq!(
            cmds,
            vec![Cmd::Fetch {
                key: key.clone(),
                cached: true
            }]
        );
        assert!(state.overlay.is_none());

        // The number is a pull request: GitHub's redirect.
        let cmds = update(
            &mut state,
            Msg::Fetched {
                key,
                result: Ok(Data::Issue(None)),
                fresh: true,
            },
        );
        let pr = PrRef::parse("a/b#9").unwrap();
        assert_eq!(
            route(&state),
            Route::Pr {
                pr: pr.clone(),
                tab: PrTab::Conversation
            }
        );
        assert_eq!(cmds[0], Cmd::FetchPr(pr));
        assert_eq!(state.tabs(), ["Home", "a/b#9"], "replaced, not pushed");

        // A bare number resolves against the repo on screen.
        press(&mut state, ":12<Enter>");
        assert_eq!(state.tabs().last().unwrap(), "a/b#12");

        press(&mut state, ":@octocat<Enter>");
        assert_eq!(route(&state), Route::User("octocat".into()));

        press(&mut state, ":help<Enter>");
        assert!(matches!(state.overlay, Some(Overlay::Help)));
        press(&mut state, "<Esc>");
        assert!(state.overlay.is_none());
    }

    #[test]
    fn palette_searches_github() {
        let state = state();
        let commands: Vec<PaletteCommand> = state
            .palette_commands("ratatui widgets")
            .into_iter()
            .map(|(c, _)| c)
            .collect();
        assert!(commands.contains(&PaletteCommand::Search("ratatui widgets".into())));
        assert_eq!(
            state.palette_commands("ratatui/ratatui")[0].0,
            PaletteCommand::Go(Target::Page(Route::Repo(RepoId::new("ratatui", "ratatui"))))
        );
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
    fn palette_list_stays_in_bounds() {
        let mut state = state();
        press(&mut state, ":");
        for _ in 0..(state.palette_commands("").len() + PALETTE_ROWS) {
            press(&mut state, "<Down>");
        }
        let Some(Overlay::Palette(palette)) = &state.overlay else {
            panic!()
        };
        assert_eq!(palette.selected, state.palette_commands("").len() - 1);
    }

    #[test]
    fn repo_tabs_filter_and_star() {
        let mut state = with_repo();
        let cmds = press(&mut state, "2");
        let issues = Route::Issues {
            repo: repo(),
            query: OPEN.into(),
        };
        assert_eq!(route(&state), issues);
        let (kind, query) = issues.search().unwrap();
        assert_eq!(
            cmds,
            vec![Cmd::Fetch {
                key: DataKey::Search(kind, query),
                cached: true
            }],
            "the header isn't fetched again"
        );
        assert_eq!(state.tabs(), ["Home", "gold-silver-copper/ghtui · Issues"]);

        press(&mut state, "f");
        let Some(Overlay::Prompt(prompt)) = &state.overlay else {
            panic!("no filter prompt")
        };
        assert_eq!(prompt.input.lines().join(""), OPEN);
        press(&mut state, " label:bug<Enter>");
        assert_eq!(
            route(&state),
            Route::Issues {
                repo: repo(),
                query: "is:open label:bug".into()
            }
        );

        press(&mut state, "3");
        assert!(matches!(route(&state), Route::Pulls { .. }));
        press(&mut state, "1");
        assert_eq!(route(&state), Route::Repo(repo()));

        let cmds = press(&mut state, "*");
        assert_eq!(
            cmds,
            vec![Cmd::SetStarred {
                repo: repo(),
                id: "R_ghtui".into(),
                starred: true
            }]
        );
        assert!(state.overview(&repo()).unwrap().starred);
        assert_eq!(state.overview(&repo()).unwrap().summary.stars, 1235);
        update(
            &mut state,
            Msg::Starred {
                repo: repo(),
                starred: true,
                result: Err(ApiError::Network("down".into())),
            },
        );
        assert!(!state.overview(&repo()).unwrap().starred, "rolled back");
        assert_eq!(state.overview(&repo()).unwrap().summary.stars, 1234);
    }

    #[test]
    fn several_links_on_a_line_ask_which() {
        let mut state = with_repo();
        // The first line: "owner / name".
        press(&mut state, "gg<Enter>");
        let Some(Overlay::Links(picker)) = &state.overlay else {
            panic!("no link picker")
        };
        assert_eq!(picker.items.len(), 2);
        press(&mut state, "<Enter>");
        assert_eq!(route(&state), Route::User("gold-silver-copper".into()));
    }

    #[test]
    fn directories_and_files_open_from_the_repo() {
        let mut state = with_repo();
        while !cursor_text(&state).contains("crates") {
            press(&mut state, "<Tab>");
        }
        let cmds = press(&mut state, "<Enter>");
        let tree = DataKey::Tree(repo(), "main".into(), "crates".into());
        assert_eq!(
            cmds,
            vec![Cmd::Fetch {
                key: tree.clone(),
                cached: true
            }]
        );
        fetched(&mut state, tree, Data::Tree(crate::fixtures::tree()));
        while !cursor_text(&state).contains("README.md") {
            press(&mut state, "<Tab>");
        }
        press(&mut state, "<Enter>");
        assert_eq!(
            route(&state),
            Route::Blob {
                repo: repo(),
                rev: "main".into(),
                path: "crates/README.md".into()
            }
        );
    }

    #[test]
    fn issue_comments_post_and_refresh() {
        let mut state = state();
        state.push(Route::Issue {
            repo: repo(),
            number: 14,
        });
        let key = DataKey::Issue(repo(), 14);
        press(&mut state, "c");
        assert!(state.overlay.is_none(), "nothing to comment on yet");
        fetched(
            &mut state,
            key.clone(),
            Data::Issue(Some(Box::new(crate::fixtures::issue()))),
        );
        press(&mut state, "c");
        let Some(Overlay::Compose(compose)) = &state.overlay else {
            panic!("no composer")
        };
        assert_eq!(compose.title(), "Comment on gold-silver-copper/ghtui#14");
        press(&mut state, "Thanks!");
        let cmds = update(
            &mut state,
            Msg::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
        );
        assert_eq!(
            cmds,
            vec![Cmd::AddComment {
                subject_id: "I_14".into(),
                body: "Thanks!".into(),
                refresh: key.clone()
            }]
        );
        let cmds = update(&mut state, Msg::Commented(key.clone(), Ok(())));
        assert!(state.overlay.is_none());
        assert_eq!(cmds, vec![Cmd::Fetch { key, cached: false }]);
    }

    #[test]
    fn search_loads_more() {
        let mut state = state();
        press(&mut state, "/");
        press(&mut state, "is:open label:bug<Enter>");
        let route = route(&state);
        let (kind, query) = route.search().unwrap();
        assert_eq!(
            kind,
            ghtui_api::browse::SearchKind::Issues,
            "issue-ish query"
        );
        let key = DataKey::Search(kind, query);
        fetched(
            &mut state,
            key.clone(),
            Data::Search(Box::new(crate::fixtures::issue_results(Some("c1")))),
        );
        press(&mut state, "G");
        assert!(cursor_text(&state).starts_with("Load more"));
        let cmds = press(&mut state, "<Enter>");
        assert_eq!(
            cmds,
            vec![Cmd::FetchMore {
                key: key.clone(),
                after: "c1".into()
            }]
        );
        update(
            &mut state,
            Msg::FetchedMore(
                key.clone(),
                Ok(Data::Search(Box::new(crate::fixtures::issue_results(None)))),
            ),
        );
        let Some(Data::Search(results)) = state.remote(&key).and_then(|r| r.data.as_ref()) else {
            panic!()
        };
        assert_eq!(browse::next_cursor(results), None);
        let SearchResults::Issues(r) = &**results else {
            panic!()
        };
        assert_eq!(r.items.len(), 6);
    }

    #[test]
    fn files_tab_opens_the_diff_once_the_pr_loads() {
        let mut state = state();
        let pr = PrRef::parse("o/r#1").unwrap();
        state.push(Route::Pr {
            pr: pr.clone(),
            tab: PrTab::Conversation,
        });
        assert!(press(&mut state, "3").is_empty(), "waits for the PR");
        assert!(matches!(state.screen(), Screen::Diff(_)));
        assert_eq!(state.busy().as_deref(), Some("Loading o/r#1"));
        let cmds = update(
            &mut state,
            Msg::Pr(pr.clone(), Box::new(Ok(crate::snapshot_tests::pr_detail()))),
        );
        assert!(
            cmds.iter().any(|c| matches!(c, Cmd::LoadDiff { .. })),
            "{cmds:?}"
        );
        press(&mut state, "2");
        assert_eq!(
            route(&state),
            Route::Pr {
                pr,
                tab: PrTab::Commits
            }
        );
        assert_eq!(state.tabs(), ["Home", "o/r#1"]);
    }

    #[test]
    fn history_folds_in_the_top_bar() {
        let mut state = state();
        for login in ["a", "b", "c", "d", "e"] {
            state.push(Route::User(login.into()));
        }
        assert_eq!(state.tabs(), ["…", "@b", "@c", "@d", "@e"]);
    }

    #[test]
    fn key_scopes_follow_the_screen() {
        let mut state = state();
        state.push(Route::Issues {
            repo: repo(),
            query: OPEN.into(),
        });
        press(&mut state, "f");
        assert!(
            matches!(state.overlay, Some(Overlay::Prompt(_))),
            "f filters on a page"
        );
        let help = state.help_entries();
        assert!(help.iter().any(|h| h.description == "Filter the list"));
        assert!(
            !help
                .iter()
                .any(|h| h.description == "Comment on the whole file")
        );
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
        mod better {
            use super::*;

            #[test]
            fn moves_are_detected_when_every_file_is_diffed() {
                let (mut s, pr) = diff_state(120);
                // The fixture has a loading file; finishing it triggers detection.
                let pending = s.diffs[&pr]
                    .doc
                    .files
                    .iter()
                    .position(|f| f.diff.is_none())
                    .unwrap();
                let cmds = update(
                    &mut s,
                    Msg::FileDiff(
                        pr.clone(),
                        pending,
                        Arc::new(FileDiff::compute(
                            "zz/pending.rs",
                            Some(b"a\n"),
                            Some(b"b\n"),
                        )),
                    ),
                );
                assert!(cmds.iter().any(
                    |c| matches!(c, Cmd::DetectMoves(p, files) if *p == pr && files.len() == 8)
                ));
                // Only once.
                let again = update(&mut s, Msg::MovesDetected(pr.clone(), Vec::new()));
                assert!(!again.iter().any(|c| matches!(c, Cmd::DetectMoves(..))));
                assert!(press(&mut s, "gm").is_empty());
                assert!(matches!(&s.notice, Some(Notice::Info(m)) if m.contains("moved")));
            }

            #[test]
            fn since_review_asks_github_then_compares() {
                let (mut s, pr) = diff_state(120);
                s.viewer = Some("me".into());
                let cmds = press(&mut s, "gl");
                assert!(matches!(&cmds[..], [Cmd::FetchLastReview { login, .. }] if login == "me"));
                let cmds = update(&mut s, Msg::LastReview(pr.clone(), Ok(Some("old".into()))));
                assert!(
                    matches!(&cmds[..], [Cmd::SinceReview { old_head, .. }] if old_head == "old")
                );

                // Nothing in the old diff matched: everything is new.
                update(
                    &mut s,
                    Msg::SinceReady(pr.clone(), "old".into(), Ok(Default::default())),
                );
                assert!(s.diffs[&pr].doc.since_active);
                assert_eq!(s.tabs().last().unwrap(), "Files · since your review");
                // Toggling back needs no new lookups.
                assert!(
                    press(&mut s, "gl")
                        .iter()
                        .all(|c| matches!(c, Cmd::Prioritize(..)))
                );
                assert!(!s.diffs[&pr].doc.since_active);
            }

            #[test]
            fn since_review_without_a_review_says_so() {
                let (mut s, pr) = diff_state(120);
                s.viewer = Some("me".into());
                press(&mut s, "gl");
                update(&mut s, Msg::LastReview(pr.clone(), Ok(None)));
                assert!(
                    matches!(&s.notice, Some(Notice::Info(m)) if m.contains("haven't reviewed"))
                );
                // Reviewing the current head means nothing's new.
                s.diffs.get_mut(&pr).unwrap().since_requested = true;
                update(&mut s, Msg::LastReview(pr.clone(), Ok(Some("h".into()))));
                assert!(matches!(&s.notice, Some(Notice::Info(m)) if m.contains("current head")));
            }

            #[test]
            fn commit_picker_loads_a_range_and_blocks_comments() {
                let (mut s, pr) = diff_state(120);
                // `ghtui pr` would have loaded metadata; the picker needs the base.
                s.prs.insert(
                    pr.clone(),
                    Remote::cached(Some(crate::snapshot_tests::pr_detail())),
                );
                assert!(matches!(&press(&mut s, "gc")[..], [Cmd::ListCommits(_)]));
                update(
                    &mut s,
                    Msg::CommitsListed(
                        pr.clone(),
                        Ok(vec![
                            ("a".repeat(40), "first".into()),
                            ("b".repeat(40), "second".into()),
                        ]),
                    ),
                );
                assert!(matches!(s.overlay, Some(Overlay::Commits(_))));
                // Mark the first commit, select the second: a range.
                press(&mut s, "jj<Space>j");
                let cmds = press(&mut s, "<Enter>");
                let [
                    Cmd::LoadDiff {
                        range: Some((from, to)),
                        ..
                    },
                ] = &cmds[..]
                else {
                    panic!("{cmds:?}")
                };
                assert_eq!(from, &format!("{}^", "a".repeat(40)));
                assert_eq!(to, &"b".repeat(40));
                assert_eq!(s.tabs().last().unwrap(), "Files · aaaaaaa..bbbbbbb");
                // Commenting needs the whole PR.
                press(&mut s, "c");
                assert!(s.overlay.is_none());
                assert!(matches!(&s.notice, Some(Notice::Error(m)) if m.contains("All changes")));
                // Back to everything.
                press(&mut s, "gc");
                let cmds = press(&mut s, "<Enter>");
                assert!(matches!(&cmds[..], [Cmd::LoadDiff { range: None, .. }]));
            }
        }

        mod review {
            use super::*;
            use crate::review::{ComposeTarget, EditPurpose, SubmitOutcome};
            use ghtui_api::model::{PatchFile, ReviewComment, ReviewEvent, ReviewThread, Side};

            fn thread(id: &str, line: Option<u32>, resolved: bool, outdated: bool) -> ReviewThread {
                ReviewThread {
                    id: id.into(),
                    path: "src/point.rs".into(),
                    side: Side::Right,
                    start_side: None,
                    line,
                    start_line: None,
                    original_line: Some(3),
                    original_start_line: None,
                    outdated,
                    resolved,
                    file_level: false,
                    can_reply: true,
                    can_resolve: true,
                    can_unresolve: true,
                    comments: vec![ReviewComment {
                        id: format!("{id}-c"),
                        author: "alice".into(),
                        body: format!("Thread {id}"),
                        created_at: "2026-10-03T09:00:00Z".into(),
                        url: String::new(),
                        original_commit: Some("abc".into()),
                        pending: false,
                    }],
                }
            }

            fn type_text(s: &mut State, text: &str) {
                for c in text.chars() {
                    update(
                        s,
                        Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)),
                    );
                }
            }

            fn ctrl(s: &mut State, c: char) -> Vec<Cmd> {
                update(
                    s,
                    Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)),
                )
            }

            fn to_line(s: &mut State, text: &str) {
                press(s, "/");
                press(s, &format!("{text}<Enter>"));
            }

            #[test]
            fn threads_show_navigate_and_resolve() {
                let (mut s, pr) = diff_state(120);
                update(
                    &mut s,
                    Msg::ThreadsLoaded(
                        pr.clone(),
                        Ok(vec![
                            thread("done", Some(1), true, false),
                            thread("open", Some(14), false, false),
                        ]),
                    ),
                );
                assert_eq!(s.diffs[&pr].doc.annotations.len(), 2);
                press(&mut s, "gg]c");
                let at = s.diffs[&pr].doc.annotation_at(screen(&s).cursor).unwrap();
                assert_eq!(
                    s.diffs[&pr].doc.annotations[at as usize].key,
                    AnnotationKey::Thread("open".into())
                );

                // Resolve: optimistic, rolled back on failure.
                let cmds = press(&mut s, "R");
                assert!(matches!(
                    &cmds[..],
                    [Cmd::SetResolved { resolved: true, .. }, ..]
                ));
                assert!(s.diffs[&pr].threads[1].resolved);
                update(
                    &mut s,
                    Msg::ResolvedSet {
                        pr: pr.clone(),
                        thread_id: "open".into(),
                        resolved: true,
                        result: Err(ApiError::Network("down".into())),
                    },
                );
                assert!(!s.diffs[&pr].threads[1].resolved);
                assert!(matches!(s.notice, Some(Notice::Error(_))));
            }

            #[test]
            fn comment_draft_edit_and_delete() {
                let (mut s, pr) = diff_state(120);
                to_line(&mut s, "origin");
                press(&mut s, "c");
                let Some(Overlay::Compose(compose)) = &s.overlay else {
                    panic!("no composer")
                };
                assert!(matches!(compose.target, ComposeTarget::Line { .. }));
                type_text(&mut s, "Use a constant");
                let cmds = ctrl(&mut s, 's');
                assert!(s.overlay.is_none());
                let Some(Cmd::SaveReview(_, review)) = cmds.first() else {
                    panic!("{cmds:?}")
                };
                assert_eq!(review.pending.len(), 1);
                let draft = &review.pending[0];
                assert_eq!((draft.line, draft.commit.as_str()), (Some(14), "h"));
                assert!(s.diffs[&pr].doc.annotations.iter().any(|a| a.is_draft()));

                // The cursor is on the commented line; Enter edits the draft.
                press(&mut s, "<Enter>");
                assert!(
                    matches!(&s.overlay, Some(Overlay::Compose(c)) if matches!(c.target, ComposeTarget::Draft { .. }))
                );
                type_text(&mut s, "!");
                ctrl(&mut s, 's');
                assert_eq!(s.diffs[&pr].review.pending[0].body, "Use a constant!");

                let cmds = press(&mut s, "D");
                assert!(matches!(&cmds[..], [Cmd::SaveReview(_, r)] if r.pending.is_empty()));
            }

            #[test]
            fn lines_outside_githubs_diff_become_file_comments() {
                let (mut s, pr) = diff_state(120);
                // GitHub's diff only covers line 1.
                update(
                    &mut s,
                    Msg::PatchesLoaded(
                        pr.clone(),
                        Ok(vec![PatchFile {
                            filename: "src/point.rs".into(),
                            previous_filename: None,
                            patch: Some("@@ -1 +1 @@\n-a\n+b".into()),
                        }]),
                    ),
                );
                to_line(&mut s, "origin");
                press(&mut s, "c");
                let Some(Overlay::Compose(compose)) = &s.overlay else {
                    panic!()
                };
                assert!(matches!(
                    &compose.target,
                    ComposeTarget::File {
                        reason: Some(_),
                        ..
                    }
                ));
            }

            #[test]
            fn selections_across_hunks_are_refused() {
                let (mut s, pr) = diff_state(120);
                update(
                    &mut s,
                    Msg::PatchesLoaded(
                        pr.clone(),
                        Ok(vec![PatchFile {
                            filename: "src/point.rs".into(),
                            previous_filename: None,
                            patch: Some("@@ -1,6 +1,6 @@\n@@ -10,4 +10,8 @@".into()),
                        }]),
                    ),
                );
                to_line(&mut s, "A point in 2D");
                press(&mut s, "V");
                to_line(&mut s, "origin");
                press(&mut s, "c");
                assert!(s.overlay.is_none());
                assert!(
                    matches!(&s.notice, Some(Notice::Error(m)) if m.contains("one hunk")),
                    "{:?}",
                    s.notice
                );
            }

            #[test]
            fn submit_with_rejection_then_fix_and_resubmit() {
                let (mut s, pr) = diff_state(120);
                to_line(&mut s, "origin");
                press(&mut s, "c");
                type_text(&mut s, "nit");
                ctrl(&mut s, 's');
                let id = s.diffs[&pr].review.pending[0].id;

                press(&mut s, "gr");
                assert!(matches!(s.overlay, Some(Overlay::Submit(_))));
                press(&mut s, "<Tab>");
                let cmds = ctrl(&mut s, 's');
                let Some(Cmd::SubmitReview { drafts, event, .. }) = cmds.first() else {
                    panic!("{cmds:?}")
                };
                assert_eq!((drafts.len(), *event), (1, ReviewEvent::Approve));

                update(
                    &mut s,
                    Msg::ReviewSubmitted(
                        pr.clone(),
                        SubmitOutcome {
                            rejected: vec![(id, "line must be part of the diff".into())],
                            ..SubmitOutcome::default()
                        },
                    ),
                );
                assert!(
                    matches!(&s.overlay, Some(Overlay::Submit(d)) if d.error.is_some() && !d.sending)
                );
                assert_eq!(
                    s.diffs[&pr].review.pending[0].error.as_deref(),
                    Some("line must be part of the diff")
                );

                // Make it a file comment and submit again.
                press(&mut s, "<Esc>");
                press(&mut s, "f");
                assert_eq!(s.diffs[&pr].review.pending[0].line, None);
                press(&mut s, "gr");
                ctrl(&mut s, 's');
                update(
                    &mut s,
                    Msg::ReviewSubmitted(
                        pr.clone(),
                        SubmitOutcome {
                            accepted: vec![id],
                            submitted: true,
                            ..SubmitOutcome::default()
                        },
                    ),
                );
                assert!(s.overlay.is_none());
                assert!(s.diffs[&pr].review.pending.is_empty());
                assert_eq!(s.diffs[&pr].review.last_reviewed_head.as_deref(), Some("h"));
            }

            #[test]
            fn request_changes_needs_a_summary() {
                let (mut s, _) = diff_state(120);
                press(&mut s, "gr");
                press(&mut s, "<Tab><Tab>");
                assert!(ctrl(&mut s, 's').is_empty());
                assert!(matches!(&s.overlay, Some(Overlay::Submit(d)) if d.error.is_some()));
            }

            #[test]
            fn replies_post_right_away() {
                let (mut s, pr) = diff_state(120);
                update(
                    &mut s,
                    Msg::ThreadsLoaded(pr.clone(), Ok(vec![thread("t", Some(14), false, false)])),
                );
                press(&mut s, "gg]c");
                press(&mut s, "a");
                type_text(&mut s, "Fixed");
                let cmds = ctrl(&mut s, 's');
                assert!(matches!(&cmds[..], [Cmd::Reply { body, .. }] if body == "Fixed"));
                update(
                    &mut s,
                    Msg::Replied(pr.clone(), Err(ApiError::Network("down".into()))),
                );
                assert!(
                    matches!(&s.overlay, Some(Overlay::Compose(c)) if c.error.is_some() && c.text() == "Fixed")
                );
                let cmds = update(&mut s, Msg::Replied(pr.clone(), Ok(())));
                assert!(s.overlay.is_none());
                assert_eq!(cmds, vec![Cmd::FetchThreads(pr)]);
            }

            #[test]
            fn suggestions_go_through_the_editor_with_a_preview() {
                let (mut s, _) = diff_state(120);
                to_line(&mut s, "origin");
                let cmds = press(&mut s, "S");
                let Some(Cmd::Edit { purpose, text }) = cmds.into_iter().next() else {
                    panic!()
                };
                assert!(text.contains("pub fn origin()"));
                update(
                    &mut s,
                    Msg::Edited(purpose, Ok("    pub fn zero() -> Self {\n".into())),
                );
                let Some(Overlay::Compose(compose)) = &s.overlay else {
                    panic!()
                };
                assert!(
                    compose
                        .text()
                        .starts_with("```suggestion\n    pub fn zero()")
                );
                let preview = compose.preview.as_ref().unwrap();
                assert_eq!(preview.suggested, ["    pub fn zero() -> Self {"]);
                assert_eq!(preview.original, ["    pub fn origin() -> Self {"]);
                // Unchanged text isn't a suggestion.
                s.overlay = None;
                let cmds = press(&mut s, "S");
                let Some(Cmd::Edit { purpose, text }) = cmds.into_iter().next() else {
                    panic!()
                };
                update(&mut s, Msg::Edited(purpose, Ok(text)));
                assert!(s.overlay.is_none());
                let _ = EditPurpose::Compose;
            }

            #[test]
            fn outdated_threads_are_mapped_forward() {
                let (mut s, pr) = diff_state(120);
                let cmds = update(
                    &mut s,
                    Msg::ThreadsLoaded(pr.clone(), Ok(vec![thread("old", None, false, true)])),
                );
                assert!(cmds.iter().any(|c| matches!(c, Cmd::MapOutdated { items, head, .. } if items.len() == 1 && head == "h")));
                let ann = &s.diffs[&pr].doc.annotations[0];
                assert!(
                    ann.outdated && ann.on_line().is_none(),
                    "unplaced until mapped"
                );
                update(
                    &mut s,
                    Msg::OutdatedMapped(pr.clone(), vec![("old".into(), Some(4))]),
                );
                let ann = &s.diffs[&pr].doc.annotations[0];
                assert_eq!((ann.on_line(), ann.moved), (Some(4), true));
            }
        }
    }
}
