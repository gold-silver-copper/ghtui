//! Application state and the single update function. Input events and async
//! results arrive as [`Msg`]s; [`update`] mutates [`State`] and returns
//! [`Cmd`]s for the runtime to execute in the background.

use std::collections::HashMap;
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use ghtui_api::ApiError;
use ghtui_api::change::Change;
use ghtui_api::model::{NodeId, PrDetail, PrRef, ViewedFiles};
use ghtui_api::rate_limit::RateLimits;
use ghtui_diff::FileDiff;
use ghtui_git::Oid;
use ghtui_store::ReviewState;
use ghtui_theme::{Bg, Theme};
use ghtui_ui::bars::Notice;
use ghtui_ui::diff_doc::Viewed;
use ghtui_ui::{Ctx, Fetched, Icons};
use ratatui_textarea::TextArea;

use crate::act;
use crate::browse::{self, Data, DataKey, Need};
use crate::diff_job::{JobId, JobMsg};
use crate::diff_screen::{self, DiffOf, DiffPrefs, DiffScreen, DiffState, Pane};
use crate::join;
use crate::keymap::{Action, Key, Keymap, Scope};
use crate::nav::{self, Hints, Menu, PageScreen, SearchBox, Visit};
use crate::picker::{self, Picker};
use crate::review::{
    Compose, ComposeTarget, EditPurpose, SubmitDialog, SubmitOutcome, on_compose_key, on_edited,
    on_submit_key, review_action,
};
use crate::route::{Route, Target};
use crate::tabs::Tab;
use ghtui_api::model::{PatchFile, ReviewEvent, ReviewThread};
use ghtui_store::DraftComment;

#[derive(Debug)]
pub enum Msg {
    Key(KeyEvent),
    Resize(u16, u16),
    Viewer(Result<String, ApiError>),
    Pr(PrRef, Box<Result<PrDetail, ApiError>>),
    /// Page data; with `cached_at` (when it was fetched), the cached copy
    /// shown while fetching.
    Fetched {
        key: DataKey,
        result: Result<Data, ApiError>,
        cached_at: Option<u64>,
    },
    /// The next page of a list, asked for after the cursor.
    FetchedMore(DataKey, String, Result<Data, ApiError>),
    /// GitHub's answer to a change.
    Changed(Change, act::By, Result<(), ApiError>),
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
    Edited(EditPurpose, Result<String, Failure>),
    /// `config.toml` was changed (or wasn't, and why): what Home shows
    /// now, and the table a removal took out.
    HomeEdited {
        edit: crate::home::Edit,
        result: Result<(Vec<crate::home::Section>, Option<String>), String>,
        what: String,
    },
    /// For a PR's diff screen.
    Diff(DiffOf, DiffMsg),
}

/// Why background work failed. Shows as its message with its causes
/// (`a: b`); `{:?}` has everything, for the log.
#[derive(Debug)]
pub struct Failure(anyhow::Error);

impl Failure {
    pub fn msg(text: impl std::fmt::Display + std::fmt::Debug + Send + Sync + 'static) -> Self {
        Self(anyhow::Error::msg(text))
    }
}

impl<E: std::error::Error + Send + Sync + 'static> From<E> for Failure {
    fn from(err: E) -> Self {
        Self(err.into())
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#}", self.0)
    }
}

/// What arrives for a PR's diff: from its job, GitHub, or the drafts.
#[derive(Debug)]
pub enum DiffMsg {
    /// From the diff job; ignored unless it's the PR's current job.
    Job(JobId, JobMsg),
    ViewedLoaded(Box<Result<ViewedFiles, ApiError>>),
    ViewedSaved {
        path: String,
        previous: Viewed,
        result: Result<(), ApiError>,
    },
    /// The saved review state, or why it couldn't be read.
    ReviewLoaded(Result<ReviewState, Failure>),
    ThreadsLoaded(Result<Vec<ReviewThread>, ApiError>),
    PatchesLoaded(Result<Vec<PatchFile>, ApiError>),
    Replied(Result<(), ApiError>),
    ResolvedSet {
        thread_id: NodeId,
        resolved: bool,
        result: Result<(), ApiError>,
    },
    ReviewSubmitted(SubmitOutcome),
    /// How many comments your pending review on GitHub holds.
    PendingReview(Result<u64, ApiError>),
    LastReview(Result<Option<String>, ApiError>),
    CommitsListed(Result<Vec<ghtui_git::repo::Commit>, Failure>),
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
    /// Change Home's sections in the config file, if it still has `shown`;
    /// `what` says it's done.
    EditHome {
        path: std::path::PathBuf,
        shown: Vec<crate::home::Section>,
        edit: crate::home::Edit,
        what: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Api {
    FetchViewer,
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
    /// Change something on GitHub, answered by [`Msg::Changed`].
    Change(Change, act::By),
    /// Search repositories for the search box's suggestions.
    Suggest(String),
    SaveVisits(Vec<Visit>),
    FetchViewed(PrRef),
    SetViewed {
        pr: PrRef,
        pull_request_id: NodeId,
        path: String,
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
        head: Oid,
        drafts: Vec<DraftComment>,
        event: ReviewEvent,
        body: String,
        /// The comments in your pending review on GitHub you were shown:
        /// if it holds more, it isn't submitted.
        seen: u64,
    },
    /// How many comments your pending review on GitHub holds.
    FetchPendingReview(PrRef),
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

/// Files' diffs, with each file's index in the listing.
pub type FileDiffs = Vec<(usize, Arc<FileDiff>)>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Git {
    /// Start (or restart) the diff job for a PR.
    /// With `range`, diff `(from, to)` instead of the whole PR.
    LoadDiff {
        of: DiffOf,
        job: JobId,
        /// What a PR's diff is from (empty for a commit).
        base: crate::diff_job::PrBase,
        range: Option<(String, String)>,
    },
    /// Diff these files next.
    Prioritize(DiffOf, Vec<usize>),
    /// Look for moved code; answered as the job's [`JobMsg::Moves`].
    DetectMoves(join::Joined<(DiffOf, JobId, FileDiffs)>),
    /// Outdated threads mapped onto a job's head, answered as its [`JobMsg::Mapped`].
    MapOutdated(join::Joined<(PrRef, JobId, Oid, Vec<OutdatedThread>)>),
    /// Block hashes of the PR's diff at an old head, for "since my last
    /// review"; answered as the job's [`JobMsg::Since`].
    SinceReview(join::Joined<(PrRef, JobId, String)>),
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
    /// Fetch again what's on screen and still running.
    Live,
    /// Home's sections, on screen, are fetched again when old.
    Home,
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
    /// When it was last asked for (Unix seconds), to know when it's old.
    pub asked_at: Option<u64>,
    /// The cursor a next page was asked after, while that page is on its way.
    more: Option<String>,
    /// A write was made since the last fetch was asked for.
    stale: bool,
    /// Pages were added with Load more: only asking refetches it.
    extended: bool,
    /// Load more was asked for while the list reloaded: asked once it's in.
    more_wanted: bool,
}

impl<T> Default for Remote<T> {
    fn default() -> Self {
        Self {
            data: None,
            loading: false,
            error: None,
            cached_at: None,
            asked_at: None,
            more: None,
            stale: false,
            extended: false,
            more_wanted: false,
        }
    }
}

impl<T> Remote<T> {
    /// What a page shows of a fetch: one that hasn't started is loading.
    #[expect(
        clippy::disallowed_methods,
        reason = "the one place a fetch's state becomes what a page shows"
    )]
    pub fn fetched<'a>(remote: Option<&'a Self>, retry: &'a str) -> Fetched<'a, T> {
        let (data, loading, error) = remote.map_or((None, false, None), |r| {
            (r.data.as_ref(), r.loading, r.error.as_deref())
        });
        Fetched::new_unchecked(data, loading, error, retry)
    }

    pub fn cached(cached: Option<ghtui_store::Cached<T>>) -> Self {
        Self {
            cached_at: cached.as_ref().map(|c| c.fetched_at),
            data: cached.map(|c| c.value),
            ..Self::default()
        }
    }

    /// Where the fetch is at, without the data.
    pub fn status(&self) -> Remote<()> {
        Remote {
            data: self.data.as_ref().map(|_| ()),
            loading: self.loading,
            error: self.error.clone(),
            cached_at: self.cached_at,
            asked_at: self.asked_at,
            more: self.more.clone(),
            stale: self.stale,
            extended: self.extended,
            more_wanted: self.more_wanted,
        }
    }

    /// Starts a fetch, unless one is running or (without `force`) the data
    /// is good: fetched here (not from the disk cache) since the last write,
    /// or extended, which a refetch would cut back to its first page.
    fn begin(&mut self, force: bool) -> bool {
        let good = self.data.is_some() && self.error.is_none() && self.cached_at.is_none();
        if self.loading || (!force && (self.extended || good && !self.stale)) {
            return false;
        }
        self.loading = true;
        self.stale = false;
        true
    }

    pub(crate) fn finish(&mut self, result: Result<T, ApiError>) {
        self.loading = false;
        match result {
            Ok(data) => {
                self.data = Some(data);
                self.error = None;
                self.cached_at = None;
                // A next page asked for before this belongs to the old list.
                self.more = None;
                self.extended = false;
            }
            Err(err) => self.error = Some(err.to_string()),
        }
    }
}

impl Remote<Data> {
    /// Asks for the next page, unless something is on its way or there is
    /// none: the cursor to ask after. Asked while the list reloads, it's
    /// asked again once the list is in.
    pub(crate) fn ask_more(&mut self) -> Option<String> {
        if self.loading {
            self.more_wanted = self.data.is_some();
            return None;
        }
        let data = (self.data.as_ref()).filter(|_| self.more.is_none())?;
        self.more = Some(data.next_cursor()?.to_owned());
        self.more.clone()
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
    pub(crate) fn new(first: Screen) -> Self {
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
    /// Asking before a change to GitHub.
    Confirm(Box<act::Confirm>),
}

pub struct State {
    /// The tab on screen's history.
    pub screens: Screens,
    /// Pages gone back from.
    pub forward: Vec<Screen>,
    /// The open tabs before and after the one on screen, in order.
    pub before: Vec<Tab>,
    pub after: Vec<Tab>,
    /// Pages visited, for the search box.
    pub visits: Vec<Visit>,
    /// Home's sections, as `config.toml` has them.
    pub home: Vec<crate::home::Section>,
    /// Where they're written back to.
    pub config_path: Option<std::path::PathBuf>,
    /// The section just removed, where it was, as its TOML.
    pub removed: Option<(usize, String)>,
    /// A change to the sections is being written.
    pub home_editing: bool,
    /// [`Timer::Home`] is scheduled.
    pub home_timer: bool,
    pub prs: HashMap<PrRef, Remote<PrDetail>>,
    /// Everything else pages show.
    pub data: HashMap<DataKey, Remote<Data>>,
    /// Bumped whenever data changes, so pages know to rebuild.
    pub data_gen: u64,
    pub diffs: HashMap<DiffOf, DiffState>,
    pub viewer: Option<String>,
    pub rate_limits: RateLimits,
    pub overlay: Option<Overlay>,
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
    /// [`Timer::Live`] is scheduled.
    pub live: bool,
    /// A change GitHub accepted that its page doesn't show yet.
    pub awaiting: Option<act::Awaiting>,
    /// An action waiting for what it's about to load.
    pub pending: Option<act::Pending>,
    pub quit: bool,
}

impl State {
    pub fn new(theme: Theme, icons: Icons, keymap: Keymap, size: (u16, u16)) -> Self {
        let mut state = Self {
            screens: Screens::new(Screen::Page(Box::new(PageScreen::new(Route::Home, None)))),
            forward: Vec::new(),
            before: Vec::new(),
            after: Vec::new(),
            visits: Vec::new(),
            home: crate::home::defaults(),
            config_path: None,
            removed: None,
            home_editing: false,
            home_timer: false,
            prs: HashMap::new(),
            data: HashMap::new(),
            data_gen: 0,
            diffs: HashMap::new(),
            viewer: None,
            rate_limits: RateLimits::default(),
            overlay: None,
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
            live: false,
            awaiting: None,
            pending: None,
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

    /// What a page needs fetched: Home's are its sections'.
    pub fn needs(&self, route: &Route) -> Vec<Need> {
        match route {
            Route::Home => self.home_needs(),
            _ => browse::needs(route),
        }
    }

    /// Fetches a page's data. The repository header shared by a repo's
    /// pages is only fetched when missing; Home's sections, also when
    /// they're old.
    #[must_use]
    pub fn ensure_route(&mut self, route: &Route, force: bool) -> Vec<Cmd> {
        let mut cmds = Vec::new();
        for need in self.needs(route) {
            let header = browse::is_header(route, &need);
            let aged = *route == Route::Home && self.aged(&need);
            cmds.extend(self.ensure(need, (force || aged) && !header));
        }
        cmds
    }

    #[must_use]
    pub fn ensure(&mut self, need: Need, force: bool) -> Vec<Cmd> {
        let now = (self.clock)();
        let (started, api) = match need {
            Need::Pr(pr) => (
                self.prs.entry(pr.clone()).or_default().begin(force),
                Api::FetchPr(pr),
            ),
            Need::Data(key) => {
                let remote = self.data.entry(key.clone()).or_default();
                // What was still running is checked again whenever it's
                // wanted (going back to it, switching to its tab).
                let running = remote.data.as_ref().is_some_and(Data::running);
                let cached = remote.data.is_none();
                let force = force || (running && !remote.extended);
                let started = remote.begin(force);
                if started {
                    remote.asked_at = Some(now);
                }
                (started, Api::Fetch { cached, key })
            }
        };
        started.then_some(Cmd::Api(api)).into_iter().collect()
    }

    /// Fetches needed for the visible screen. `force` refreshes data we
    /// already have.
    #[must_use]
    pub fn load_visible(&mut self, force: bool) -> Vec<Cmd> {
        let mut cmds = Vec::new();
        if self.viewer.is_none() && force {
            cmds.push(Cmd::Api(Api::FetchViewer));
        }
        match self.screen() {
            Screen::Page(p) => {
                let route = p.route.clone();
                cmds.extend(self.ensure_route(&route, force));
                // An issue or pull request list's open and closed counts
                // are the header's, so refreshing the list refreshes them.
                if force && let Route::Issues { repo, .. } | Route::Pulls { repo, .. } = &route {
                    let header = Need::Data(DataKey::Repo(repo.clone()));
                    cmds.extend(self.ensure(header, true));
                }
            }
            Screen::Diff(d) => {
                let of = d.of.clone();
                if let DiffOf::Pr(pr) = &of {
                    cmds.extend(self.ensure(Need::Pr(pr.clone()), force));
                }
                // Settling starts it (again); a PR's waits for the PR, which may have merged.
                if force {
                    self.diffs.remove(&of);
                }
            }
        }
        cmds
    }

    /// Opens a diff (a PR's starts once the PR's metadata is in).
    #[must_use]
    pub fn open_diff(&mut self, of: DiffOf) -> Vec<Cmd> {
        let reuse = self.diffs.get(&of).is_some_and(|d| d.error.is_none());
        let cmds = if reuse {
            Vec::new()
        } else {
            self.start_diff(&of)
        };
        let screen = DiffScreen::new(of, DiffPrefs::fit(self.size.0));
        self.screens.push(Screen::Diff(Box::new(screen)));
        cmds
    }

    /// The branch the PR merges into, once the PR has loaded.
    /// What the PR's diff is from, once the PR has loaded: its base
    /// branch, and for a merged or closed PR the base it had then (with a
    /// merge commit, its head is in the branch, which would leave nothing).
    pub(crate) fn pr_base(&self, pr: &PrRef) -> Option<crate::diff_job::PrBase> {
        let d = self.prs.get(pr)?.data.as_ref()?;
        let done = matches!(
            d.summary.state,
            ghtui_api::browse::IssueState::Merged | ghtui_api::browse::IssueState::Closed
        );
        Some(crate::diff_job::PrBase {
            branch: d.base_ref.clone(),
            oid: done.then(|| Oid::parse(&d.base_oid).ok()).flatten(),
        })
    }

    #[must_use]
    fn start_diff(&mut self, of: &DiffOf) -> Vec<Cmd> {
        let base = match of {
            DiffOf::Pr(pr) => match self.pr_base(pr) {
                Some(base) => base,
                None => return Vec::new(),
            },
            DiffOf::Commit(..) | DiffOf::Range(..) => crate::diff_job::PrBase::default(),
        };
        let diff = DiffState::fresh();
        let job = diff.job;
        self.diffs.insert(of.clone(), diff);
        let mut cmds = vec![Cmd::Git(Git::LoadDiff {
            of: of.clone(),
            job,
            base,
            range: None,
        })];
        // Reviews, threads and viewed files are a PR's.
        if let DiffOf::Pr(pr) = of {
            cmds.extend([
                Cmd::Api(Api::FetchViewed(pr.clone())),
                Cmd::LoadReview(pr.clone()),
                Cmd::Api(Api::FetchThreads(pr.clone())),
                Cmd::Api(Api::FetchPatches(pr.clone())),
            ]);
        }
        cmds
    }

    /// Rebuilds the page if its data changed and keeps the page (or the
    /// diff) in bounds.
    #[must_use]
    pub fn settle(&mut self) -> Vec<Cmd> {
        let mut cmds = self.settle_ref();
        self.sync_page();
        cmds.extend(self.settle_diff());
        cmds
    }

    /// Opens the page an [`Route::Unsplit`] one is, once where its ref
    /// ends is known.
    #[must_use]
    fn settle_ref(&mut self) -> Vec<Cmd> {
        let Screen::Page(p) = self.screen() else {
            return Vec::new();
        };
        let Some(route) = self.split(&p.route, None) else {
            return Vec::new();
        };
        let anchor = p.anchor.clone();
        self.replace(route, false, anchor)
    }

    /// `route` followed from the page on screen: split where its ref ends
    /// if that's known, as when a page at a revision links to that
    /// revision's files.
    pub(crate) fn followed(&self, route: Route) -> Route {
        let rev = (self.route())
            .filter(|here| here.repo() == route.repo())
            .and_then(Route::rev);
        self.split(&route, rev).unwrap_or(route)
    }

    /// The page an [`Route::Unsplit`] one is, if where its ref ends is
    /// known here: GitHub said, or its ref-and-path starts with the
    /// repository's default branch or with `rev` (the revision of the page
    /// that links to it). Git keeps a branch from being the start of
    /// another, so a branch already known is the one (short of a tag that
    /// extends it).
    pub(crate) fn split(&self, route: &Route, rev: Option<&str>) -> Option<Route> {
        let Route::Unsplit { repo, spot, .. } = route else {
            return None;
        };
        let key = DataKey::RefIn(repo.clone(), spot.clone());
        if let Some(found) = self.picked::<Option<String>>(&key) {
            return route.split(found.as_deref());
        }
        let default = self
            .overview(repo)
            .and_then(|o| o.default_branch.as_deref());
        let starts = |r: &&str| {
            spot.strip_prefix(*r)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
        };
        let known = [rev, default].into_iter().flatten().find(starts)?;
        route.split(Some(known))
    }

    /// Starts the diff on screen once it can and the joined work that's
    /// due, and re-runs the diff screen's clamping and prioritization.
    #[must_use]
    pub fn settle_diff(&mut self) -> Vec<Cmd> {
        let mut cmds = match self.screen() {
            Screen::Diff(screen)
                if !self.diffs.contains_key(&screen.of)
                    && (screen.of.pr())
                        .is_none_or(|pr| self.prs.get(pr).is_none_or(|r| !r.loading)) =>
            {
                self.start_diff(&screen.of.clone())
            }
            _ => Vec::new(),
        };
        cmds.extend(join::join(self));
        let content = self.layout().content;
        if let Some((screen, diff)) = self.diff_parts() {
            cmds.extend(diff_screen::settle(screen, diff, content));
        }
        cmds
    }

    /// What's on screen and still running (a run, a job, its log,
    /// checks, a pull request GitHub is still checking), to fetch again
    /// until it's done. A list extended with Load more isn't: fetching it
    /// again would cut it back to its first page.
    pub fn running_here(&self) -> Vec<Need> {
        let Screen::Page(p) = self.screen() else {
            return Vec::new();
        };
        self.needs(&p.route)
            .into_iter()
            .filter(|need| match need {
                Need::Data(key) => self
                    .data
                    .get(key)
                    .is_some_and(|r| !r.extended && r.data.as_ref().is_some_and(Data::running)),
                Need::Pr(pr) => (self.prs.get(pr))
                    .and_then(|r| r.data.as_ref())
                    .is_some_and(PrDetail::settling),
            })
            .collect()
    }

    /// What's loading on the visible screen, for the status bar.
    pub fn busy(&self) -> Option<String> {
        if let Some(waiting) = act::waiting(self) {
            return Some(waiting);
        }
        match self.screen() {
            Screen::Page(p) if self.fetches(&p.route).any(|r| r.more.is_some()) => {
                Some("Loading more".into())
            }
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
            Screen::Diff(screen) => match self.diffs.get(&screen.of) {
                Some(diff) => diff.status(),
                None => Some(format!("Loading {}", screen.of)),
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
#[must_use]
pub fn apply_msg(state: &mut State, msg: Msg) -> Vec<Cmd> {
    // Pages are built from GitHub's data; only these change it.
    let data = matches!(
        msg,
        Msg::Viewer(_)
            | Msg::Pr(..)
            | Msg::Fetched { .. }
            | Msg::FetchedMore(..)
            | Msg::Changed(..)
    );
    if data {
        state.data_gen += 1;
    }
    // A write changes what GitHub shows, wherever it shows it.
    let wrote = match &msg {
        Msg::Changed(_, _, r) | Msg::Diff(_, DiffMsg::Replied(r)) => r.is_ok(),
        // Drafts GitHub accepted wait in a pending review there.
        Msg::Diff(_, DiffMsg::ReviewSubmitted(o)) => o.error.is_none() || !o.accepted.is_empty(),
        _ => false,
    };
    if wrote {
        state.prs.values_mut().for_each(|r| r.stale = true);
        state.data.values_mut().for_each(|r| r.stale = true);
    }
    let mut cmds = handle(state, msg);
    if data {
        cmds.extend(act::follow(state));
    }
    cmds
}

/// Timers the screen needs after an update: the notice expiring (errors
/// stay longer), the spinner's next frame while something loads. The
/// runtime calls this; `last_notice` is the notice it last saw.
#[must_use]
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
    if !state.live && (!state.running_here().is_empty() || act::awaiting_here(state).is_some()) {
        state.live = true;
        cmds.push(Cmd::Timer(Timer::Live, LIVE_MS));
    }
    // Home keeps itself fresh while it's on screen, and only then.
    if !state.home_timer && state.route() == Some(&Route::Home) {
        state.home_timer = true;
        cmds.push(Cmd::Timer(Timer::Home, crate::home::FRESH_SECS * 1000));
    }
    cmds
}

/// How often what's running on screen is fetched again.
const LIVE_MS: u64 = 5_000;

#[must_use]
fn handle(state: &mut State, msg: Msg) -> Vec<Cmd> {
    match msg {
        Msg::Diff(of, msg) => return diff_screen::update(state, &of, msg),
        Msg::Key(key) => return on_key(state, key),
        Msg::Resize(w, h) => state.size = (w, h),
        Msg::Viewer(Ok(login)) => state.viewer = Some(login),
        Msg::Viewer(Err(err)) => tracing::warn!(%err, "could not fetch viewer"),
        Msg::Pr(pr, result) => {
            if let Err(err) = result.as_ref() {
                tracing::warn!(%pr, %err, "PR fetch failed");
            }
            state.prs.entry(pr).or_default().finish(*result);
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
                // A first look while it loads: shown over nothing, or over
                // an older one.
                Some(at) => {
                    let older = remote.data.is_none() || remote.cached_at.is_some_and(|c| c <= at);
                    if older && let Ok(data) = result {
                        remote.data = Some(data);
                        remote.cached_at = Some(at);
                    }
                }
            }
            // A write landed while this was on its way, so it may miss it.
            if remote.stale && !is_pr {
                return state.ensure(Need::Data(key), false);
            }
            // Load more, asked while it reloaded.
            if !remote.loading
                && std::mem::take(&mut remote.more_wanted)
                && remote.error.is_none()
                && let Some(after) = remote.ask_more()
            {
                return vec![Cmd::Api(Api::FetchMore { key, after })];
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
                let anchor = match state.screen() {
                    Screen::Page(p) => p.anchor.clone(),
                    Screen::Diff(_) => None,
                };
                return state.replace(Route::pr(PrRef { repo, number }), true, anchor);
            }
        }
        Msg::FetchedMore(key, after, result) => {
            let Some(remote) = (state.data.get_mut(&key)).filter(|r| r.more == Some(after)) else {
                return Vec::new();
            };
            remote.more = None;
            match result {
                Ok(more) => {
                    if let Some(data) = &mut remote.data {
                        data.append(more);
                        remote.extended = true;
                    }
                }
                Err(err) => state.error(format!("Couldn't load more: {err}")),
            }
        }
        Msg::Changed(change, by, result) => return act::on_changed(state, &change, by, result),
        Msg::Mouse(ev) => return nav::on_mouse(state, ev),
        Msg::Timer(Timer::ExpireNotice(id)) => {
            if id == state.notice_id {
                state.notice = None;
            }
        }
        Msg::Timer(Timer::Spin) => {
            state.spinning = false;
            state.spinner = state.spinner.wrapping_add(1);
        }
        Msg::Timer(Timer::Live) => {
            state.live = false;
            let mut cmds = Vec::new();
            let mut needs = state.running_here();
            needs.extend(act::poll(state));
            for need in needs {
                cmds.extend(state.ensure(need, true));
            }
            return cmds;
        }
        Msg::Timer(Timer::Home) => {
            state.home_timer = false;
            if state.route() == Some(&Route::Home) {
                return state.ensure_route(&Route::Home, false);
            }
        }
        Msg::Timer(Timer::Minute) => {
            state.data_gen += 1;
            return vec![Cmd::Timer(Timer::Minute, 60_000)];
        }
        Msg::SuggestDue(q) => {
            if let Some(Overlay::Search(sb)) = &state.overlay
                && sb.query() == q
            {
                return vec![Cmd::Api(Api::Suggest(q))];
            }
        }
        Msg::Suggested(q, result) => {
            if let (Some(Overlay::Search(sb)), Ok(repos)) = (&mut state.overlay, result) {
                sb.suggested(q, repos);
            }
        }
        Msg::RateLimits(limits) => state.rate_limits = limits,
        Msg::Notice(notice) => state.notice = Some(notice),
        Msg::Problem(problem, Some(text)) => {
            state.problems.insert(problem, text);
        }
        Msg::Problem(problem, None) => {
            state.problems.remove(&problem);
        }
        Msg::Edited(purpose, result) => return on_edited(state, purpose, result),
        Msg::HomeEdited { edit, result, what } => return state.home_edited(&edit, result, what),
    }
    Vec::new()
}

#[must_use]
fn on_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        state.quit = crate::review::may_discard(state, "ctrl-c", "quit and discard");
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
        Some(Overlay::Confirm(_)) => return act::on_confirm_key(state, key),
        None => {}
    }
    // Any keypress dismisses the last notice; Esc on an error does only
    // that (the messages list keeps it).
    let dismissing = matches!(state.notice, Some(Notice::Error(_)));
    state.notice = None;
    if dismissing && key.code == KeyCode::Esc {
        return Vec::new();
    }
    // A key bound only on the other kind of screen still runs, to say
    // where it works.
    let other = match state.scope() {
        Scope::Page => Scope::Diff,
        _ => Scope::Page,
    };
    let key = Key::from(key);
    match [state.scope(), other]
        .into_iter()
        .find_map(|scope| state.keymap.resolve(key, scope))
    {
        Some(action) => apply(state, action),
        None => Vec::new(),
    }
}

#[must_use]
fn on_search_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    let Some(Overlay::DiffSearch(input)) = &mut state.overlay else {
        return Vec::new();
    };
    match key.code {
        KeyCode::Esc => state.overlay = None,
        KeyCode::Enter => {
            let query = input.lines().join("");
            state.overlay = None;
            if !query.is_empty()
                && let Some((screen, diff)) = state.diff_parts()
            {
                state.notice = Some(diff_screen::search(screen, diff, query));
            }
        }
        _ => nav::type_into(input, key),
    }
    Vec::new()
}

/// Runs `action`. The actions every screen shares are here; the rest go to
/// the screen's own handler, which says where an action works when it
/// doesn't work there.
#[must_use]
pub fn apply(state: &mut State, action: Action) -> Vec<Cmd> {
    // On the diff: whether a search or a selection is active.
    let diff = match state.screen() {
        Screen::Diff(screen) => Some(screen.search.is_some() || screen.selection.is_some()),
        Screen::Page(_) => None,
    };
    match action {
        Action::Quit => state.quit = true,
        // Esc ends a search or selection before it leaves.
        Action::Back if diff == Some(true) => {
            if let Screen::Diff(screen) = state.screen_mut() {
                (screen.search, screen.selection) = (None, None);
            }
            state.info("Cleared · esc again goes back");
        }
        // Esc ends a log search before it leaves, too.
        Action::Back if matches!(state.route(), Some(Route::Job { query, .. }) if !query.is_empty()) =>
        {
            nav::search_log(state, "");
        }
        Action::Back => return state.back(),
        Action::Forward => return state.go_forward(),
        // From the files, up is the pull request.
        Action::UpLevel if diff.is_some() => return nav::switch_tab(state, 1),
        Action::UpLevel => return nav::up_level(state),
        Action::NextTab => return nav::step_tab(state, true),
        Action::PrevTab => return nav::step_tab(state, false),
        Action::Tab1 => return nav::switch_tab(state, 1),
        Action::Tab2 => return nav::switch_tab(state, 2),
        Action::Tab3 => return nav::switch_tab(state, 3),
        Action::Tab4 => return nav::switch_tab(state, 4),
        Action::Tab5 => return nav::switch_tab(state, 5),
        Action::Tab6 => return nav::switch_tab(state, 6),
        Action::Tab7 => return nav::switch_tab(state, 7),
        Action::Tab8 => return nav::switch_tab(state, 8),
        Action::Tab9 => return nav::switch_tab(state, 9),
        Action::NextOpenTab => return state.step_open_tab(true),
        Action::PrevOpenTab => return state.step_open_tab(false),
        Action::OpenInTab => return state.open_in_tab(),
        Action::CloseTab => return state.close_tab(),
        Action::OpenTab1
        | Action::OpenTab2
        | Action::OpenTab3
        | Action::OpenTab4
        | Action::OpenTab5
        | Action::OpenTab6
        | Action::OpenTab7
        | Action::OpenTab8
        | Action::OpenTab9 => {
            let n = Action::OPEN_TABS.iter().position(|a| *a == action);
            return state.switch_to_tab(n.unwrap_or(0));
        }
        Action::GoHome if state.route() == Some(&Route::Home) => state.info("Already home"),
        Action::GoHome => return state.push(Route::Home),
        Action::Search if diff.is_some() => {
            let input = nav::new_input(&state.theme, Bg::Container);
            state.overlay = Some(Overlay::DiffSearch(Box::new(input)));
        }
        Action::Search => return state.open_search(),
        Action::FindFile if diff.is_some() => return state.open_picker(picker::Kind::DiffFiles),
        Action::FindFile => return state.open_finder(false),
        Action::Menu => nav::open_menu(state),
        Action::CommandPalette => return state.open_picker(picker::Kind::Commands),
        Action::Messages => {
            let kept = state.messages.iter().rev().cloned().collect();
            return state.open_picker(picker::Kind::Messages(kept));
        }
        Action::Refresh => return state.load_visible(true),
        Action::Copy => return nav::copy_link(state),
        Action::OpenInBrowser => return state.go(Target::External(state.here_url())),
        _ if act::ACTIONS.contains(&action) => return act::act(state, action),
        _ if diff.is_some() => return diff_action(state, action),
        _ => return nav::page_action(state, action),
    }
    Vec::new()
}

/// Actions on the diff screen: reviewing, and the rest on the diff itself.
#[must_use]
fn diff_action(state: &mut State, action: Action) -> Vec<Cmd> {
    let content = state.layout().content;
    let on_annotation = match (state.screen(), state.diff()) {
        (Screen::Diff(screen), Some(diff)) => {
            screen.focus == Pane::Diff
                && diff
                    .doc
                    .annotation_under(screen.cursor, screen.half)
                    .is_some()
        }
        _ => false,
    };
    match action {
        Action::Comment
        | Action::FileComment
        | Action::Suggest
        | Action::ResolveThread
        | Action::Delete
        | Action::UndoDelete
        | Action::SubmitReview
        | Action::ToggleSinceReview
        | Action::PickCommits => return review_action(state, action),
        Action::Open if on_annotation => return review_action(state, action),
        _ => {}
    }
    let State {
        screens,
        diffs,
        notice,
        ..
    } = &mut *state;
    let Screen::Diff(screen) = screens.last_mut() else {
        return Vec::new();
    };
    match diffs.get_mut(&screen.of) {
        Some(diff) => diff_screen::apply(screen, diff, action, content, notice),
        None => {
            *notice = Some(Notice::Info("The diff hasn't loaded yet".into()));
            Vec::new()
        }
    }
}

// ---- reviewing ---------------------------------------------------------------

impl State {
    /// Opens the composer on `target`, starting with `text`.
    #[must_use]
    pub fn compose(&mut self, target: ComposeTarget, text: &str) -> Vec<Cmd> {
        let compose = Compose::new(&self.theme, target, text);
        self.overlay = Some(Overlay::Compose(Box::new(compose)));
        Vec::new()
    }

    /// The diff on screen, if it's one.
    pub fn diff(&self) -> Option<&DiffState> {
        match self.screen() {
            Screen::Diff(screen) => self.diffs.get(&screen.of),
            Screen::Page(_) => None,
        }
    }

    pub(crate) fn diff_parts(&mut self) -> Option<(&mut DiffScreen, &mut DiffState)> {
        let State { screens, diffs, .. } = self;
        let Screen::Diff(screen) = screens.last_mut() else {
            return None;
        };
        let diff = diffs.get_mut(&screen.of)?;
        Some((screen, diff))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::fixtures::{answer, diff_msg, fetched, press, thread};
    use crate::picker::{Choice, fuzzy_score};
    use crate::route::OPEN;
    use ghtui_api::browse::IssueState;
    use ghtui_api::browse::SearchResults;
    use ghtui_api::model::RepoId;
    use ghtui_theme::{ColorDepth, DEFAULT_SEED, Mode};
    use ghtui_ui::annotations::AnnotationKey;
    use ghtui_ui::overlays::PALETTE_ROWS;
    use ghtui_ui::pages::PrTab;

    /// The overlay showing, which must be a `$kind`.
    macro_rules! overlay {
        ($state:expr, $kind:ident) => {
            match &$state.overlay {
                Some(Overlay::$kind(overlay)) => overlay,
                _ => panic!("no {} overlay", stringify!($kind)),
            }
        };
    }

    pub(crate) fn state() -> State {
        let mut state = State::new(
            Theme::new(DEFAULT_SEED, Mode::Dark, ColorDepth::TrueColor),
            Icons::default(),
            Keymap::default(),
            (100, 30),
        );
        // Never written: changes to Home's sections are commands.
        state.config_path = Some("config.toml".into());
        state
    }

    /// Your `n` pull requests of `total`, as Home's second section finds them.
    fn mine(n: u64, total: u64) -> SearchResults {
        let row = |n| {
            crate::fixtures::found_pr(&format!("o/r#{n}"), &format!("PR {n}"), IssueState::Open)
        };
        crate::fixtures::found_prs((1..=n).map(row).collect(), total)
    }

    /// Home's second section: your pull requests.
    fn mine_key(state: &State) -> DataKey {
        state.home[1].search.clone().unwrap().key()
    }

    /// Home's second section's fetch failed.
    fn mine_failed(state: &mut State, err: ApiError) {
        let key = mine_key(state);
        update(
            state,
            Msg::Fetched {
                key,
                result: Err(err),
                cached_at: None,
            },
        );
    }

    fn with_home(n: u64) -> State {
        let mut state = state();
        crate::fixtures::section(&mut state, 1, mine(n, n));
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
        let item = p.page().items[p.selected.expect("a selection")];
        p.page().lines[item.start].text()
    }

    /// Fetching `key`, the cached copy first.
    fn fetch(key: DataKey) -> Cmd {
        Cmd::Api(Api::Fetch { key, cached: true })
    }

    /// Commands other than saving visits.
    #[must_use]
    fn fetches(cmds: Vec<Cmd>) -> Vec<Cmd> {
        cmds.into_iter()
            .filter(|c| !matches!(c, Cmd::Api(Api::SaveVisits(_))))
            .collect()
    }

    fn repo() -> RepoId {
        RepoId::new("gold-silver-copper", "ghtui")
    }

    fn issues(query: &str) -> Route {
        let (repo, query) = (repo(), query.to_owned());
        Route::Issues { repo, query }
    }

    fn issue(number: u64) -> Route {
        let repo = repo();
        Route::Issue { repo, number }
    }

    fn blob(rev: &str, path: &str) -> Route {
        Route::blob(repo(), rev.to_owned(), path.to_owned())
    }

    fn with_repo() -> State {
        let mut state = state();
        let _ = state.push(Route::Repo(repo()));
        fetched(
            &mut state,
            DataKey::Repo(repo()),
            Data::Repo(Box::new(crate::fixtures::overview())),
        );
        let readme = Some(Box::new(crate::fixtures::readme()));
        fetched(&mut state, DataKey::Readme(repo()), Data::Readme(readme));
        state
    }

    #[must_use]
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
        let cols = ghtui_ui::page::columns(p.page(), area);
        for (row, line) in p.page().lines.iter().enumerate().skip(p.scroll) {
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
        let mut state = with_home(3);
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
                fetch(DataKey::PrActivity(pr.clone()))
            ]
        );
        assert_eq!(route(&state), Route::pr(pr.clone()));
        assert_eq!(state.busy().as_deref(), Some("Loading"));

        update(
            &mut state,
            Msg::Pr(pr.clone(), Box::new(Err(ApiError::Network("down".into())))),
        );
        let text: Vec<String> = page(&state)
            .page()
            .lines
            .iter()
            .map(ghtui_ui::page::PageLine::text)
            .collect();
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
        assert_eq!(route(&state), Route::pr(pr), "forward again");
        press(&mut state, "H");
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
        assert_eq!(p.scroll, p.page().height() - state.page_height());
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
        let _ = state.push(issue(14));
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
        let compose = overlay!(state, Compose);
        assert!(
            compose.text().starts_with("> @hubot wrote:\n> Agreed."),
            "{}",
            compose.text()
        );
    }

    #[test]
    fn duplicate_fetches_are_suppressed() {
        let mut state = state();
        let sections: Vec<Cmd> = (state.home.iter())
            .map(|s| fetch(s.search.clone().unwrap().key()))
            .collect();
        assert_eq!(state.load_visible(false), sections);
        assert_eq!(state.load_visible(true), vec![Cmd::Api(Api::FetchViewer)]);
    }

    #[test]
    fn cached_data_shows_while_fetching() {
        let mut state = state();
        let _ = state.push(Route::user("octocat"));
        update(
            &mut state,
            Msg::Fetched {
                key: DataKey::Profile("octocat".into()),
                result: Ok(Data::Profile(Box::new(crate::fixtures::profile()))),
                cached_at: Some(0),
            },
        );
        assert!(page(&state).page().lines[0].text().contains("The Octocat"));
        let busy = state.busy().unwrap_or_default();
        assert!(busy.starts_with("Refreshing · cached "), "{busy}");
    }

    #[test]
    fn failed_refresh_keeps_cached_data() {
        let mut state = with_home(2);
        let _ = state.load_visible(true);
        mine_failed(&mut state, ApiError::RateLimited(30));
        assert!(selected_text(&state).contains("PR 1"));
        let first = page(&state).page().lines[0].text();
        assert!(
            first.contains("Couldn't refresh") && first.contains("rate limited"),
            "{first}"
        );
    }

    /// One outage that fails each of the page's needs is said once.
    #[test]
    fn a_refresh_error_shared_by_needs_is_said_once() {
        let mut state = with_home(2);
        let repos = state.home[2].search.clone().unwrap().key();
        let fetched = |result| Msg::Fetched {
            key: repos.clone(),
            result,
            cached_at: None,
        };
        let none = SearchResults::Repos(ghtui_api::browse::Results::default());
        update(&mut state, fetched(Ok(Data::Search(Box::new(none)))));
        let _ = state.load_visible(true);
        mine_failed(&mut state, ApiError::RateLimited(30));
        update(&mut state, fetched(Err(ApiError::RateLimited(30))));
        let first = page(&state).page().lines[0].text();
        assert_eq!(first.matches("rate limited").count(), 1, "{first}");
    }

    /// A section shows its search's first rows; the rest are a row that
    /// opens the search's own list in ghtui: the same fetch, so the same
    /// order and count.
    #[test]
    fn a_section_opens_the_rest_of_its_list() {
        let mut state = state();
        crate::fixtures::section(&mut state, 1, mine(30, 31));
        state.home[1].search.as_mut().unwrap().rows = 5;
        state.data_gen += 1;
        state.sync_page();
        let text = page(&state)
            .page()
            .lines
            .iter()
            .map(ghtui_ui::page::PageLine::text)
            .collect::<Vec<_>>();
        assert!(
            text.iter()
                .any(|l| l.contains("Your pull requests  5 of 31")),
            "{text:?}"
        );
        press(&mut state, "G");
        assert_eq!(selected_text(&state).trim(), "… 26 more");
        let key = mine_key(&state);
        press(&mut state, "<Enter>");
        let DataKey::Search(kind, query) = key.clone() else {
            panic!("{key:?}")
        };
        assert_eq!(route(&state), Route::Search { kind, query });
        assert!(selected_text(&state).contains("PR 1"), "the same list");
        press(&mut state, "<Esc>");
        let cmds = press(&mut state, "o");
        let [Cmd::OpenUrl(url)] = cmds.as_slice() else {
            panic!("{cmds:?}")
        };
        assert!(url.contains("author:@me"), "{url}");
    }

    #[test]
    fn browser_and_copy_use_the_selection_or_the_page() {
        let mut state = with_home(1);
        assert_eq!(
            press(&mut state, "o"),
            vec![Cmd::OpenUrl("https://github.com/o/r/pull/1".into())]
        );
        assert_eq!(
            press(&mut state, "y"),
            vec![Cmd::Copy("https://github.com/o/r/pull/1".into())]
        );
        let _ = state.push(issues("is:closed"));
        assert_eq!(
            press(&mut state, "o"),
            vec![Cmd::OpenUrl(
                "https://github.com/gold-silver-copper/ghtui/issues?q=is:closed".into()
            )]
        );
    }

    #[test]
    fn palette_goes_places_and_runs_actions() {
        let mut state = with_home(1);
        press(&mut state, ":");
        assert!(matches!(state.overlay, Some(Overlay::Picker(_))));
        let cmds = fetches(press(&mut state, "a/b#9<Enter>"));
        let key = DataKey::Issue(RepoId::new("a", "b"), 9);
        assert_eq!(
            cmds,
            vec![
                fetch(DataKey::Repo(RepoId::new("a", "b"))),
                fetch(key.clone())
            ]
        );
        assert!(state.overlay.is_none());

        // The number is a pull request: GitHub's redirect.
        let cmds = fetched(&mut state, key, Data::Issue(None));
        let pr = PrRef::parse("a/b#9").unwrap();
        assert_eq!(route(&state), Route::pr(pr.clone()));
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

    /// A branch's commits page: the code toolbar's link opens it, and it
    /// loads more like any list.
    #[test]
    fn commit_history_loads_more() {
        let mut state = with_repo();
        let url = format!("https://github.com/{}/commits/main/src", repo());
        let Target::Page(route) = Target::from_url(&url) else {
            panic!("{url}");
        };
        let key = DataKey::History(repo(), "main".into(), "src".into());
        let cmds = state.go(Target::Page(route));
        assert!(cmds.contains(&fetch(key.clone())), "{cmds:?}");
        fetched(
            &mut state,
            key.clone(),
            Data::History(Box::new(crate::fixtures::history(Some("h1")))),
        );
        press(&mut state, "G");
        assert!(selected_text(&state).starts_with("Load more"));
        let more_at = page(&state).selected;
        let cmds = press(&mut state, "<Enter>");
        assert_eq!(
            cmds,
            vec![Cmd::Api(Api::FetchMore {
                key: key.clone(),
                after: "h1".into()
            })]
        );
        // The next page: other commits.
        let mut more = crate::fixtures::history(None);
        for c in &mut more.items {
            c.oid = c.oid.replace('a', "b");
        }
        update(
            &mut state,
            Msg::FetchedMore(key.clone(), "h1".into(), Ok(Data::History(Box::new(more)))),
        );
        let Some(Data::History(h)) = state.get(&key) else {
            panic!()
        };
        assert_eq!((h.items.len(), h.next.as_deref()), (4, None));
        // The first commit loaded takes the place of "Load more".
        let p = page(&state);
        assert_eq!(p.selected, more_at);
        assert!(matches!(
            p.selected_link(),
            Some(ghtui_ui::page::Link::Url(_))
        ));
    }

    /// A list extended with Load more is fetched again only when asked: a
    /// pending run or a write elsewhere would cut it back to its first page.
    #[test]
    fn an_extended_list_is_kept_until_asked() {
        let mut state = with_repo();
        let file = "ci.yml".to_owned();
        let key = DataKey::WorkflowRuns(repo(), file.clone());
        let _ = state.go(Target::Page(Route::Workflow { repo: repo(), file }));
        let page = |id: u64, next: Option<&str>| {
            let mut runs = crate::fixtures::workflow_runs().1;
            for (i, run) in runs.items.iter_mut().enumerate() {
                (run.id, run.outcome) = (id + i as u64, ghtui_api::browse::CheckOutcome::Pending);
            }
            runs.next = next.map(str::to_owned);
            Data::Runs(Box::new(runs))
        };
        let refetch = Cmd::Api(Api::Fetch {
            key: key.clone(),
            cached: false,
        });
        fetched(&mut state, key.clone(), page(10, Some("2")));
        assert!(state.load_visible(false).contains(&refetch), "pending runs");
        fetched(&mut state, key.clone(), page(10, Some("2")));
        press(&mut state, "G");
        press(&mut state, "<Enter>");
        update(
            &mut state,
            Msg::FetchedMore(key.clone(), "2".into(), Ok(page(20, None))),
        );
        let star = Change::Star {
            repo: repo(),
            id: NodeId::new("R_1"),
            starred: true,
        };
        update(&mut state, Msg::Changed(star, act::By::Star, Ok(())));
        assert!(!state.load_visible(false).contains(&refetch));
        let Some(Data::Runs(runs)) = state.get(&key) else {
            panic!()
        };
        assert_eq!(runs.items.len(), 4);
        assert!(
            state.load_visible(true).contains(&refetch),
            "r refetches it"
        );
    }

    /// A review GitHub turned down whole changed nothing there, so what's
    /// held stays fresh; one that went through makes it stale.
    #[test]
    fn only_a_review_that_went_through_counts_as_a_write() {
        let mut state = with_repo();
        let key = DataKey::Issue(repo(), 14);
        let _ = state.push(issue(14));
        let issue = Data::Issue(Some(Box::new(crate::fixtures::issue())));
        fetched(&mut state, key.clone(), issue);
        let of = DiffOf::Pr(PrRef::parse("o/r#1").unwrap());
        let mut outcome = crate::review::SubmitOutcome {
            error: Some("Couldn't reach the PR".into()),
            ..Default::default()
        };
        let submitted = |o| Msg::Diff(of.clone(), DiffMsg::ReviewSubmitted(o));
        update(&mut state, submitted(outcome.clone()));
        assert_eq!(state.load_visible(false), Vec::new());
        outcome.accepted.push(1);
        update(&mut state, submitted(outcome));
        let refetch = Cmd::Api(Api::Fetch { key, cached: false });
        assert!(state.load_visible(false).contains(&refetch));
    }

    /// A next page asked for before a refresh belongs to the old list: it
    /// lands after the refreshed list's own Load more, and must neither be
    /// appended to the new list nor make that newer page be dropped.
    #[test]
    fn a_next_page_from_before_a_refresh_is_not_appended() {
        let mut state = with_repo();
        let url = format!("https://github.com/{}/commits/main/src", repo());
        let Target::Page(route) = Target::from_url(&url) else {
            panic!("{url}");
        };
        let key = DataKey::History(repo(), "main".into(), "src".into());
        let _ = state.go(Target::Page(route));
        // Commits named `c0`, `c1`, ...
        let page = |c: char, next: Option<&str>| {
            let mut page = crate::fixtures::history(next);
            for (i, commit) in page.items.iter_mut().enumerate() {
                commit.oid = format!("{c}{i}");
            }
            Data::History(Box::new(page))
        };
        fetched(&mut state, key.clone(), page('a', Some("h1")));
        press(&mut state, "G");
        let old = press(&mut state, "<Enter>");
        assert_eq!(
            old,
            vec![Cmd::Api(Api::FetchMore {
                key: key.clone(),
                after: "h1".into()
            })]
        );
        // r, and the refreshed list arrives before the old next page.
        let _ = press(&mut state, "r");
        fetched(&mut state, key.clone(), page('c', Some("h2")));
        press(&mut state, "G");
        let new = press(&mut state, "<Enter>");
        assert_eq!(
            new,
            vec![Cmd::Api(Api::FetchMore {
                key: key.clone(),
                after: "h2".into()
            })]
        );
        // The page after the old list's h1, then the one after h2.
        update(
            &mut state,
            Msg::FetchedMore(key.clone(), "h1".into(), Ok(page('b', None))),
        );
        update(
            &mut state,
            Msg::FetchedMore(key.clone(), "h2".into(), Ok(page('d', None))),
        );
        let Some(Data::History(h)) = state.get(&key) else {
            panic!()
        };
        let oids: Vec<&str> = h.items.iter().map(|c| c.oid.as_str()).collect();
        assert_eq!(
            oids,
            ["c0", "c1", "d0", "d1"],
            "the refreshed list then its own next page"
        );
    }

    /// Load more pressed while the list refreshes isn't dropped: it's
    /// asked of the refreshed list once that's in.
    #[test]
    fn load_more_during_a_refresh_waits_for_it() {
        let mut state = with_repo();
        let url = format!("https://github.com/{}/commits/main/src", repo());
        let Target::Page(route) = Target::from_url(&url) else {
            panic!("{url}");
        };
        let key = DataKey::History(repo(), "main".into(), "src".into());
        let _ = state.go(Target::Page(route));
        let page = |next| Data::History(Box::new(crate::fixtures::history(next)));
        fetched(&mut state, key.clone(), page(Some("h1")));
        let _ = press(&mut state, "r");
        press(&mut state, "G");
        assert_eq!(press(&mut state, "<Enter>"), Vec::new());
        let cmds = fetched(&mut state, key.clone(), page(Some("h2")));
        let more = Cmd::Api(Api::FetchMore {
            key,
            after: "h2".into(),
        });
        assert_eq!(cmds, vec![more]);
    }

    /// A refresh that fails leaves the list as it was, so the next page
    /// asked for before it still belongs there.
    #[test]
    fn a_next_page_survives_a_failed_refresh() {
        let mut state = with_repo();
        let url = format!("https://github.com/{}/commits/main/src", repo());
        let Target::Page(route) = Target::from_url(&url) else {
            panic!("{url}");
        };
        let key = DataKey::History(repo(), "main".into(), "src".into());
        let _ = state.go(Target::Page(route));
        let page = |next: Option<&str>| Data::History(Box::new(crate::fixtures::history(next)));
        fetched(&mut state, key.clone(), page(Some("h1")));
        press(&mut state, "G");
        press(&mut state, "<Enter>");
        let _ = press(&mut state, "r");
        let result = Err(ApiError::Network("offline".into()));
        let cached_at = None;
        update(
            &mut state,
            Msg::Fetched {
                key: key.clone(),
                result,
                cached_at,
            },
        );
        update(
            &mut state,
            Msg::FetchedMore(key.clone(), "h1".into(), Ok(page(None))),
        );
        let Some(Data::History(h)) = state.get(&key) else {
            panic!()
        };
        // The same commits, so only the cursor moves.
        assert_eq!(h.next, None, "the next page was dropped");
    }

    /// Sections are changed from Home: moved, renamed, removed and
    /// brought back, each written to config.toml; Home then shows what the
    /// file says.
    #[test]
    fn home_sections_are_changed_in_the_config() {
        use crate::home::Edit;
        let mut state = with_home(2);
        press(&mut state, "j");
        assert_eq!(state.section_here(), Some(1));
        let cmds = press(&mut state, "<A-Up>");
        let [Cmd::EditHome { edit, shown, .. }] = cmds.as_slice() else {
            panic!("{cmds:?}")
        };
        assert_eq!(*edit, Edit::Move { at: 1, up: true });
        assert_eq!(*shown, crate::home::defaults());
        assert!(
            press(&mut state, "<A-Up>").is_empty(),
            "one change at a time"
        );
        let mut moved = crate::home::defaults();
        moved.swap(0, 1);
        update(
            &mut state,
            Msg::HomeEdited {
                edit: edit.clone(),
                result: Ok((moved.clone(), None)),
                what: "Moved".into(),
            },
        );
        assert_eq!(state.home, moved);
        assert_eq!(state.section_here(), Some(0), "the selection moved with it");
        let cmds = press(&mut state, "<Delete>");
        let [Cmd::EditHome { edit, .. }] = cmds.as_slice() else {
            panic!("{cmds:?}")
        };
        assert_eq!(*edit, Edit::Remove { at: 0 });
        let (removed, table) = (moved[1..].to_vec(), "[[home]]\ntitle = \"x\"".to_owned());
        update(
            &mut state,
            Msg::HomeEdited {
                edit: edit.clone(),
                result: Ok((removed, Some(table.clone()))),
                what: "Removed".into(),
            },
        );
        let cmds = press(&mut state, "<C-z>");
        assert!(
            matches!(cmds.as_slice(), [Cmd::EditHome { edit: Edit::Restore { at: 0, table: t }, .. }] if *t == table),
            "{cmds:?}"
        );
        // A change GitHub's file refused says why.
        update(
            &mut state,
            Msg::HomeEdited {
                edit: Edit::Remove { at: 0 },
                result: Err("config.toml changed since ghtui read it".into()),
                what: String::new(),
            },
        );
        assert!(matches!(&state.notice, Some(Notice::Error(e)) if e.contains("changed since")));
    }

    /// Any list of issues, pull requests or repositories can be saved to
    /// Home, under a title you give it.
    #[test]
    fn a_list_is_saved_to_home() {
        let mut state = state();
        let _ = state.push(Route::Search {
            kind: ghtui_api::browse::SearchKind::Issues,
            query: "repo:o/r is:pr label:bug".into(),
        });
        let _ = act(&mut state, Action::SaveSection);
        let cmds = press(&mut state, "<C-u>Bugs<Enter>");
        assert!(state.overlay.is_none(), "the prompt closed");
        assert_eq!(
            cmds,
            vec![Cmd::EditHome {
                path: "config.toml".into(),
                shown: crate::home::defaults(),
                edit: crate::home::Edit::Add {
                    title: "Bugs".into(),
                    kind: ghtui_api::browse::SearchKind::Pulls,
                    query: "repo:o/r is:pr label:bug".into(),
                },
                what: "Added “Bugs” to Home".into(),
            }]
        );
        let _ = state.push(Route::user("octocat"));
        let _ = act(&mut state, Action::SaveSection);
        assert!(state.overlay.is_none());
    }

    /// Home on screen fetches its sections again once they're old, one
    /// fetch at a time; a Home in another tab or history doesn't.
    #[test]
    fn home_stays_fresh_only_on_screen() {
        static NOW: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1000);
        let mut state = state();
        state.clock = || NOW.load(std::sync::atomic::Ordering::Relaxed);
        let _ = state.load_visible(false);
        let mut last = None;
        let cmds = timers(&mut state, &mut last);
        let home = Cmd::Timer(Timer::Home, crate::home::FRESH_SECS * 1000);
        assert!(cmds.contains(&home), "{cmds:?}");
        assert!(!timers(&mut state, &mut last).contains(&home), "one timer");
        // Still loading: nothing more is asked.
        NOW.store(
            1000 + crate::home::FRESH_SECS,
            std::sync::atomic::Ordering::Relaxed,
        );
        assert!(update(&mut state, Msg::Timer(Timer::Home)).is_empty());
        crate::fixtures::section(&mut state, 1, mine(1, 1));
        NOW.store(
            1000 + 2 * crate::home::FRESH_SECS,
            std::sync::atomic::Ordering::Relaxed,
        );
        let asked = |cmds: &[Cmd], key: &DataKey| {
            cmds.iter()
                .any(|c| matches!(c, Cmd::Api(Api::Fetch { key: k, .. }) if k == key))
        };
        let cmds = update(&mut state, Msg::Timer(Timer::Home));
        assert_eq!(cmds.len(), 1, "{cmds:?}");
        assert!(asked(&cmds, &mine_key(&state)), "{cmds:?}");
        // Away from Home, its timer does nothing and isn't set again.
        let _ = state.push(Route::user("octocat"));
        crate::fixtures::section(&mut state, 1, mine(1, 1));
        NOW.store(
            1000 + 4 * crate::home::FRESH_SECS,
            std::sync::atomic::Ordering::Relaxed,
        );
        assert!(update(&mut state, Msg::Timer(Timer::Home)).is_empty());
        assert!(!timers(&mut state, &mut last).contains(&home));
        // Back home, what's old is fetched again.
        let cmds = state.back();
        assert!(asked(&cmds, &mine_key(&state)), "{cmds:?}");
    }

    /// Home's sections from the disk cache are from an earlier session:
    /// going back to Home fetches them.
    #[test]
    fn going_back_home_fetches_cached_sections() {
        let mut state = state();
        let key = mine_key(&state);
        let (value, fetched_at) = (Data::Search(Box::new(mine(1, 1))), 0);
        let cached = Remote::cached(Some(ghtui_store::Cached { value, fetched_at }));
        state.data.insert(key.clone(), cached);
        let _ = state.start_at(Target::Page(Route::Repo(repo())));
        let cmds = state.back();
        let asked = |c: &Cmd| matches!(c, Cmd::Api(Api::Fetch { key: k, .. }) if *k == key);
        assert!(cmds.iter().any(asked), "{cmds:?}");
    }

    /// Live suggestions answer in any order: an answer for an older input
    /// doesn't replace the one for what's typed.
    #[test]
    fn an_older_suggestion_reply_does_not_replace_a_newer_one() {
        let mut state = with_repo();
        press(&mut state, "/");
        press(&mut state, "ra");
        let cmds = update(&mut state, Msg::SuggestDue("ra".into()));
        assert_eq!(cmds, vec![Cmd::Api(Api::Suggest("ra".into()))]);
        press(&mut state, "t");
        let cmds = update(&mut state, Msg::SuggestDue("rat".into()));
        assert_eq!(cmds, vec![Cmd::Api(Api::Suggest("rat".into()))]);
        let repos = |name: &str| Ok(vec![crate::fixtures::repo_summary(name, 5)]);
        update(&mut state, Msg::Suggested("rat".into(), repos("o/ratatui")));
        update(&mut state, Msg::Suggested("ra".into(), repos("o/rails")));
        let sb = overlay!(state, Search);
        let picks: Vec<nav::Pick> = state
            .suggestions(sb)
            .into_iter()
            .filter_map(|(_, p)| p)
            .collect();
        assert_eq!(
            picks.last(),
            Some(&nav::Pick::Go(Target::Page(Route::Repo(RepoId::new(
                "o", "ratatui"
            ))))),
            "the suggestions for `rat` vanished"
        );
    }

    /// `ghtui <pr>/files` with the PR in the disk cache: the cached copy is
    /// from an earlier session, so the PR is fetched again (it may have
    /// been merged or force-pushed since).
    #[test]
    fn a_pr_diff_opened_from_a_cached_pr_fetches_the_pr() {
        let mut state = state();
        let pr = PrRef::parse("o/r#1").unwrap();
        state.prs.insert(
            pr.clone(),
            Remote::cached(Some(ghtui_store::Cached {
                value: crate::snapshot_tests::pr_detail(),
                fetched_at: 0,
            })),
        );
        let cmds = state.start_at(Target::Files(DiffOf::Pr(pr.clone())));
        assert!(
            cmds.contains(&Cmd::Api(Api::FetchPr(pr))),
            "the cached PR is never refreshed: {cmds:?}"
        );
    }

    /// Security advisories come a page at a time; past the first page,
    /// the list offers the rest like any other list.
    #[test]
    fn advisories_load_more() {
        let mut state = with_repo();
        let route = Route::Advisories(Some(repo()));
        let key = DataKey::Advisories(Some(repo()));
        let cmds = state.go(Target::Page(route));
        assert!(cmds.contains(&fetch(key.clone())), "{cmds:?}");
        let list = ghtui_api::browse::Results {
            total: 3,
            items: crate::fixtures::advisories(),
            next: Some("c1".into()),
        };
        fetched(&mut state, key.clone(), Data::Advisories(Box::new(list)));
        press(&mut state, "G");
        assert!(
            selected_text(&state).starts_with("Load more"),
            "{:?}",
            selected_text(&state)
        );
        let cmds = press(&mut state, "<Enter>");
        assert_eq!(
            cmds,
            vec![Cmd::Api(Api::FetchMore {
                key,
                after: "c1".into()
            })]
        );
    }

    /// An organization's People box shows a few members; it says how many
    /// it left out, as every other cut-short list does.
    #[test]
    fn org_people_say_how_many_are_left_out() {
        let mut state = state();
        let _ = state.push(Route::user("ratatui"));
        // 3 of 14 members.
        fetched(
            &mut state,
            DataKey::Profile("ratatui".into()),
            Data::Profile(Box::new(crate::fixtures::org_profile())),
        );
        let lines: Vec<String> = page(&state)
            .page()
            .lines
            .iter()
            .map(ghtui_ui::page::PageLine::text)
            .collect();
        let people = lines
            .iter()
            .position(|l| l.contains("People"))
            .unwrap_or_else(|| panic!("{lines:#?}"));
        assert!(
            lines
                .iter()
                .skip(people)
                .take_while(|l| !l.contains("Top languages"))
                .any(|l| l.contains("11 more")),
            "{lines:#?}"
        );
    }

    /// A list that stops short of GitHub's count, as a milestone's past
    /// search's 1000 does, keeps the count and says what's left out.
    #[test]
    fn a_list_cut_short_keeps_its_count() {
        let mut state = with_repo();
        let (repo, number) = (repo(), 3);
        let _ = state.push(Route::Milestone {
            repo: repo.clone(),
            number,
        });
        let mut milestone = crate::fixtures::milestone();
        milestone.items.total = 1500;
        milestone.items.next = None;
        fetched(
            &mut state,
            DataKey::Milestone(repo, number),
            Data::Milestone(Box::new(milestone)),
        );
        let text: String = page(&state)
            .page()
            .lines
            .iter()
            .map(|l| l.text() + "\n")
            .collect();
        assert!(text.contains("2 of 1500"), "{text}");
        assert!(text.contains("1498 more on GitHub"), "{text}");
    }

    /// A profile's repositories sort by name and stars as well as last
    /// update, and `/` there searches that owner's repositories.
    #[test]
    fn profile_repositories_sort_and_filter() {
        use ghtui_api::browse::RepoSort;
        use ghtui_ui::pages::ProfileTab;
        let mut state = state();
        let tab = |sort| Route::User {
            login: "octocat".into(),
            tab: ProfileTab::Repositories(sort),
        };
        let _ = state.push(tab(RepoSort::Updated));
        for sort in [RepoSort::Name, RepoSort::Stars, RepoSort::Updated] {
            let cmds = act(&mut state, Action::Sort);
            assert_eq!(route(&state), tab(sort));
            // Last updated is still loading from when the tab opened.
            let key = DataKey::OwnerRepos("octocat".into(), sort);
            assert!(
                sort == RepoSort::Updated || cmds.contains(&fetch(key)),
                "{sort:?}"
            );
        }
        press(&mut state, "/");
        let sb = overlay!(state, Search);
        assert_eq!(sb.input.lines().join(""), "user:octocat ");
    }

    mod tabs {
        use super::*;

        /// Two routes to tell tabs apart by.
        fn issue(n: u64) -> Route {
            Route::Issue {
                repo: repo(),
                number: n,
            }
        }

        #[test]
        fn open_switch_and_close() {
            let mut s = with_repo();
            assert_eq!(s.tab_count(), 1);
            // Nothing selected on a reading page: T opens it again.
            let _ = s.push(issue(1));
            let _ = act(&mut s, Action::OpenInTab);
            assert_eq!((s.tab_count(), s.active_tab()), (2, 1));
            assert_eq!(route(&s), issue(1));
            let _ = s.go(Target::Page(issue(2)));
            press(&mut s, "[");
            assert_eq!((s.active_tab(), route(&s)), (0, issue(1)));
            press(&mut s, "]");
            assert_eq!((s.active_tab(), route(&s)), (1, issue(2)));
            // Wrapping around, and straight to a number.
            press(&mut s, "]");
            assert_eq!(s.active_tab(), 0);
            press(&mut s, "<A-2>");
            assert_eq!(s.active_tab(), 1);
            press(&mut s, "<A-5>");
            assert!(matches!(&s.notice, Some(Notice::Info(n)) if n.contains("No tab 5")));
            // Closing shows the next tab, or the previous at the end.
            press(&mut s, "<C-w>");
            assert_eq!((s.tab_count(), route(&s)), (1, issue(1)));
            press(&mut s, "<C-w>");
            assert_eq!(s.tab_count(), 1);
            assert!(matches!(&s.notice, Some(Notice::Info(n)) if n.contains("last tab")));
        }

        /// Back and forward stay within a tab.
        #[test]
        fn each_tab_has_its_own_history() {
            let mut s = with_repo();
            let _ = s.push(issue(1));
            let _ = act(&mut s, Action::OpenInTab);
            let _ = s.push(issue(2));
            let _ = s.push(issue(3));
            act(&mut s, Action::Back);
            assert_eq!(route(&s), issue(2));
            press(&mut s, "[");
            assert_eq!(route(&s), issue(1));
            act(&mut s, Action::Forward);
            assert_eq!(route(&s), issue(1), "nothing to go forward to here");
            press(&mut s, "]");
            act(&mut s, Action::Forward);
            assert_eq!(route(&s), issue(3));
        }

        /// Two tabs on one pull request share its diff (and so its review).
        #[test]
        fn tabs_share_a_pull_requests_diff() {
            let (mut s, pr) = crate::state::tests::diff::diff_state(120);
            if let Screen::Diff(d) = s.screen_mut() {
                d.focus = crate::diff_screen::Pane::Tree;
            }
            let _ = act(&mut s, Action::OpenInTab);
            assert_eq!(s.tab_count(), 2);
            assert!(matches!(s.screen(), Screen::Diff(d) if d.of == DiffOf::Pr(pr.clone())));
            assert_eq!(s.diffs.len(), 1, "one diff for both tabs");
            // From the diff, T opens the pull request's page.
            let _ = act(&mut s, Action::OpenInTab);
            assert_eq!(route(&s), Route::pr(pr));
        }

        /// The tabs open at quit come back, in order, the first on screen.
        #[test]
        fn tabs_reopen() {
            let mut s = with_repo();
            let _ = s.push(issue(1));
            let _ = act(&mut s, Action::OpenInTab);
            let _ = s.push(issue(2));
            let urls = s.tab_urls();
            let mut again = state();
            again.restore_tabs(&urls);
            assert_eq!(again.tab_titles(), ["#1", "#2"]);
            assert_eq!((again.active_tab(), route(&again)), (0, issue(1)));
        }

        /// A tab opened from a link to a commit's or a range's diff
        /// reopens on what it showed: a diff by full ids stays a diff, and
        /// a short SHA or a ref (which open the commit page) stay pages.
        #[test]
        fn a_tab_from_a_diff_link_reopens_on_it() {
            let (a, b) = ("a".repeat(40), "b".repeat(40));
            for (link, diff) in [
                (
                    "https://github.com/o/r/commit/abc1234.diff".to_owned(),
                    false,
                ),
                ("https://github.com/o/r/commit/main.diff".to_owned(), false),
                (format!("https://github.com/o/r/commit/{a}.diff"), true),
                (format!("https://github.com/o/r/commit/{a}#files"), true),
                (
                    format!("https://github.com/o/r/compare/{a}..{b}#files"),
                    true,
                ),
            ] {
                let mut s = state();
                s.restore_tabs(std::slice::from_ref(&link));
                let shown = (s.tab_titles(), matches!(s.screen(), Screen::Diff(_)));
                assert_eq!(shown.1, diff, "{link}");
                let urls = s.tab_urls();
                let mut again = state();
                again.restore_tabs(&urls);
                let reopened = (
                    again.tab_titles(),
                    matches!(again.screen(), Screen::Diff(_)),
                );
                assert_eq!(reopened, shown, "{link} saved as {urls:?}");
            }
        }

        /// A tab on a branch with a slash in its name (`feature/x`)
        /// reopens on that branch, not on `feature` with `x` as a path.
        #[test]
        fn a_tab_on_a_slashed_branch_reopens_on_it() {
            let tree = Route::Tree {
                repo: RepoId::new("o", "r"),
                rev: "feature/x".into(),
                path: "src".into(),
            };
            let mut s = state();
            let _ = s.push(tree.clone());
            let urls = s.tab_urls();
            let mut again = state();
            again.restore_tabs(&urls);
            assert_eq!(route(&again), tree, "{urls:?}");
        }

        /// A page asked for on the command line loads, also when it's the
        /// last of the reopened tabs.
        #[test]
        fn the_page_asked_for_loads_after_reopened_tabs() {
            let releases = Route::Releases(RepoId::new("o", "r"));
            let mut s = state();
            s.restore_tabs(&[issue(1).url(), releases.url()]);
            let cmds = s.start_at(Target::Page(releases));
            assert_eq!(s.tab_count(), 3);
            let fetches = |key: &DataKey| {
                cmds.iter()
                    .any(|c| matches!(c, Cmd::Api(Api::Fetch { key: k, .. }) if k == key))
            };
            assert!(
                fetches(&DataKey::Releases(RepoId::new("o", "r"))),
                "{cmds:?}"
            );
        }

        /// A comment's link keeps its anchor however it's opened: in a new
        /// tab, or from the command line after reopened tabs.
        #[test]
        fn anchors_survive_every_way_in() {
            let anchor = |s: &State| match s.screen() {
                Screen::Page(p) => p.anchor.clone(),
                Screen::Diff(_) => None,
            };
            let url = "https://github.com/o/r/issues/1#issuecomment-5";
            let mut s = state();
            let _ = s.open_url(url, true);
            assert_eq!(s.tab_count(), 2);
            assert_eq!(anchor(&s).as_deref(), Some("issuecomment-5"));
            let mut s = state();
            s.restore_tabs(&[issue(2).url(), issue(3).url()]);
            let _ = s.start_at(Target::from_url(url));
            s.anchor_at(url);
            assert_eq!(anchor(&s).as_deref(), Some("issuecomment-5"));
        }

        /// A plain link to a file on a slashed branch waits for GitHub to
        /// say where the branch ends, then is that file, in its place.
        /// Links at the default branch or at the revision on screen don't
        /// ask.
        #[test]
        fn plain_links_open_the_branch_github_names() {
            let mut s = with_repo();
            let url = "https://github.com/gold-silver-copper/ghtui/blob/feature/x/src/lib.rs#L2";
            let key = DataKey::RefIn(repo(), "feature/x/src/lib.rs".into());
            assert_eq!(fetches(s.open_url(url, false)), vec![fetch(key.clone())]);
            let depth = s.screens.len();
            let cmds = fetched(&mut s, key, Data::RefIn(Some("feature/x".into())));
            let file = Route::Blob {
                repo: repo(),
                rev: "feature/x".into(),
                path: "src/lib.rs".into(),
                lines: Some((2, 2)),
            };
            assert_eq!(route(&s), file);
            assert_eq!(s.screens.len(), depth, "in its place");
            let text = DataKey::Blob(repo(), "feature/x".into(), "src/lib.rs".into());
            assert_eq!(fetches(cmds), vec![fetch(text)]);

            let base = "https://github.com/gold-silver-copper/ghtui";
            let _ = s.open_url(&format!("{base}/tree/feature/x/src"), false);
            assert_eq!(
                route(&s),
                Route::Tree {
                    repo: repo(),
                    rev: "feature/x".into(),
                    path: "src".into()
                }
            );
            let _ = s.open_url(&format!("{base}/blob/main/README.md"), false);
            assert_eq!(route(&s), blob("main", "README.md"));
        }

        /// A comment's link on an issue number that's a pull request keeps
        /// its anchor through GitHub's redirect to the pull request.
        #[test]
        fn an_issue_link_redirected_to_its_pr_keeps_the_anchor() {
            let url = "https://github.com/o/r/issues/7#issuecomment-5";
            let mut s = state();
            let _ = s.open_url(url, true);
            let anchor = |s: &State| match s.screen() {
                Screen::Page(p) => p.anchor.clone(),
                Screen::Diff(_) => None,
            };
            assert_eq!(anchor(&s).as_deref(), Some("issuecomment-5"));
            let _ = fetched(
                &mut s,
                DataKey::Issue(RepoId::new("o", "r"), 7),
                Data::Issue(None),
            );
            assert_eq!(
                s.route(),
                Some(&Route::pr(PrRef::parse("o/r#7").unwrap())),
                "redirected"
            );
            assert_eq!(anchor(&s).as_deref(), Some("issuecomment-5"));
        }

        /// A capital hint letter follows the link in a new tab.
        #[test]
        fn capital_hints_open_a_tab() {
            let mut s = with_repo();
            press(&mut s, "i");
            let label = overlay!(s, Hints).labels[0].label.to_ascii_uppercase();
            press(&mut s, &label);
            assert_eq!(s.tab_count(), 2);
        }
    }

    /// `/` on a job searches its log: the steps with a match open, the
    /// page opens at the first match, every match is picked out, and `n`
    /// `p` go through them. Nothing is hidden. Esc ends the search, back
    /// on the same page as it was.
    #[test]
    fn a_jobs_log_search_goes_through_its_matches() {
        let mut s = with_repo();
        let job_route = Route::Job {
            repo: repo(),
            run: Some(7),
            job: 2,
            step: Some((1, 1)),
            query: String::new(),
        };
        let _ = s.push(job_route.clone());
        let depth = s.screens.len();
        let (job, log) = crate::fixtures::job();
        fetched(&mut s, DataKey::Job(repo(), 2), Data::Job(Box::new(job)));
        fetched(&mut s, DataKey::JobLog(repo(), 2), Data::Log(Arc::new(log)));
        press(&mut s, "/");
        press(&mut s, "TEST");
        press(&mut s, "<Enter>");
        assert!(matches!(route(&s), Route::Job { query, step: Some(_), .. } if query == "TEST"));
        let Screen::Page(p) = s.screen() else {
            panic!()
        };
        let page = p.page();
        let line = |i: usize| -> String { page.lines[i].text() };
        assert!(!page.marks.is_empty());
        for &m in &page.marks {
            assert!(line(m).to_lowercase().contains("test"), "{}", line(m));
            let picked = page.lines[m].segs.iter().any(|s| {
                s.text().eq_ignore_ascii_case("test") && s.role == ghtui_ui::page::Role::Match
            });
            assert!(picked, "{}", line(m));
        }
        // The failed step's other lines are still there.
        assert!((0..page.lines.len()).any(|i| line(i).contains("Compiling")));
        assert_eq!(page.jump, page.marks.first().copied());
        let n = page.marks.len();
        assert!(n > 1, "{n}");
        assert_eq!(s.notice, Some(Notice::Info(format!("Match 1 of {n}"))));
        press(&mut s, "n");
        assert_eq!(s.notice, Some(Notice::Info(format!("Match 2 of {n}"))));
        let Screen::Page(p) = s.screen() else {
            panic!()
        };
        assert_eq!(p.current_match(), p.page().marks.get(1).copied());
        press(&mut s, "p");
        assert_eq!(s.notice, Some(Notice::Info(format!("Match 1 of {n}"))));
        press(&mut s, "<Esc>");
        assert_eq!(route(&s), job_route);
        assert_eq!(s.screens.len(), depth);
        press(&mut s, "/");
        press(&mut s, "no such text");
        press(&mut s, "<Enter>");
        let none = Notice::Info("No matches for “no such text”".into());
        assert_eq!(s.notice, Some(none.clone()));
        press(&mut s, "n");
        assert_eq!(s.notice, Some(none));
    }

    /// A failed job opens at its failing step's first error.
    #[test]
    fn a_failed_job_opens_at_its_first_error() {
        let mut s = with_repo();
        let _ = s.push(Route::Job {
            repo: repo(),
            run: Some(7),
            job: 2,
            step: None,
            query: String::new(),
        });
        let (job, log) = crate::fixtures::job();
        fetched(&mut s, DataKey::Job(repo(), 2), Data::Job(Box::new(job)));
        fetched(&mut s, DataKey::JobLog(repo(), 2), Data::Log(Arc::new(log)));
        let Screen::Page(p) = s.screen() else {
            panic!()
        };
        let page = p.page();
        let jump = page.jump.expect("it opens at a line");
        let text: String = page.lines[jump]
            .segs
            .iter()
            .map(ghtui_ui::page::Seg::text)
            .collect();
        assert!(text.contains("exit code 101"), "{text}");
    }

    /// A running job, and its log (which GitHub doesn't have until the
    /// job ends), are fetched again when their page is back on screen.
    #[test]
    fn a_running_jobs_log_is_checked_again() {
        let mut s = with_repo();
        let job_route = Route::Job {
            repo: repo(),
            run: Some(7),
            job: 2,
            step: None,
            query: String::new(),
        };
        let _ = s.push(job_route);
        let (mut job, _) = crate::fixtures::job();
        job.outcome = ghtui_api::browse::CheckOutcome::Pending;
        let running = ghtui_api::browse::JobLog {
            running: true,
            ..Default::default()
        };
        fetched(&mut s, DataKey::Job(repo(), 2), Data::Job(Box::new(job)));
        fetched(
            &mut s,
            DataKey::JobLog(repo(), 2),
            Data::Log(Arc::new(running)),
        );
        let _ = s.push(Route::Repo(repo()));
        let fetched_keys = |cmds: Vec<Cmd>| -> Vec<DataKey> {
            fetches(cmds)
                .into_iter()
                .filter_map(|c| match c {
                    Cmd::Api(Api::Fetch { key, .. }) => Some(key),
                    _ => None,
                })
                .collect()
        };
        let keys = fetched_keys(s.back());
        assert!(keys.contains(&DataKey::JobLog(repo(), 2)), "{keys:?}");
        assert!(keys.contains(&DataKey::Job(repo(), 2)), "{keys:?}");
        // A finished one isn't.
        let (job, log) = crate::fixtures::job();
        fetched(&mut s, DataKey::Job(repo(), 2), Data::Job(Box::new(job)));
        fetched(&mut s, DataKey::JobLog(repo(), 2), Data::Log(Arc::new(log)));
        let _ = s.push(Route::Repo(repo()));
        assert_eq!(fetched_keys(s.back()), Vec::new());
    }

    /// A link to a comment opens its conversation scrolled to it.
    #[test]
    fn comment_links_scroll_to_the_comment() {
        let mut s = state();
        s.size = (100, 12);
        let comment = "https://github.com/cli/cli/discussions/14603#discussioncomment-18765100";
        let _ = s.follow(&ghtui_ui::page::Link::Url(comment.into()));
        let of = ghtui_api::browse::DiscussionsOf::Repo(RepoId::new("cli", "cli"));
        let detail = Data::Discussion(Box::new(crate::fixtures::discussion()));
        fetched(&mut s, DataKey::Discussion(of, 14603), detail);
        let Screen::Page(p) = s.screen() else {
            panic!()
        };
        let line = p.page().anchors["discussioncomment-18765100"];
        assert!(
            p.scroll > 0 && p.scroll <= line && line < p.scroll + 10,
            "{} vs {line}",
            p.scroll
        );

        // Issue comments too.
        let mut s = with_repo();
        s.size = (100, 12);
        let url = format!(
            "https://github.com/{}/issues/14#issuecomment-1000001",
            repo()
        );
        let _ = s.follow(&ghtui_ui::page::Link::Url(url));
        let issue = Data::Issue(Some(Box::new(crate::fixtures::issue())));
        fetched(&mut s, DataKey::Issue(repo(), 14), issue);
        let Screen::Page(p) = s.screen() else {
            panic!()
        };
        assert!(p.page().anchors.contains_key("issuecomment-1000001"));
        assert!(p.scroll > 0);
    }

    /// A comment's link opened on a cached issue whose body has grown
    /// since: the fresh copy moves the comment, and the page follows it,
    /// unless you've scrolled away by then.
    #[test]
    fn a_comment_link_follows_its_comment_when_the_issue_refreshes() {
        let open = |moved: &dyn Fn(&mut State)| {
            let mut s = with_repo();
            s.size = (100, 12);
            let url = format!(
                "https://github.com/{}/issues/14#issuecomment-1000001",
                repo()
            );
            let _ = s.follow(&ghtui_ui::page::Link::Url(url));
            let key = DataKey::Issue(repo(), 14);
            let cached = Data::Issue(Some(Box::new(crate::fixtures::issue())));
            let _ = update(
                &mut s,
                Msg::Fetched {
                    key: key.clone(),
                    result: Ok(cached),
                    cached_at: Some(0),
                },
            );
            moved(&mut s);
            let shown = |s: &State| {
                let p = page(s);
                let lines = p.page().lines.iter().skip(p.scroll).take(10);
                (
                    p.scroll,
                    lines
                        .map(ghtui_ui::page::PageLine::text)
                        .collect::<Vec<_>>(),
                )
            };
            let at = shown(&s);
            let mut issue = crate::fixtures::issue();
            issue.body.push_str(&"\n\nMore.".repeat(20));
            fetched(&mut s, key, Data::Issue(Some(Box::new(issue))));
            (
                at,
                shown(&s),
                page(&s).page().anchors["issuecomment-1000001"],
            )
        };
        let ((before, _), (after, _), line) = open(&|_| {});
        assert!(after > before, "{before} -> {after}");
        assert!(after <= line && line < after + 10, "{after} vs {line}");
        let ((before, _), (after, _), _) = open(&|s| drop(press(s, "g")));
        assert_eq!(after, before, "yanked back after scrolling");
        // Selecting an item on screen, as j can, is moving too.
        let select = |s: &mut State| {
            let Screen::Page(p) = s.screen_mut() else {
                panic!("not on a page")
            };
            let shown = p.page().items.iter().position(|i| i.end > p.scroll);
            assert!(shown.is_some_and(|i| p.page().items[i].start < p.scroll + 10));
            p.selected = shown;
        };
        let ((_, before), (_, after), _) = open(&select);
        assert_eq!(after, before, "yanked back after selecting");
    }
    /// People and repository lists load more like any list.
    #[test]
    fn every_list_appends_its_next_page() {
        use crate::fixtures::{forks, people};
        let mut users = Data::Users(Box::new(people()));
        assert_eq!(users.next_cursor(), Some("u1"));
        let mut more = people();
        for u in &mut more.items {
            u.login.push_str("-2");
        }
        users.append(Data::Users(Box::new(more)));
        assert!(matches!(&users, Data::Users(r) if r.items.len() == 6));
        let mut repos = Data::RepoPage(Box::new(forks()));
        assert_eq!(repos.next_cursor(), Some("f1"));
        let mut more = forks();
        for r in &mut more.items {
            r.repo = RepoId::new(&r.repo.owner, format!("{}-2", r.repo.name));
        }
        repos.append(Data::RepoPage(Box::new(more)));
        assert!(matches!(&repos, Data::RepoPage(r) if r.items.len() == 4));
    }

    /// A commit's diff starts only the diff job; what belongs to pull
    /// requests says so.
    #[test]
    fn a_commit_diff_has_no_review() {
        let mut s = state();
        let of = DiffOf::Commit(RepoId::new("o", "r"), Oid::parse(&"a".repeat(40)).unwrap());
        let cmds = s.open_diff(of.clone());
        assert!(matches!(
            &cmds[..],
            [Cmd::Git(Git::LoadDiff { of: o, base, .. })]
                if *o == of && *base == crate::diff_job::PrBase::default()
        ));
        s.diffs.insert(of, crate::snapshot_tests::diff_fixture());
        for action in [Action::Comment, Action::ToggleViewed, Action::MarkReviewed] {
            s.notice = None;
            assert!(apply(&mut s, action).is_empty(), "{action:?}");
            assert!(
                matches!(&s.notice, Some(Notice::Info(n)) if n.contains("pull request")),
                "{action:?}: {:?}",
                s.notice
            );
        }
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
    fn palette_list_stays_in_bounds() {
        let mut state = state();
        press(&mut state, ":");
        for _ in 0..(state.commands("").len() + PALETTE_ROWS) {
            press(&mut state, "<Down>");
        }
        let palette = overlay!(state, Picker);
        assert_eq!(palette.selected, state.commands("").len() - 1);
    }

    #[test]
    fn a_repositorys_pages_are_under_their_tabs() {
        let mut state = with_repo();
        let active = |state: &State| {
            let c = state.chrome();
            c.active
                .and_then(|i| c.tabs.get(i))
                .map(|(t, _)| t.label.clone())
        };
        let pages = [
            (
                Route::Milestone {
                    repo: repo(),
                    number: 3,
                },
                "Issues",
            ),
            (
                Route::Discussions {
                    of: ghtui_api::browse::DiscussionsOf::Repo(repo()),
                    category: None,
                },
                "Discussions",
            ),
            (Route::Advisories(Some(repo())), "Security"),
            (
                Route::Wiki {
                    repo: repo(),
                    page: None,
                },
                "Wiki",
            ),
            (Route::Branches(repo()), "Code"),
            (
                Route::Job {
                    repo: repo(),
                    run: Some(1),
                    job: 2,
                    step: None,
                    query: String::new(),
                },
                "Actions",
            ),
        ];
        for (route, tab) in pages {
            let _ = state.push(route.clone());
            assert_eq!(active(&state).as_deref(), Some(tab), "{route:?}");
        }
        // Every tab has a number key: `5` is Actions, `7` Security.
        let _ = state.push(Route::Repo(repo()));
        press(&mut state, "5");
        assert_eq!(route(&state), Route::Actions(repo()));
        press(&mut state, "7");
        assert_eq!(route(&state), Route::Advisories(Some(repo())));
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
        assert_eq!(
            tabs,
            [
                "Code",
                "Issues",
                "Pull requests",
                "Discussions",
                "Actions",
                "Wiki",
                "Security"
            ]
        );
        let cmds = fetches(press(&mut state, "2"));
        let open = issues(OPEN);
        assert_eq!(route(&state), open);
        let (kind, query) = open.search().unwrap();
        assert_eq!(
            cmds,
            vec![fetch(DataKey::Search(kind, query))],
            "the header isn't fetched again"
        );
        assert_eq!(state.chrome().active, Some(1));
        // r refreshes the list's counts too, which the header holds.
        let cmds = fetches(press(&mut state, "r"));
        let header = Cmd::Api(Api::Fetch {
            key: DataKey::Repo(repo()),
            cached: false,
        });
        assert!(cmds.contains(&header), "{cmds:?}");

        // `/` on a list edits its filter.
        press(&mut state, "/");
        let sb = overlay!(state, Search);
        assert!(sb.filter);
        assert_eq!(sb.input.lines().join(""), "is:open ");
        press(&mut state, "label:bug<Enter>");
        assert_eq!(route(&state), issues("is:open label:bug"));
        act(&mut state, Action::ToggleState);
        assert_eq!(route(&state), issues("is:closed label:bug"));
        act(&mut state, Action::Sort);
        assert_eq!(
            route(&state),
            issues("is:closed label:bug sort:created-asc")
        );

        press(&mut state, "<Right>");
        assert!(
            matches!(route(&state), Route::Pulls { .. }),
            "→: the next tab"
        );
        press(&mut state, "1");
        assert_eq!(route(&state), Route::Repo(repo()));

        let cmds = press(&mut state, "s");
        let star = Change::Star {
            repo: repo(),
            id: NodeId::new("R_ghtui"),
            starred: true,
        };
        assert_eq!(cmds, vec![Cmd::Api(Api::Change(star, act::By::Star))]);
        assert!(state.overview(&repo()).unwrap().starred);
        assert_eq!(state.overview(&repo()).unwrap().summary.stars, 1235);
        answer(&mut state, &cmds, Err(ApiError::Network("down".into())));
        assert!(!state.overview(&repo()).unwrap().starred, "rolled back");
        assert_eq!(state.overview(&repo()).unwrap().summary.stars, 1234);
    }

    #[test]
    fn letter_hints_follow_links() {
        let mut state = with_repo();
        press(&mut state, "i");
        let hints = overlay!(state, Hints);
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
                fetch(tree.clone()),
                fetch(DataKey::LastCommits(repo(), "main".into(), "crates".into()))
            ],
            "the listing, and each entry's latest commit"
        );
        fetched(&mut state, tree, Data::Tree(crate::fixtures::tree()));
        while !selected_text(&state).contains("README.md") {
            press(&mut state, "j");
        }
        press(&mut state, "<Enter>");
        assert_eq!(route(&state), blob("main", "crates/README.md"));
    }

    /// A list the picker couldn't fetch says why, rather than loading for
    /// good, and opening the picker again asks again.
    #[test]
    fn a_picker_list_that_failed_says_why() {
        let mut state = with_repo();
        for (keys, key, why) in [
            (
                "f",
                DataKey::Files(repo(), "main".into()),
                "Couldn't load files",
            ),
            ("b", DataKey::Refs(repo()), "Couldn't load branches"),
        ] {
            assert_eq!(press(&mut state, keys), vec![fetch(key.clone())]);
            let result = Err(ApiError::Network("timed out".into()));
            let cached_at = None;
            let _ = update(
                &mut state,
                Msg::Fetched {
                    key: key.clone(),
                    result,
                    cached_at,
                },
            );
            let f = overlay!(state, Picker);
            let rows: Vec<String> = state
                .picker_rows(f)
                .into_iter()
                .map(|(i, _)| i.label)
                .collect();
            assert_eq!(rows, [format!("{why}: network error: timed out")]);
            press(&mut state, "<Esc>");
            assert_eq!(press(&mut state, keys), vec![fetch(key)]);
            press(&mut state, "<Esc>");
        }
    }

    #[test]
    fn go_to_file_and_switch_branches() {
        let mut state = with_repo();
        let cmds = press(&mut state, "f");
        let files = DataKey::Files(repo(), "main".into());
        assert_eq!(cmds, vec![fetch(files.clone())]);
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
        let f = overlay!(state, Picker);
        let rows: Vec<String> = state
            .picker_rows(f)
            .into_iter()
            .map(|(i, _)| i.label)
            .collect();
        assert_eq!(rows, ["src/main.rs", "crates/ui/src/main_view.rs"]);
        press(&mut state, "<Enter>");
        assert_eq!(route(&state), blob("main", "src/main.rs"));
        let cmds = press(&mut state, "b");
        assert_eq!(cmds, vec![fetch(DataKey::Refs(repo()))]);
        fetched(
            &mut state,
            DataKey::Refs(repo()),
            Data::Refs(Box::new(ghtui_api::browse::Refs {
                branches: ghtui_api::model::Capped::new(vec!["main".into(), "next".into()], 300),
                tags: vec!["v1.0".into()].into(),
            })),
        );
        // Branches it didn't fetch can't be picked, and it says so.
        let f = overlay!(state, Picker);
        let rows: Vec<String> = state
            .picker_rows(f)
            .into_iter()
            .map(|(i, _)| i.label)
            .collect();
        assert_eq!(
            rows.last().map(String::as_str),
            Some("298 more branches on GitHub")
        );
        press(&mut state, "next<Enter>");
        assert_eq!(
            route(&state),
            blob("next", "src/main.rs"),
            "the same file on the other branch"
        );
    }

    #[test]
    fn issue_comments_post_and_refresh() {
        let mut state = state();
        let _ = state.push(issue(14));
        let key = DataKey::Issue(repo(), 14);
        press(&mut state, "c");
        assert!(state.overlay.is_none(), "nothing to comment on yet");
        fetched(
            &mut state,
            key.clone(),
            Data::Issue(Some(Box::new(crate::fixtures::issue()))),
        );
        press(&mut state, "c");
        let compose = overlay!(state, Compose);
        assert_eq!(compose.title(), "Comment on gold-silver-copper/ghtui#14");
        press(&mut state, "Thanks!");
        let cmds = update(
            &mut state,
            Msg::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
        );
        let comment = Change::Comment {
            subject: NodeId::new("I_14"),
            body: "Thanks!".into(),
        };
        let sent = vec![Cmd::Api(Api::Change(comment, act::By::Compose))];
        assert_eq!(cmds, sent);
        let cmds = answer(&mut state, &sent, Ok(()));
        assert!(state.overlay.is_none());
        let refetch = Cmd::Api(Api::Fetch {
            key: key.clone(),
            cached: false,
        });
        assert!(cmds.contains(&refetch), "{cmds:?}");
        // Another comment, posted while that fetch is on its way: the fetch
        // may miss it, so the page is fetched again once it lands.
        let cmds = answer(&mut state, &sent, Ok(()));
        assert!(!cmds.contains(&refetch));
        let refetch = vec![refetch];
        let issue = || Data::Issue(Some(Box::new(crate::fixtures::issue())));
        assert_eq!(fetched(&mut state, key.clone(), issue()), refetch);
        assert_eq!(fetched(&mut state, key, issue()), Vec::new(), "only once");
    }

    #[test]
    fn search_box_jumps_searches_and_loads_more() {
        let mut state = with_repo();
        press(&mut state, "/");
        let sb = overlay!(state, Search);
        assert!(!sb.filter);
        press(&mut state, "bug");
        let sb = overlay!(state, Search);
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
        let sb = overlay!(state, Search);
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
        // The next page: other issues.
        let mut more = crate::fixtures::issue_results(None);
        if let SearchResults::Issues(r) = &mut more {
            for i in &mut r.items {
                i.number += 100;
            }
        }
        update(
            &mut state,
            Msg::FetchedMore(key.clone(), "c1".into(), Ok(Data::Search(Box::new(more)))),
        );
        let Some(Data::Search(results)) = state.get(&key) else {
            panic!()
        };
        assert_eq!(browse::next_cursor(results), None);
        let SearchResults::Issues(r) = &**results else {
            panic!()
        };
        assert_eq!(r.items.len(), 6);

        // Loading more doesn't block a refresh, and a next page that
        // arrives after the refresh (for the old list) is dropped.
        press(&mut state, "G");
        fetched(
            &mut state,
            key.clone(),
            Data::Search(Box::new(crate::fixtures::issue_results(Some("c2")))),
        );
        press(&mut state, "G<Enter>");
        assert_eq!(state.busy().as_deref(), Some("Loading more"));
        let cmds = press(&mut state, "r");
        assert!(
            cmds.iter()
                .any(|c| matches!(c, Cmd::Api(Api::Fetch { .. })))
        );
        fetched(
            &mut state,
            key.clone(),
            Data::Search(Box::new(crate::fixtures::issue_results(Some("c2")))),
        );
        update(
            &mut state,
            Msg::FetchedMore(
                key.clone(),
                "c2".into(),
                Ok(Data::Search(Box::new(crate::fixtures::issue_results(None)))),
            ),
        );
        let Some(Data::Search(results)) = state.get(&key) else {
            panic!()
        };
        let SearchResults::Issues(r) = &**results else {
            panic!()
        };
        assert_eq!(r.items.len(), 3, "the stale page wasn't appended");
    }

    #[test]
    fn visited_pages_come_back_as_suggestions() {
        let mut state = with_repo();
        let _ = state.push(Route::user("octocat"));
        let _ = state.push(Route::Home);
        press(&mut state, "/");
        let sb = overlay!(state, Search);
        let labels: Vec<String> = state
            .suggestions(sb)
            .into_iter()
            .filter_map(|(row, _)| match row {
                ghtui_ui::chrome::SuggestRow::Item { label, .. } => Some(label),
                ghtui_ui::chrome::SuggestRow::Heading(_) => None,
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

    /// An open PR's diff is from its base branch; a merged or closed one's
    /// from the base it had then, which a merge commit has moved past.
    #[test]
    fn a_merged_prs_diff_is_from_its_own_base() {
        let pr = PrRef::parse("o/r#1").unwrap();
        for (pr_state, oid) in [
            (IssueState::Open, None),
            (
                IssueState::Merged,
                Some("0123456789abcdef0123456789abcdef01234567"),
            ),
            (
                IssueState::Closed,
                Some("0123456789abcdef0123456789abcdef01234567"),
            ),
        ] {
            let mut state = state();
            let mut detail = crate::snapshot_tests::pr_detail();
            detail.summary.state = pr_state;
            let _ = update(&mut state, Msg::Pr(pr.clone(), Box::new(Ok(detail))));
            let cmds = state.open_diff(DiffOf::Pr(pr.clone()));
            let base = cmds.iter().find_map(|c| match c {
                Cmd::Git(Git::LoadDiff { base, .. }) => Some(base.clone()),
                _ => None,
            });
            assert_eq!(base.and_then(|b| b.oid).as_deref(), oid, "{pr_state:?}");
        }
    }

    /// git finding no changes in a PR GitHub says has some (as a merged
    /// PR's diff once did) is shown as an error, not as an empty diff.
    #[test]
    fn an_empty_diff_of_a_pr_with_changes_is_an_error() {
        let mut state = state();
        let pr = PrRef::parse("o/r#1").unwrap();
        let detail = crate::snapshot_tests::pr_detail();
        let head = ghtui_git::Oid::parse(&detail.head_oid).unwrap();
        let _ = update(&mut state, Msg::Pr(pr.clone(), Box::new(Ok(detail))));
        let cmds = state.open_diff(DiffOf::Pr(pr.clone()));
        let job = cmds
            .iter()
            .find_map(|c| match c {
                Cmd::Git(Git::LoadDiff { job, .. }) => Some(*job),
                _ => None,
            })
            .unwrap();
        let refs = ghtui_git::repo::PrRefs {
            head,
            base: ghtui_git::Oid::parse(&"b".repeat(40)).unwrap(),
            merge_base: ghtui_git::Oid::parse(&"b".repeat(40)).unwrap(),
        };
        let files = crate::diff_job::DiffFiles {
            refs,
            files: Vec::new(),
            generated: std::collections::HashSet::new(),
        };
        let msg = DiffMsg::Job(job, crate::diff_job::JobMsg::Files(Box::new(files)));
        let _ = diff_msg(&mut state, &pr, msg);
        let error = state.diffs[&DiffOf::Pr(pr)].error.clone();
        assert!(error.is_some_and(|e| e.contains("GitHub says 7 files changed")));
    }

    /// Opens `pr`'s diff (its metadata already in) and returns the job's id.
    fn start_pr_diff(state: &mut State, pr: &PrRef) -> crate::diff_job::JobId {
        let cmds = state.open_diff(DiffOf::Pr(pr.clone()));
        cmds.iter()
            .find_map(|c| match c {
                Cmd::Git(Git::LoadDiff { job, .. }) => Some(*job),
                _ => None,
            })
            .unwrap()
    }

    /// git's listing of a PR's diff: at `head`, with no files.
    fn no_files(job: crate::diff_job::JobId, head: &str) -> DiffMsg {
        let refs = ghtui_git::repo::PrRefs {
            head: ghtui_git::Oid::parse(head).unwrap(),
            base: ghtui_git::Oid::parse(&"b".repeat(40)).unwrap(),
            merge_base: ghtui_git::Oid::parse(&"b".repeat(40)).unwrap(),
        };
        let files = crate::diff_job::DiffFiles {
            refs,
            files: Vec::new(),
            generated: std::collections::HashSet::new(),
        };
        DiffMsg::Job(job, crate::diff_job::JobMsg::Files(Box::new(files)))
    }

    /// Review threads from GraphQL often beat git's fetch on an uncached
    /// repo: the outdated ones are still mapped once the diff's head is known.
    #[test]
    fn outdated_threads_that_beat_the_diff_are_mapped_when_it_lists() {
        let mut state = state();
        let pr = PrRef::parse("o/r#1").unwrap();
        let detail = crate::snapshot_tests::pr_detail();
        let head = detail.head_oid.clone();
        let _ = update(&mut state, Msg::Pr(pr.clone(), Box::new(Ok(detail))));
        let job = start_pr_diff(&mut state, &pr);
        let early = diff_msg(
            &mut state,
            &pr,
            DiffMsg::ThreadsLoaded(Ok(vec![thread("old", None, false, true)])),
        );
        assert!(
            !early
                .iter()
                .any(|c| matches!(c, Cmd::Git(Git::MapOutdated(_)))),
            "no head to map onto yet"
        );
        let cmds = diff_msg(&mut state, &pr, no_files(job, &head));
        assert!(
            cmds.iter()
                .any(|c| matches!(c, Cmd::Git(Git::MapOutdated(j)) if j.get().3.len() == 1)),
            "the outdated thread is never mapped: {cmds:?}"
        );
    }

    /// A PR shown from the cache may have been merged since: once GitHub's
    /// fresh copy arrives, git's diff is still checked against it.
    #[test]
    fn a_diff_listed_under_a_cached_pr_is_checked_when_the_pr_refreshes() {
        let mut state = state();
        let pr = PrRef::parse("o/r#1").unwrap();
        let detail = crate::snapshot_tests::pr_detail();
        let head = detail.head_oid.clone();
        state.prs.insert(
            pr.clone(),
            Remote::cached(Some(ghtui_store::Cached {
                value: detail.clone(),
                fetched_at: 0,
            })),
        );
        let job = start_pr_diff(&mut state, &pr);
        let _ = diff_msg(&mut state, &pr, no_files(job, &head));
        assert!(
            state.diffs[&DiffOf::Pr(pr.clone())].error.is_none(),
            "a cached PR isn't trusted for the check"
        );
        let _ = update(&mut state, Msg::Pr(pr.clone(), Box::new(Ok(detail))));
        let error = state.diffs[&DiffOf::Pr(pr.clone())].error.clone();
        assert!(
            error
                .as_deref()
                .is_some_and(|e| e.contains("GitHub says 7 files changed")),
            "the empty diff was never checked against the fresh PR: error = {error:?}"
        );
    }

    /// After a push, git's diff is at a newer head than the PR we hold;
    /// refreshing the diff fetches the PR again, so the check can agree.
    #[test]
    fn refreshing_a_pr_diff_refetches_the_pr() {
        let mut state = state();
        let pr = PrRef::parse("o/r#1").unwrap();
        let detail = crate::snapshot_tests::pr_detail();
        let _ = update(&mut state, Msg::Pr(pr.clone(), Box::new(Ok(detail))));
        let job = start_pr_diff(&mut state, &pr);
        let _ = diff_msg(&mut state, &pr, no_files(job, &"c".repeat(40)));
        assert!(
            matches!(&state.notice, Some(Notice::Error(m)) if m.contains("moved while loading")),
            "{:?}",
            state.notice
        );
        let cmds = act(&mut state, Action::Refresh);
        assert!(
            cmds.contains(&Cmd::Api(Api::FetchPr(pr.clone()))),
            "r reloads the diff but not the PR it's checked against: {cmds:?}"
        );
    }

    /// A PR held as open may have merged since: r starts its diff again
    /// from the PR it fetches, not the one it had.
    #[test]
    fn refreshing_a_pr_diff_waits_for_the_pr() {
        let mut state = state();
        let pr = PrRef::parse("o/r#1").unwrap();
        let mut detail = crate::snapshot_tests::pr_detail();
        detail.summary.state = ghtui_api::browse::IssueState::Open;
        let _ = update(
            &mut state,
            Msg::Pr(pr.clone(), Box::new(Ok(detail.clone()))),
        );
        let _ = start_pr_diff(&mut state, &pr);
        let load = |cmds: &[Cmd]| {
            cmds.iter().find_map(|c| match c {
                Cmd::Git(Git::LoadDiff { base, .. }) => Some(base.clone()),
                _ => None,
            })
        };
        let cmds = act(&mut state, Action::Refresh);
        assert_eq!(load(&cmds), None, "started from the PR it had: {cmds:?}");
        detail.summary.state = ghtui_api::browse::IssueState::Merged;
        detail.base_oid = "c".repeat(40);
        let cmds = update(&mut state, Msg::Pr(pr.clone(), Box::new(Ok(detail))));
        assert_eq!(
            load(&cmds).and_then(|b| b.oid),
            Oid::parse(&"c".repeat(40)).ok()
        );
    }

    /// A diff you've left isn't checked when its PR refreshes (on the PR's
    /// page, "moved while loading" would be wrong), only once it's back.
    #[test]
    fn a_hidden_diff_is_checked_when_it_shows_again() {
        let mut state = state();
        let pr = PrRef::parse("o/r#1").unwrap();
        let mut detail = crate::snapshot_tests::pr_detail();
        detail.changed_files = 0;
        let head = detail.head_oid.clone();
        let _ = update(
            &mut state,
            Msg::Pr(pr.clone(), Box::new(Ok(detail.clone()))),
        );
        let job = start_pr_diff(&mut state, &pr);
        let _ = diff_msg(&mut state, &pr, no_files(job, &head));
        let _ = state.back();
        state.notice = None;
        detail.head_oid = "d".repeat(40);
        let _ = update(&mut state, Msg::Pr(pr.clone(), Box::new(Ok(detail))));
        assert_eq!(state.notice, None, "a diff off screen was checked");
        let _ = state.open_diff(DiffOf::Pr(pr.clone()));
        let _ = state.settle();
        assert!(
            matches!(&state.notice, Some(Notice::Error(m)) if m.contains("moved while loading")),
            "{:?}",
            state.notice
        );
    }

    /// "Since my last review" asked for while the diff loads says it'll
    /// wait, and is dropped if you leave before it's in.
    #[test]
    fn since_review_asked_on_a_diff_you_left_is_dropped() {
        let mut state = state();
        let pr = PrRef::parse("o/r#1").unwrap();
        let mut detail = crate::snapshot_tests::pr_detail();
        detail.changed_files = 0;
        let head = detail.head_oid.clone();
        let _ = update(&mut state, Msg::Pr(pr.clone(), Box::new(Ok(detail))));
        let job = start_pr_diff(&mut state, &pr);
        let _ = diff_msg(
            &mut state,
            &pr,
            DiffMsg::LastReview(Ok(Some("a".repeat(40)))),
        );
        let _ = act(&mut state, Action::ToggleSinceReview);
        assert!(
            matches!(&state.notice, Some(Notice::Info(m)) if m.contains("once the diff has loaded")),
            "{:?}",
            state.notice
        );
        let _ = state.back();
        let _ = diff_msg(&mut state, &pr, no_files(job, &head));
        let mut cmds = state.open_diff(DiffOf::Pr(pr.clone()));
        cmds.extend(state.settle());
        assert!(
            !cmds
                .iter()
                .any(|c| matches!(c, Cmd::Git(Git::SinceReview(_)))),
            "a request from before you left came back: {cmds:?}"
        );
    }

    #[test]
    fn files_tab_opens_the_diff_once_the_pr_loads() {
        let mut state = state();
        let pr = PrRef::parse("o/r#1").unwrap();
        let _ = state.push(Route::pr(pr.clone()));
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
        let _ = click(&mut state, x, y);
        assert!(
            matches!(route(&state), Route::Tree { .. }),
            "a click follows a link"
        );
        let _ = state.back();
        // Tabs.
        let lay = state.layout();
        let tabs: Vec<_> = state.chrome().tabs.into_iter().map(|(t, _)| t).collect();
        let rects = ghtui_ui::chrome::tab_layout(lay.tabs.unwrap(), &tabs, state.chrome().active);
        let _ = click(&mut state, rects[2].x + 1, rects[2].y);
        assert!(matches!(route(&state), Route::Pulls { .. }));
        // The header's search field.
        let field = ghtui_ui::chrome::header_layout(lay.header, &[], &[]).search;
        let chrome = state.chrome();
        let crumbs: Vec<_> = chrome.crumbs.iter().map(|(c, _)| c.clone()).collect();
        let right: Vec<_> = chrome.right.iter().map(|(t, _)| t.clone()).collect();
        let field2 = ghtui_ui::chrome::header_layout(lay.header, &crumbs, &right).search;
        assert_eq!(field.y, field2.y);
        let _ = click(&mut state, field2.x + 1, field2.y);
        assert!(matches!(state.overlay, Some(Overlay::Search(_))));
        let bottom = state.size.1 - 1;
        let _ = click(&mut state, 0, bottom);
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
        let menu = overlay!(state, Menu);
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
                .any(|c| matches!(c, Cmd::Api(Api::Change(Change::Star { .. }, _)))),
            "{cmds:?}"
        );
        let cmds = state.follow(&ghtui_ui::page::Link::Star);
        assert!(
            cmds.iter()
                .any(|c| matches!(c, Cmd::Api(Api::Change(Change::Star { .. }, _))))
        );
    }

    /// Esc on an error dismisses it (and goes nowhere); the messages list
    /// keeps it, and choosing it copies it.
    #[test]
    fn errors_are_dismissed_and_kept() {
        let mut state = with_repo();
        let depth = state.screens.len();
        state.error("boom");
        let _ = timers(&mut state, &mut None);
        press(&mut state, "<Esc>");
        assert!(state.notice.is_none());
        assert_eq!(state.screens.len(), depth, "Esc only dismissed");
        act(&mut state, Action::Messages);
        let p = overlay!(state, Picker);
        let rows = state.picker_rows(p);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0.label, "boom");
        let cmds = press(&mut state, "<Enter>");
        assert!(matches!(&cmds[..], [Cmd::Copy(t)] if t == "boom"));
    }

    /// A message arriving while the list is open doesn't move the choice
    /// to another message.
    #[test]
    fn a_new_message_keeps_the_chosen_one() {
        let mut state = with_repo();
        let mut last = None;
        for text in ["one", "two"] {
            state.error(text);
            let _ = timers(&mut state, &mut last);
        }
        act(&mut state, Action::Messages);
        press(&mut state, "<Down>");
        state.error("three");
        let _ = timers(&mut state, &mut last);
        let cmds = press(&mut state, "<Enter>");
        assert!(
            matches!(&cmds[..], [Cmd::Copy(t)] if t == "one"),
            "{cmds:?}"
        );
    }

    /// The fresh copy of a cached list, with a new issue on top, keeps the
    /// selection on the issue you picked.
    #[test]
    fn a_fresh_copy_keeps_the_selected_issue() {
        let mut state = state();
        let _ = state.push(issues("is:open"));
        let (kind, query) = route(&state).search().unwrap();
        let key = DataKey::Search(kind, query);
        let list = |numbers: &[u64]| {
            Data::Search(Box::new(SearchResults::Issues(
                ghtui_api::browse::Results {
                    total: numbers.len() as u64,
                    items: numbers
                        .iter()
                        .map(|&n| crate::fixtures::issue_summary(n, false, IssueState::Open))
                        .collect(),
                    next: None,
                },
            )))
        };
        update(
            &mut state,
            Msg::Fetched {
                key: key.clone(),
                result: Ok(list(&[14, 11, 8])),
                cached_at: Some(0),
            },
        );
        while !selected_text(&state).contains("Issue 11") {
            press(&mut state, "<Down>");
        }
        fetched(&mut state, key, list(&[20, 14, 11, 8]));
        let text = selected_text(&state);
        assert!(text.contains("Issue 11"), "{text}");
    }

    /// Editing the comment you selected keeps it selected, and the view on
    /// it, when the fresh copy arrives.
    #[test]
    fn an_edited_comment_stays_selected() {
        let mut s = with_repo();
        s.size = (100, 12);
        let _ = s.push(Route::Issue {
            repo: repo(),
            number: 14,
        });
        let key = DataKey::Issue(repo(), 14);
        let mut issue = crate::fixtures::issue();
        update(
            &mut s,
            Msg::Fetched {
                key: key.clone(),
                result: Ok(Data::Issue(Some(Box::new(issue.clone())))),
                cached_at: Some(0),
            },
        );
        let Screen::Page(p) = s.screen_mut() else {
            panic!("not on a page")
        };
        let page_ = p.page();
        let at = |i: &ghtui_ui::page::Item| page_.lines[i.start].text().contains("hubot");
        let n = page_.items.iter().position(at).unwrap();
        (p.selected, p.scroll) = (Some(n), page_.items[n].start);
        let top = |s: &State| page(s).page().lines[page(s).scroll].text();
        let before = (selected_text(&s), top(&s));
        issue.body.push_str("\n\nEdited.");
        issue.comments = vec![ghtui_api::browse::Comment {
            body: "Edited.".into(),
            ..issue.comments[0].clone()
        }]
        .into();
        fetched(&mut s, key, Data::Issue(Some(Box::new(issue))));
        assert_eq!((selected_text(&s), top(&s)), before);
    }

    /// A failed refresh puts its banner above the cached copy without
    /// moving what you scrolled to.
    #[test]
    fn a_failed_refresh_keeps_the_view() {
        let mut state = with_home(40);
        for _ in 0..25 {
            press(&mut state, "<Down>");
        }
        let top = |state: &State| {
            let p = page(state);
            p.page().lines[p.scroll].text()
        };
        let before = top(&state);
        assert!(page(&state).scroll > 0, "scrolled");
        let _ = state.load_visible(true);
        mine_failed(&mut state, ApiError::RateLimited(30));
        assert!(
            page(&state).page().lines[0]
                .text()
                .contains("Couldn't refresh")
        );
        assert_eq!(top(&state), before);
    }

    #[test]
    fn the_menu_lists_and_runs_what_you_can_do() {
        let mut state = with_repo();
        press(&mut state, "<Space>");
        let menu = overlay!(state, Menu);
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
        press(&mut state, "i");
        assert!(
            matches!(state.overlay, Some(Overlay::Hints(_))),
            "l: link letters on a page"
        );
        let doables = state.doables();
        assert!(doables.iter().any(|d| d.action == Action::Hints));
        assert!(!doables.iter().any(|d| d.action == Action::FileComment));
    }

    pub(super) mod diff {
        use super::*;
        use crate::diff_screen::DiffScreen;
        use ghtui_api::model::{ViewedFiles, ViewedState};
        use ghtui_ui::diff_doc::{Doc, Pos, Row, Viewed};

        /// A diff screen on o/r#7 built from the snapshot fixture, before
        /// its saved review is read.
        fn unread_diff_state(width: u16) -> (State, PrRef) {
            let mut s = state();
            s.size = (width, 40);
            let pr = PrRef::parse("o/r#7").unwrap();
            s.diffs.insert(
                DiffOf::Pr(pr.clone()),
                crate::snapshot_tests::diff_fixture(),
            );
            s.screens.push(Screen::Diff(Box::new(DiffScreen::new(
                DiffOf::Pr(pr.clone()),
                DiffPrefs::fit(width),
            ))));
            let _ = s.settle_diff();
            (s, pr)
        }

        /// The same, with an empty saved review read.
        pub(in crate::state) fn diff_state(width: u16) -> (State, PrRef) {
            let (mut s, pr) = unread_diff_state(width);
            diff_msg(
                &mut s,
                &pr,
                DiffMsg::ReviewLoaded(Ok(ReviewState::default())),
            );
            (s, pr)
        }

        fn screen(s: &State) -> &DiffScreen {
            match s.screen() {
                Screen::Diff(screen) => screen,
                Screen::Page(_) => panic!("not on the diff"),
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

        /// Where the cursor is, and the document it's in.
        fn cursor(s: &State) -> (Pos, &Doc) {
            let screen = screen(s);
            (screen.cursor, &s.diffs[&screen.of].doc)
        }

        /// The cursor on a removed line stays on it when the view turns
        /// split, though the split row also shows the added line beside it.
        #[test]
        fn a_removed_line_keeps_the_cursor_across_split() {
            let (mut s, _) = diff_state(120);
            let (_, doc) = cursor(&s);
            let rows = doc.files()[0].rows().len();
            let removed = (0..rows)
                .map(|row| Pos { file: 0, row })
                .find(|p| doc.row_text(*p) == "/// A point.")
                .expect("the unified view shows the removed line");
            if let Screen::Diff(d) = s.screens.last_mut() {
                d.cursor = removed;
            }
            act(&mut s, Action::ToggleSplit);
            let (pos, doc) = cursor(&s);
            assert!(doc.opts().split);
            let text = doc.row_text(pos);
            assert!(
                text.lines().any(|l| l == "/// A point."),
                "the cursor left the removed line for {pos:?}: {text:?}"
            );
        }

        /// Threads arriving above the cursor push its line down, and the
        /// cursor goes with it.
        #[test]
        fn threads_arriving_keep_the_cursor_on_its_line() {
            let (mut s, pr) = diff_state(120);
            let (_, doc) = cursor(&s);
            let rows = doc.files()[0].rows().len();
            let last = (0..rows)
                .map(|row| Pos { file: 0, row })
                .rfind(|p| matches!(doc.row(*p), Some(Row::Line(_) | Row::Split { .. })))
                .expect("the file shows lines");
            let text = doc.row_text(last);
            if let Screen::Diff(d) = s.screens.last_mut() {
                d.cursor = last;
            }
            let threads = vec![thread("t", Some(14), false, false)];
            diff_msg(&mut s, &pr, DiffMsg::ThreadsLoaded(Ok(threads)));
            let (pos, doc) = cursor(&s);
            assert!(pos.row > last.row, "the thread is above the cursor");
            assert_eq!(doc.row_text(pos), text);
        }

        /// The cursor on a review thread stays on that thread when the
        /// rows are rebuilt (here: the view turns split).
        #[test]
        fn a_thread_row_keeps_the_cursor_across_a_rebuild() {
            let (mut s, pr) = diff_state(120);
            diff_msg(
                &mut s,
                &pr,
                DiffMsg::ThreadsLoaded(Ok(vec![thread("t", Some(14), false, false)])),
            );
            let (_, doc) = cursor(&s);
            let rows = doc.files()[0].rows().len();
            let on_thread = (0..rows)
                .map(|row| Pos { file: 0, row })
                .find(|p| matches!(doc.row(*p), Some(Row::Thread(_))))
                .expect("the thread has a row");
            if let Screen::Diff(d) = s.screens.last_mut() {
                d.cursor = on_thread;
            }
            act(&mut s, Action::ToggleSplit);
            let (pos, doc) = cursor(&s);
            assert!(doc.opts().split);
            assert!(
                matches!(doc.row(pos), Some(Row::Thread(_))),
                "the cursor left the thread for {pos:?}: {:?}",
                doc.row(pos)
            );
        }

        #[test]
        fn viewed_state_applies_and_rolls_back_on_failure() {
            let (mut s, pr) = diff_state(120);
            // Before GitHub's state arrives, toggling explains why not.
            assert!(press(&mut s, "v").is_empty());
            assert!(matches!(s.notice, Some(Notice::Error(_))));

            diff_msg(
                &mut s,
                &pr,
                DiffMsg::ViewedLoaded(Box::new(Ok(viewed_files("PR_1")))),
            );
            assert_eq!(
                s.diffs[&DiffOf::Pr(pr.clone())].doc.files()[0].viewed,
                Viewed::Viewed
            );
            assert!(
                s.diffs[&DiffOf::Pr(pr.clone())].doc.files()[0].collapsed(),
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
                Cmd::Api(Api::SetViewed { viewed: true, previous: Viewed::Unviewed, pull_request_id, .. })
                    if pull_request_id.as_str() == "PR_1"
            )));
            assert_eq!(
                s.diffs[&DiffOf::Pr(pr.clone())].doc.files()[1].viewed,
                Viewed::Viewed
            );
            let path = s.diffs[&DiffOf::Pr(pr.clone())].doc.files()[1]
                .meta
                .path()
                .to_owned();
            diff_msg(
                &mut s,
                &pr,
                DiffMsg::ViewedSaved {
                    path,
                    previous: Viewed::Unviewed,
                    result: Err(ApiError::Network("offline".into())),
                },
            );
            assert_eq!(
                s.diffs[&DiffOf::Pr(pr.clone())].doc.files()[1].viewed,
                Viewed::Unviewed
            );
            assert!(matches!(s.notice, Some(Notice::Error(_))));
        }

        /// Marks made before the saved review arrives aren't saved over
        /// it; they're merged into it once it's in.
        #[test]
        fn nothing_is_saved_before_the_saved_review_is_read() {
            let (mut s, pr) = unread_diff_state(120);
            press(&mut s, "njjj");
            let cmds = press(&mut s, "m");
            assert!(!cmds.iter().any(|c| matches!(c, Cmd::SaveReview(..))));

            let saved = ReviewState {
                reviewed_hunks: vec!["older".into()],
                ..ReviewState::default()
            };
            let cmds = diff_msg(&mut s, &pr, DiffMsg::ReviewLoaded(Ok(saved)));
            let [Cmd::SaveReview(_, review)] = &cmds[..] else {
                panic!("{cmds:?}")
            };
            assert_eq!(review.reviewed_hunks.len(), 2);
            assert!(review.reviewed_hunks.contains(&"older".to_owned()));
        }

        /// A saved review read before anything was done is not written back.
        #[test]
        fn reading_the_saved_review_alone_saves_nothing() {
            let (mut s, pr) = unread_diff_state(120);
            let saved = ReviewState {
                reviewed_hunks: vec!["older".into()],
                ..ReviewState::default()
            };
            let cmds = diff_msg(&mut s, &pr, DiffMsg::ReviewLoaded(Ok(saved)));
            assert!(cmds.is_empty(), "{cmds:?}");
        }

        #[test]
        fn unreadable_drafts_raise_a_lasting_problem() {
            let (mut s, pr) = diff_state(120);
            diff_msg(
                &mut s,
                &pr,
                DiffMsg::ReviewLoaded(Err(Failure::msg("disk full"))),
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
            assert_eq!(
                s.diffs[&DiffOf::Pr(pr)]
                    .inputs()
                    .review
                    .reviewed_hunks
                    .len(),
                1
            );
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
            assert!(
                s.diffs[&DiffOf::Pr(pr)]
                    .doc
                    .row_text(cursor)
                    .contains("origin")
            );
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
            let p = overlay!(s, Picker);
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
            let text = s.diffs[&DiffOf::Pr(pr.clone())]
                .doc
                .row_text(screen(&s).cursor);
            act(&mut s, Action::FullFile);
            assert!(s.diffs[&DiffOf::Pr(pr.clone())].doc.files()[0].full);
            assert_eq!(
                s.diffs[&DiffOf::Pr(pr.clone())]
                    .doc
                    .row_text(screen(&s).cursor),
                text
            );
            press(&mut s, "w");
            assert_eq!(
                s.diffs[&DiffOf::Pr(pr)].doc.opts().whitespace,
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
                let pending = s.diffs[&DiffOf::Pr(pr.clone())]
                    .doc
                    .files()
                    .iter()
                    .position(|f| f.diff.is_none())
                    .unwrap();
                let job = s.diffs[&DiffOf::Pr(pr.clone())].job;
                let cmds = diff_msg(
                    &mut s,
                    &pr,
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
                );
                assert!(cmds.iter().any(
                    |c| matches!(c, Cmd::Git(Git::DetectMoves(m)) if matches!(m.get(), (p, j, files) if *p == DiffOf::Pr(pr.clone()) && *j == job && files.len() == 8))
                ));
                // Only once.
                let again = diff_msg(&mut s, &pr, DiffMsg::Job(job, JobMsg::Moves(Vec::new())));
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
                let old = s.diffs[&DiffOf::Pr(pr.clone())].job;
                s.diffs
                    .get_mut(&DiffOf::Pr(pr.clone()))
                    .unwrap()
                    .restart(None);
                let stale = Arc::new(FileDiff::compute("a.rs", Some(b"a\n"), Some(b"b\n")));
                diff_msg(&mut s, &pr, DiffMsg::Job(old, JobMsg::File(0, stale)));
                diff_msg(
                    &mut s,
                    &pr,
                    DiffMsg::Job(old, JobMsg::Failed(Failure::msg("old"))),
                );
                let diff = &s.diffs[&DiffOf::Pr(pr.clone())];
                assert_ne!(diff.job, old);
                assert!(diff.doc.is_empty());
                assert_eq!(diff.error, None);
                // The current job's messages still count.
                let job = diff.job;
                diff_msg(
                    &mut s,
                    &pr,
                    DiffMsg::Job(job, JobMsg::Failed(Failure::msg("new"))),
                );
                assert_eq!(
                    s.diffs[&DiffOf::Pr(pr.clone())].error.as_deref(),
                    Some("new")
                );
            }

            #[test]
            fn since_review_asks_github_then_compares() {
                let (mut s, pr) = diff_state(120);
                s.viewer = Some("me".into());
                let cmds = act(&mut s, Action::ToggleSinceReview);
                assert!(
                    matches!(&cmds[..], [Cmd::Api(Api::FetchLastReview { login, .. })] if login == "me")
                );
                let cmds = diff_msg(&mut s, &pr, DiffMsg::LastReview(Ok(Some("a".repeat(40)))));
                assert!(
                    matches!(&cmds[..], [Cmd::Git(Git::SinceReview(j))] if j.get().2 == "a".repeat(40))
                );

                // Nothing in the old diff matched: everything is new.
                let job = s.diffs[&DiffOf::Pr(pr.clone())].job;
                diff_msg(
                    &mut s,
                    &pr,
                    DiffMsg::Job(job, JobMsg::Since("a".repeat(40), Ok(Default::default()))),
                );
                assert!(s.diffs[&DiffOf::Pr(pr.clone())].doc.since_active());
                assert_eq!(s.chrome().tabs[3].0.label, "Files · since your review");
                // Toggling back needs no new lookups.
                assert!(
                    act(&mut s, Action::ToggleSinceReview)
                        .iter()
                        .all(|c| matches!(c, Cmd::Git(Git::Prioritize(..))))
                );
                assert!(!s.diffs[&DiffOf::Pr(pr.clone())].doc.since_active());
            }

            #[test]
            fn since_review_finishing_after_you_left_is_dropped() {
                let (mut s, pr) = diff_state(120);
                s.viewer = Some("me".into());
                act(&mut s, Action::ToggleSinceReview);
                diff_msg(&mut s, &pr, DiffMsg::LastReview(Ok(Some("old".into()))));
                let _ = s.back();
                s.notice = None;
                let job = s.diffs[&DiffOf::Pr(pr.clone())].job;
                diff_msg(
                    &mut s,
                    &pr,
                    DiffMsg::Job(job, JobMsg::Since("old".into(), Ok(Default::default()))),
                );
                assert!(!s.diffs[&DiffOf::Pr(pr.clone())].doc.since_active());
                assert_eq!(s.notice, None);
            }

            #[test]
            fn since_review_without_a_review_says_so() {
                let (mut s, pr) = diff_state(120);
                s.viewer = Some("me".into());
                act(&mut s, Action::ToggleSinceReview);
                diff_msg(&mut s, &pr, DiffMsg::LastReview(Ok(None)));
                assert!(
                    matches!(&s.notice, Some(Notice::Info(m)) if m.contains("haven't reviewed"))
                );
                // Reviewing the current head means nothing's new.
                s.diffs
                    .get_mut(&DiffOf::Pr(pr.clone()))
                    .unwrap()
                    .since_requested = true;
                diff_msg(&mut s, &pr, DiffMsg::LastReview(Ok(Some("e".repeat(40)))));
                assert!(matches!(&s.notice, Some(Notice::Info(m)) if m.contains("current head")));
            }

            #[test]
            fn a_failed_review_lookup_isnt_taken_for_no_review() {
                let (mut s, pr) = diff_state(120);
                s.viewer = Some("me".into());
                act(&mut s, Action::ToggleSinceReview);
                diff_msg(
                    &mut s,
                    &pr,
                    DiffMsg::LastReview(Err(ApiError::Network("offline".into()))),
                );
                assert!(
                    !matches!(&s.notice, Some(Notice::Info(m)) if m.contains("haven't reviewed")),
                    "{:?}",
                    s.notice
                );
                // Asking again asks GitHub again.
                let cmds = act(&mut s, Action::ToggleSinceReview);
                assert!(
                    matches!(&cmds[..], [Cmd::Api(Api::FetchLastReview { .. })]),
                    "{cmds:?}"
                );
            }

            /// Going by the review made here when GitHub can't be asked
            /// says so: GitHub may know a later one.
            #[test]
            fn a_failed_review_lookup_says_so_beside_the_local_one() {
                let (mut s, pr) = diff_state(120);
                s.viewer = Some("me".into());
                s.diffs
                    .get_mut(&DiffOf::Pr(pr.clone()))
                    .unwrap()
                    .edit(|i| i.review.last_reviewed_head = Some("old".into()));
                act(&mut s, Action::ToggleSinceReview);
                let cmds = diff_msg(
                    &mut s,
                    &pr,
                    DiffMsg::LastReview(Err(ApiError::Network("offline".into()))),
                );
                assert!(
                    matches!(&cmds[..], [Cmd::Git(Git::SinceReview(j))] if j.get().2 == "old"),
                    "{cmds:?}"
                );
                assert!(
                    matches!(&s.notice, Some(Notice::Info(m)) if m.contains("offline") && m.contains("made here")),
                    "{:?}",
                    s.notice
                );
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
                diff_msg(
                    &mut s,
                    &pr,
                    DiffMsg::CommitsListed(Ok(vec![
                        ghtui_git::repo::Commit {
                            oid: Oid::parse(&"a".repeat(40)).unwrap(),
                            subject: "first".into(),
                        },
                        ghtui_git::repo::Commit {
                            oid: Oid::parse(&"b".repeat(40)).unwrap(),
                            subject: "second".into(),
                        },
                    ])),
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
            use ghtui_api::model::{PatchFile, ReviewEvent};

            fn to_line(s: &mut State, text: &str) {
                press(s, "/");
                press(s, &format!("{text}<Enter>"));
            }

            #[test]
            fn threads_show_navigate_and_resolve() {
                let (mut s, pr) = diff_state(120);
                diff_msg(
                    &mut s,
                    &pr,
                    DiffMsg::ThreadsLoaded(Ok(vec![
                        thread("done", Some(1), true, false),
                        thread("open", Some(14), false, false),
                    ])),
                );
                assert_eq!(s.diffs[&DiffOf::Pr(pr.clone())].doc.annotations().len(), 2);
                press(&mut s, "g");
                act(&mut s, Action::NextThread);
                let at = s.diffs[&DiffOf::Pr(pr.clone())]
                    .doc
                    .annotation_at(screen(&s).cursor)
                    .unwrap();
                assert_eq!(
                    s.diffs[&DiffOf::Pr(pr.clone())].doc.annotations()[at as usize].key,
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
                assert!(s.diffs[&DiffOf::Pr(pr.clone())].inputs().threads[1].resolved);
                diff_msg(
                    &mut s,
                    &pr,
                    DiffMsg::ResolvedSet {
                        thread_id: NodeId::new("open"),
                        resolved: true,
                        result: Err(ApiError::Network("down".into())),
                    },
                );
                assert!(!s.diffs[&DiffOf::Pr(pr.clone())].inputs().threads[1].resolved);
                assert!(matches!(s.notice, Some(Notice::Error(_))));
            }

            #[test]
            fn comment_draft_edit_and_delete() {
                let (mut s, pr) = diff_state(120);
                to_line(&mut s, "origin");
                press(&mut s, "c");
                let compose = overlay!(s, Compose);
                assert!(matches!(compose.target, ComposeTarget::Line { .. }));
                press(&mut s, "Use a constant");
                let cmds = press(&mut s, "<C-s>");
                assert!(s.overlay.is_none());
                let Some(Cmd::SaveReview(_, review)) = cmds.first() else {
                    panic!("{cmds:?}")
                };
                assert_eq!(review.pending.len(), 1);
                let draft = &review.pending[0];
                assert_eq!(
                    (draft.line, draft.commit.as_str()),
                    (Some(14), &*"e".repeat(40))
                );
                assert!(
                    s.diffs[&DiffOf::Pr(pr.clone())]
                        .doc
                        .annotations()
                        .iter()
                        .any(ghtui_ui::annotations::Annotation::is_draft)
                );

                // The cursor is on the commented line; Enter edits the draft.
                press(&mut s, "<Enter>");
                assert!(
                    matches!(&s.overlay, Some(Overlay::Compose(c)) if matches!(c.target, ComposeTarget::Draft { .. }))
                );
                press(&mut s, "!");
                press(&mut s, "<C-s>");
                assert_eq!(
                    s.diffs[&DiffOf::Pr(pr.clone())].inputs().review.pending[0].body,
                    "Use a constant!"
                );

                let cmds = press(&mut s, "<Delete>");
                assert!(matches!(&cmds[..], [Cmd::SaveReview(_, r)] if r.pending.is_empty()));
                assert!(matches!(&s.notice, Some(Notice::Info(m)) if m.contains("ctrl-z")));

                // Deleting can be undone.
                let cmds = press(&mut s, "<C-z>");
                assert!(
                    matches!(&cmds[..], [Cmd::SaveReview(_, r)] if r.pending[0].body == "Use a constant!")
                );
                assert!(
                    s.diffs[&DiffOf::Pr(pr)]
                        .doc
                        .annotations()
                        .iter()
                        .any(ghtui_ui::annotations::Annotation::is_draft)
                );
                press(&mut s, "<C-z>");
                assert!(matches!(&s.notice, Some(Notice::Info(m)) if m.starts_with("No deleted")));
            }

            #[test]
            fn lines_outside_githubs_diff_become_file_comments() {
                let (mut s, pr) = diff_state(120);
                // GitHub's diff only covers line 1.
                diff_msg(
                    &mut s,
                    &pr,
                    DiffMsg::PatchesLoaded(Ok(vec![PatchFile {
                        filename: "src/point.rs".into(),
                        previous_filename: None,
                        patch: Some("@@ -1 +1 @@\n-a\n+b".into()),
                    }])),
                );
                to_line(&mut s, "origin");
                press(&mut s, "c");
                let compose = overlay!(s, Compose);
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
                diff_msg(
                    &mut s,
                    &pr,
                    DiffMsg::PatchesLoaded(Ok(vec![PatchFile {
                        filename: "src/point.rs".into(),
                        previous_filename: None,
                        patch: Some("@@ -1,6 +1,6 @@\n@@ -10,4 +10,8 @@".into()),
                    }])),
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
            fn typed_text_survives_one_press_of_each_discard_key() {
                let (mut s, _) = diff_state(120);
                to_line(&mut s, "origin");
                press(&mut s, "cnit<C-c>");
                assert!(!s.quit);
                assert!(matches!(&s.overlay, Some(Overlay::Compose(c)) if c.text() == "nit"));
                press(&mut s, "<C-c>");
                assert!(s.quit);

                // Each key warns for itself: esc then ctrl-c still asks.
                let (mut s, _) = diff_state(120);
                to_line(&mut s, "origin");
                press(&mut s, "cnit<Esc><C-c>");
                assert!(!s.quit);
                assert!(matches!(&s.overlay, Some(Overlay::Compose(c)) if c.text() == "nit"));

                let (mut s, _) = diff_state(120);
                press(&mut s, "alooks good<Esc>");
                assert!(
                    matches!(&s.overlay, Some(Overlay::Submit(d)) if d.input.lines() == ["looks good"])
                );
                press(&mut s, "!<Esc><Esc>");
                assert!(s.overlay.is_none());
            }

            #[test]
            fn submit_with_rejection_then_fix_and_resubmit() {
                let (mut s, pr) = diff_state(120);
                to_line(&mut s, "origin");
                press(&mut s, "c");
                press(&mut s, "nit");
                press(&mut s, "<C-s>");
                let id = s.diffs[&DiffOf::Pr(pr.clone())].inputs().review.pending[0].id;

                press(&mut s, "a");
                assert!(matches!(s.overlay, Some(Overlay::Submit(_))));
                press(&mut s, "<Tab>");
                let cmds = press(&mut s, "<C-s>");
                let Some(Cmd::Api(Api::SubmitReview { drafts, event, .. })) = cmds.first() else {
                    panic!("{cmds:?}")
                };
                assert_eq!((drafts.len(), *event), (1, ReviewEvent::Approve));

                diff_msg(
                    &mut s,
                    &pr,
                    DiffMsg::ReviewSubmitted(SubmitOutcome {
                        rejected: vec![(id, "line must be part of the diff".into())],
                        ..SubmitOutcome::default()
                    }),
                );
                assert!(
                    matches!(&s.overlay, Some(Overlay::Submit(d)) if d.error.is_some() && !d.sending)
                );
                assert_eq!(
                    s.diffs[&DiffOf::Pr(pr.clone())].inputs().review.pending[0]
                        .error
                        .as_deref(),
                    Some("line must be part of the diff")
                );

                // Make it a file comment and submit again.
                press(&mut s, "<Esc>");
                act(&mut s, Action::FileComment);
                assert_eq!(
                    s.diffs[&DiffOf::Pr(pr.clone())].inputs().review.pending[0].line,
                    None
                );
                press(&mut s, "a");
                press(&mut s, "<C-s>");
                diff_msg(
                    &mut s,
                    &pr,
                    DiffMsg::ReviewSubmitted(SubmitOutcome {
                        accepted: vec![id],
                        submitted: true,
                        ..SubmitOutcome::default()
                    }),
                );
                assert!(s.overlay.is_none());
                assert!(
                    s.diffs[&DiffOf::Pr(pr.clone())]
                        .inputs()
                        .review
                        .pending
                        .is_empty()
                );
                assert_eq!(
                    s.diffs[&DiffOf::Pr(pr.clone())]
                        .inputs()
                        .review
                        .last_reviewed_head
                        .as_deref(),
                    Some(&*"e".repeat(40))
                );
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
                diff_msg(
                    &mut s,
                    &pr,
                    DiffMsg::ThreadsLoaded(Ok(vec![thread("t", Some(14), false, false)])),
                );
                press(&mut s, "g");
                act(&mut s, Action::NextThread);
                press(&mut s, "c");
                press(&mut s, "Fixed");
                let cmds = press(&mut s, "<C-s>");
                assert!(
                    matches!(&cmds[..], [Cmd::Api(Api::Reply { body, .. })] if body == "Fixed")
                );
                diff_msg(
                    &mut s,
                    &pr,
                    DiffMsg::Replied(Err(ApiError::Network("down".into()))),
                );
                assert!(
                    matches!(&s.overlay, Some(Overlay::Compose(c)) if c.error.is_some() && c.text() == "Fixed")
                );
                let cmds = diff_msg(&mut s, &pr, DiffMsg::Replied(Ok(())));
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
                let compose = overlay!(s, Compose);
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
                let cmds = diff_msg(
                    &mut s,
                    &pr,
                    DiffMsg::ThreadsLoaded(Ok(vec![thread("old", None, false, true)])),
                );
                assert!(cmds.iter().any(|c| matches!(c, Cmd::Git(Git::MapOutdated(j)) if j.get().3.len() == 1 && *j.get().2 == "e".repeat(40))));
                let ann = &s.diffs[&DiffOf::Pr(pr.clone())].doc.annotations()[0];
                assert!(
                    ann.outdated && ann.on_line().is_none(),
                    "unplaced until mapped"
                );
                let job = s.diffs[&DiffOf::Pr(pr.clone())].job;
                let mapped = JobMsg::Mapped(vec![(NodeId::new("old"), Some(4))]);
                diff_msg(&mut s, &pr, DiffMsg::Job(job, mapped));
                let ann = &s.diffs[&DiffOf::Pr(pr.clone())].doc.annotations()[0];
                assert_eq!((ann.on_line().map(|p| p.line), ann.moved), (Some(4), true));
            }

            /// Outdated threads are mapped onto each job's head: another
            /// range drops the old lines, and a late reply for the old job
            /// doesn't land on the new one.
            #[test]
            fn an_outdated_mapping_is_for_its_own_job() {
                let (mut s, pr) = diff_state(120);
                let of = DiffOf::Pr(pr.clone());
                let thread = DiffMsg::ThreadsLoaded(Ok(vec![thread("old", None, false, true)]));
                let _ = diff_msg(&mut s, &pr, thread);
                let old = s.diffs[&of].job;
                let line = |n| JobMsg::Mapped(vec![(NodeId::new("old"), Some(n))]);
                diff_msg(&mut s, &pr, DiffMsg::Job(old, line(4)));
                s.diffs.get_mut(&of).unwrap().restart(None);
                assert!(s.diffs[&of].mapped().is_empty(), "the old job's lines kept");
                diff_msg(&mut s, &pr, DiffMsg::Job(old, line(9)));
                assert!(s.diffs[&of].mapped().is_empty(), "a late reply landed");
                let new = s.diffs[&of].job;
                diff_msg(&mut s, &pr, DiffMsg::Job(new, line(7)));
                assert_eq!(s.diffs[&of].mapped()[&NodeId::new("old")], Some(7));
            }
        }

        /// What a diff is built from (GitHub's patches, viewed marks, the
        /// screen's toggles) outlasts each rebuild of what's shown.
        mod inputs_survive {
            use super::*;
            use crate::diff_job::{DiffFiles, JobMsg};
            use crate::diff_screen::DiffState;
            use crate::review::ComposeTarget;
            use ghtui_api::model::PatchFile;
            use std::sync::Arc;

            const PATH: &str = "lines.txt";

            fn text(changed: bool) -> String {
                (1..=20)
                    .map(|n| {
                        if changed && n == 15 {
                            "fifteen changed\n".to_owned()
                        } else {
                            format!("line {n}\n")
                        }
                    })
                    .collect()
            }

            /// GitHub's diff of `lines.txt` covers only its first line.
            fn patches() -> DiffMsg {
                DiffMsg::PatchesLoaded(Ok(vec![PatchFile {
                    filename: PATH.into(),
                    previous_filename: None,
                    patch: Some("@@ -1 +1 @@\n-a\n+b".into()),
                }]))
            }

            /// The diff job's answer for `job`: `lines.txt`, line 15 changed.
            fn deliver(s: &mut State, pr: &PrRef, job: crate::diff_job::JobId) {
                let files = DiffFiles {
                    refs: ghtui_git::repo::PrRefs {
                        head: Oid::parse(&"e".repeat(40)).unwrap(),
                        base: Oid::parse(&"b".repeat(40)).unwrap(),
                        merge_base: Oid::parse(&"c".repeat(40)).unwrap(),
                    },
                    files: vec![ghtui_git::files::ChangedFile {
                        status: ghtui_git::files::FileStatus::Modified,
                        old_path: Some(PATH.into()),
                        new_path: Some(PATH.into()),
                        old_mode: 0o100644,
                        new_mode: 0o100644,
                        old_oid: Oid::parse(&"1".repeat(40)).unwrap(),
                        new_oid: Oid::parse(&"2".repeat(40)).unwrap(),
                        similarity: None,
                    }],
                    generated: std::collections::HashSet::new(),
                };
                diff_msg(s, pr, DiffMsg::Job(job, JobMsg::Files(Box::new(files))));
                let diff = ghtui_diff::FileDiff::compute(
                    PATH,
                    Some(text(false).as_bytes()),
                    Some(text(true).as_bytes()),
                );
                diff_msg(s, pr, DiffMsg::Job(job, JobMsg::File(0, Arc::new(diff))));
                let _ = s.settle_diff();
            }

            /// A diff screen on o/r#7 whose job hasn't answered yet, its
            /// saved review read and its metadata (for the commit picker) in.
            fn loading() -> (State, PrRef, crate::diff_job::JobId) {
                let mut s = state();
                s.size = (120, 40);
                let pr = PrRef::parse("o/r#7").unwrap();
                let of = DiffOf::Pr(pr.clone());
                let diff = DiffState::fresh();
                let job = diff.job;
                s.diffs.insert(of.clone(), diff);
                s.screens.push(Screen::Diff(Box::new(DiffScreen::new(
                    of,
                    DiffPrefs::fit(120),
                ))));
                s.prs.insert(
                    pr.clone(),
                    Remote::cached(Some(ghtui_store::Cached {
                        value: crate::snapshot_tests::pr_detail(),
                        fetched_at: 0,
                    })),
                );
                diff_msg(
                    &mut s,
                    &pr,
                    DiffMsg::ReviewLoaded(Ok(ReviewState::default())),
                );
                (s, pr, job)
            }

            /// Where `c` on the changed line 15 would put a comment.
            fn comment_target(s: &mut State) -> ComposeTarget {
                press(s, "/");
                press(s, "fifteen changed<Enter>");
                press(s, "c");
                let target = overlay!(s, Compose).target.clone();
                press(s, "<C-c>");
                s.overlay = None;
                target
            }

            fn is_file_comment(target: &ComposeTarget) -> bool {
                matches!(
                    target,
                    ComposeTarget::File {
                        reason: Some(_),
                        ..
                    }
                )
            }

            /// Opens the commit picker on one commit and presses `keys` (`jj<Enter>`
            /// picks the commit, `<Enter>` "All"); the new job's id.
            fn pick_commit(s: &mut State, pr: &PrRef, keys: &str) -> crate::diff_job::JobId {
                act(s, Action::PickCommits);
                diff_msg(
                    s,
                    pr,
                    DiffMsg::CommitsListed(Ok(vec![ghtui_git::repo::Commit {
                        oid: Oid::parse(&"a".repeat(40)).unwrap(),
                        subject: "first".into(),
                    }])),
                );
                assert!(matches!(s.overlay, Some(Overlay::Picker(_))));
                let cmds = press(s, keys);
                cmds.iter()
                    .find_map(|c| match c {
                        Cmd::Git(Git::LoadDiff { job, .. }) => Some(*job),
                        _ => None,
                    })
                    .unwrap_or_else(|| panic!("{cmds:?}"))
            }

            /// GitHub's patches usually arrive before git has listed the
            /// files; lines outside GitHub's diff still become file comments.
            #[test]
            fn patches_that_arrive_before_the_files_still_place_comments() {
                let (mut s, pr, job) = loading();
                diff_msg(&mut s, &pr, patches());
                deliver(&mut s, &pr, job);
                let target = comment_target(&mut s);
                assert!(is_file_comment(&target), "{target:?}");
            }

            /// Back to the whole PR after a commit, GitHub's patches still
            /// decide where comments go.
            #[test]
            fn the_whole_pr_after_a_commit_keeps_githubs_patches() {
                let (mut s, pr, job) = loading();
                deliver(&mut s, &pr, job);
                diff_msg(&mut s, &pr, patches());
                let target = comment_target(&mut s);
                assert!(is_file_comment(&target), "before: {target:?}");

                let job = pick_commit(&mut s, &pr, "jj<Enter>");
                deliver(&mut s, &pr, job);
                let job = pick_commit(&mut s, &pr, "<Enter>");
                deliver(&mut s, &pr, job);
                let target = comment_target(&mut s);
                assert!(is_file_comment(&target), "after All: {target:?}");
            }

            /// Ignoring whitespace, split and the tree are the screen's
            /// choices; picking a commit doesn't undo them.
            #[test]
            fn choosing_a_commit_keeps_the_view_toggles() {
                let (mut s, pr, job) = loading();
                deliver(&mut s, &pr, job);
                press(&mut s, "w");
                press(&mut s, "S");
                press(&mut s, "t");
                let before = {
                    let d = screen(&s);
                    (
                        d.prefs.ignore_whitespace,
                        d.prefs.split_override,
                        d.prefs.tree_visible,
                    )
                };
                assert!(before.0 && before.1.is_some() && !before.2, "{before:?}");
                let job = pick_commit(&mut s, &pr, "jj<Enter>");
                deliver(&mut s, &pr, job);
                let d = screen(&s);
                assert_eq!(
                    (
                        d.prefs.ignore_whitespace,
                        d.prefs.split_override,
                        d.prefs.tree_visible
                    ),
                    before,
                    "(ignore_whitespace, split_override, tree_visible)"
                );
            }

            /// A file marked viewed stays viewed when another range of
            /// commits is shown.
            #[test]
            fn a_file_marked_viewed_stays_viewed_in_another_range() {
                let (mut s, pr, job) = loading();
                deliver(&mut s, &pr, job);
                diff_msg(
                    &mut s,
                    &pr,
                    DiffMsg::ViewedLoaded(Box::new(Ok(ViewedFiles {
                        pull_request_id: NodeId::new("PR_1"),
                        states: std::collections::HashMap::new(),
                    }))),
                );
                let cmds = press(&mut s, "v");
                assert!(
                    cmds.iter()
                        .any(|c| matches!(c, Cmd::Api(Api::SetViewed { viewed: true, .. }))),
                    "{cmds:?}"
                );
                diff_msg(
                    &mut s,
                    &pr,
                    DiffMsg::ViewedSaved {
                        path: PATH.into(),
                        previous: Viewed::Unviewed,
                        result: Ok(()),
                    },
                );
                let of = DiffOf::Pr(pr.clone());
                assert_eq!(s.diffs[&of].doc.files()[0].viewed, Viewed::Viewed);

                let job = pick_commit(&mut s, &pr, "jj<Enter>");
                deliver(&mut s, &pr, job);
                assert_eq!(s.diffs[&of].doc.files()[0].viewed, Viewed::Viewed);
            }
        }
    }
}
