//! Application state and the single update function. Input events and async
//! results arrive as [`Msg`]s; [`update`] mutates [`State`] and returns
//! [`Cmd`]s for the runtime to execute in the background.

use std::collections::HashMap;
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use ghtui_api::ApiError;
use ghtui_api::model::{Inbox, NodeId, PrDetail, PrRef, RepoId, ViewedFiles};
use ghtui_api::rate_limit::RateLimits;
use ghtui_diff::FileDiff;
use ghtui_store::ReviewState;
use ghtui_theme::{Bg, Theme};
use ghtui_ui::bars::Notice;
use ghtui_ui::diff_doc::Viewed;
use ghtui_ui::pages::PrTab;
use ghtui_ui::{Ctx, Icons};
use ratatui::layout::Rect;
use ratatui_textarea::TextArea;

use crate::browse::{self, Data, DataKey, Need, PageScreen};
use crate::diff_job::{JobId, JobMsg};
use crate::diff_screen::{self, DiffScreen, DiffState};
use crate::keymap::{Action, Key, Keymap, Resolution, Scope};
use crate::nav::{self, Hints, Menu, SearchBox, Visit};
use crate::picker::{self, Picker};
use crate::review::{
    self, Compose, ComposeTarget, EditPurpose, SubmitDialog, SubmitOutcome, on_compose_key,
    on_edited, on_submit_key, review_action,
};
use crate::route::{Route, Target};
use ghtui_api::model::{PatchFile, ReviewEvent, ReviewThread};
use ghtui_store::DraftComment;

#[derive(Debug)]
pub enum Msg {
    Key(KeyEvent),
    Resize(u16, u16),
    Viewer(Result<String, ApiError>),
    Inbox(Result<Inbox, ApiError>),
    Pr(PrRef, Box<Result<PrDetail, ApiError>>),
    /// Page data; with `cached_at` (when it was fetched), the cached copy
    /// shown while fetching.
    Fetched {
        key: DataKey,
        result: Result<Data, ApiError>,
        cached_at: Option<u64>,
    },
    /// The next page of a list.
    FetchedMore(DataKey, Result<Data, ApiError>),
    Commented(DataKey, Result<(), ApiError>),
    Starred {
        repo: RepoId,
        starred: bool,
        result: Result<(), ApiError>,
    },
    Mouse(MouseEvent),
    Timer(Timer),
    /// The search box's input settled; fetch live suggestions if it's
    /// still this.
    SuggestDue(String),
    Suggested(
        String,
        Result<Vec<ghtui_api::browse::RepoSummary>, ApiError>,
    ),
    RateLimits(RateLimits),
    Notice(Notice),
    /// A lasting problem started (`Some`) or cleared up (`None`).
    Problem(Problem, Option<String>),
    /// `$EDITOR` finished (or failed to start).
    Edited(EditPurpose, Result<String, String>),
    /// For a PR's diff screen.
    Diff(PrRef, DiffMsg),
}

/// What arrives for a PR's diff: from its job, GitHub, or the drafts.
#[derive(Debug)]
pub enum DiffMsg {
    /// From the diff job; ignored unless it's the PR's current job.
    Job(JobId, JobMsg),
    ViewedLoaded(Box<Result<ViewedFiles, ApiError>>),
    ViewedSaved {
        file: usize,
        previous: Viewed,
        result: Result<(), ApiError>,
    },
    /// The saved review state, or why it couldn't be read.
    ReviewLoaded(Result<ReviewState, String>),
    ThreadsLoaded(Result<Vec<ReviewThread>, ApiError>),
    PatchesLoaded(Result<Vec<PatchFile>, ApiError>),
    OutdatedMapped(Vec<(NodeId, Option<u32>)>),
    Replied(Result<(), ApiError>),
    ResolvedSet {
        thread_id: NodeId,
        resolved: bool,
        result: Result<(), ApiError>,
    },
    ReviewSubmitted(SubmitOutcome),
    LastReview(Result<Option<String>, ApiError>),
    SinceReady(String, Result<std::collections::HashSet<String>, String>),
    CommitsListed(Result<Vec<(String, String)>, String>),
}

/// What `update` asks the runtime to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cmd {
    /// A GitHub request.
    Api(Api),
    /// Work on a PR's diff (git and the diff engine).
    Git(Git),
    OpenUrl(String),
    /// Put text on the clipboard (OSC 52).
    Copy(String),
    /// Send [`Msg::Timer`] after this many milliseconds.
    Timer(Timer, u64),
    /// Send [`Msg::SuggestDue`] after a pause.
    SuggestLater(String),
    LoadReview(PrRef),
    SaveReview(PrRef, ReviewState),
    /// Suspend the TUI and edit `text` in `$EDITOR`.
    Edit {
        purpose: EditPurpose,
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Api {
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
        subject_id: NodeId,
        body: String,
        refresh: DataKey,
    },
    SetStarred {
        repo: RepoId,
        id: NodeId,
        starred: bool,
    },
    /// Search repositories for the search box's suggestions.
    Suggest(String),
    SaveVisits(Vec<Visit>),
    FetchViewed(PrRef),
    SetViewed {
        pr: PrRef,
        pull_request_id: NodeId,
        path: String,
        file: usize,
        viewed: bool,
        previous: Viewed,
    },
    FetchThreads(PrRef),
    FetchPatches(PrRef),
    Reply {
        pr: PrRef,
        thread_id: NodeId,
        body: String,
    },
    SetResolved {
        pr: PrRef,
        thread_id: NodeId,
        resolved: bool,
    },
    SubmitReview {
        pr: PrRef,
        head: String,
        drafts: Vec<DraftComment>,
        event: ReviewEvent,
        body: String,
    },
    FetchLastReview {
        pr: PrRef,
        login: String,
    },
}

/// An outdated thread to place on the current diff: where it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutdatedThread {
    pub thread: NodeId,
    pub path: String,
    pub commit: String,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Git {
    /// Start (or restart) the diff job for a PR.
    /// With `range`, diff `(from, to)` instead of the whole PR.
    LoadDiff {
        pr: PrRef,
        job: JobId,
        base_ref: String,
        range: Option<(String, String)>,
    },
    /// Diff these files next.
    Prioritize(PrRef, Vec<usize>),
    /// Look for moved code; answered as the job's [`JobMsg::Moves`].
    DetectMoves(PrRef, JobId, Vec<(usize, Arc<FileDiff>)>),
    MapOutdated {
        pr: PrRef,
        head: String,
        threads: Vec<OutdatedThread>,
    },
    /// Block hashes of the diff at `old_head`, for "since my last review".
    SinceReview {
        pr: PrRef,
        old_head: String,
    },
    ListCommits(PrRef),
}

/// Problems that last until they're fixed, shown in a banner above the
/// status bar instead of as a notice that expires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Problem {
    /// GitHub rejects the token: nothing loads until it's fixed.
    Auth,
    /// Review drafts can't be written to disk.
    Drafts,
}

/// Things that happen later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timer {
    /// Clear notice number N, if it's still showing.
    ExpireNotice(u64),
    /// The next frame of the loading spinner.
    Spin,
    /// Relative times ("3m ago") move on.
    Minute,
}

/// Braille spinner frames.
pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Data that comes from GitHub: what we have (possibly cached), whether a
/// refresh is in flight, and the last refresh error.
#[derive(Debug)]
pub struct Remote<T> {
    pub data: Option<T>,
    pub loading: bool,
    pub error: Option<String>,
    /// When `data` was fetched, while it's a copy from the cache.
    pub cached_at: Option<u64>,
}

impl<T> Default for Remote<T> {
    fn default() -> Self {
        Self {
            data: None,
            loading: false,
            error: None,
            cached_at: None,
        }
    }
}

impl<T> Remote<T> {
    pub fn cached(cached: Option<ghtui_store::Cached<T>>) -> Self {
        Self {
            cached_at: cached.as_ref().map(|c| c.fetched_at),
            data: cached.map(|c| c.value),
            ..Self::default()
        }
    }

    /// Starts a fetch, unless one is running or (without `force`) the
    /// data is good.
    fn begin(&mut self, force: bool) -> bool {
        if self.loading || (!force && self.data.is_some() && self.error.is_none()) {
            return false;
        }
        self.loading = true;
        true
    }

    fn finish(&mut self, result: Result<T, ApiError>) {
        self.loading = false;
        match result {
            Ok(data) => {
                self.data = Some(data);
                self.error = None;
                self.cached_at = None;
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

/// Navigation history, like a browser's. The first screen can't be
/// removed, so there's always one to show.
pub struct Screens {
    first: Screen,
    rest: Vec<Screen>,
}

impl Screens {
    fn new(first: Screen) -> Self {
        Self {
            first,
            rest: Vec::new(),
        }
    }

    pub fn last(&self) -> &Screen {
        self.rest.last().unwrap_or(&self.first)
    }

    pub fn last_mut(&mut self) -> &mut Screen {
        self.rest.last_mut().unwrap_or(&mut self.first)
    }

    pub fn push(&mut self, screen: Screen) {
        self.rest.push(screen);
    }

    /// The last screen, unless it's the first.
    pub fn pop(&mut self) -> Option<Screen> {
        self.rest.pop()
    }

    pub fn len(&self) -> usize {
        self.rest.len() + 1
    }

    /// Keeps the first `n` screens (at least one).
    #[cfg(test)]
    pub fn truncate(&mut self, n: usize) {
        self.rest.truncate(n.saturating_sub(1));
    }
}

pub enum Overlay {
    /// `/` prompt on the diff screen.
    DiffSearch(Box<TextArea<'static>>),
    /// The header's search box with its suggestions.
    Search(Box<SearchBox>),
    /// Letters over the links on screen.
    Hints(Box<Hints>),
    /// Everything you can do here.
    Menu(Box<Menu>),
    /// The command palette, go to file, branches, commits.
    Picker(Box<Picker>),
    /// Writing a comment, reply or suggestion.
    Compose(Box<Compose>),
    /// Submitting the review.
    Submit(Box<SubmitDialog>),
}

pub struct State {
    pub screens: Screens,
    /// Pages gone back from.
    pub forward: Vec<Screen>,
    /// Pages visited, for the search box.
    pub visits: Vec<Visit>,
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
    /// Notices shown this session, newest last, with when (Unix seconds).
    pub messages: std::collections::VecDeque<(u64, Notice)>,
    /// Lasting problems, most important first; the first is shown.
    pub problems: std::collections::BTreeMap<Problem, String>,
    /// Terminal size.
    pub size: (u16, u16),
    pub keymap: Keymap,
    pub theme: Theme,
    pub icons: Icons,
    pub density: crate::config::Density,
    /// Unix seconds; pages show relative times.
    pub clock: fn() -> u64,
    /// Which notice is showing, to expire the right one.
    pub notice_id: u64,
    /// The spinner's frame, and whether its next frame is scheduled.
    pub spinner: usize,
    pub spinning: bool,
    pub quit: bool,
}

impl State {
    pub fn new(theme: Theme, icons: Icons, keymap: Keymap, size: (u16, u16)) -> Self {
        let mut state = Self {
            screens: Screens::new(Screen::Page(Box::new(PageScreen::new(Route::Home)))),
            forward: Vec::new(),
            visits: Vec::new(),
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
            messages: Default::default(),
            problems: Default::default(),
            size,
            keymap,
            theme,
            icons,
            density: crate::config::Density::default(),
            clock: ghtui_store::now,
            notice_id: 0,
            spinner: 0,
            spinning: false,
            quit: false,
        };
        state.sync_page();
        state
    }

    /// Says `text` in the status bar for a few seconds.
    pub fn info(&mut self, text: impl Into<String>) {
        self.notice = Some(Notice::Info(text.into()));
    }

    /// Says what went wrong, in the status bar (and the messages list).
    pub fn error(&mut self, text: impl Into<String>) {
        self.notice = Some(Notice::Error(text.into()));
    }

    pub fn screen(&self) -> &Screen {
        self.screens.last()
    }

    pub fn screen_mut(&mut self) -> &mut Screen {
        self.screens.last_mut()
    }

    pub fn ctx(&self, now: u64) -> Ctx<'_> {
        Ctx {
            theme: &self.theme,
            icons: self.icons,
            now,
        }
    }

    /// The area between the chrome and the status bar.
    pub fn content_area(&self) -> Rect {
        self.layout().content
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

    /// Fetches a page's data. The repository header shared by a repo's
    /// pages is only fetched when missing.
    pub fn ensure_route(&mut self, route: &Route, force: bool) -> Vec<Cmd> {
        let mut cmds = Vec::new();
        for need in browse::needs(route) {
            let header = browse::is_header(route, &need);
            cmds.extend(self.ensure(need, force && !header));
        }
        cmds
    }

    pub fn ensure(&mut self, need: Need, force: bool) -> Vec<Cmd> {
        match need {
            Need::Inbox if self.inbox.begin(force) => vec![Cmd::Api(Api::FetchInbox)],
            Need::Pr(pr) => self.ensure_pr(&pr, force),
            Need::Data(key) => {
                let remote = self.data.entry(key.clone()).or_default();
                if !remote.begin(force) {
                    return Vec::new();
                }
                vec![Cmd::Api(Api::Fetch {
                    cached: remote.data.is_none(),
                    key,
                })]
            }
            Need::Inbox => Vec::new(),
        }
    }

    pub fn ensure_pr(&mut self, pr: &PrRef, force: bool) -> Vec<Cmd> {
        if self.prs.entry(pr.clone()).or_default().begin(force) {
            vec![Cmd::Api(Api::FetchPr(pr.clone()))]
        } else {
            Vec::new()
        }
    }

    /// Fetches needed for the visible screen. `force` refreshes data we
    /// already have.
    pub fn load_visible(&mut self, force: bool) -> Vec<Cmd> {
        let mut cmds = Vec::new();
        if self.viewer.is_none() && force {
            cmds.push(Cmd::Api(Api::FetchViewer));
        }
        // What's on screen, without cloning the screen (a page holds its
        // whole rendered content).
        enum Here {
            Page(Route),
            Diff(PrRef),
        }
        let here = match self.screen() {
            Screen::Page(p) => Here::Page(p.route.clone()),
            Screen::Diff(d) => Here::Diff(d.pr.clone()),
        };
        match here {
            Here::Page(route) => cmds.extend(self.ensure_route(&route, force)),
            Here::Diff(pr) => {
                cmds.extend(self.ensure_pr(&pr, false));
                if force || !self.diffs.contains_key(&pr) {
                    cmds.extend(self.start_diff(&pr));
                }
            }
        }
        cmds
    }

    /// Opens the diff of a PR (it starts once the PR's metadata is in).
    pub fn open_diff(&mut self, pr: PrRef) -> Vec<Cmd> {
        let reuse = self.diffs.get(&pr).is_some_and(|d| d.error.is_none());
        let cmds = if reuse {
            Vec::new()
        } else {
            self.start_diff(&pr)
        };
        let screen = DiffScreen::new(pr, self.size.0);
        self.screens.push(Screen::Diff(Box::new(screen)));
        cmds
    }

    /// The branch the PR merges into, once the PR has loaded.
    pub(crate) fn base_ref(&self, pr: &PrRef) -> Option<String> {
        let detail = self.prs.get(pr)?.data.as_ref()?;
        Some(detail.base_ref.clone())
    }

    fn start_diff(&mut self, pr: &PrRef) -> Vec<Cmd> {
        let Some(base_ref) = self.base_ref(pr) else {
            return Vec::new();
        };
        let diff = DiffState::loading();
        let job = diff.job;
        self.diffs.insert(pr.clone(), diff);
        vec![
            Cmd::Git(Git::LoadDiff {
                pr: pr.clone(),
                job,
                base_ref,
                range: None,
            }),
            Cmd::Api(Api::FetchViewed(pr.clone())),
            Cmd::LoadReview(pr.clone()),
            Cmd::Api(Api::FetchThreads(pr.clone())),
            Cmd::Api(Api::FetchPatches(pr.clone())),
        ]
    }

    /// Rebuilds the page if its data changed and keeps the page (or the
    /// diff) in bounds.
    pub fn settle(&mut self) -> Vec<Cmd> {
        self.sync_page();
        self.settle_diff()
    }

    /// Re-runs the diff screen's clamping and prioritization.
    pub fn settle_diff(&mut self) -> Vec<Cmd> {
        let content = self.content_area();
        match self.diff_parts() {
            Some((screen, diff)) => diff_screen::settle(screen, diff, content),
            None => Vec::new(),
        }
    }

    /// What's loading on the visible screen, for the status bar.
    pub fn busy(&self) -> Option<String> {
        match self.screen() {
            Screen::Page(p) if self.page_loading(&p.route) => {
                // Cached data on screen says how old it is.
                Some(match self.page_cached_at(&p.route) {
                    Some(at) => {
                        let ago = ghtui_ui::time::ago(at, (self.clock)());
                        format!("Refreshing · cached {ago}")
                    }
                    None => "Loading".to_owned(),
                })
            }
            Screen::Page(_) => None,
            Screen::Diff(screen) => match self.diffs.get(&screen.pr) {
                Some(diff) => diff.status(),
                None => Some(format!("Loading {}", screen.pr)),
            },
        }
    }
}

/// Handles a message and settles the screen.
#[cfg(test)]
pub fn update(state: &mut State, msg: Msg) -> Vec<Cmd> {
    let mut cmds = apply_msg(state, msg);
    cmds.extend(state.settle());
    cmds
}

/// Handles a message without settling the screen: the runtime handles a
/// burst of background results, then settles once.
pub fn apply_msg(state: &mut State, msg: Msg) -> Vec<Cmd> {
    // Pages are built from GitHub's data; only these change it.
    if matches!(
        msg,
        Msg::Viewer(_)
            | Msg::Inbox(_)
            | Msg::Pr(..)
            | Msg::Fetched { .. }
            | Msg::FetchedMore(..)
            | Msg::Commented(..)
            | Msg::Starred { .. }
    ) {
        state.data_gen += 1;
    }
    let mut cmds = handle(state, msg);
    // Until a PR's saved review is read, saving would overwrite it.
    cmds.retain(|cmd| match cmd {
        Cmd::SaveReview(pr, _) => state.diffs.get(pr).is_some_and(|d| d.review_loaded),
        _ => true,
    });
    cmds
}

/// Timers the screen needs after an update: the notice expiring (errors
/// stay longer), the spinner's next frame while something loads. The
/// runtime calls this; `last_notice` is the notice it last saw.
pub fn timers(state: &mut State, last_notice: &mut Option<Notice>) -> Vec<Cmd> {
    let mut cmds = Vec::new();
    if state.notice != *last_notice {
        if let Some(notice) = &state.notice {
            const KEPT: usize = 50;
            if state.messages.len() == KEPT {
                state.messages.pop_front();
            }
            state.messages.push_back(((state.clock)(), notice.clone()));
            state.notice_id += 1;
            let ms = match notice {
                Notice::Info(_) => 4_000,
                Notice::Error(_) => 10_000,
            };
            cmds.push(Cmd::Timer(Timer::ExpireNotice(state.notice_id), ms));
        }
        *last_notice = state.notice.clone();
    }
    if state.busy().is_some() && !state.spinning {
        state.spinning = true;
        cmds.push(Cmd::Timer(Timer::Spin, 90));
    }
    cmds
}

fn handle(state: &mut State, msg: Msg) -> Vec<Cmd> {
    match msg {
        Msg::Diff(pr, msg) => diff_screen::update(state, pr, msg),
        Msg::Key(key) => on_key(state, key),
        Msg::Resize(w, h) => {
            state.size = (w, h);
            Vec::new()
        }
        Msg::Viewer(Ok(login)) => {
            state.viewer = Some(login);
            Vec::new()
        }
        Msg::Viewer(Err(err)) => {
            tracing::warn!(%err, "could not fetch viewer");
            Vec::new()
        }
        Msg::Inbox(result) => {
            if let Err(err) = &result {
                tracing::warn!(%err, "inbox fetch failed");
            }
            state.inbox.finish(result);
            Vec::new()
        }
        Msg::Pr(pr, result) => {
            if let Err(err) = result.as_ref() {
                tracing::warn!(%pr, %err, "PR fetch failed");
            }
            state.prs.entry(pr.clone()).or_default().finish(*result);
            // The diff was opened before the PR's metadata arrived.
            match state.screen() {
                Screen::Diff(screen) if screen.pr == pr && !state.diffs.contains_key(&pr) => {
                    state.start_diff(&pr)
                }
                _ => Vec::new(),
            }
        }
        Msg::Fetched {
            key,
            result,
            cached_at,
        } => {
            if let Err(err) = &result {
                tracing::warn!(?key, %err, "fetch failed");
            }
            let is_pr = matches!(result, Ok(Data::Issue(None)));
            let remote = state.data.entry(key.clone()).or_default();
            match cached_at {
                None => remote.finish(result),
                Some(at) => {
                    if remote.data.is_none()
                        && let Ok(data) = result
                    {
                        remote.data = Some(data);
                        remote.cached_at = Some(at);
                    }
                }
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
                    state.error(format!("Couldn't load more: {err}"));
                }
            }
            Vec::new()
        }
        Msg::Commented(key, Ok(())) => {
            state.overlay = None;
            state.info("Comment posted");
            state.ensure(Need::Data(key), true)
        }
        Msg::Commented(_, Err(err)) => {
            if let Some(Overlay::Compose(compose)) = &mut state.overlay {
                compose.sending = false;
                compose.error = Some(err.to_string());
            }
            Vec::new()
        }
        Msg::Starred {
            repo,
            starred,
            result,
        } => {
            match result {
                Ok(()) => {
                    let verb = if starred { "Starred" } else { "Unstarred" };
                    state.info(format!("{verb} {repo}"));
                }
                Err(err) => {
                    nav::set_starred(state, &repo, !starred);
                    state.error(format!("GitHub didn't save that: {err}"));
                }
            }
            Vec::new()
        }
        Msg::Mouse(ev) => nav::on_mouse(state, ev),
        Msg::Timer(Timer::ExpireNotice(id)) => {
            if id == state.notice_id {
                state.notice = None;
            }
            Vec::new()
        }
        Msg::Timer(Timer::Spin) => {
            state.spinning = false;
            state.spinner = state.spinner.wrapping_add(1);
            Vec::new()
        }
        Msg::Timer(Timer::Minute) => {
            state.data_gen += 1;
            vec![Cmd::Timer(Timer::Minute, 60_000)]
        }
        Msg::SuggestDue(q) => match &state.overlay {
            Some(Overlay::Search(sb)) if sb.input.lines().join("").trim() == q => {
                vec![Cmd::Api(Api::Suggest(q))]
            }
            _ => Vec::new(),
        },
        Msg::Suggested(q, result) => {
            if let (Some(Overlay::Search(sb)), Ok(repos)) = (&mut state.overlay, result) {
                sb.remote = repos;
                sb.remote_for = q;
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
        Msg::Problem(problem, Some(text)) => {
            state.problems.insert(problem, text);
            Vec::new()
        }
        Msg::Problem(problem, None) => {
            state.problems.remove(&problem);
            Vec::new()
        }
        Msg::Edited(purpose, result) => on_edited(state, purpose, result),
    }
}

fn on_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        state.quit = true;
        return Vec::new();
    }
    match &mut state.overlay {
        Some(Overlay::Picker(_)) => return picker::on_key(state, key),
        Some(Overlay::DiffSearch(_)) => return on_search_key(state, key),
        Some(Overlay::Search(_)) => return nav::on_search_box_key(state, key),
        Some(Overlay::Hints(_)) => return nav::on_hints_key(state, key),
        Some(Overlay::Menu(_)) => return nav::on_menu_key(state, key),
        Some(Overlay::Compose(_)) => return on_compose_key(state, key),
        Some(Overlay::Submit(_)) => return on_submit_key(state, key),
        None => {}
    }
    // Any keypress dismisses the last notice; Esc on an error does only
    // that (the messages list keeps it).
    let dismissing = matches!(state.notice, Some(Notice::Error(_)));
    state.notice = None;
    if dismissing && key.code == KeyCode::Esc && state.pending.is_empty() {
        return Vec::new();
    }
    state.pending.push(Key::from(key));
    match state.keymap.resolve(&state.pending, state.scope()) {
        Resolution::Action(action) => {
            state.pending.clear();
            apply(state, action)
        }
        Resolution::Pending => Vec::new(),
        Resolution::Unbound => {
            // A key that works elsewhere says where.
            let other = match state.scope() {
                Scope::Page => Scope::Diff,
                _ => Scope::Page,
            };
            if let Resolution::Action(action) = state.keymap.resolve(&state.pending, other) {
                let place = match other {
                    Scope::Diff => "in a pull request's Files changed tab",
                    _ => "on pages, not in the diff",
                };
                let what = action.description();
                state.info(format!("{what}: {place}"));
            }
            state.pending.clear();
            Vec::new()
        }
    }
}

fn on_search_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    let Some(Overlay::DiffSearch(input)) = &mut state.overlay else {
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
            if let Some((screen, diff)) = state.diff_parts() {
                let found = diff_screen::search(screen, diff, query);
                state.notice = Some(found);
            }
            Vec::new()
        }
        _ => {
            input.input(key);
            Vec::new()
        }
    }
}

pub fn apply(state: &mut State, action: Action) -> Vec<Cmd> {
    let content = state.content_area();
    // In the diff, a few keys mean their nearest thing there.
    let mut action = action;
    if let Screen::Diff(screen) = state.screen() {
        let searching = screen.search.is_some();
        match action {
            Action::PageDown | Action::PageUp => {
                let half = if action == Action::PageDown {
                    Action::HalfPageDown
                } else {
                    Action::HalfPageUp
                };
                let mut cmds = apply(state, half);
                cmds.extend(apply(state, half));
                return cmds;
            }
            Action::UpLevel => return nav::switch_tab(state, 1),
            // While searching, n and p go through the matches.
            Action::NextHunk if searching => action = Action::SearchNext,
            Action::PrevHunk if searching => action = Action::SearchPrev,
            // Esc ends a search before it leaves.
            Action::Back if searching => {
                if let Screen::Diff(screen) = state.screen_mut() {
                    screen.search = None;
                }
                state.info("Search cleared");
                return Vec::new();
            }
            _ => {}
        }
    }
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
        if let Screen::Diff(screen) = screens.last_mut()
            && let Some(diff) = diffs.get_mut(&screen.pr)
            && let Some(cmds) = diff_screen::apply(screen, diff, action, content, notice)
        {
            return cmds;
        }
    }
    if let Some(cmds) = nav::page_action(state, action) {
        return cmds;
    }
    match action {
        Action::Quit => state.quit = true,
        Action::Back => return state.back(),
        Action::Forward => return state.go_forward(),
        Action::Menu => nav::open_menu(state),
        Action::CommandPalette => return state.open_picker(picker::Kind::Commands),
        Action::Messages => return state.open_picker(picker::Kind::Messages),
        Action::Refresh => return state.load_visible(true),
        Action::Copy => return nav::copy_link(state),
        Action::OpenInBrowser => return state.go(Target::External(state.here_url())),
        Action::Search => {
            if let Screen::Diff(_) = state.screen() {
                let input = nav::new_input(&state.theme, Bg::Container);
                state.overlay = Some(Overlay::DiffSearch(Box::new(input)));
            }
        }
        Action::FindFile => {
            if let Screen::Diff(_) = state.screen() {
                return state.open_picker(picker::Kind::DiffFiles);
            }
        }
        Action::GoHome => {
            if state.route() != Some(&Route::Home) {
                return state.push(Route::Home);
            }
        }
        Action::Tab1 => return nav::switch_tab(state, 1),
        Action::Tab2 => return nav::switch_tab(state, 2),
        Action::Tab3 => return nav::switch_tab(state, 3),
        Action::Tab4 => return nav::switch_tab(state, 4),
        _ => {}
    }
    Vec::new()
}

// ---- reviewing ---------------------------------------------------------------

impl State {
    /// Asks the diff job to map outdated threads, once both the threads and
    /// the diff's head are known.
    pub(crate) fn map_outdated(&mut self, pr: &PrRef) -> Vec<Cmd> {
        let Some(diff) = self.diffs.get_mut(pr) else {
            return Vec::new();
        };
        let Some(head) = diff.head() else {
            return Vec::new();
        };
        if diff.mapping_requested {
            return Vec::new();
        }
        let threads = review::outdated_to_map(&diff.threads);
        if threads.is_empty() {
            return Vec::new();
        }
        diff.mapping_requested = true;
        vec![Cmd::Git(Git::MapOutdated {
            pr: pr.clone(),
            head,
            threads,
        })]
    }

    /// Opens the composer on `target`, starting with `text`.
    pub fn compose(&mut self, target: ComposeTarget, text: &str) -> Vec<Cmd> {
        let compose = Compose::new(&self.theme, target, text);
        self.overlay = Some(Overlay::Compose(Box::new(compose)));
        Vec::new()
    }

    /// The diff on screen, if it's one.
    pub fn diff(&self) -> Option<&DiffState> {
        match self.screen() {
            Screen::Diff(screen) => self.diffs.get(&screen.pr),
            Screen::Page(_) => None,
        }
    }

    pub(crate) fn diff_parts(&mut self) -> Option<(&mut DiffScreen, &mut DiffState)> {
        let State { screens, diffs, .. } = self;
        let Screen::Diff(screen) = screens.last_mut() else {
            return None;
        };
        let diff = diffs.get_mut(&screen.pr)?;
        Some((screen, diff))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::fixtures::press;
    use crate::picker::{Choice, fuzzy_score};
    use crate::route::OPEN;
    use ghtui_api::browse::SearchResults;
    use ghtui_api::model::{PrState, PrSummary};
    use ghtui_theme::{ColorDepth, DEFAULT_SEED, Mode};
    use ghtui_ui::annotations::AnnotationKey;
    use ghtui_ui::overlays::PALETTE_ROWS;

    pub(crate) fn state() -> State {
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

    /// Runs an action that has no default key (it's in the menu).
    fn act(state: &mut State, action: Action) -> Vec<Cmd> {
        let mut cmds = apply(state, action);
        state.sync_page();
        cmds.extend(state.settle_diff());
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

    /// The text of the selected row's first line.
    fn selected_text(state: &State) -> String {
        let p = page(state);
        let item = p.page.items[p.selected.expect("a selection")];
        p.page.lines[item.start].text()
    }

    fn fetched(state: &mut State, key: DataKey, data: Data) -> Vec<Cmd> {
        update(
            state,
            Msg::Fetched {
                key,
                result: Ok(data),
                cached_at: None,
            },
        )
    }

    /// Commands other than saving visits.
    fn fetches(cmds: Vec<Cmd>) -> Vec<Cmd> {
        cmds.into_iter()
            .filter(|c| !matches!(c, Cmd::Api(Api::SaveVisits(_))))
            .collect()
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

    fn click(state: &mut State, x: u16, y: u16) -> Vec<Cmd> {
        update(
            state,
            Msg::Mouse(MouseEvent {
                kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
                column: x,
                row: y,
                modifiers: KeyModifiers::NONE,
            }),
        )
    }

    /// Where `text` is drawn on the page.
    fn find(state: &State, text: &str) -> (u16, u16) {
        let p = page(state);
        let area = state.page_area();
        let cols = ghtui_ui::page::columns(&p.page, area);
        for (row, line) in p.page.lines.iter().enumerate().skip(p.scroll) {
            let at = line.text().find(text);
            if let Some(at) = at {
                let x = cols.main_x + 2 + line.indent + u16::try_from(at).unwrap();
                return (x, area.y + u16::try_from(row - p.scroll).unwrap());
            }
        }
        panic!("{text} isn't on screen")
    }

    #[test]
    fn home_selects_the_first_row_and_opens_it() {
        let mut state = with_inbox(3);
        assert_eq!(route(&state), Route::Home);
        assert!(
            selected_text(&state).contains("PR 1"),
            "a fresh page selects its first row"
        );
        press(&mut state, "j");
        assert!(selected_text(&state).contains("PR 2"));
        press(&mut state, "k");
        let cmds = fetches(press(&mut state, "<Enter>"));
        let pr = PrRef::parse("o/r#1").unwrap();
        assert_eq!(
            cmds,
            vec![
                Cmd::Api(Api::FetchPr(pr.clone())),
                Cmd::Api(Api::Fetch {
                    key: DataKey::PrActivity(pr.clone()),
                    cached: true
                })
            ]
        );
        assert_eq!(
            route(&state),
            Route::Pr {
                pr: pr.clone(),
                tab: PrTab::Conversation
            }
        );
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
            vec![
                Cmd::Api(Api::FetchViewer),
                Cmd::Api(Api::FetchPr(pr.clone()))
            ],
            "the activity is still loading"
        );

        press(&mut state, "<Esc>");
        assert_eq!(route(&state), Route::Home);
        assert!(
            selected_text(&state).contains("PR 1"),
            "the selection survives"
        );
        act(&mut state, Action::Forward);
        assert_eq!(
            route(&state),
            Route::Pr {
                pr,
                tab: PrTab::Conversation
            },
            "forward again"
        );
        press(&mut state, "h");
        assert!(!state.quit);
        press(&mut state, "q");
        assert!(state.quit);
    }

    #[test]
    fn j_moves_between_rows_and_scrolls_through_text() {
        let mut state = with_repo();
        state.size = (100, 20);
        update(&mut state, Msg::Resize(100, 20));
        let first = selected_text(&state);
        assert!(first.contains(".github"), "{first}");
        // Through the files...
        for _ in 0..3 {
            press(&mut state, "j");
        }
        assert!(selected_text(&state).contains("README.md"));
        // ...then the README scrolls, and the last row leaves the screen.
        for _ in 0..12 {
            press(&mut state, "j");
        }
        let p = page(&state);
        assert!(p.scroll > 0);
        assert_eq!(p.selected, None, "nothing selected while reading");
        press(&mut state, "G");
        let p = page(&state);
        assert_eq!(p.scroll, p.page.height() - state.page_height());
        // Back up: rows get selected again once they're near.
        for _ in 0..20 {
            press(&mut state, "k");
        }
        assert!(page(&state).selected.is_some());
        press(&mut state, "g");
        assert_eq!(page(&state).scroll, 0);
        assert_eq!(page(&state).selected, Some(0));
    }

    #[test]
    fn h_l_and_u_move_like_a_file_manager() {
        let mut state = with_repo();
        press(&mut state, "j<Enter>");
        let crates = Route::Tree {
            repo: repo(),
            rev: "main".into(),
            path: "crates".into(),
        };
        assert_eq!(route(&state), crates, "l opens");
        press(&mut state, "u");
        assert_eq!(route(&state), Route::Repo(repo()), "u: the folder above");
        press(&mut state, "<Esc>");
        assert_eq!(route(&state), crates, "esc: back");
        press(&mut state, "uu");
        assert_eq!(
            route(&state),
            Route::user("gold-silver-copper"),
            "up from a repository: its owner"
        );
        press(&mut state, "<Esc><Esc>");
        assert_eq!(route(&state), crates);
    }

    #[test]
    fn arrows_move_comment_by_comment_and_enter_quotes() {
        let mut state = state();
        update(&mut state, Msg::Resize(100, 14));
        state.push(Route::Issue {
            repo: repo(),
            number: 14,
        });
        let mut issue = crate::fixtures::issue();
        issue.body = (1..=30).map(|n| format!("Line {n}.\n\n")).collect();
        fetched(
            &mut state,
            DataKey::Issue(repo(), 14),
            Data::Issue(Some(Box::new(issue))),
        );
        assert_eq!(
            page(&state).selected,
            None,
            "reading pages start unselected"
        );
        press(&mut state, "<Down>");
        assert_eq!(page(&state).selected, Some(0), "the first comment");
        // The long first comment scrolls before the next one is selected.
        let scroll = page(&state).scroll;
        press(&mut state, "<Down>");
        assert_eq!(page(&state).selected, Some(0));
        assert!(page(&state).scroll > scroll);
        for _ in 0..40 {
            press(&mut state, "<Down>");
        }
        assert_eq!(page(&state).selected, Some(2), "the comment box at the end");
        press(&mut state, "<Up>");
        assert_eq!(page(&state).selected, Some(1), "hubot's comment");
        press(&mut state, "<Enter>");
        let Some(Overlay::Compose(compose)) = &state.overlay else {
            panic!("no composer")
        };
        assert!(
            compose.text().starts_with("> @hubot wrote:\n> Agreed."),
            "{}",
            compose.text()
        );
    }

    #[test]
    fn duplicate_fetches_are_suppressed() {
        let mut state = state();
        assert_eq!(
            state.load_visible(false),
            vec![
                Cmd::Api(Api::FetchInbox),
                Cmd::Api(Api::Fetch {
                    key: DataKey::ViewerRepos,
                    cached: true
                })
            ]
        );
        assert_eq!(state.load_visible(true), vec![Cmd::Api(Api::FetchViewer)]);
    }

    #[test]
    fn cached_data_shows_while_fetching() {
        let mut state = state();
        state.push(Route::user("octocat"));
        update(
            &mut state,
            Msg::Fetched {
                key: DataKey::Profile("octocat".into()),
                result: Ok(Data::Profile(Box::new(crate::fixtures::profile()))),
                cached_at: Some(0),
            },
        );
        assert!(page(&state).page.lines[0].text().contains("The Octocat"));
        let busy = state.busy().unwrap_or_default();
        assert!(busy.starts_with("Refreshing · cached "), "{busy}");
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
    fn browser_and_copy_use_the_selection_or_the_page() {
        let mut state = with_inbox(1);
        assert_eq!(
            press(&mut state, "o"),
            vec![Cmd::OpenUrl("https://github.com/o/r/pull/1".into())]
        );
        assert_eq!(
            press(&mut state, "y"),
            vec![Cmd::Copy("https://github.com/o/r/pull/1".into())]
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
        assert!(matches!(state.overlay, Some(Overlay::Picker(_))));
        let cmds = fetches(press(&mut state, "a/b#9<Enter>"));
        let key = DataKey::Issue(RepoId::new("a", "b"), 9);
        assert_eq!(
            cmds,
            vec![
                Cmd::Api(Api::Fetch {
                    key: DataKey::Repo(RepoId::new("a", "b")),
                    cached: true
                }),
                Cmd::Api(Api::Fetch {
                    key: key.clone(),
                    cached: true
                })
            ]
        );
        assert!(state.overlay.is_none());

        // The number is a pull request: GitHub's redirect.
        let cmds = update(
            &mut state,
            Msg::Fetched {
                key,
                result: Ok(Data::Issue(None)),
                cached_at: None,
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
        assert_eq!(cmds[0], Cmd::Api(Api::FetchPr(pr)));
        assert_eq!(state.screens.len(), 2, "replaced, not pushed");

        // A bare number resolves against the repo on screen.
        press(&mut state, ":12<Enter>");
        assert_eq!(
            route(&state),
            Route::Issue {
                repo: RepoId::new("a", "b"),
                number: 12
            }
        );

        press(&mut state, ":@octocat<Enter>");
        assert_eq!(route(&state), Route::user("octocat"));

        press(&mut state, ":actions menu<Enter>");
        assert!(matches!(state.overlay, Some(Overlay::Menu(_))));
        press(&mut state, "<Esc>");
        assert!(state.overlay.is_none());
        press(&mut state, "<C-k>");
        assert!(
            matches!(state.overlay, Some(Overlay::Picker(_))),
            "GitHub's palette key"
        );
    }

    #[test]
    fn palette_searches_github() {
        let state = state();
        let choices: Vec<Choice> = state
            .commands("ratatui widgets")
            .into_iter()
            .filter_map(|(_, c)| c)
            .collect();
        assert!(choices.contains(&Choice::Search("ratatui widgets".into())));
        assert_eq!(
            state.commands("ratatui/ratatui")[0].1,
            Some(Choice::Go(Target::Page(Route::Repo(RepoId::new(
                "ratatui", "ratatui"
            )))))
        );
    }

    #[test]
    fn palette_fuzzy_matches() {
        let state = state();
        let labels: Vec<String> = state
            .commands("open on")
            .into_iter()
            .map(|(item, _)| item.label)
            .collect();
        assert_eq!(labels.first().map(String::as_str), Some("Open on GitHub"));
        assert!(fuzzy_score("xyz", "Refresh").is_none());
        assert!(fuzzy_score("rfr", "Refresh").is_some());
    }

    #[test]
    fn pending_keys_show_what_can_follow() {
        // Defaults are single keys; sequences only come from config.
        let mut state = with_inbox(1);
        state.keymap = Keymap::with_overrides(&std::collections::HashMap::from([
            ("tab_2".to_owned(), vec!["zi".to_owned()]),
            ("tab_3".to_owned(), vec!["zp".to_owned()]),
        ]))
        .unwrap();
        press(&mut state, "z");
        assert_eq!(state.pending.len(), 1);
        let next: Vec<String> = state.continuations().into_iter().map(|r| r.key).collect();
        assert_eq!(next, ["i", "p"]);
        press(&mut state, "x");
        assert!(state.pending.is_empty());
        assert!(state.continuations().is_empty());
    }

    #[test]
    fn palette_list_stays_in_bounds() {
        let mut state = state();
        press(&mut state, ":");
        for _ in 0..(state.commands("").len() + PALETTE_ROWS) {
            press(&mut state, "<Down>");
        }
        let Some(Overlay::Picker(palette)) = &state.overlay else {
            panic!()
        };
        assert_eq!(palette.selected, state.commands("").len() - 1);
    }

    #[test]
    fn repo_tabs_filters_state_sort_and_star() {
        let mut state = with_repo();
        let tabs: Vec<String> = state
            .chrome()
            .tabs
            .iter()
            .map(|(t, _)| t.label.clone())
            .collect();
        assert_eq!(tabs, ["Code", "Issues", "Pull requests", "Actions"]);
        let cmds = fetches(press(&mut state, "2"));
        let issues = Route::Issues {
            repo: repo(),
            query: OPEN.into(),
        };
        assert_eq!(route(&state), issues);
        let (kind, query) = issues.search().unwrap();
        assert_eq!(
            cmds,
            vec![Cmd::Api(Api::Fetch {
                key: DataKey::Search(kind, query),
                cached: true
            })],
            "the header isn't fetched again"
        );
        assert_eq!(state.chrome().active, Some(1));

        // `/` on a list edits its filter.
        press(&mut state, "/");
        let Some(Overlay::Search(sb)) = &state.overlay else {
            panic!("no search box")
        };
        assert!(sb.filter);
        assert_eq!(sb.input.lines().join(""), "is:open ");
        press(&mut state, "label:bug<Enter>");
        assert_eq!(
            route(&state),
            Route::Issues {
                repo: repo(),
                query: "is:open label:bug".into()
            }
        );
        act(&mut state, Action::ToggleState);
        assert_eq!(
            route(&state),
            Route::Issues {
                repo: repo(),
                query: "is:closed label:bug".into()
            }
        );
        act(&mut state, Action::Sort);
        assert_eq!(
            route(&state),
            Route::Issues {
                repo: repo(),
                query: "is:closed label:bug sort:created-asc".into()
            }
        );

        press(&mut state, "<Right>");
        assert!(
            matches!(route(&state), Route::Pulls { .. }),
            "→: the next tab"
        );
        press(&mut state, "1");
        assert_eq!(route(&state), Route::Repo(repo()));

        let cmds = press(&mut state, "s");
        assert_eq!(
            cmds,
            vec![Cmd::Api(Api::SetStarred {
                repo: repo(),
                id: NodeId::new("R_ghtui"),
                starred: true
            })]
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
    fn letter_hints_follow_links() {
        let mut state = with_repo();
        press(&mut state, "l");
        let Some(Overlay::Hints(hints)) = &state.overlay else {
            panic!("no hints")
        };
        let i = hints
            .links
            .iter()
            .position(|u| u.url() == Some("https://github.com/gold-silver-copper"))
            .expect("the owner is a link");
        let label = hints.labels[i].label.clone();
        press(&mut state, &label);
        assert!(state.overlay.is_none());
        assert_eq!(route(&state), Route::user("gold-silver-copper"));
        // A wrong letter cancels.
        press(&mut state, "<Esc>l");
        press(&mut state, "<Esc>");
        assert!(state.overlay.is_none());
    }

    #[test]
    fn directories_and_files_open_from_the_repo() {
        let mut state = with_repo();
        while !selected_text(&state).contains("crates") {
            press(&mut state, "j");
        }
        let cmds = fetches(press(&mut state, "<Enter>"));
        let tree = DataKey::Tree(repo(), "main".into(), "crates".into());
        assert_eq!(
            cmds,
            vec![
                Cmd::Api(Api::Fetch {
                    key: tree.clone(),
                    cached: true
                }),
                Cmd::Api(Api::Fetch {
                    key: DataKey::LastCommits(repo(), "main".into(), "crates".into()),
                    cached: true
                })
            ],
            "the listing, and each entry's latest commit"
        );
        fetched(&mut state, tree, Data::Tree(crate::fixtures::tree()));
        while !selected_text(&state).contains("README.md") {
            press(&mut state, "j");
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
    fn go_to_file_and_switch_branches() {
        let mut state = with_repo();
        let cmds = press(&mut state, "f");
        let files = DataKey::Files(repo(), "main".into());
        assert_eq!(
            cmds,
            vec![Cmd::Api(Api::Fetch {
                key: files.clone(),
                cached: true
            })]
        );
        fetched(
            &mut state,
            files,
            Data::Files(
                Arc::new(vec![
                    "src/main.rs".into(),
                    "crates/ui/src/main_view.rs".into(),
                    "README.md".into(),
                ]),
                false,
            ),
        );
        press(&mut state, "main");
        let Some(Overlay::Picker(f)) = &state.overlay else {
            panic!("no finder")
        };
        let rows: Vec<String> = state
            .picker_rows(f)
            .into_iter()
            .map(|(i, _)| i.label)
            .collect();
        assert_eq!(rows, ["src/main.rs", "crates/ui/src/main_view.rs"]);
        press(&mut state, "<Enter>");
        assert_eq!(
            route(&state),
            Route::Blob {
                repo: repo(),
                rev: "main".into(),
                path: "src/main.rs".into()
            }
        );
        let cmds = press(&mut state, "b");
        assert_eq!(
            cmds,
            vec![Cmd::Api(Api::Fetch {
                key: DataKey::Refs(repo()),
                cached: true
            })]
        );
        fetched(
            &mut state,
            DataKey::Refs(repo()),
            Data::Refs(Box::new(ghtui_api::browse::Refs {
                branches: vec!["main".into(), "next".into()],
                tags: vec!["v1.0".into()],
            })),
        );
        press(&mut state, "next<Enter>");
        assert_eq!(
            route(&state),
            Route::Blob {
                repo: repo(),
                rev: "next".into(),
                path: "src/main.rs".into()
            },
            "the same file on the other branch"
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
            vec![Cmd::Api(Api::AddComment {
                subject_id: NodeId::new("I_14"),
                body: "Thanks!".into(),
                refresh: key.clone()
            })]
        );
        let cmds = update(&mut state, Msg::Commented(key.clone(), Ok(())));
        assert!(state.overlay.is_none());
        assert_eq!(cmds, vec![Cmd::Api(Api::Fetch { key, cached: false })]);
    }

    #[test]
    fn search_box_jumps_searches_and_loads_more() {
        let mut state = with_repo();
        press(&mut state, "/");
        let Some(Overlay::Search(sb)) = &state.overlay else {
            panic!("no search box")
        };
        assert!(!sb.filter);
        press(&mut state, "bug");
        let Some(Overlay::Search(sb)) = &state.overlay else {
            panic!()
        };
        let rows: Vec<Option<nav::Pick>> =
            state.suggestions(sb).into_iter().map(|(_, p)| p).collect();
        assert_eq!(
            rows.iter().flatten().next(),
            Some(&nav::Pick::Search(
                ghtui_api::browse::SearchKind::Issues,
                "repo:gold-silver-copper/ghtui bug".into()
            )),
            "on a repository, its own issues first"
        );
        // Live suggestions arrive after a pause, below what's there.
        let cmds = update(&mut state, Msg::SuggestDue("bug".into()));
        assert_eq!(cmds, vec![Cmd::Api(Api::Suggest("bug".into()))]);
        update(
            &mut state,
            Msg::Suggested(
                "bug".into(),
                Ok(vec![crate::fixtures::repo_summary("o/bugs", 5)]),
            ),
        );
        let Some(Overlay::Search(sb)) = &state.overlay else {
            panic!()
        };
        let picks: Vec<nav::Pick> = state
            .suggestions(sb)
            .into_iter()
            .filter_map(|(_, p)| p)
            .collect();
        assert_eq!(
            picks.last(),
            Some(&nav::Pick::Go(Target::Page(Route::Repo(RepoId::new(
                "o", "bugs"
            )))))
        );
        press(&mut state, "<Enter>");
        let route = route(&state);
        let (kind, query) = route.search().unwrap();
        let key = DataKey::Search(kind, query);
        fetched(
            &mut state,
            key.clone(),
            Data::Search(Box::new(crate::fixtures::issue_results(Some("c1")))),
        );
        press(&mut state, "G");
        assert!(selected_text(&state).starts_with("Load more"));
        let cmds = press(&mut state, "<Enter>");
        assert_eq!(
            cmds,
            vec![Cmd::Api(Api::FetchMore {
                key: key.clone(),
                after: "c1".into()
            })]
        );
        update(
            &mut state,
            Msg::FetchedMore(
                key.clone(),
                Ok(Data::Search(Box::new(crate::fixtures::issue_results(None)))),
            ),
        );
        let Some(Data::Search(results)) = state.get(&key) else {
            panic!()
        };
        assert_eq!(browse::next_cursor(results), None);
        let SearchResults::Issues(r) = &**results else {
            panic!()
        };
        assert_eq!(r.items.len(), 6);
    }

    #[test]
    fn visited_pages_come_back_as_suggestions() {
        let mut state = with_repo();
        state.push(Route::user("octocat"));
        state.push(Route::Home);
        press(&mut state, "/");
        let Some(Overlay::Search(sb)) = &state.overlay else {
            panic!()
        };
        let labels: Vec<String> = state
            .suggestions(sb)
            .into_iter()
            .filter_map(|(row, _)| match row {
                ghtui_ui::chrome::SuggestRow::Item { label, .. } => Some(label),
                _ => None,
            })
            .collect();
        assert!(labels.contains(&"@octocat".to_owned()), "{labels:?}");
        assert!(
            labels.contains(&"gold-silver-copper/ghtui".to_owned()),
            "{labels:?}"
        );
        press(&mut state, "octo<Enter>");
        assert_eq!(route(&state), Route::user("octocat"));
    }

    #[test]
    fn files_tab_opens_the_diff_once_the_pr_loads() {
        let mut state = state();
        let pr = PrRef::parse("o/r#1").unwrap();
        state.push(Route::Pr {
            pr: pr.clone(),
            tab: PrTab::Conversation,
        });
        assert!(press(&mut state, "4").is_empty(), "waits for the PR");
        assert!(matches!(state.screen(), Screen::Diff(_)));
        assert_eq!(state.chrome().active, Some(3));
        assert_eq!(state.busy().as_deref(), Some("Loading o/r#1"));
        let cmds = update(
            &mut state,
            Msg::Pr(pr.clone(), Box::new(Ok(crate::snapshot_tests::pr_detail()))),
        );
        assert!(
            cmds.iter()
                .any(|c| matches!(c, Cmd::Git(Git::LoadDiff { .. }))),
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
        assert_eq!(state.screens.len(), 2, "back on the PR page, not a new one");
    }

    #[test]
    fn the_mouse_follows_links_switches_tabs_and_scrolls() {
        let mut state = with_repo();
        let (x, y) = find(&state, "crates");
        click(&mut state, x, y);
        assert!(
            matches!(route(&state), Route::Tree { .. }),
            "a click follows a link"
        );
        state.back();
        // Tabs.
        let lay = state.layout();
        let tabs: Vec<_> = state.chrome().tabs.into_iter().map(|(t, _)| t).collect();
        let rects = ghtui_ui::chrome::tab_layout(lay.tabs.unwrap(), &tabs, state.chrome().active);
        click(&mut state, rects[2].x + 1, rects[2].y);
        assert!(matches!(route(&state), Route::Pulls { .. }));
        // The header's search field.
        let field = ghtui_ui::chrome::header_layout(lay.header, &[], &[]).search;
        let chrome = state.chrome();
        let crumbs: Vec<_> = chrome.crumbs.iter().map(|(c, _)| c.clone()).collect();
        let right: Vec<_> = chrome.right.iter().map(|(t, _)| t.clone()).collect();
        let field2 = ghtui_ui::chrome::header_layout(lay.header, &crumbs, &right).search;
        assert_eq!(field.y, field2.y);
        click(&mut state, field2.x + 1, field2.y);
        assert!(matches!(state.overlay, Some(Overlay::Search(_))));
        let bottom = state.size.1 - 1;
        click(&mut state, 0, bottom);
        assert!(state.overlay.is_none(), "a click outside closes it");
        // The wheel.
        press(&mut state, "1");
        update(&mut state, Msg::Resize(100, 16));
        let before = page(&state).scroll;
        update(
            &mut state,
            Msg::Mouse(MouseEvent {
                kind: crossterm::event::MouseEventKind::ScrollDown,
                column: 10,
                row: 10,
                modifiers: KeyModifiers::NONE,
            }),
        );
        assert_eq!(page(&state).scroll, before + 3);
    }

    /// `/` filters the menu; letters then narrow it instead of running
    /// rows, and Esc stops filtering before it closes the menu.
    #[test]
    fn the_menu_filters_as_you_type() {
        let mut state = with_repo();
        press(&mut state, "<Space>/");
        press(&mut state, "go to f");
        let Some(Overlay::Menu(menu)) = &state.overlay else {
            panic!("no menu")
        };
        let shown: Vec<&str> = menu.shown().iter().map(|d| d.label.as_str()).collect();
        assert_eq!(shown, ["Go to file"]);
        press(&mut state, "<Esc>");
        let Some(Overlay::Menu(menu)) = &state.overlay else {
            panic!("Esc stopped filtering, not the menu")
        };
        assert!(menu.filter.is_none() && menu.shown().len() > 1);
        press(&mut state, "/gotofile");
        let cmds = press(&mut state, "<Enter>");
        assert!(
            state
                .overlay
                .as_ref()
                .is_none_or(|o| !matches!(o, Overlay::Menu(_)))
        );
        assert!(!cmds.is_empty() || matches!(state.overlay, Some(Overlay::Picker(_))));
    }

    /// Messages that don't carry page data (rate limits after every
    /// request, notices, diff progress) don't rebuild the page.
    #[test]
    fn only_page_data_rebuilds_pages() {
        let mut state = with_repo();
        let generation = state.data_gen;
        update(&mut state, Msg::RateLimits(RateLimits::default()));
        update(&mut state, Msg::Notice(Notice::Info("hi".into())));
        update(&mut state, Msg::Problem(Problem::Drafts, None));
        assert_eq!(state.data_gen, generation);
        update(&mut state, Msg::Viewer(Ok("me".into())));
        assert_eq!(state.data_gen, generation + 1);
    }

    /// A URL is only ever a URL: even one spelled like an old internal
    /// action can't star a repository.
    #[test]
    fn urls_never_run_page_actions() {
        let mut state = with_repo();
        let cmds = state.follow(&ghtui_ui::page::Link::from("ghtui:star"));
        assert!(
            !cmds
                .iter()
                .any(|c| matches!(c, Cmd::Api(Api::SetStarred { .. }))),
            "{cmds:?}"
        );
        let cmds = state.follow(&ghtui_ui::page::Link::Star);
        assert!(
            cmds.iter()
                .any(|c| matches!(c, Cmd::Api(Api::SetStarred { .. })))
        );
    }

    /// Esc on an error dismisses it (and goes nowhere); the messages list
    /// keeps it, and choosing it copies it.
    #[test]
    fn errors_are_dismissed_and_kept() {
        let mut state = with_repo();
        let depth = state.screens.len();
        state.error("boom");
        timers(&mut state, &mut None);
        press(&mut state, "<Esc>");
        assert!(state.notice.is_none());
        assert_eq!(state.screens.len(), depth, "Esc only dismissed");
        act(&mut state, Action::Messages);
        let Some(Overlay::Picker(p)) = &state.overlay else {
            panic!("no messages list")
        };
        let rows = state.picker_rows(p);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0.label, "boom");
        let cmds = press(&mut state, "<Enter>");
        assert!(matches!(&cmds[..], [Cmd::Copy(t)] if t == "boom"));
    }

    #[test]
    fn the_menu_lists_and_runs_what_you_can_do() {
        let mut state = with_repo();
        press(&mut state, "<Space>");
        let Some(Overlay::Menu(menu)) = &state.overlay else {
            panic!("no menu")
        };
        let labels: Vec<&str> = menu.rows.iter().map(|d| d.label.as_str()).collect();
        assert!(labels.contains(&"Go to file"), "{labels:?}");
        assert!(labels.contains(&"Star"), "{labels:?}");
        let fwd = menu
            .rows
            .iter()
            .find(|d| d.action == Action::Forward)
            .unwrap();
        assert!(fwd.unavailable.is_some(), "nothing to go forward to");
        // A row's own key runs it.
        let cmds = press(&mut state, "f");
        assert!(matches!(state.overlay, Some(Overlay::Picker(_))));
        assert!(!cmds.is_empty());
    }

    #[test]
    fn status_bar_hints_follow_the_context() {
        let mut state = with_repo();
        let hints: Vec<String> = state.key_hints().into_iter().map(|(_, w)| w).collect();
        assert_eq!(hints[..2], ["menu", "open"]);
        assert!(hints.contains(&"go to file".to_owned()));
        press(&mut state, "2");
        let hints: Vec<String> = state.key_hints().into_iter().map(|(_, w)| w).collect();
        assert!(
            hints.contains(&"filter".to_owned()) && hints.contains(&"tabs".to_owned()),
            "{hints:?}"
        );
    }

    #[test]
    fn key_scopes_follow_the_screen() {
        let mut state = with_repo();
        press(&mut state, "a");
        assert!(
            matches!(&state.notice, Some(Notice::Info(n)) if n.contains("Files changed")),
            "a review key on a page says where it works"
        );
        press(&mut state, "l");
        assert!(
            matches!(state.overlay, Some(Overlay::Hints(_))),
            "l: link letters on a page"
        );
        let doables = state.doables();
        assert!(doables.iter().any(|d| d.action == Action::Hints));
        assert!(!doables.iter().any(|d| d.action == Action::FileComment));
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
            update(
                &mut s,
                Msg::Diff(
                    pr.clone(),
                    DiffMsg::ReviewLoaded(Ok(ReviewState::default())),
                ),
            );
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
                pull_request_id: NodeId::new(pr_id),
                states: [("src/point.rs".to_owned(), ViewedState::Viewed)].into(),
            }
        }

        #[test]
        fn split_is_automatic_by_width_and_toggles() {
            let (mut narrow, _) = diff_state(120);
            assert!(!narrow.diffs.values().next().unwrap().doc.opts().split);
            act(&mut narrow, Action::ToggleSplit);
            assert!(narrow.diffs.values().next().unwrap().doc.opts().split);
            let (wide, _) = diff_state(220);
            assert!(wide.diffs.values().next().unwrap().doc.opts().split);
        }

        #[test]
        fn viewed_state_applies_and_rolls_back_on_failure() {
            let (mut s, pr) = diff_state(120);
            // Before GitHub's state arrives, toggling explains why not.
            assert!(press(&mut s, "v").is_empty());
            assert!(matches!(s.notice, Some(Notice::Error(_))));

            update(
                &mut s,
                Msg::Diff(
                    pr.clone(),
                    DiffMsg::ViewedLoaded(Box::new(Ok(viewed_files("PR_1")))),
                ),
            );
            assert_eq!(s.diffs[&pr].doc.files()[0].viewed, Viewed::Viewed);
            assert!(
                s.diffs[&pr].doc.files()[0].collapsed(),
                "viewed files collapse"
            );

            // Next unviewed skips the viewed file.
            press(&mut s, "g");
            act(&mut s, Action::NextUnviewed);
            assert_eq!(screen(&s).cursor.file, 1);

            // Toggle file 1 viewed: optimistic, then GitHub refuses.
            let cmds = press(&mut s, "v");
            assert!(cmds.iter().any(|c| matches!(
                c,
                Cmd::Api(Api::SetViewed { file: 1, viewed: true, previous: Viewed::Unviewed, pull_request_id, .. })
                    if pull_request_id.as_str() == "PR_1"
            )));
            assert_eq!(s.diffs[&pr].doc.files()[1].viewed, Viewed::Viewed);
            update(
                &mut s,
                Msg::Diff(
                    pr.clone(),
                    DiffMsg::ViewedSaved {
                        file: 1,
                        previous: Viewed::Unviewed,
                        result: Err(ApiError::Network("offline".into())),
                    },
                ),
            );
            assert_eq!(s.diffs[&pr].doc.files()[1].viewed, Viewed::Unviewed);
            assert!(matches!(s.notice, Some(Notice::Error(_))));
        }

        /// Marks made before the saved review arrives aren't saved over
        /// it; they're merged into it once it's in.
        #[test]
        fn nothing_is_saved_before_the_saved_review_is_read() {
            let mut s = state();
            s.size = (120, 40);
            let pr = PrRef::parse("o/r#7").unwrap();
            let mut diff = crate::snapshot_tests::diff_fixture();
            diff.review_loaded = false;
            s.diffs.insert(pr.clone(), diff);
            s.screens
                .push(Screen::Diff(Box::new(DiffScreen::new(pr.clone(), 120))));
            let _ = s.settle_diff();
            press(&mut s, "njjj");
            let cmds = press(&mut s, "m");
            assert!(!cmds.iter().any(|c| matches!(c, Cmd::SaveReview(..))));

            let saved = ReviewState {
                reviewed_hunks: vec!["older".into()],
                ..ReviewState::default()
            };
            let cmds = update(&mut s, Msg::Diff(pr, DiffMsg::ReviewLoaded(Ok(saved))));
            let [Cmd::SaveReview(_, review)] = &cmds[..] else {
                panic!("{cmds:?}")
            };
            assert_eq!(review.reviewed_hunks.len(), 2);
            assert!(review.reviewed_hunks.contains(&"older".to_owned()));
        }

        #[test]
        fn unreadable_drafts_raise_a_lasting_problem() {
            let (mut s, pr) = diff_state(120);
            update(
                &mut s,
                Msg::Diff(pr, DiffMsg::ReviewLoaded(Err("disk full".into()))),
            );
            assert!(s.problems[&Problem::Drafts].contains("disk full"));
            assert!(s.layout().problem.is_some());
            update(&mut s, Msg::Problem(Problem::Drafts, None));
            assert!(s.layout().problem.is_none());
        }

        #[test]
        fn review_marks_toggle_and_persist() {
            let (mut s, pr) = diff_state(120);
            // Off a changed line: nothing to mark.
            assert!(press(&mut s, "g").is_empty());
            assert!(press(&mut s, "m").is_empty());
            // Onto the first change.
            press(&mut s, "njjj");
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
            assert!(matches!(s.overlay, Some(Overlay::DiffSearch(_))));
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
        fn n_and_p_follow_a_search_and_esc_ends_it() {
            let (mut s, _) = diff_state(120);
            press(&mut s, "/");
            press(&mut s, "Point<Enter>");
            let first = screen(&s).cursor;
            press(&mut s, "n");
            assert_ne!(screen(&s).cursor, first, "n: the next match");
            press(&mut s, "p");
            assert_eq!(screen(&s).cursor, first, "p: back");
            let depth = s.screens.len();
            press(&mut s, "<Esc>");
            assert_eq!(s.screens.len(), depth, "esc ends the search first");
            press(&mut s, "<Esc>");
            assert_eq!(s.screens.len(), depth - 1, "then goes back");
        }

        #[test]
        fn file_finder_jumps_to_files() {
            let (mut s, _) = diff_state(120);
            press(&mut s, "fgone");
            let Some(Overlay::Picker(p)) = &s.overlay else {
                panic!("no finder")
            };
            assert_eq!(s.picker_rows(p)[0].0.label, "gone.py");
            press(&mut s, "<Enter>");
            assert!(s.overlay.is_none());
            assert_eq!(screen(&s).cursor.file, 3);
            assert_eq!(screen(&s).cursor.row, 0);
        }

        #[test]
        fn whitespace_and_full_file_keep_the_cursor_line() {
            let (mut s, pr) = diff_state(120);
            press(&mut s, "njjj");
            let text = s.diffs[&pr].doc.row_text(screen(&s).cursor);
            act(&mut s, Action::FullFile);
            assert!(s.diffs[&pr].doc.files()[0].full);
            assert_eq!(s.diffs[&pr].doc.row_text(screen(&s).cursor), text);
            press(&mut s, "w");
            assert_eq!(
                s.diffs[&pr].doc.opts().whitespace,
                ghtui_diff::Whitespace::Ignore
            );
            press(&mut s, "e");
        }
        mod better {
            use super::*;

            #[test]
            fn moves_are_detected_when_every_file_is_diffed() {
                let (mut s, pr) = diff_state(120);
                // The fixture has a loading file; finishing it triggers detection.
                let pending = s.diffs[&pr]
                    .doc
                    .files()
                    .iter()
                    .position(|f| f.diff.is_none())
                    .unwrap();
                let job = s.diffs[&pr].job;
                let cmds = update(
                    &mut s,
                    Msg::Diff(
                        pr.clone(),
                        DiffMsg::Job(
                            job,
                            JobMsg::File(
                                pending,
                                Arc::new(FileDiff::compute(
                                    "zz/pending.rs",
                                    Some(b"a\n"),
                                    Some(b"b\n"),
                                )),
                            ),
                        ),
                    ),
                );
                assert!(cmds.iter().any(
                    |c| matches!(c, Cmd::Git(Git::DetectMoves(p, j, files)) if *p == pr && *j == job && files.len() == 8)
                ));
                // Only once.
                let again = update(
                    &mut s,
                    Msg::Diff(pr, DiffMsg::Job(job, JobMsg::Moves(Vec::new()))),
                );
                assert!(
                    !again
                        .iter()
                        .any(|c| matches!(c, Cmd::Git(Git::DetectMoves(..))))
                );
                assert!(act(&mut s, Action::JumpMove).is_empty());
                assert!(matches!(&s.notice, Some(Notice::Info(m)) if m.contains("moved")));
            }

            /// After a restart (a refresh, another commit range), the old
            /// job's results don't land in the new diff.
            #[test]
            fn a_restarted_job_ignores_the_old_one() {
                let (mut s, pr) = diff_state(120);
                let old = s.diffs[&pr].job;
                s.diffs.get_mut(&pr).unwrap().restart(None);
                let stale = Arc::new(FileDiff::compute("a.rs", Some(b"a\n"), Some(b"b\n")));
                update(
                    &mut s,
                    Msg::Diff(pr.clone(), DiffMsg::Job(old, JobMsg::File(0, stale))),
                );
                update(
                    &mut s,
                    Msg::Diff(pr.clone(), DiffMsg::Job(old, JobMsg::Failed("old".into()))),
                );
                let diff = &s.diffs[&pr];
                assert_ne!(diff.job, old);
                assert!(diff.doc.is_empty());
                assert_eq!(diff.error, None);
                // The current job's messages still count.
                let job = diff.job;
                update(
                    &mut s,
                    Msg::Diff(pr.clone(), DiffMsg::Job(job, JobMsg::Failed("new".into()))),
                );
                assert_eq!(s.diffs[&pr].error.as_deref(), Some("new"));
            }

            #[test]
            fn since_review_asks_github_then_compares() {
                let (mut s, pr) = diff_state(120);
                s.viewer = Some("me".into());
                let cmds = act(&mut s, Action::ToggleSinceReview);
                assert!(
                    matches!(&cmds[..], [Cmd::Api(Api::FetchLastReview { login, .. })] if login == "me")
                );
                let cmds = update(
                    &mut s,
                    Msg::Diff(pr.clone(), DiffMsg::LastReview(Ok(Some("old".into())))),
                );
                assert!(
                    matches!(&cmds[..], [Cmd::Git(Git::SinceReview { old_head, .. })] if old_head == "old")
                );

                // Nothing in the old diff matched: everything is new.
                update(
                    &mut s,
                    Msg::Diff(
                        pr.clone(),
                        DiffMsg::SinceReady("old".into(), Ok(Default::default())),
                    ),
                );
                assert!(s.diffs[&pr].doc.since_active());
                assert_eq!(s.chrome().tabs[3].0.label, "Files · since your review");
                // Toggling back needs no new lookups.
                assert!(
                    act(&mut s, Action::ToggleSinceReview)
                        .iter()
                        .all(|c| matches!(c, Cmd::Git(Git::Prioritize(..))))
                );
                assert!(!s.diffs[&pr].doc.since_active());
            }

            #[test]
            fn since_review_without_a_review_says_so() {
                let (mut s, pr) = diff_state(120);
                s.viewer = Some("me".into());
                act(&mut s, Action::ToggleSinceReview);
                update(&mut s, Msg::Diff(pr.clone(), DiffMsg::LastReview(Ok(None))));
                assert!(
                    matches!(&s.notice, Some(Notice::Info(m)) if m.contains("haven't reviewed"))
                );
                // Reviewing the current head means nothing's new.
                s.diffs.get_mut(&pr).unwrap().since_requested = true;
                update(
                    &mut s,
                    Msg::Diff(pr.clone(), DiffMsg::LastReview(Ok(Some("h".into())))),
                );
                assert!(matches!(&s.notice, Some(Notice::Info(m)) if m.contains("current head")));
            }

            #[test]
            fn commit_picker_loads_a_range_and_blocks_comments() {
                let (mut s, pr) = diff_state(120);
                // `ghtui pr` would have loaded metadata; the picker needs the base.
                s.prs.insert(
                    pr.clone(),
                    Remote::cached(Some(ghtui_store::Cached {
                        value: crate::snapshot_tests::pr_detail(),
                        fetched_at: 0,
                    })),
                );
                assert!(matches!(
                    &act(&mut s, Action::PickCommits)[..],
                    [Cmd::Git(Git::ListCommits(_))]
                ));
                update(
                    &mut s,
                    Msg::Diff(
                        pr,
                        DiffMsg::CommitsListed(Ok(vec![
                            ("a".repeat(40), "first".into()),
                            ("b".repeat(40), "second".into()),
                        ])),
                    ),
                );
                assert!(matches!(s.overlay, Some(Overlay::Picker(_))));
                // Mark the first commit, select the second: a range.
                press(&mut s, "jj<Space>j");
                let cmds = press(&mut s, "<Enter>");
                let [
                    Cmd::Git(Git::LoadDiff {
                        range: Some((from, to)),
                        ..
                    }),
                ] = &cmds[..]
                else {
                    panic!("{cmds:?}")
                };
                assert_eq!(from, &format!("{}^", "a".repeat(40)));
                assert_eq!(to, &"b".repeat(40));
                assert_eq!(s.chrome().tabs[3].0.label, "Files · aaaaaaa..bbbbbbb");
                // Commenting needs the whole PR.
                press(&mut s, "c");
                assert!(s.overlay.is_none());
                assert!(matches!(&s.notice, Some(Notice::Error(m)) if m.contains("All changes")));
                // Back to everything.
                act(&mut s, Action::PickCommits);
                let cmds = press(&mut s, "<Enter>");
                assert!(matches!(
                    &cmds[..],
                    [Cmd::Git(Git::LoadDiff { range: None, .. })]
                ));
            }
        }

        mod review {
            use super::*;
            use crate::review::{ComposeTarget, EditPurpose, SubmitOutcome};
            use ghtui_api::model::{PatchFile, ReviewComment, ReviewEvent, ReviewThread, Side};

            fn thread(id: &str, line: Option<u32>, resolved: bool, outdated: bool) -> ReviewThread {
                ReviewThread {
                    id: NodeId::new(id),
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
                        id: NodeId::new(format!("{id}-c")),
                        author: "alice".into(),
                        body: format!("Thread {id}"),
                        created_at: "2026-10-03T09:00:00Z".into(),
                        url: String::new(),
                        original_commit: Some("abc".into()),
                        pending: false,
                    }],
                }
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
                    Msg::Diff(
                        pr.clone(),
                        DiffMsg::ThreadsLoaded(Ok(vec![
                            thread("done", Some(1), true, false),
                            thread("open", Some(14), false, false),
                        ])),
                    ),
                );
                assert_eq!(s.diffs[&pr].doc.annotations().len(), 2);
                press(&mut s, "g");
                act(&mut s, Action::NextThread);
                let at = s.diffs[&pr].doc.annotation_at(screen(&s).cursor).unwrap();
                assert_eq!(
                    s.diffs[&pr].doc.annotations()[at as usize].key,
                    AnnotationKey::Thread(NodeId::new("open"))
                );
                // The status bar leads with what fits a thread.
                let hints = s.key_hints();
                assert_eq!(
                    hints.get(1..3),
                    Some(&[("c".into(), "reply".into()), ("R".into(), "resolve".into())][..])
                );

                // Resolve: optimistic, rolled back on failure.
                let cmds = act(&mut s, Action::ResolveThread);
                assert!(matches!(
                    &cmds[..],
                    [Cmd::Api(Api::SetResolved { resolved: true, .. }), ..]
                ));
                assert!(s.diffs[&pr].threads[1].resolved);
                update(
                    &mut s,
                    Msg::Diff(
                        pr.clone(),
                        DiffMsg::ResolvedSet {
                            thread_id: NodeId::new("open"),
                            resolved: true,
                            result: Err(ApiError::Network("down".into())),
                        },
                    ),
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
                press(&mut s, "Use a constant");
                let cmds = press(&mut s, "<C-s>");
                assert!(s.overlay.is_none());
                let Some(Cmd::SaveReview(_, review)) = cmds.first() else {
                    panic!("{cmds:?}")
                };
                assert_eq!(review.pending.len(), 1);
                let draft = &review.pending[0];
                assert_eq!((draft.line, draft.commit.as_str()), (Some(14), "h"));
                assert!(s.diffs[&pr].doc.annotations().iter().any(|a| a.is_draft()));

                // The cursor is on the commented line; Enter edits the draft.
                press(&mut s, "<Enter>");
                assert!(
                    matches!(&s.overlay, Some(Overlay::Compose(c)) if matches!(c.target, ComposeTarget::Draft { .. }))
                );
                press(&mut s, "!");
                press(&mut s, "<C-s>");
                assert_eq!(s.diffs[&pr].review.pending[0].body, "Use a constant!");

                let cmds = press(&mut s, "<Delete>");
                assert!(matches!(&cmds[..], [Cmd::SaveReview(_, r)] if r.pending.is_empty()));
                assert!(matches!(&s.notice, Some(Notice::Info(m)) if m.contains("ctrl-z")));

                // Deleting can be undone.
                let cmds = press(&mut s, "<C-z>");
                assert!(
                    matches!(&cmds[..], [Cmd::SaveReview(_, r)] if r.pending[0].body == "Use a constant!")
                );
                assert!(s.diffs[&pr].doc.annotations().iter().any(|a| a.is_draft()));
                press(&mut s, "<C-z>");
                assert!(matches!(&s.notice, Some(Notice::Info(m)) if m.starts_with("No deleted")));
            }

            #[test]
            fn lines_outside_githubs_diff_become_file_comments() {
                let (mut s, pr) = diff_state(120);
                // GitHub's diff only covers line 1.
                update(
                    &mut s,
                    Msg::Diff(
                        pr,
                        DiffMsg::PatchesLoaded(Ok(vec![PatchFile {
                            filename: "src/point.rs".into(),
                            previous_filename: None,
                            patch: Some("@@ -1 +1 @@\n-a\n+b".into()),
                        }])),
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
                    Msg::Diff(
                        pr,
                        DiffMsg::PatchesLoaded(Ok(vec![PatchFile {
                            filename: "src/point.rs".into(),
                            previous_filename: None,
                            patch: Some("@@ -1,6 +1,6 @@\n@@ -10,4 +10,8 @@".into()),
                        }])),
                    ),
                );
                to_line(&mut s, "A point in 2D");
                press(&mut s, "x");
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
                press(&mut s, "nit");
                press(&mut s, "<C-s>");
                let id = s.diffs[&pr].review.pending[0].id;

                press(&mut s, "a");
                assert!(matches!(s.overlay, Some(Overlay::Submit(_))));
                press(&mut s, "<Tab>");
                let cmds = press(&mut s, "<C-s>");
                let Some(Cmd::Api(Api::SubmitReview { drafts, event, .. })) = cmds.first() else {
                    panic!("{cmds:?}")
                };
                assert_eq!((drafts.len(), *event), (1, ReviewEvent::Approve));

                update(
                    &mut s,
                    Msg::Diff(
                        pr.clone(),
                        DiffMsg::ReviewSubmitted(SubmitOutcome {
                            rejected: vec![(id, "line must be part of the diff".into())],
                            ..SubmitOutcome::default()
                        }),
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
                act(&mut s, Action::FileComment);
                assert_eq!(s.diffs[&pr].review.pending[0].line, None);
                press(&mut s, "a");
                press(&mut s, "<C-s>");
                update(
                    &mut s,
                    Msg::Diff(
                        pr.clone(),
                        DiffMsg::ReviewSubmitted(SubmitOutcome {
                            accepted: vec![id],
                            submitted: true,
                            ..SubmitOutcome::default()
                        }),
                    ),
                );
                assert!(s.overlay.is_none());
                assert!(s.diffs[&pr].review.pending.is_empty());
                assert_eq!(s.diffs[&pr].review.last_reviewed_head.as_deref(), Some("h"));
            }

            #[test]
            fn request_changes_needs_a_summary() {
                let (mut s, _) = diff_state(120);
                press(&mut s, "a");
                press(&mut s, "<Tab><Tab>");
                assert!(press(&mut s, "<C-s>").is_empty());
                assert!(matches!(&s.overlay, Some(Overlay::Submit(d)) if d.error.is_some()));
            }

            #[test]
            fn replies_post_right_away() {
                let (mut s, pr) = diff_state(120);
                update(
                    &mut s,
                    Msg::Diff(
                        pr.clone(),
                        DiffMsg::ThreadsLoaded(Ok(vec![thread("t", Some(14), false, false)])),
                    ),
                );
                press(&mut s, "g");
                act(&mut s, Action::NextThread);
                press(&mut s, "c");
                press(&mut s, "Fixed");
                let cmds = press(&mut s, "<C-s>");
                assert!(
                    matches!(&cmds[..], [Cmd::Api(Api::Reply { body, .. })] if body == "Fixed")
                );
                update(
                    &mut s,
                    Msg::Diff(
                        pr.clone(),
                        DiffMsg::Replied(Err(ApiError::Network("down".into()))),
                    ),
                );
                assert!(
                    matches!(&s.overlay, Some(Overlay::Compose(c)) if c.error.is_some() && c.text() == "Fixed")
                );
                let cmds = update(&mut s, Msg::Diff(pr.clone(), DiffMsg::Replied(Ok(()))));
                assert!(s.overlay.is_none());
                assert_eq!(cmds, vec![Cmd::Api(Api::FetchThreads(pr))]);
            }

            #[test]
            fn suggestions_go_through_the_editor_with_a_preview() {
                let (mut s, _) = diff_state(120);
                to_line(&mut s, "origin");
                let cmds = act(&mut s, Action::Suggest);
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
                let cmds = act(&mut s, Action::Suggest);
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
                    Msg::Diff(
                        pr.clone(),
                        DiffMsg::ThreadsLoaded(Ok(vec![thread("old", None, false, true)])),
                    ),
                );
                assert!(cmds.iter().any(|c| matches!(c, Cmd::Git(Git::MapOutdated { threads, head, .. }) if threads.len() == 1 && head == "h")));
                let ann = &s.diffs[&pr].doc.annotations()[0];
                assert!(
                    ann.outdated && ann.on_line().is_none(),
                    "unplaced until mapped"
                );
                update(
                    &mut s,
                    Msg::Diff(
                        pr.clone(),
                        DiffMsg::OutdatedMapped(vec![(NodeId::new("old"), Some(4))]),
                    ),
                );
                let ann = &s.diffs[&pr].doc.annotations()[0];
                assert_eq!((ann.on_line(), ann.moved), (Some(4), true));
            }
        }
    }
}
