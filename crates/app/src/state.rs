//! Application state and the single update function. Input events and async
//! results arrive as [`Msg`]s; [`update`] mutates [`State`] and returns
//! [`Cmd`]s for the runtime to execute in the background.

use std::collections::HashMap;
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use ghtui_api::ApiError;
use ghtui_api::model::{Inbox, PrDetail, PrRef, RepoId, ViewedFiles};
use ghtui_api::rate_limit::RateLimits;
use ghtui_diff::FileDiff;
use ghtui_store::ReviewState;
use ghtui_theme::{Bg, Theme};
use ghtui_ui::bars::Notice;
use ghtui_ui::diff_doc::{Doc, Viewed};
use ghtui_ui::pages::PrTab;
use ghtui_ui::text::short_sha;
use ghtui_ui::{Ctx, Icons};
use ratatui::layout::Rect;
use ratatui_textarea::TextArea;

use crate::browse::{self, Data, DataKey, Need, PageScreen};
use crate::diff_job::{DiffFiles, JobId, JobMsg};
use crate::diff_screen::{self, DiffScreen, DiffState};
use crate::keymap::{Action, Key, Keymap, Resolution, Scope};
use crate::nav::{self, Hints, Menu, SearchBox, Visit};
use crate::picker::{self, PickItem, Picker};
use crate::review::{
    self, Compose, ComposeTarget, EditPurpose, Preview, SubmitDialog, SubmitOutcome,
};
use crate::route::{Route, Target};
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
    /// From a PR's diff job; ignored unless it's the PR's current job.
    Job(PrRef, JobId, JobMsg),
    ViewedLoaded(PrRef, Box<Result<ViewedFiles, ApiError>>),
    ViewedSaved {
        pr: PrRef,
        file: usize,
        previous: Viewed,
        result: Result<(), ApiError>,
    },
    /// The PR's saved review state, or why it couldn't be read.
    ReviewLoaded(PrRef, Result<ReviewState, String>),
    /// A lasting problem started (`Some`) or cleared up (`None`).
    Problem(Problem, Option<String>),
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
    /// Put text on the clipboard (OSC 52).
    Copy(String),
    /// Send [`Msg::Timer`] after this many milliseconds.
    Timer(Timer, u64),
    /// Send [`Msg::SuggestDue`] after a pause.
    SuggestLater(String),
    /// Search repositories for the search box's suggestions.
    Suggest(String),
    SaveVisits(Vec<Visit>),
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
    /// Look for moved code; answered as the job's [`JobMsg::Moves`].
    DetectMoves(PrRef, JobId, Vec<(usize, Arc<FileDiff>)>),
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

/// Problems that last until they're fixed, shown in a banner above the
/// status bar instead of as a notice that expires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Problem {
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
    /// Lasting problems, most important first; the first is shown.
    pub problems: std::collections::BTreeMap<Problem, String>,
    /// Terminal size.
    pub size: (u16, u16),
    pub keymap: Keymap,
    pub theme: Theme,
    pub icons: Icons,
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
            problems: Default::default(),
            size,
            keymap,
            theme,
            icons,
            clock: ghtui_store::now,
            notice_id: 0,
            spinner: 0,
            spinning: false,
            quit: false,
        };
        state.sync_page();
        state
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
            Need::Inbox if self.inbox.begin(force) => vec![Cmd::FetchInbox],
            Need::Pr(pr) => self.ensure_pr(&pr, force),
            Need::Data(key) => {
                let remote = self.data.entry(key.clone()).or_default();
                if !remote.begin(force) {
                    return Vec::new();
                }
                vec![Cmd::Fetch {
                    cached: remote.data.is_none(),
                    key,
                }]
            }
            Need::Inbox => Vec::new(),
        }
    }

    pub fn ensure_pr(&mut self, pr: &PrRef, force: bool) -> Vec<Cmd> {
        if self.prs.entry(pr.clone()).or_default().begin(force) {
            vec![Cmd::FetchPr(pr.clone())]
        } else {
            Vec::new()
        }
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
    fn base_ref(&self, pr: &PrRef) -> Option<String> {
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
            Cmd::LoadDiff {
                pr: pr.clone(),
                job,
                base_ref,
                range: None,
            },
            Cmd::FetchViewed(pr.clone()),
            Cmd::LoadReview(pr.clone()),
            Cmd::FetchThreads(pr.clone()),
            Cmd::FetchPatches(pr.clone()),
        ]
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
            Screen::Page(p) if self.page_loading(&p.route) => Some("Loading".to_owned()),
            Screen::Page(_) => None,
            Screen::Diff(screen) => match self.diffs.get(&screen.pr) {
                Some(diff) => diff.status(),
                None => Some(format!("Loading {}", screen.pr)),
            },
        }
    }
}

pub fn update(state: &mut State, msg: Msg) -> Vec<Cmd> {
    if !matches!(msg, Msg::Key(_) | Msg::Mouse(_) | Msg::Timer(_)) {
        state.data_gen += 1;
    }
    // Then keep the page (or the diff) in bounds.
    let mut cmds = handle(state, msg);
    // Until a PR's saved review is read, saving would overwrite it.
    cmds.retain(|cmd| match cmd {
        Cmd::SaveReview(pr, _) => state.diffs.get(pr).is_some_and(|d| d.review_loaded),
        _ => true,
    });
    state.sync_page();
    cmds.extend(state.settle_diff());
    cmds
}

/// Timers the screen needs after an update: the notice expiring (errors
/// stay longer), the spinner's next frame while something loads. The
/// runtime calls this; `last_notice` is the notice it last saw.
pub fn timers(state: &mut State, last_notice: &mut Option<Notice>) -> Vec<Cmd> {
    let mut cmds = Vec::new();
    if state.notice != *last_notice {
        if let Some(notice) = &state.notice {
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
        Msg::Commented(key, Ok(())) => {
            state.overlay = None;
            state.notice = Some(Notice::Info("Comment posted".into()));
            state.ensure(Need::Data(key), true)
        }
        Msg::Replied(pr, Ok(())) => {
            state.overlay = None;
            state.notice = Some(Notice::Info("Reply posted".into()));
            vec![Cmd::FetchThreads(pr)]
        }
        // The text stays in the composer, with GitHub's reason.
        Msg::Commented(_, Err(err)) | Msg::Replied(_, Err(err)) => {
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
                    state.notice = Some(Notice::Info(format!("{verb} {repo}")));
                }
                Err(err) => {
                    nav::set_starred(state, &repo, !starred);
                    state.notice = Some(Notice::Error(format!("GitHub didn't save that: {err}")));
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
                vec![Cmd::Suggest(q)]
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
        Msg::Job(pr, job, msg) => {
            if state.diffs.get(&pr).is_some_and(|d| d.job == job) {
                on_job(state, &pr, msg)
            } else {
                Vec::new()
            }
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
                        diff_screen::preserving_position(screen, &mut diff.doc, |doc| {
                            doc.set_since(Some(hashes), true)
                        });
                        state.notice = Some(Notice::Info(format!(
                            "Showing changes since your review of {}",
                            short_sha(&old_head)
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
            Vec::new()
        }
        Msg::CommitsListed(pr, result) => match result {
            Ok(commits) => {
                if let Some(diff) = state.diffs.get_mut(&pr) {
                    diff.commits = commits;
                }
                state.notice = None;
                state.open_picker(picker::Kind::Commits { mark: None })
            }
            Err(err) => {
                state.notice = Some(Notice::Error(format!("Couldn't list commits: {err}")));
                Vec::new()
            }
        },
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
            Vec::new()
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
            Vec::new()
        }
        Msg::ReviewLoaded(pr, Ok(saved)) => {
            let Some(diff) = state.diffs.get_mut(&pr) else {
                return Vec::new();
            };
            // Anything done before it arrived is kept, and saved.
            if diff.merge_saved_review(saved) {
                vec![Cmd::SaveReview(pr, diff.review.clone())]
            } else {
                Vec::new()
            }
        }
        Msg::ReviewLoaded(pr, Err(err)) => {
            tracing::warn!(%pr, %err, "reading review state failed");
            state.problems.insert(
                Problem::Drafts,
                format!("Drafts for {pr} are not being saved: {err}"),
            );
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
            state.map_outdated(&pr)
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
            Vec::new()
        }
        Msg::ReviewSubmitted(pr, outcome) => on_submitted(state, &pr, &outcome),
        Msg::Edited(purpose, result) => on_edited(state, purpose, result),
    }
}

/// Applies the result of submitting a review: accepted drafts leave the
/// local queue, rejected ones keep their text and GitHub's reason.
fn on_submitted(state: &mut State, pr: &PrRef, outcome: &SubmitOutcome) -> Vec<Cmd> {
    let mut cmds = vec![Cmd::FetchThreads(pr.clone())];
    if let Some(diff) = state.diffs.get_mut(pr) {
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
            diff.review.last_reviewed_head = diff.head();
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
    let theme = &state.theme;
    match purpose {
        EditPurpose::Compose => {
            if let Some(Overlay::Compose(compose)) = &mut state.overlay {
                let target = compose.target.clone();
                let preview = compose.preview.clone();
                **compose = Compose::new(theme, target, &text);
                compose.preview = preview;
            }
        }
        EditPurpose::Summary => {
            if let Some(Overlay::Submit(dialog)) = &mut state.overlay {
                let event = dialog.event;
                **dialog = SubmitDialog::new(theme);
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
            let mut compose = Compose::new(theme, ComposeTarget::Line { path, start, end }, &body);
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
        Some(Overlay::Picker(_)) => return picker::on_key(state, key),
        Some(Overlay::DiffSearch(_)) => return on_search_key(state, key),
        Some(Overlay::Search(_)) => return nav::on_search_box_key(state, key),
        Some(Overlay::Hints(_)) => return nav::on_hints_key(state, key),
        Some(Overlay::Menu(_)) => return nav::on_menu_key(state, key),
        Some(Overlay::Compose(_)) => return on_compose_key(state, key),
        Some(Overlay::Submit(_)) => return on_submit_key(state, key),
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
                state.notice = Some(Notice::Info(format!("{what}: {place}")));
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
                state.notice = Some(Notice::Info("Search cleared".into()));
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
    fn map_outdated(&mut self, pr: &PrRef) -> Vec<Cmd> {
        let Some(diff) = self.diffs.get_mut(pr) else {
            return Vec::new();
        };
        let Some(head) = diff.head() else {
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

    fn diff_parts(&mut self) -> Option<(&mut DiffScreen, &mut DiffState)> {
        let State { screens, diffs, .. } = self;
        let Screen::Diff(screen) = screens.last_mut() else {
            return None;
        };
        let diff = diffs.get_mut(&screen.pr)?;
        Some((screen, diff))
    }
}

/// How to get back to the whole PR, for notices.
fn all_changes(state: &State) -> String {
    let key = state.first_key(Action::PickCommits);
    format!("pick “All changes” ({key})")
}

/// Review actions on the diff screen. `None` lets other handlers try.
fn review_action(state: &mut State, action: Action) -> Option<Vec<Cmd>> {
    let viewer = state.viewer.clone();
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
        let all = all_changes(state);
        return notice(
            state,
            Notice::Error(format!("Comments anchor to the whole PR: {all} to comment")),
        );
    }
    match action {
        Action::ToggleSinceReview => {
            if in_range {
                let all = all_changes(state);
                return notice(state, Notice::Error(format!("First {all}")));
            }
            if diff.doc.since.is_some() {
                let active = !diff.doc.since_active;
                diff_screen::preserving_position(screen, &mut diff.doc, |doc| {
                    doc.set_since(doc.since.clone(), active)
                });
                state.notice = Some(Notice::Info(
                    if active {
                        "Showing changes since your last review"
                    } else {
                        "Showing all changes"
                    }
                    .into(),
                ));
                return Some(Vec::new());
            }
            diff.since_requested = true;
            if diff.last_review.is_none() {
                let Some(login) = viewer else {
                    // Without a login, only the local record can say.
                    diff.last_review = Some(None);
                    return Some(start_since_review(state));
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
            Some(state.open_picker(picker::Kind::Commits { mark: None }))
        }
        Action::Back if screen.selection.is_some() => {
            screen.selection = None;
            Some(Vec::new())
        }
        // `c` on a thread replies to it.
        Action::Comment
            if annotation
                .as_ref()
                .is_some_and(|a| matches!(a.key, AnnotationKey::Thread(_))) =>
        {
            let ann = annotation?;
            let AnnotationKey::Thread(thread_id) = ann.key else {
                return None;
            };
            if !ann.can_reply {
                return notice(state, Notice::Info("You can't reply to this one".into()));
            }
            Some(state.compose(ComposeTarget::Reply { thread_id }, ""))
        }
        Action::Comment => {
            let selection = screen.selection.take();
            match review::target(&diff.doc, cursor, selection) {
                Ok(target) => {
                    let reason = match &target {
                        ComposeTarget::File { reason, .. } => reason.clone(),
                        _ => None,
                    };
                    state.compose(target, "");
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
            Some(state.compose(ComposeTarget::File { path, reason: None }, ""))
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
                return Some(state.compose(ComposeTarget::Draft { id }, &body));
            }
            diff.doc.toggle_thread(ann_index);
            Some(Vec::new())
        }
        Action::SubmitReview => {
            if diff.refs.is_none() {
                return notice(state, Notice::Error("The diff hasn't loaded yet".into()));
            }
            state.overlay = Some(Overlay::Submit(Box::new(SubmitDialog::new(&state.theme))));
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
    let head = diff.head();
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

/// Shows what `choice` picks: the whole PR, the changes since your last
/// review, or a commit (from `mark` to it, if a range was marked).
pub fn apply_commit_choice(
    state: &mut State,
    choice: PickItem,
    mark: Option<PickItem>,
) -> Vec<Cmd> {
    let width = state.size.0;
    let pr = match state.screen() {
        Screen::Diff(screen) => screen.pr.clone(),
        _ => return Vec::new(),
    };
    let Some(base_ref) = state.base_ref(&pr) else {
        return Vec::new();
    };
    let Some((screen, diff)) = state.diff_parts() else {
        return Vec::new();
    };
    let since = choice == PickItem::SinceReview;
    let range = match choice {
        // Already on the whole PR: only "since your review" changes.
        PickItem::All | PickItem::SinceReview if diff.range.is_none() => {
            if diff.doc.since_active == since {
                return Vec::new();
            }
            return review_action(state, Action::ToggleSinceReview).unwrap_or_default();
        }
        PickItem::All | PickItem::SinceReview => None,
        PickItem::Commit(i) => {
            let j = match mark {
                Some(PickItem::Commit(m)) => m,
                _ => i,
            };
            let (first, last) = (i.min(j), i.max(j));
            // The picker's indices, into the list it showed.
            let (Some((from, _)), Some((to, _))) =
                (diff.commits.get(first), diff.commits.get(last))
            else {
                return Vec::new();
            };
            let label = if first == last {
                short_sha(to).to_owned()
            } else {
                format!("{}..{}", short_sha(from), short_sha(to))
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
    // Since your review: compared once the whole PR is back.
    diff.since_requested = since;
    let job = diff.job;
    *screen = DiffScreen::new(pr.clone(), width);
    vec![Cmd::LoadDiff {
        pr,
        job,
        base_ref,
        range: cmd_range,
    }]
}

/// A message from the PR's current diff job.
fn on_job(state: &mut State, pr: &PrRef, msg: JobMsg) -> Vec<Cmd> {
    match msg {
        JobMsg::Progress(line) => {
            if let Some(diff) = state.diffs.get_mut(pr)
                && !diff.listed()
            {
                diff.progress = Some(line);
            }
            Vec::new()
        }
        JobMsg::Files(files) => {
            let Some(diff) = state.diffs.get_mut(pr) else {
                return Vec::new();
            };
            let DiffFiles {
                refs,
                files,
                generated,
            } = *files;
            diff.set_files(refs, Doc::new(files, &generated));
            // "Since my last review" chosen while switching ranges.
            if std::mem::take(&mut diff.since_requested) {
                return review_action(state, Action::ToggleSinceReview).unwrap_or_default();
            }
            Vec::new()
        }
        JobMsg::File(index, file) => {
            let mut cmds = Vec::new();
            if let Some(diff) = state.diffs.get_mut(pr) {
                diff.set_file(index, file);
                if let Some(inputs) = diff.take_move_inputs() {
                    cmds.push(Cmd::DetectMoves(pr.clone(), diff.job, inputs));
                }
            }
            cmds
        }
        JobMsg::Moves(moves) => {
            match state.diff_parts() {
                Some((screen, diff)) if screen.pr == *pr => {
                    diff_screen::preserving_position(screen, &mut diff.doc, |doc| {
                        doc.set_moves(moves)
                    });
                }
                _ => {
                    if let Some(diff) = state.diffs.get_mut(pr) {
                        diff.doc.set_moves(moves);
                    }
                }
            }
            Vec::new()
        }
        JobMsg::Failed(error) => {
            if let Some(diff) = state.diffs.get_mut(pr) {
                diff.progress = None;
                diff.error = Some(error);
            }
            Vec::new()
        }
    }
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
            compose.error = Some("Press esc again to discard this comment".into());
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

/// `ctrl-s` in the composer: drafts join the pending review (and are saved);
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
            let head = diff.head().unwrap_or_default();
            let id = diff.review.next_draft_id();
            let Some(draft) = review::draft(&target, &body, id, &head) else {
                return Vec::new();
            };
            diff.review.pending.push(draft);
            diff.refresh_annotations();
            let save = Cmd::SaveReview(pr, diff.review.clone());
            let count = diff.review.pending.len();
            state.overlay = None;
            state.notice = Some(Notice::Info(format!(
                "Added to your review ({count} pending). Submit with {}",
                state.first_key(Action::SubmitReview)
            )));
            vec![save]
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
            let head = diff.head().unwrap_or_default();
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
    use crate::fixtures::press;
    use crate::picker::{Choice, fuzzy_score};
    use crate::route::OPEN;
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
                fresh: true,
            },
        )
    }

    /// Commands other than saving visits.
    fn fetches(cmds: Vec<Cmd>) -> Vec<Cmd> {
        cmds.into_iter()
            .filter(|c| !matches!(c, Cmd::SaveVisits(_)))
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
                Cmd::FetchPr(pr.clone()),
                Cmd::Fetch {
                    key: DataKey::PrActivity(pr.clone()),
                    cached: true
                }
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
            vec![Cmd::FetchViewer, Cmd::FetchPr(pr.clone())],
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
        state.push(Route::user("octocat"));
        update(
            &mut state,
            Msg::Fetched {
                key: DataKey::Profile("octocat".into()),
                result: Ok(Data::Profile(Box::new(crate::fixtures::profile()))),
                fresh: false,
            },
        );
        assert!(page(&state).page.lines[0].text().contains("The Octocat"));
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
                Cmd::Fetch {
                    key: DataKey::Repo(RepoId::new("a", "b")),
                    cached: true
                },
                Cmd::Fetch {
                    key: key.clone(),
                    cached: true
                }
            ]
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
            vec![Cmd::Fetch {
                key: DataKey::Search(kind, query),
                cached: true
            }],
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
    fn letter_hints_follow_links() {
        let mut state = with_repo();
        press(&mut state, "l");
        let Some(Overlay::Hints(hints)) = &state.overlay else {
            panic!("no hints")
        };
        let i = hints
            .links
            .iter()
            .position(|u| u == "https://github.com/gold-silver-copper")
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
                Cmd::Fetch {
                    key: tree.clone(),
                    cached: true
                },
                Cmd::Fetch {
                    key: DataKey::LastCommits(repo(), "main".into(), "crates".into()),
                    cached: true
                }
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
            vec![Cmd::Fetch {
                key: files.clone(),
                cached: true
            }]
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
            vec![Cmd::Fetch {
                key: DataKey::Refs(repo()),
                cached: true
            }]
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
        assert_eq!(cmds, vec![Cmd::Suggest("bug".into())]);
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
                Msg::ReviewLoaded(pr.clone(), Ok(ReviewState::default())),
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
                pull_request_id: pr_id.into(),
                states: [("src/point.rs".to_owned(), ViewedState::Viewed)].into(),
            }
        }

        #[test]
        fn split_is_automatic_by_width_and_toggles() {
            let (mut narrow, _) = diff_state(120);
            assert!(!narrow.diffs.values().next().unwrap().doc.opts.split);
            act(&mut narrow, Action::ToggleSplit);
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

            // Next unviewed skips the viewed file.
            press(&mut s, "g");
            act(&mut s, Action::NextUnviewed);
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
            let cmds = update(&mut s, Msg::ReviewLoaded(pr, Ok(saved)));
            let [Cmd::SaveReview(_, review)] = &cmds[..] else {
                panic!("{cmds:?}")
            };
            assert_eq!(review.reviewed_hunks.len(), 2);
            assert!(review.reviewed_hunks.contains(&"older".to_owned()));
        }

        #[test]
        fn unreadable_drafts_raise_a_lasting_problem() {
            let (mut s, pr) = diff_state(120);
            update(&mut s, Msg::ReviewLoaded(pr, Err("disk full".into())));
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
            assert!(s.diffs[&pr].doc.files[0].full);
            assert_eq!(s.diffs[&pr].doc.row_text(screen(&s).cursor), text);
            press(&mut s, "w");
            assert_eq!(
                s.diffs[&pr].doc.opts.whitespace,
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
                    .files
                    .iter()
                    .position(|f| f.diff.is_none())
                    .unwrap();
                let job = s.diffs[&pr].job;
                let cmds = update(
                    &mut s,
                    Msg::Job(
                        pr.clone(),
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
                );
                assert!(cmds.iter().any(
                    |c| matches!(c, Cmd::DetectMoves(p, j, files) if *p == pr && *j == job && files.len() == 8)
                ));
                // Only once.
                let again = update(&mut s, Msg::Job(pr, job, JobMsg::Moves(Vec::new())));
                assert!(!again.iter().any(|c| matches!(c, Cmd::DetectMoves(..))));
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
                update(&mut s, Msg::Job(pr.clone(), old, JobMsg::File(0, stale)));
                update(
                    &mut s,
                    Msg::Job(pr.clone(), old, JobMsg::Failed("old".into())),
                );
                let diff = &s.diffs[&pr];
                assert_ne!(diff.job, old);
                assert!(diff.doc.is_empty());
                assert_eq!(diff.error, None);
                // The current job's messages still count.
                let job = diff.job;
                update(
                    &mut s,
                    Msg::Job(pr.clone(), job, JobMsg::Failed("new".into())),
                );
                assert_eq!(s.diffs[&pr].error.as_deref(), Some("new"));
            }

            #[test]
            fn since_review_asks_github_then_compares() {
                let (mut s, pr) = diff_state(120);
                s.viewer = Some("me".into());
                let cmds = act(&mut s, Action::ToggleSinceReview);
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
                assert_eq!(s.chrome().tabs[3].0.label, "Files · since your review");
                // Toggling back needs no new lookups.
                assert!(
                    act(&mut s, Action::ToggleSinceReview)
                        .iter()
                        .all(|c| matches!(c, Cmd::Prioritize(..)))
                );
                assert!(!s.diffs[&pr].doc.since_active);
            }

            #[test]
            fn since_review_without_a_review_says_so() {
                let (mut s, pr) = diff_state(120);
                s.viewer = Some("me".into());
                act(&mut s, Action::ToggleSinceReview);
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
                assert!(matches!(
                    &act(&mut s, Action::PickCommits)[..],
                    [Cmd::ListCommits(_)]
                ));
                update(
                    &mut s,
                    Msg::CommitsListed(
                        pr,
                        Ok(vec![
                            ("a".repeat(40), "first".into()),
                            ("b".repeat(40), "second".into()),
                        ]),
                    ),
                );
                assert!(matches!(s.overlay, Some(Overlay::Picker(_))));
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
                assert_eq!(s.chrome().tabs[3].0.label, "Files · aaaaaaa..bbbbbbb");
                // Commenting needs the whole PR.
                press(&mut s, "c");
                assert!(s.overlay.is_none());
                assert!(matches!(&s.notice, Some(Notice::Error(m)) if m.contains("All changes")));
                // Back to everything.
                act(&mut s, Action::PickCommits);
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
                press(&mut s, "g");
                act(&mut s, Action::NextThread);
                let at = s.diffs[&pr].doc.annotation_at(screen(&s).cursor).unwrap();
                assert_eq!(
                    s.diffs[&pr].doc.annotations[at as usize].key,
                    AnnotationKey::Thread("open".into())
                );

                // Resolve: optimistic, rolled back on failure.
                let cmds = act(&mut s, Action::ResolveThread);
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
                press(&mut s, "Use a constant");
                let cmds = press(&mut s, "<C-s>");
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
                press(&mut s, "!");
                press(&mut s, "<C-s>");
                assert_eq!(s.diffs[&pr].review.pending[0].body, "Use a constant!");

                let cmds = press(&mut s, "<Delete>");
                assert!(matches!(&cmds[..], [Cmd::SaveReview(_, r)] if r.pending.is_empty()));
            }

            #[test]
            fn lines_outside_githubs_diff_become_file_comments() {
                let (mut s, pr) = diff_state(120);
                // GitHub's diff only covers line 1.
                update(
                    &mut s,
                    Msg::PatchesLoaded(
                        pr,
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
                        pr,
                        Ok(vec![PatchFile {
                            filename: "src/point.rs".into(),
                            previous_filename: None,
                            patch: Some("@@ -1,6 +1,6 @@\n@@ -10,4 +10,8 @@".into()),
                        }]),
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
                act(&mut s, Action::FileComment);
                assert_eq!(s.diffs[&pr].review.pending[0].line, None);
                press(&mut s, "a");
                press(&mut s, "<C-s>");
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
                    Msg::ThreadsLoaded(pr.clone(), Ok(vec![thread("t", Some(14), false, false)])),
                );
                press(&mut s, "g");
                act(&mut s, Action::NextThread);
                press(&mut s, "c");
                press(&mut s, "Fixed");
                let cmds = press(&mut s, "<C-s>");
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
