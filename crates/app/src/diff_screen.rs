//! The diff screen: file tree plus diff, and its key handling.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use ghtui_api::model::{NodeId, PatchFile, PrRef, RepoId, ReviewThread, ViewedFiles, ViewedState};
use ghtui_diff::anchor::Commentable;
use ghtui_diff::{FileDiff, Whitespace};
use ghtui_git::Oid;
use ghtui_git::repo::{Commit, PrRefs};
use ghtui_store::ReviewState;
use ghtui_ui::bars::Notice;
use ghtui_ui::diff_doc::{Doc, Note, Pos, Row, ViewOptions, Viewed};
use ghtui_ui::file_tree::{TreeRow, row_of_file, tree_rows};
use ghtui_ui::text::short_sha;
use ratatui::layout::Rect;

use crate::diff_job::{DiffFiles, JobId, JobMsg, next_job};
use crate::keymap::Action;
use crate::picker;
use crate::review::{on_submitted, review_action, start_since_review};
use crate::state::{Api, Cmd, DiffMsg, Git, Overlay, Problem, State};

/// What GitHub says about your last submitted review of a PR.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum LastReview {
    /// Not asked yet.
    #[default]
    Unknown,
    /// You haven't submitted one.
    None,
    /// The head commit it was on.
    At(Oid),
}

/// What a diff screen shows: a pull request's changes, or one commit's.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DiffOf {
    Pr(PrRef),
    /// A commit (its full ID) against its first parent.
    Commit(RepoId, String),
    /// From one commit to another (full IDs), as `a..b`: a comparison's
    /// files, from the merge base its page found.
    Range(RepoId, String, String),
}

impl DiffOf {
    /// The pull request, for what only pull requests have: reviews,
    /// threads, viewed files.
    pub fn pr(&self) -> Option<&PrRef> {
        match self {
            DiffOf::Pr(pr) => Some(pr),
            DiffOf::Commit(..) | DiffOf::Range(..) => None,
        }
    }

    pub fn repo(&self) -> &RepoId {
        match self {
            DiffOf::Pr(pr) => &pr.repo,
            DiffOf::Commit(repo, _) | DiffOf::Range(repo, ..) => repo,
        }
    }
}

impl From<PrRef> for DiffOf {
    fn from(pr: PrRef) -> Self {
        DiffOf::Pr(pr)
    }
}

impl std::fmt::Display for DiffOf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiffOf::Pr(pr) => pr.fmt(f),
            DiffOf::Commit(repo, oid) => write!(f, "{repo}@{}", short_sha(oid)),
            DiffOf::Range(repo, from, to) => {
                write!(f, "{repo}@{}...{}", short_sha(from), short_sha(to))
            }
        }
    }
}

/// Split view turns on automatically from this diff-pane width.
pub const SPLIT_MIN_WIDTH: u16 = 160;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Tree,
    Diff,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffScreen {
    pub of: DiffOf,
    pub cursor: Pos,
    pub top: Pos,
    pub tree_visible: bool,
    pub focus: Pane,
    pub tree_selected: usize,
    pub tree_scroll: usize,
    /// `None` picks split or unified by width.
    pub split_override: Option<bool>,
    pub ignore_whitespace: bool,
    /// The last search query, for `n` / `N`.
    pub search: Option<String>,
    /// Where a visual line selection started.
    pub selection: Option<Pos>,
    /// What a link pointed at, to go to once it's loaded: a file
    /// (`diff-<hash>`, GitHub's SHA-256 of its path) or a review comment
    /// (`discussion_r<id>`, `r<id>`).
    pub anchor: Option<String>,
}

impl DiffScreen {
    pub fn new(of: DiffOf, width: u16) -> Self {
        Self {
            of,
            cursor: Pos::default(),
            top: Pos::default(),
            tree_visible: width >= 100,
            focus: Pane::Diff,
            tree_selected: 0,
            tree_scroll: 0,
            split_override: None,
            ignore_whitespace: false,
            search: None,
            selection: None,
            anchor: None,
        }
    }

    /// The view options this screen wants at `content` size.
    pub fn options(&self, content: Rect) -> ViewOptions {
        let lay = layout(content, self.tree_visible);
        ViewOptions {
            split: self
                .split_override
                .unwrap_or(lay.diff.width >= SPLIT_MIN_WIDTH),
            whitespace: if self.ignore_whitespace {
                Whitespace::Ignore
            } else {
                Whitespace::Exact
            },
            // Thread cards start after the gutter; keep comments readable.
            wrap: lay.diff.width.saturating_sub(24).clamp(20, 100),
        }
    }
}

/// A commit range being viewed: label, from (exclusive), to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeView {
    pub label: String,
    pub from: String,
    pub to: String,
}

/// Everything known about one PR's diff.
#[derive(Debug, Default)]
pub struct DiffState {
    pub doc: Doc,
    pub tree: Vec<TreeRow>,
    pub refs: Option<PrRefs>,
    /// What the background job is doing, while it works.
    pub progress: Option<String>,
    pub error: Option<String>,
    /// GitHub's viewed state and the PR's node ID.
    pub viewed: Option<ViewedFiles>,
    /// Locally persisted review marks and draft comments.
    pub review: ReviewState,
    /// Review threads from GitHub.
    pub threads: Vec<ReviewThread>,
    /// Outdated threads mapped onto the current diff (`None`: can't be).
    pub mapped: HashMap<NodeId, Option<u32>>,
    /// Outdated-thread mapping was requested.
    pub mapping_requested: bool,
    /// Move detection was requested (once every file is diffed).
    pub moves_requested: bool,
    /// Your last submitted review on GitHub.
    pub last_review: LastReview,
    /// "Since my last review" was asked for and is waiting on data.
    pub since_requested: bool,
    /// The PR's commits, oldest first.
    pub commits: Vec<Commit>,
    /// Showing a sub-range of commits instead of the whole PR.
    pub range: Option<RangeView>,
    /// Files we've already asked the job to prioritize.
    requested: HashSet<usize>,
    /// The diff job whose results this shows; older jobs' are ignored.
    pub job: JobId,
    /// The saved review state has been read (saving before that would
    /// overwrite it).
    pub review_loaded: bool,
    /// The draft deleted last, for undo.
    pub deleted: Option<ghtui_store::DraftComment>,
}

impl DiffState {
    /// The file list has arrived.
    pub fn listed(&self) -> bool {
        self.refs.is_some()
    }

    /// The head commit the diff was computed at.
    pub fn head(&self) -> Option<Oid> {
        self.refs.as_ref().map(|r| r.head.clone())
    }

    pub fn loading() -> Self {
        Self {
            progress: Some("Preparing".into()),
            job: next_job(),
            ..Self::default()
        }
    }

    pub fn set_files(&mut self, refs: PrRefs, doc: Doc) {
        self.tree = tree_rows(&doc);
        self.doc = doc;
        self.refs = Some(refs);
        self.progress = if self.doc.is_empty() {
            None
        } else {
            Some("Computing diffs".into())
        };
        self.apply_viewed();
        self.apply_review();
    }

    pub fn set_file(&mut self, index: usize, diff: Arc<FileDiff>) {
        self.doc.set_diff(index, diff);
        if self.doc.ready_count() == self.doc.files().len() {
            self.progress = None;
        }
    }

    /// Starts over for a different commit range, keeping threads, drafts,
    /// viewed state and commits.
    pub fn restart(&mut self, range: Option<RangeView>) {
        self.doc = Doc::default();
        self.tree.clear();
        self.refs = None;
        self.progress = Some("Preparing".into());
        self.error = None;
        self.requested.clear();
        self.moves_requested = false;
        self.mapping_requested = false;
        self.since_requested = false;
        self.range = range;
        self.job = next_job();
    }

    /// Every file's diff, once all have arrived and moves haven't been
    /// looked for yet.
    pub fn take_move_inputs(&mut self) -> Option<Vec<(usize, Arc<FileDiff>)>> {
        if self.moves_requested || !self.listed() || self.doc.ready_count() < self.doc.files().len()
        {
            return None;
        }
        self.moves_requested = true;
        Some(
            self.doc
                .files()
                .iter()
                .enumerate()
                .filter_map(|(i, f)| f.diff.clone().map(|d| (i, d)))
                .collect(),
        )
    }

    pub fn set_viewed_states(&mut self, viewed: ViewedFiles) {
        self.viewed = Some(viewed);
        self.apply_viewed();
    }

    #[cfg(test)]
    pub fn set_review(&mut self, review: ReviewState) {
        self.review = review;
        self.review_loaded = true;
        self.apply_review();
    }

    /// Takes in the saved review state, keeping whatever was done before
    /// it arrived. True if that left something new to save.
    pub fn merge_saved_review(&mut self, saved: ReviewState) -> bool {
        let early = std::mem::replace(&mut self.review, saved);
        let changed = early != ReviewState::default();
        let review = &mut self.review;
        for hunk in early.reviewed_hunks {
            if !review.reviewed_hunks.contains(&hunk) {
                review.reviewed_hunks.push(hunk);
            }
        }
        review.reviewed_hunks.sort();
        for mut draft in early.pending {
            draft.id = review.next_draft_id();
            review.pending.push(draft);
        }
        if early.last_reviewed_head.is_some() {
            review.last_reviewed_head = early.last_reviewed_head;
        }
        self.review_loaded = true;
        self.apply_review();
        changed
    }

    fn apply_viewed(&mut self) {
        let Some(viewed) = &self.viewed else { return };
        let updates: Vec<(usize, Viewed)> = self
            .doc
            .files()
            .iter()
            .enumerate()
            .filter_map(|(i, f)| {
                let state = match viewed.states.get(f.meta.path()) {
                    Some(ViewedState::Viewed) => Viewed::Viewed,
                    Some(ViewedState::Dismissed) => Viewed::Dismissed,
                    _ => Viewed::Unviewed,
                };
                (f.viewed != state).then_some((i, state))
            })
            .collect();
        for (i, state) in updates {
            self.doc.set_viewed(i, state);
        }
    }

    fn apply_review(&mut self) {
        self.doc.reviewed = self.review.reviewed_hunks.iter().cloned().collect();
        self.refresh_annotations();
    }

    pub fn set_threads(&mut self, threads: Vec<ReviewThread>) {
        self.threads = threads;
        self.refresh_annotations();
    }

    pub fn set_patches(&mut self, patches: Vec<PatchFile>) {
        let map = patches
            .into_iter()
            .filter_map(|f| Some((f.filename, Commentable::from_patch(&f.patch?))))
            .collect();
        self.doc.set_patches(map);
    }

    /// Rebuilds thread and draft annotations from current data.
    pub fn refresh_annotations(&mut self) {
        let annotations =
            crate::review::annotations(&self.threads, &self.mapped, &self.review.pending);
        if annotations != self.doc.annotations() {
            self.doc.set_annotations(annotations);
        }
    }

    pub fn status(&self) -> Option<String> {
        let progress = self.progress.as_ref()?;
        if self.listed() && !self.doc.is_empty() {
            Some(format!(
                "Diffing {}/{} files",
                self.doc.ready_count(),
                self.doc.files().len()
            ))
        } else {
            Some(progress.clone())
        }
    }
}

pub struct Layout {
    pub tree: Option<Rect>,
    pub diff: Rect,
}

/// Splits the content area; shared by the view and key handling so both
/// agree on viewport sizes.
pub fn layout(content: Rect, tree_visible: bool) -> Layout {
    if !tree_visible || content.width < 60 {
        return Layout {
            tree: None,
            diff: content,
        };
    }
    let width = (content.width / 4).clamp(24, 40);
    Layout {
        tree: Some(Rect { width, ..content }),
        diff: Rect {
            x: content.x + width,
            width: content.width - width,
            ..content
        },
    }
}

/// Rows of the tree list below its title.
fn tree_list_height(tree: Rect) -> usize {
    usize::from(tree.height.saturating_sub(3))
}

/// Runs `change` on the document, keeping the cursor and the top of the
/// view on the same source lines.
pub fn preserving_position(screen: &mut DiffScreen, doc: &mut Doc, change: impl FnOnce(&mut Doc)) {
    let cursor = doc.anchor(screen.cursor);
    let top = doc.anchor(screen.top);
    change(doc);
    screen.cursor = doc.locate(cursor);
    screen.top = doc.locate(top);
}

/// Handles `action` on the diff screen. Those that don't work here say
/// where they do.
#[must_use]
pub fn apply(
    screen: &mut DiffScreen,
    state: &mut DiffState,
    action: Action,
    content: Rect,
    notice: &mut Option<Notice>,
) -> Vec<Cmd> {
    let lay = layout(content, screen.tree_visible);
    let half = (usize::from(lay.diff.height) / 2).max(1).cast_signed();
    let tree_focused = screen.focus == Pane::Tree && lay.tree.is_some();
    // While searching, n and p go through the matches.
    let action = match action {
        Action::NextHunk if screen.search.is_some() => Action::SearchNext,
        Action::PrevHunk if screen.search.is_some() => Action::SearchPrev,
        action => action,
    };
    let mut cmds = Vec::new();
    match action {
        // A page is two half pages, so the cursor keeps its place on screen.
        Action::PageDown | Action::PageUp => {
            let half = if action == Action::PageDown {
                Action::HalfPageDown
            } else {
                Action::HalfPageUp
            };
            cmds = apply(screen, state, half, content, notice);
            cmds.extend(apply(screen, state, half, content, notice));
        }
        Action::ToggleTree => {
            screen.tree_visible = !screen.tree_visible;
            if !screen.tree_visible {
                screen.focus = Pane::Diff;
            }
        }
        Action::SwitchPane => {
            if lay.tree.is_some() {
                screen.focus = match screen.focus {
                    Pane::Tree => Pane::Diff,
                    Pane::Diff => Pane::Tree,
                };
            }
        }
        Action::ToggleSplit => {
            screen.split_override = Some(!screen.options(content).split);
        }
        Action::IgnoreWhitespace => {
            screen.ignore_whitespace = !screen.ignore_whitespace;
            *notice = Some(Notice::Info(
                if screen.ignore_whitespace {
                    "Ignoring whitespace changes"
                } else {
                    "Showing whitespace changes"
                }
                .into(),
            ));
        }
        // Moving in the file tree; everything else acts on the diff.
        Action::Open if tree_focused => screen.focus = Pane::Diff,
        Action::Down
        | Action::Up
        | Action::HalfPageDown
        | Action::HalfPageUp
        | Action::Top
        | Action::Bottom
            if tree_focused =>
        {
            let tree_half = (tree_list_height(lay.tree.unwrap_or_default()) / 2)
                .max(1)
                .cast_signed();
            let delta = match action {
                Action::Down => 1,
                Action::Up => -1,
                Action::HalfPageDown => tree_half,
                Action::HalfPageUp => -tree_half,
                Action::Top => isize::MIN,
                _ => isize::MAX,
            };
            let last = state.tree.len().saturating_sub(1);
            screen.tree_selected = screen.tree_selected.saturating_add_signed(delta).min(last);
            // The diff follows the tree selection.
            if let Some(TreeRow::File { index, .. }) = state.tree.get(screen.tree_selected) {
                jump(
                    screen,
                    Pos {
                        file: *index,
                        row: 0,
                    },
                );
            }
        }
        Action::Down => screen.cursor = state.doc.offset(screen.cursor, 1),
        Action::Up => screen.cursor = state.doc.offset(screen.cursor, -1),
        Action::HalfPageDown => {
            screen.cursor = state.doc.offset(screen.cursor, half);
            screen.top = state.doc.offset(screen.top, half);
        }
        Action::HalfPageUp => {
            screen.cursor = state.doc.offset(screen.cursor, -half);
            screen.top = state.doc.offset(screen.top, -half);
        }
        Action::Top => screen.cursor = Pos::default(),
        Action::Bottom => screen.cursor = state.doc.last(),
        Action::NextHunk
        | Action::PrevHunk
        | Action::NextFile
        | Action::PrevFile
        | Action::NextThread
        | Action::PrevThread => {
            let (doc, at) = (&state.doc, screen.cursor);
            let (found, none) = match action {
                Action::NextHunk => (doc.next_hunk(at), "No more changes"),
                Action::PrevHunk => (doc.prev_hunk(at), "No more changes"),
                Action::NextFile => (doc.next_file(at), "No more files"),
                Action::PrevFile => (doc.prev_file(at), "No more files"),
                Action::NextThread => (doc.next_thread(at), "No more unresolved threads"),
                _ => (doc.prev_thread(at), "No more unresolved threads"),
            };
            match found {
                // A file starts at the top of the view.
                Some(pos) if matches!(action, Action::NextFile | Action::PrevFile) => {
                    jump(screen, pos);
                }
                Some(pos) => screen.cursor = pos,
                None => *notice = Some(Notice::Info(none.into())),
            }
        }
        Action::VisualLines => {
            let (selection, what) = match screen.selection {
                Some(_) => (None, "Selection cleared"),
                None => (Some(screen.cursor), "Selecting lines"),
            };
            screen.selection = selection;
            *notice = Some(Notice::Info(what.into()));
        }
        Action::NextUnviewed => match state.doc.next_unviewed(screen.cursor) {
            Some(pos) => jump(screen, pos),
            None => *notice = Some(Notice::Info("Every file is viewed".into())),
        },
        Action::ExpandContext => {
            let pos = screen.cursor;
            let rows = |doc: &Doc| doc.files().get(pos.file).map(|f| f.rows().len());
            let before = rows(&state.doc);
            preserving_position(screen, &mut state.doc, |doc| doc.expand(pos));
            if rows(&state.doc) == before {
                *notice = Some(Notice::Info("No more context here".into()));
            }
        }
        Action::FullFile => {
            let file = screen.cursor.file;
            preserving_position(screen, &mut state.doc, |doc| doc.toggle_full(file));
        }
        Action::JumpMove => match state.doc.move_at(screen.cursor) {
            Some((mv, from)) => {
                if let Some(pos) = state.doc.move_target(mv, from) {
                    screen.cursor = pos;
                }
            }
            None => *notice = Some(Notice::Info("Not on moved code".into())),
        },
        Action::Open => match state.doc.row(screen.cursor) {
            Some(Row::Fold { .. }) => {
                let pos = screen.cursor;
                preserving_position(screen, &mut state.doc, |doc| {
                    doc.unfold(pos);
                });
            }
            Some(Row::Gap { .. }) => {
                let pos = screen.cursor;
                preserving_position(screen, &mut state.doc, |doc| doc.expand(pos));
            }
            Some(Row::Header | Row::Note(Note::Collapsed | Note::Viewed))
                if state
                    .doc
                    .files()
                    .get(screen.cursor.file)
                    .is_some_and(|f| f.collapsed() || f.expanded) =>
            {
                let file = screen.cursor.file;
                state.doc.toggle_expanded(file);
                // Expanded files are diffed on demand.
                state.requested.remove(&file);
                screen.cursor.row = 0;
            }
            _ => *notice = Some(Notice::Info("Nothing to open here".into())),
        },
        Action::ToggleViewed => cmds.extend(toggle_viewed(screen, state, notice)),
        Action::MarkReviewed => cmds.extend(toggle_reviewed(screen, state, notice)),
        Action::SearchNext | Action::SearchPrev => {
            let Some(query) = screen.search.clone() else {
                *notice = Some(Notice::Info("No search yet".into()));
                return Vec::new();
            };
            match state
                .doc
                .search(&query, screen.cursor, action == Action::SearchNext)
            {
                Some(pos) => screen.cursor = pos,
                None => *notice = Some(Notice::Error(format!("No match for “{query}”"))),
            }
        }
        _ => *notice = Some(Notice::Info(action.not_here())),
    }
    cmds
}

/// Puts `pos` at the top of the view with the cursor on it.
fn jump(screen: &mut DiffScreen, pos: Pos) {
    screen.cursor = pos;
    screen.top = pos;
}

/// Starts a search from the cursor.
pub fn search(screen: &mut DiffScreen, state: &DiffState, query: String) -> Notice {
    let notice = match state.doc.search(&query, screen.cursor, true) {
        Some(pos) => {
            screen.cursor = pos;
            let n = state.doc.count_matches(&query);
            Notice::Info(format!(
                "{n} line{} match “{query}”",
                if n == 1 { "" } else { "s" }
            ))
        }
        None => Notice::Error(format!("No match for “{query}”")),
    };
    screen.search = Some(query);
    notice
}

#[must_use]
fn toggle_viewed(
    screen: &mut DiffScreen,
    state: &mut DiffState,
    notice: &mut Option<Notice>,
) -> Vec<Cmd> {
    let Some(pr) = screen.of.pr().cloned() else {
        *notice = Some(Notice::Info("Viewed files are a pull request's".into()));
        return Vec::new();
    };
    let Some(viewed) = &state.viewed else {
        *notice = Some(Notice::Error(
            "Viewed state hasn't loaded from GitHub yet".into(),
        ));
        return Vec::new();
    };
    let index = screen.cursor.file;
    let Some(file) = state.doc.files().get(index) else {
        return Vec::new();
    };
    let previous = file.viewed;
    let now = if previous == Viewed::Viewed {
        Viewed::Unviewed
    } else {
        Viewed::Viewed
    };
    let cmd = Cmd::Api(Api::SetViewed {
        pr,
        pull_request_id: viewed.pull_request_id.clone(),
        path: file.meta.path().to_owned(),
        file: index,
        viewed: now == Viewed::Viewed,
        previous,
    });
    // Optimistic: show it now, roll back if GitHub refuses.
    state.doc.set_viewed(index, now);
    screen.cursor = Pos {
        file: index,
        row: 0,
    };
    vec![cmd]
}

#[must_use]
fn toggle_reviewed(
    screen: &DiffScreen,
    state: &mut DiffState,
    notice: &mut Option<Notice>,
) -> Vec<Cmd> {
    let Some(pr) = screen.of.pr().cloned() else {
        *notice = Some(Notice::Info("Reviewed marks are a pull request's".into()));
        return Vec::new();
    };
    let Some(block) = state.doc.block_at(screen.cursor) else {
        *notice = Some(Notice::Info(
            "Move to a changed line to mark it reviewed".into(),
        ));
        return Vec::new();
    };
    let hash = block.hash.clone();
    let hunks = &mut state.review.reviewed_hunks;
    if let Some(i) = hunks.iter().position(|h| *h == hash) {
        hunks.remove(i);
    } else {
        hunks.push(hash);
        hunks.sort();
    }
    state.doc.reviewed = hunks.iter().cloned().collect();
    vec![Cmd::SaveReview(pr, state.review.clone())]
}

/// How GitHub names a file in its diffs' anchors: the SHA-256 of its path,
/// in hex.
pub fn path_hash(path: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(path.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Where a link's anchor is in the diff: `Some(None)` when it isn't there,
/// `None` while the review threads it needs are still on their way.
fn anchor_pos(anchor: &str, state: &DiffState) -> Option<Option<Pos>> {
    let doc = &state.doc;
    if let Some(hash) = anchor.strip_prefix("diff-") {
        let hash = hash.get(..64).unwrap_or(hash);
        let file = doc
            .files()
            .iter()
            .position(|f| path_hash(f.meta.path()) == hash);
        return Some(file.map(|file| Pos { file, row: 0 }));
    }
    let id = anchor
        .strip_prefix("discussion_r")
        .or_else(|| anchor.strip_prefix('r'))
        .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))?;
    if state.threads.is_empty() {
        return None;
    }
    let comment = format!("#discussion_r{id}");
    // Annotations list the threads first, in order.
    let thread = state
        .threads
        .iter()
        .position(|t| t.comments.iter().any(|c| c.url.ends_with(&comment)));
    Some(thread.and_then(|t| doc.annotation_pos(u32::try_from(t).ok()?)))
}

/// Applies view options, clamps positions, keeps the cursor visible (clear
/// of the sticky header), syncs the tree with the diff, and asks the job to
/// prioritize files that are on screen but not diffed yet.
#[must_use]
pub fn settle(screen: &mut DiffScreen, state: &mut DiffState, content: Rect) -> Vec<Cmd> {
    let lay = layout(content, screen.tree_visible);
    if lay.tree.is_none() {
        screen.focus = Pane::Diff;
    }
    let opts = screen.options(content);
    if state.doc.opts() != opts {
        preserving_position(screen, &mut state.doc, |doc| doc.set_options(opts));
    }
    let doc = &state.doc;
    if doc.is_empty() {
        return Vec::new();
    }
    if let Some(anchor) = screen.anchor.take() {
        match anchor_pos(&anchor, state) {
            Some(Some(pos)) => screen.cursor = pos,
            // A review comment waits for the threads.
            None => screen.anchor = Some(anchor),
            Some(None) => {}
        }
    }
    let doc = &state.doc;
    screen.cursor = doc.clamp(screen.cursor);
    let height = usize::from(lay.diff.height).max(1);
    let total = doc.total_rows();
    let cursor = doc.to_global(screen.cursor);
    let mut top = doc.to_global(screen.top).min(total.saturating_sub(height));
    // Row 0 of the viewport is covered by the sticky header unless it's the
    // file's own header.
    let upper = if screen.cursor.row == 0 {
        cursor
    } else {
        cursor.saturating_sub(1)
    };
    if top > upper {
        top = upper;
    }
    if cursor >= top + height {
        top = cursor + 1 - height;
    }
    screen.top = doc.to_pos(top);

    if screen.focus == Pane::Diff
        && let Some(row) = row_of_file(&state.tree, screen.cursor.file)
    {
        screen.tree_selected = row;
    }
    if let Some(tree) = lay.tree {
        let rows = tree_list_height(tree).max(1);
        if screen.tree_selected < screen.tree_scroll {
            screen.tree_scroll = screen.tree_selected;
        } else if screen.tree_selected >= screen.tree_scroll + rows {
            screen.tree_scroll = screen.tree_selected + 1 - rows;
        }
    }

    let mut wanted = vec![screen.cursor.file];
    wanted.extend(doc.loading_in_view(screen.top, height));
    let wanted: Vec<usize> = wanted
        .into_iter()
        .filter(|f| {
            doc.files()
                .get(*f)
                .is_some_and(|file| file.diff.is_none() && !file.collapsed())
                && !state.requested.contains(f)
        })
        .collect();
    if wanted.is_empty() {
        return Vec::new();
    }
    state.requested.extend(wanted.iter().copied());
    vec![Cmd::Git(Git::Prioritize(screen.of.clone(), wanted))]
}

/// A message from a diff's current job.
#[must_use]
pub(crate) fn on_job(state: &mut State, of: &DiffOf, msg: JobMsg) -> Vec<Cmd> {
    match msg {
        JobMsg::Progress(line) => {
            if let Some(diff) = state.diffs.get_mut(of)
                && !diff.listed()
            {
                diff.progress = Some(line);
            }
        }
        JobMsg::Files(files) => {
            let Some(diff) = state.diffs.get_mut(of) else {
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
                return review_action(state, Action::ToggleSinceReview);
            }
        }
        JobMsg::File(index, file) => {
            if let Some(diff) = state.diffs.get_mut(of) {
                diff.set_file(index, file);
                if let Some(inputs) = diff.take_move_inputs() {
                    return vec![Cmd::Git(Git::DetectMoves(of.clone(), diff.job, inputs))];
                }
            }
        }
        JobMsg::Moves(moves) => match state.diff_parts() {
            Some((screen, diff)) if screen.of == *of => {
                preserving_position(screen, &mut diff.doc, |doc| doc.set_moves(moves));
            }
            _ => {
                if let Some(diff) = state.diffs.get_mut(of) {
                    diff.doc.set_moves(moves);
                }
            }
        },
        JobMsg::Failed(error) => {
            if let Some(diff) = state.diffs.get_mut(of) {
                diff.progress = None;
                diff.error = Some(error.to_string());
            }
        }
    }
    Vec::new()
}

/// A message for a diff screen.
#[must_use]
pub(crate) fn update(state: &mut State, of: &DiffOf, msg: DiffMsg) -> Vec<Cmd> {
    if let DiffMsg::Job(job, msg) = msg {
        if state.diffs.get(of).is_some_and(|d| d.job == job) {
            return on_job(state, of, msg);
        }
        return Vec::new();
    }
    // The rest is about reviewing a pull request.
    let DiffOf::Pr(pr) = of.clone() else {
        return Vec::new();
    };
    match msg {
        // The text stays in the composer, with GitHub's reason.
        DiffMsg::Replied(Err(err)) => {
            if let Some(Overlay::Compose(compose)) = &mut state.overlay {
                compose.sending = false;
                compose.error = Some(err.to_string());
            }
        }
        DiffMsg::Replied(Ok(())) => {
            state.overlay = None;
            state.info("Reply posted");
            return vec![Cmd::Api(Api::FetchThreads(pr))];
        }
        DiffMsg::LastReview(result) => {
            let commit = result.unwrap_or_else(|err| {
                tracing::warn!(%pr, %err, "last review lookup failed");
                None
            });
            if let Some(diff) = state.diffs.get_mut(of) {
                diff.last_review = commit.map_or(LastReview::None, |c| LastReview::At(Oid::new(c)));
                if diff.since_requested {
                    return start_since_review(state);
                }
            }
        }
        DiffMsg::SinceReady(old_head, Ok(hashes)) => {
            if let Some((screen, diff)) = state.diff_parts()
                && screen.of == *of
            {
                diff.since_requested = false;
                preserving_position(screen, &mut diff.doc, |doc| {
                    doc.set_since(Some(hashes), true);
                });
                state.info(format!(
                    "Showing changes since your review of {}",
                    short_sha(&old_head)
                ));
            }
        }
        DiffMsg::SinceReady(_, Err(err)) => {
            if let Some(diff) = state.diffs.get_mut(of) {
                diff.since_requested = false;
            }
            tracing::warn!(%pr, ?err, "comparing with the last review failed");
            state.error(format!("Couldn't compare with your last review: {err}"));
        }
        DiffMsg::CommitsListed(Ok(commits)) => {
            if let Some(diff) = state.diffs.get_mut(of) {
                diff.commits = commits;
            }
            state.notice = None;
            return state.open_picker(picker::Kind::Commits { mark: None });
        }
        DiffMsg::CommitsListed(Err(err)) => {
            tracing::warn!(%pr, ?err, "listing commits failed");
            state.error(format!("Couldn't list commits: {err}"));
        }
        DiffMsg::ViewedLoaded(result) => match *result {
            Ok(viewed) => {
                if let Some(diff) = state.diffs.get_mut(of) {
                    diff.set_viewed_states(viewed);
                }
            }
            Err(err) => {
                tracing::warn!(%pr, %err, "viewed state fetch failed");
                state.error(format!("Couldn't load viewed files: {err}"));
            }
        },
        DiffMsg::ViewedSaved {
            file,
            previous,
            result,
        } => {
            if let Err(err) = result {
                tracing::warn!(%pr, %err, "viewed state update failed");
                if let Some(diff) = state.diffs.get_mut(of) {
                    diff.doc.set_viewed(file, previous);
                }
                state.error(format!("GitHub didn't save “viewed”: {err}"));
            }
        }
        DiffMsg::ReviewLoaded(Ok(saved)) => {
            // Anything done before it arrived is kept, and saved.
            if let Some(diff) = state.diffs.get_mut(of)
                && diff.merge_saved_review(saved)
            {
                return vec![Cmd::SaveReview(pr, diff.review.clone())];
            }
        }
        DiffMsg::ReviewLoaded(Err(err)) => {
            tracing::warn!(%pr, ?err, "reading review state failed");
            state.problems.insert(
                Problem::Drafts,
                format!("Drafts for {pr} are not being saved: {err}"),
            );
        }
        DiffMsg::ThreadsLoaded(result) => {
            match result {
                Ok(threads) => {
                    if let Some(diff) = state.diffs.get_mut(of) {
                        diff.set_threads(threads);
                    }
                }
                Err(err) => {
                    tracing::warn!(%pr, %err, "review threads fetch failed");
                    state.error(format!("Couldn't load review threads: {err}"));
                }
            }
            return state.map_outdated(&pr);
        }
        DiffMsg::PatchesLoaded(Ok(patches)) => {
            if let Some(diff) = state.diffs.get_mut(of) {
                diff.set_patches(patches);
            }
        }
        // The local fallback covers commenting; just note it.
        DiffMsg::PatchesLoaded(Err(err)) => {
            tracing::warn!(%pr, %err, "GitHub patches unavailable; using local hunks");
        }
        DiffMsg::OutdatedMapped(mapped) => {
            if let Some(diff) = state.diffs.get_mut(of) {
                diff.mapped.extend(mapped);
                diff.refresh_annotations();
            }
        }
        DiffMsg::ResolvedSet {
            thread_id,
            resolved,
            result: Err(err),
        } => {
            if let Some(diff) = state.diffs.get_mut(of) {
                if let Some(t) = diff.threads.iter_mut().find(|t| t.id == thread_id) {
                    t.resolved = !resolved;
                }
                diff.refresh_annotations();
            }
            state.error(format!("GitHub didn't save that: {err}"));
        }
        // Jobs are handled above.
        DiffMsg::ResolvedSet { .. } | DiffMsg::Job(..) => {}
        DiffMsg::ReviewSubmitted(outcome) => return on_submitted(state, &pr, &outcome),
    }
    Vec::new()
}
