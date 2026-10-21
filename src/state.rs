//! Application state and the single update function. Input events and async
//! results arrive as [`Msg`]s; [`update`] mutates [`State`] and returns
//! [`Cmd`]s for the runtime to execute in the background.

use crate::route::{Route, Target, app, code, projects, runs, settings};
use std::collections::HashMap;
use std::sync::Arc;

use crate::api::ApiError;
use crate::api::model::{NodeId, PrRef, ViewedFiles};
use crate::api::rate_limit::RateLimits;
use crate::diff::FileDiff;
use crate::git::Oid;
use crate::store::ReviewState;
use crate::theme::{Bg, Theme};
use crate::ui::bars::Notice;
use crate::ui::{Ctx, Fetched, Icons};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use ratatui_textarea::TextArea;

use crate::act;
use crate::api::model::{PatchFile, ReviewThread};
use crate::browse;
use crate::compose::{self, Compose};
use crate::data::{Data, DataKey};
use crate::diff_job::{JobId, JobMsg};
use crate::diff_screen::{self, DiffOf, DiffPrefs, DiffScreen, DiffState, Pane};
use crate::join;
use crate::keymap::{Action, Context, Keymap};
use crate::nav::{self, Hints, Menu, PageScreen, SearchBox, Visit};
use crate::notice::Notices;
use crate::picker::{self, Picker};
use crate::review::{EditPurpose, SubmitDialog, on_edited, on_submit_key, review_action};
use crate::tabs::Tab;
use crate::work::Failure;

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
    /// Page data; with `cached_at` (when it was fetched), the cached copy
    /// shown while fetching. Where git could have answered, `source` says
    /// where it came from.
    Fetched {
        key: DataKey,
        result: Result<Data, ApiError>,
        cached_at: Option<u64>,
        source: Option<crate::source::From>,
    },
    /// The next page of a list, asked for after the cursor, and where it
    /// came from.
    FetchedMore(
        DataKey,
        String,
        Result<Data, ApiError>,
        Option<crate::source::From>,
    ),
    Mouse(MouseEvent),
    Timer(Timer),
    /// The search box's input settled; fetch live suggestions if it's
    /// still this.
    SuggestDue(String),
    RateLimits(RateLimits),
    Notice(Notice),
    /// A lasting problem started (`Some`) or cleared up (`None`).
    Problem(Problem, Option<String>),
    /// `$EDITOR` finished (or failed to start).
    Edited(EditPurpose, Result<String, Failure>),
    /// For a PR's diff screen.
    Diff(DiffOf, DiffMsg),
    /// Text pasted into the terminal, whole.
    Paste(String),
    /// The terminal came into focus (`true`) or left it.
    Focus(bool),
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
    /// What came of work in the background.
    Done(crate::work::Done),
    /// Whether GitHub can be reached, the queued writes, the kept
    /// repositories ([`crate::offline`]).
    Offline(crate::offline::Msg),
}

impl Msg {
    /// A fetch's answer.
    pub fn fetched(key: DataKey, result: Result<Data, ApiError>) -> Self {
        Msg::fetched_at(key, result, None)
    }

    /// A fetch's answer, as the disk cache had it at `cached_at` if it did.
    pub fn fetched_at(
        key: DataKey,
        result: Result<Data, ApiError>,
        cached_at: Option<u64>,
    ) -> Self {
        Msg::Fetched {
            key,
            result,
            cached_at,
            source: None,
        }
    }
}

/// What arrives for a PR's diff: from its job, GitHub, or the drafts.
#[derive(Debug)]
pub enum DiffMsg {
    /// From the diff job; ignored unless it's the PR's current job.
    Job(JobId, JobMsg),
    ViewedLoaded(Box<Result<ViewedFiles, ApiError>>),
    /// The saved review state, or why it couldn't be read.
    ReviewLoaded(Result<ReviewState, Failure>),
    ThreadsLoaded(Result<Vec<ReviewThread>, ApiError>),
    PatchesLoaded(Result<Vec<PatchFile>, ApiError>),
    /// How many comments your pending review on GitHub holds.
    PendingReview(Result<u64, ApiError>),
    LastReview(Result<Option<String>, ApiError>),
    CommitsListed(Result<Vec<crate::git::repo::Commit>, Failure>),
}

/// Commands for the runtime. Dropping them loses their effects, so every
/// function that returns them must have them used.
#[must_use]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cmds(Vec<Cmd>);

impl Cmds {
    pub fn new() -> Self {
        Self::default()
    }
}

impl std::ops::Deref for Cmds {
    type Target = Vec<Cmd>;
    fn deref(&self) -> &Vec<Cmd> {
        &self.0
    }
}

impl std::ops::DerefMut for Cmds {
    fn deref_mut(&mut self) -> &mut Vec<Cmd> {
        &mut self.0
    }
}

impl From<Vec<Cmd>> for Cmds {
    fn from(cmds: Vec<Cmd>) -> Self {
        Self(cmds)
    }
}

impl From<Cmd> for Cmds {
    fn from(cmd: Cmd) -> Self {
        Self(vec![cmd])
    }
}

impl FromIterator<Cmd> for Cmds {
    fn from_iter<I: IntoIterator<Item = Cmd>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl IntoIterator for Cmds {
    type Item = Cmd;
    type IntoIter = std::vec::IntoIter<Cmd>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a Cmds {
    type Item = &'a Cmd;
    type IntoIter = std::slice::Iter<'a, Cmd>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

#[cfg(test)]
impl PartialEq<Vec<Cmd>> for Cmds {
    fn eq(&self, other: &Vec<Cmd>) -> bool {
        self.0 == *other
    }
}

#[cfg(test)]
impl<const N: usize> PartialEq<[Cmd; N]> for Cmds {
    fn eq(&self, other: &[Cmd; N]) -> bool {
        self.0 == *other
    }
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
    /// A write to GitHub, or git in your clone; answered by
    /// [`Msg::Done`].
    Work(crate::work::Work),
    /// Asking whether GitHub is back, the queue, the kept repositories;
    /// answered by [`Msg::Offline`].
    Offline(crate::offline::Cmd),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Api {
    FetchViewer,
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
    SaveVisits(Vec<Visit>),
    FetchViewed(PrRef),
    FetchThreads(PrRef),
    FetchPatches(PrRef),
    /// How many comments your pending review on GitHub holds.
    FetchPendingReview(PrRef),
    FetchLastReview {
        pr: PrRef,
        login: String,
    },
    /// Whether a watched pull request changed, asked with ETags.
    Poll {
        pr: PrRef,
        asks: Vec<crate::live::Ask>,
    },
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
    /// Asking before a write: a form's, or an action's change.
    Ask(Box<crate::forms::Ask>),
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
    /// Everything else pages show.
    pub data: HashMap<DataKey, Remote<Data>>,
    /// Bumped whenever what pages show besides GitHub's data and forms
    /// changes (writes on their way, who's signed in, whether GitHub can
    /// be reached), so pages know to rebuild; GitHub's data has its own
    /// versions ([`Remote`]), and forms theirs ([`crate::forms::Forms`]).
    pub shown_gen: u64,
    pub diffs: HashMap<DiffOf, DiffState>,
    pub viewer: Option<String>,
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
    /// The page's other tabs as they were left, to come back to.
    pub parked: Vec<Screen>,
    pub theme: Theme,
    pub icons: Icons,
    pub density: crate::config::Density,
    /// Unix seconds; pages show relative times.
    pub clock: fn() -> u64,
    /// When [`Timer::Minute`] last fired (Unix seconds): a page is built
    /// again for a time it shows only once it reads differently by then,
    /// so all of a minute's times move on together.
    pub minute: u64,
    /// The spinner's frame, and whether its next frame is scheduled.
    pub spinner: usize,
    pub spinning: bool,
    /// [`Timer::Live`] is scheduled.
    pub live: bool,
    /// A change GitHub accepted that its page doesn't show yet.
    pub following: Option<crate::follow::Following>,
    /// An action waiting for what it's about to load.
    pub pending: Option<act::Pending>,
    /// The pull request under review on screen, watched for changes.
    pub watch: Option<crate::live::Watch>,
    /// Another page on screen watched for changes: a running job, runs,
    /// an issue, a discussion.
    pub page_watch: Option<crate::live::page::PageWatch>,
    /// The terminal has the focus.
    pub focused: bool,
    /// The forms being written, by their page: leaving one keeps it.
    pub forms: crate::forms::Forms,
    /// What housekeeping's question is about, and a branch to delete.
    pub housekeeping: crate::housekeeping::Housekeeping,
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
            screens: Screens::new(Screen::Page(Box::new(PageScreen::new(
                Route::App(app::Home),
                None,
            )))),
            forward: Vec::new(),
            before: Vec::new(),
            after: Vec::new(),
            visits: Vec::new(),
            home: crate::home::defaults(),
            config_path: None,
            removed: None,
            home_editing: false,
            home_timer: false,
            data: HashMap::new(),
            shown_gen: 0,
            diffs: HashMap::new(),
            viewer: None,
            unread_poll: 0,
            rate_limits: RateLimits::default(),
            overlay: None,
            notices: Notices::default(),
            problems: Default::default(),
            size,
            keymap,
            parked: Vec::new(),
            theme,
            icons,
            density: crate::config::Density::default(),
            clock: crate::store::now,
            minute: 0,
            spinner: 0,
            spinning: false,
            live: false,
            following: None,
            pending: None,
            watch: None,
            page_watch: None,
            focused: true,
            forms: crate::forms::Forms::default(),
            housekeeping: crate::housekeeping::Housekeeping::default(),
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

    /// Says `text` in the status bar, and does nothing else: why an
    /// action can't go ahead, mostly.
    pub fn just_say(&mut self, text: impl Into<String>) -> Cmds {
        self.info(text);
        Cmds::new()
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
                Route::Runs(runs::Job { .. }) => Context::Log,
                Route::Code(code::Blob { .. }) | Route::Code(code::Blame { .. }) => Context::File,
                Route::Projects(projects::Project { .. })
                | Route::Projects(projects::ProjectItem { .. }) => Context::Project,
                Route::Settings(settings::Page { .. }) => Context::Settings,
                route if route.family().query().is_some() => Context::List,
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
    pub fn settle(&mut self) -> Cmds {
        let mut cmds = self.settle_ref();
        self.sync_page();
        cmds.extend(self.settle_diff());
        cmds
    }

    /// Opens the page an unsplit one ([`code::Unsplit`]) is, once where its ref
    /// ends is known.
    fn settle_ref(&mut self) -> Cmds {
        let Screen::Page(p) = self.screen() else {
            return Cmds::new();
        };
        let Some(route) = self.split(&p.route, None) else {
            return Cmds::new();
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

    /// The page an unsplit one ([`code::Unsplit`]) is, if where its ref ends is
    /// known here: GitHub said, or its ref-and-path starts with the
    /// repository's default branch or with `rev` (the revision of the page
    /// that links to it). Git keeps a branch from being the start of
    /// another, so a branch already known is the one (short of a tag that
    /// extends it).
    pub(crate) fn split(&self, route: &Route, rev: Option<&str>) -> Option<Route> {
        let Route::Code(code::Unsplit { repo, spot, .. }) = route else {
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
