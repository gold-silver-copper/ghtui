//! Application state and the single update function. Input events and async
//! results arrive as [`Msg`]s; [`update`] mutates [`State`] and returns
//! [`Cmd`]s for the runtime to execute in the background.

use std::collections::HashMap;
use std::sync::Arc;

use crate::api::ApiError;
use crate::api::change::Change;
use crate::api::model::{NodeId, PrDetail, PrRef, ViewedFiles};
use crate::api::rate_limit::RateLimits;
use crate::diff::FileDiff;
use crate::git::Oid;
use crate::store::ReviewState;
use crate::theme::{Bg, Theme};
use crate::ui::bars::Notice;
use crate::ui::diff_doc::Viewed;
use crate::ui::{Ctx, Fetched, Icons};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use ratatui_textarea::TextArea;

use crate::act;
use crate::api::model::{PatchFile, ReviewEvent, ReviewThread};
use crate::browse::{self, Need};
use crate::data::{Data, DataKey};
use crate::diff_job::{JobId, JobMsg};
use crate::diff_screen::{self, DiffOf, DiffPrefs, DiffScreen, DiffState, Pane};
use crate::join;
use crate::keymap::{Action, Context, Key, Keymap, Resolved};
use crate::nav::{self, Hints, Menu, PageScreen, SearchBox, Visit};
use crate::notice::Notices;
use crate::picker::{self, Picker};
use crate::review::{
    Compose, ComposeTarget, EditPurpose, SubmitDialog, SubmitOutcome, on_compose_key, on_edited,
    on_submit_key, review_action,
};
use crate::route::{Route, Target};
use crate::store::DraftComment;
use crate::tabs::Tab;

mod diff;
mod dispatch;
mod fetch;
mod unread;

pub use dispatch::*;
pub use fetch::*;

#[derive(Debug)]
pub enum Msg {
    Key(KeyEvent),
    Resize(u16, u16),
    Viewer(Result<String, ApiError>),
    /// How many notifications are unread.
    Unread(Result<crate::api::notifications::Unread, ApiError>),
    Pr(PrRef, Box<Result<PrDetail, ApiError>>),
    /// The cache's copy of a pull request, shown while GitHub can't be
    /// reached (sent before its failed fetch).
    PrCached(PrRef, Box<crate::store::Cached<PrDetail>>),
    /// Page data; with `cached_at` (when it was fetched), the cached copy
    /// shown while fetching.
    Fetched {
        key: DataKey,
        result: Result<Data, ApiError>,
        cached_at: Option<u64>,
    },
    /// The next page of a list, asked for after the cursor.
    FetchedMore(DataKey, String, Result<Data, ApiError>),
    /// Where a page's data came from, git or the API; sent before it.
    Source(DataKey, crate::source::From),
    /// GitHub's answer to a change.
    Changed(Change, act::By, Result<(), ApiError>),
    Mouse(MouseEvent),
    Timer(Timer),
    /// The search box's input settled; fetch live suggestions if it's
    /// still this.
    SuggestDue(String),
    Suggested(
        String,
        Result<Vec<crate::api::browse::RepoSummary>, ApiError>,
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
    /// Text pasted into the terminal, whole.
    Paste(String),
    /// The terminal came into focus (`true`) or left it.
    Focus(bool),
    /// Who and what a repository's comments can mention.
    Mentionable(
        crate::api::model::RepoId,
        Result<crate::api::mention::Mentionable, ApiError>,
    ),
    /// What the repository's clone (or code search) answered about code.
    Code(
        crate::code_nav::Ask,
        Result<crate::code_nav::Answer, String>,
    ),
    /// What GitHub answered when asked whether a watched pull request
    /// changed.
    Polled(
        PrRef,
        Vec<(crate::live::Probe, Result<crate::api::Polled, ApiError>)>,
    ),
    /// What GitHub answered when asked whether another watched page
    /// changed.
    Watched(
        crate::live::page::Watched,
        Vec<(
            crate::live::page::Probe,
            Result<crate::api::Polled, ApiError>,
        )>,
    ),
    /// New issues and pull requests, and checking one out.
    Create(crate::create::Done),
    /// Releases, gists, wiki pages and subscriptions.
    Write(crate::writes::Done),
    /// An artifact saved, a workflow run by hand.
    Actions(crate::actions::Done),
    /// Whether GitHub can be reached, the queued writes, the kept
    /// repositories ([`crate::offline`]).
    Offline(crate::offline::Msg),
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
    ResolvedSet {
        thread_id: NodeId,
        resolved: bool,
        result: Result<(), ApiError>,
    },
    ReviewSubmitted(SubmitOutcome),
    /// How many comments your pending review on GitHub holds.
    PendingReview(Result<u64, ApiError>),
    LastReview(Result<Option<String>, ApiError>),
    CommitsListed(Result<Vec<crate::git::repo::Commit>, Failure>),
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
    /// Making an issue or pull request, or git work in your clone;
    /// answered by [`Msg::Create`].
    Create(crate::create::Work),
    /// A release, a gist, a wiki page or a subscription; answered by
    /// [`Msg::Write`].
    Write(crate::writes::Work),
    /// Asking whether GitHub is back, the queue, the kept repositories;
    /// answered by [`Msg::Offline`].
    Offline(crate::offline::Cmd),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Api {
    FetchViewer,
    /// How many notifications are unread, answered by [`Msg::Unread`].
    FetchUnread,
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
    /// Who and what the repository's comments can mention.
    FetchMentionable(crate::api::model::RepoId),
    /// Whether a watched pull request changed, asked with ETags.
    Poll {
        pr: PrRef,
        asks: Vec<crate::live::Ask>,
    },
    /// Saving an artifact, running a workflow by hand.
    Actions(crate::actions::Work),
    /// Whether another watched page changed, asked with ETags.
    Watch {
        what: crate::live::page::Watched,
        asks: Vec<crate::live::Ask<crate::live::page::Probe>>,
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
    /// A question about the code at a revision (definitions, symbols,
    /// references), answered by [`Msg::Code`].
    Code(crate::code_nav::Ask),
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
    /// Poll the unread count, if this is still the latest poll.
    Unread(u64),
    /// Ask whether the pull request watched (by its watch's id) changed.
    Watch(u64),
    /// Ask whether GitHub can be reached again.
    Probe,
}

/// Braille spinner frames.
pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

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

    /// The screen under the last, if there's one.
    pub fn below(&self) -> Option<&Screen> {
        match self.rest.len() {
            0 => None,
            1 => Some(&self.first),
            n => self.rest.get(n - 2),
        }
    }

    /// The last screen, unless it's the first.
    pub fn pop(&mut self) -> Option<Screen> {
        self.rest.pop()
    }

    pub fn len(&self) -> usize {
        self.rest.len() + 1
    }
}

pub enum Overlay {
    /// `/` prompt: searching the diff or the page on screen.
    Find(Box<TextArea<'static>>),
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
    /// Asking before pushing a branch or opening a pull request.
    Ask(Box<crate::create::Ask>),
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
    /// Where the data git can answer came from, git or the API.
    pub sources: HashMap<DataKey, crate::source::From>,
    pub diffs: HashMap<DiffOf, DiffState>,
    pub viewer: Option<String>,
    /// How many notifications are unread, for the header.
    pub unread: Option<crate::api::notifications::Unread>,
    /// The latest poll of the unread count: an older poll's timer is
    /// let go.
    pub unread_poll: u64,
    pub rate_limits: RateLimits,
    pub overlay: Option<Overlay>,
    pub notices: Notices,
    /// Lasting problems, most important first; the first is shown.
    pub problems: std::collections::BTreeMap<Problem, String>,
    /// Terminal size.
    pub size: (u16, u16),
    pub keymap: Keymap,
    /// Keys typed so far that start a longer binding.
    pub typed: Vec<Key>,
    /// A count typed before them.
    pub count: Option<usize>,
    /// The page's other tabs as they were left, to come back to.
    pub parked: Vec<Screen>,
    /// The last search went backwards (`?`): `n` goes up.
    pub backwards: bool,
    pub theme: Theme,
    pub icons: Icons,
    pub density: crate::config::Density,
    /// Unix seconds; pages show relative times.
    pub clock: fn() -> u64,
    /// The spinner's frame, and whether its next frame is scheduled.
    pub spinner: usize,
    pub spinning: bool,
    /// [`Timer::Live`] is scheduled.
    pub live: bool,
    /// A change GitHub accepted that its page doesn't show yet.
    pub awaiting: Option<act::Awaiting>,
    /// An action waiting for what it's about to load.
    pub pending: Option<act::Pending>,
    /// The pull request under review on screen, watched for changes.
    pub watch: Option<crate::live::Watch>,
    /// Another page on screen watched for changes: a running job, runs,
    /// an issue, a discussion.
    pub page_watch: Option<crate::live::page::PageWatch>,
    /// The terminal has the focus.
    pub focused: bool,
    /// Who and what each repository's comments can mention, once asked.
    pub mentionable: HashMap<crate::api::model::RepoId, crate::api::mention::Mentionable>,
    /// New issues and pull requests being written, and your clones.
    pub creating: crate::create::Creating,
    /// Releases, gists and wiki pages being written, and the change one
    /// waits to see.
    pub writing: crate::writes::Writing,
    /// What the pickers that manage an issue or pull request list.
    pub managing: crate::manage::Managing,
    /// A settings form sent and not yet taken.
    pub settings: crate::settings::Kept,
    /// Workflows being run by hand: their forms.
    pub dispatching: crate::actions::Dispatching,
    /// Conversations: long reviews unfolded, and the like.
    pub talk: crate::talk::Talk,
    /// Whether GitHub can be reached, the writes waiting for it, and the
    /// repositories kept offline.
    pub offline: crate::offline::Offline,
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
            sources: HashMap::new(),
            diffs: HashMap::new(),
            viewer: None,
            unread: None,
            unread_poll: 0,
            rate_limits: RateLimits::default(),
            overlay: None,
            notices: Notices::default(),
            problems: Default::default(),
            size,
            keymap,
            typed: Vec::new(),
            count: None,
            parked: Vec::new(),
            backwards: false,
            theme,
            icons,
            density: crate::config::Density::default(),
            clock: crate::store::now,
            spinner: 0,
            spinning: false,
            live: false,
            awaiting: None,
            pending: None,
            watch: None,
            page_watch: None,
            focused: true,
            mentionable: HashMap::new(),
            creating: crate::create::Creating::default(),
            writing: crate::writes::Writing::default(),
            managing: crate::manage::Managing::default(),
            settings: crate::settings::Kept::default(),
            dispatching: crate::actions::Dispatching::default(),
            talk: crate::talk::Talk::default(),
            offline: crate::offline::Offline::default(),
            quit: false,
        };
        state.sync_page();
        state
    }

    /// Says `text` in the status bar for a few seconds.
    pub fn info(&mut self, text: impl Into<String>) {
        self.notices.info(text);
    }

    /// Says what went wrong, in the status bar (and the messages list).
    pub fn error(&mut self, text: impl Into<String>) {
        self.notices.error(text);
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

    /// The key contexts active, innermost first.
    pub fn contexts(&self) -> &'static [Context] {
        let innermost = match self.screen() {
            Screen::Diff(d) if d.focus == Pane::Tree && d.prefs.tree_visible => Context::Tree,
            Screen::Diff(_) => Context::Diff,
            Screen::Page(p) => match &p.route {
                Route::Job { .. } => Context::Log,
                Route::Blob { .. } | Route::Blame { .. } => Context::File,
                Route::Project { .. } | Route::ProjectItem { .. } => Context::Project,
                Route::Settings { .. } => Context::Settings,
                route if route.list_query().is_some() => Context::List,
                _ => Context::Page,
            },
        };
        innermost.stack()
    }

    /// The page on screen, if it is one.
    pub fn route(&self) -> Option<&Route> {
        match self.screen() {
            Screen::Page(p) => Some(&p.route),
            Screen::Diff(_) => None,
        }
    }
}

impl State {
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
}

#[cfg(test)]
pub(crate) mod tests;
