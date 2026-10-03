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
use crate::review::{
    self, Compose, ComposeTarget, EditPurpose, Preview, SubmitDialog, SubmitOutcome,
};
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
    /// Writing a comment, reply or suggestion.
    Compose(Box<Compose>),
    /// Submitting the review.
    Submit(Box<SubmitDialog>),
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
        Some(Overlay::FindFile(_)) => return on_finder_key(state, key),
        Some(Overlay::Compose(_)) => return on_compose_key(state, key),
        Some(Overlay::Submit(_)) => return on_submit_key(state, key),
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
        | Action::SearchPrev
        | Action::Comment
        | Action::VisualLines
        | Action::Suggest
        | Action::ReplyThread
        | Action::ResolveThread
        | Action::DeleteDraft
        | Action::FileComment
        | Action::SubmitReview
        | Action::NextThread
        | Action::PrevThread => {}
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
    match action {
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
        for _ in 0..(state.palette_commands("").len() + PALETTE_ROWS) {
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
