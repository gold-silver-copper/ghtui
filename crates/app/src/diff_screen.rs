//! The diff screen: file tree plus diff, and its key handling.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use ghtui_api::model::{PatchFile, PrRef, ReviewThread, ViewedFiles, ViewedState};
use ghtui_diff::anchor::Commentable;
use ghtui_diff::{FileDiff, Whitespace};
use ghtui_git::repo::PrRefs;
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

/// Split view turns on automatically from this diff-pane width.
pub const SPLIT_MIN_WIDTH: u16 = 160;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Tree,
    Diff,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffScreen {
    pub pr: PrRef,
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
}

impl DiffScreen {
    pub fn new(pr: PrRef, width: u16) -> Self {
        Self {
            pr,
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
    pub mapped: HashMap<String, Option<u32>>,
    /// Outdated-thread mapping was requested.
    pub mapping_requested: bool,
    /// Move detection was requested (once every file is diffed).
    pub moves_requested: bool,
    /// Head of your last submitted review on GitHub: `None` until asked,
    /// `Some(None)` if there isn't one.
    pub last_review: Option<Option<String>>,
    /// "Since my last review" was asked for and is waiting on data.
    pub since_requested: bool,
    /// The PR's commits, oldest first: `(sha, subject)`.
    pub commits: Vec<(String, String)>,
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
    pub fn head(&self) -> Option<String> {
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
        let states: &HashMap<String, ViewedState> = &viewed.states;
        let updates: Vec<(usize, Viewed)> = self
            .doc
            .files()
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let state = match states.get(f.meta.path()) {
                    Some(ViewedState::Viewed) => Viewed::Viewed,
                    Some(ViewedState::Dismissed) => Viewed::Dismissed,
                    _ => Viewed::Unviewed,
                };
                (i, state)
            })
            .filter(|(i, state)| self.doc.files().get(*i).is_some_and(|f| f.viewed != *state))
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

/// Handles `action` on the diff screen. `None` means it isn't a diff-screen
/// action (the caller handles it).
pub fn apply(
    screen: &mut DiffScreen,
    state: &mut DiffState,
    action: Action,
    content: Rect,
    notice: &mut Option<Notice>,
) -> Option<Vec<Cmd>> {
    let lay = layout(content, screen.tree_visible);
    let half = (usize::from(lay.diff.height) / 2).max(1) as isize;
    let mut cmds = Vec::new();
    match action {
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
        _ if screen.focus == Pane::Tree && lay.tree.is_some() => {
            let tree_half = (tree_list_height(lay.tree.unwrap_or_default()) / 2).max(1) as isize;
            let delta = match action {
                Action::Down => 1,
                Action::Up => -1,
                Action::HalfPageDown => tree_half,
                Action::HalfPageUp => -tree_half,
                Action::Top => isize::MIN / 2,
                Action::Bottom => isize::MAX / 2,
                Action::Open => {
                    screen.focus = Pane::Diff;
                    return Some(Vec::new());
                }
                _ => return None,
            };
            let last = state.tree.len().saturating_sub(1) as isize;
            screen.tree_selected = (screen.tree_selected as isize)
                .saturating_add(delta)
                .clamp(0, last) as usize;
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
        Action::NextHunk => {
            if let Some(pos) = state.doc.next_hunk(screen.cursor) {
                screen.cursor = pos;
            }
        }
        Action::PrevHunk => {
            if let Some(pos) = state.doc.prev_hunk(screen.cursor) {
                screen.cursor = pos;
            }
        }
        Action::NextFile => {
            if let Some(pos) = state.doc.next_file(screen.cursor) {
                jump(screen, pos);
            }
        }
        Action::PrevFile => {
            if let Some(pos) = state.doc.prev_file(screen.cursor) {
                jump(screen, pos);
            }
        }
        Action::NextThread | Action::PrevThread => {
            let found = if action == Action::NextThread {
                state.doc.next_thread(screen.cursor)
            } else {
                state.doc.prev_thread(screen.cursor)
            };
            match found {
                Some(pos) => screen.cursor = pos,
                None => *notice = Some(Notice::Info("No more unresolved threads".into())),
            }
        }
        Action::VisualLines => {
            screen.selection = match screen.selection {
                Some(_) => None,
                None => Some(screen.cursor),
            };
        }
        Action::NextUnviewed => match state.doc.next_unviewed(screen.cursor) {
            Some(pos) => jump(screen, pos),
            None => *notice = Some(Notice::Info("Every file is viewed".into())),
        },
        Action::ExpandContext => {
            let pos = screen.cursor;
            preserving_position(screen, &mut state.doc, |doc| doc.expand(pos));
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
            _ => return Some(Vec::new()),
        },
        Action::ToggleViewed => cmds.extend(toggle_viewed(screen, state, notice)),
        Action::MarkReviewed => cmds.extend(toggle_reviewed(screen, state, notice)),
        Action::SearchNext | Action::SearchPrev => {
            let Some(query) = screen.search.clone() else {
                *notice = Some(Notice::Info("No search yet".into()));
                return Some(Vec::new());
            };
            match state
                .doc
                .search(&query, screen.cursor, action == Action::SearchNext)
            {
                Some(pos) => screen.cursor = pos,
                None => *notice = Some(Notice::Error(format!("No match for “{query}”"))),
            }
        }
        _ => return None,
    }
    Some(cmds)
}

/// Puts `pos` at the top of the view with the cursor on it.
fn jump(screen: &mut DiffScreen, pos: Pos) {
    screen.cursor = pos;
    screen.top = pos;
}

/// Starts a search from the cursor.
pub fn search(screen: &mut DiffScreen, state: &mut DiffState, query: String) -> Notice {
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

fn toggle_viewed(
    screen: &mut DiffScreen,
    state: &mut DiffState,
    notice: &mut Option<Notice>,
) -> Vec<Cmd> {
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
        pr: screen.pr.clone(),
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

fn toggle_reviewed(
    screen: &mut DiffScreen,
    state: &mut DiffState,
    notice: &mut Option<Notice>,
) -> Vec<Cmd> {
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
    vec![Cmd::SaveReview(screen.pr.clone(), state.review.clone())]
}

/// Applies view options, clamps positions, keeps the cursor visible (clear
/// of the sticky header), syncs the tree with the diff, and asks the job to
/// prioritize files that are on screen but not diffed yet.
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
    vec![Cmd::Git(Git::Prioritize(screen.pr.clone(), wanted))]
}

/// A message from the PR's current diff job.
pub(crate) fn on_job(state: &mut State, pr: &PrRef, msg: JobMsg) -> Vec<Cmd> {
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
                    cmds.push(Cmd::Git(Git::DetectMoves(pr.clone(), diff.job, inputs)));
                }
            }
            cmds
        }
        JobMsg::Moves(moves) => {
            match state.diff_parts() {
                Some((screen, diff)) if screen.pr == *pr => {
                    preserving_position(screen, &mut diff.doc, |doc| doc.set_moves(moves));
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

/// A message for a PR's diff screen.
pub(crate) fn update(state: &mut State, pr: PrRef, msg: DiffMsg) -> Vec<Cmd> {
    match msg {
        DiffMsg::Replied(Err(err)) => {
            if let Some(Overlay::Compose(compose)) = &mut state.overlay {
                compose.sending = false;
                compose.error = Some(err.to_string());
            }
            Vec::new()
        }
        DiffMsg::Replied(Ok(())) => {
            state.overlay = None;
            state.info("Reply posted");
            vec![Cmd::Api(Api::FetchThreads(pr))]
        }
        // The text stays in the composer, with GitHub's reason.
        DiffMsg::Job(job, msg) => {
            if state.diffs.get(&pr).is_some_and(|d| d.job == job) {
                on_job(state, &pr, msg)
            } else {
                Vec::new()
            }
        }
        DiffMsg::LastReview(result) => {
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
        DiffMsg::SinceReady(old_head, result) => {
            match result {
                Ok(hashes) => {
                    if let Some((screen, diff)) = state.diff_parts()
                        && screen.pr == pr
                    {
                        diff.since_requested = false;
                        preserving_position(screen, &mut diff.doc, |doc| {
                            doc.set_since(Some(hashes), true)
                        });
                        state.info(format!(
                            "Showing changes since your review of {}",
                            short_sha(&old_head)
                        ));
                    }
                }
                Err(err) => {
                    if let Some(diff) = state.diffs.get_mut(&pr) {
                        diff.since_requested = false;
                    }
                    state.error(format!("Couldn't compare with your last review: {err}"));
                }
            }
            Vec::new()
        }
        DiffMsg::CommitsListed(result) => match result {
            Ok(commits) => {
                if let Some(diff) = state.diffs.get_mut(&pr) {
                    diff.commits = commits;
                }
                state.notice = None;
                state.open_picker(picker::Kind::Commits { mark: None })
            }
            Err(err) => {
                state.error(format!("Couldn't list commits: {err}"));
                Vec::new()
            }
        },
        DiffMsg::ViewedLoaded(result) => {
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
        DiffMsg::ViewedSaved {
            file,
            previous,
            result,
        } => {
            if let Err(err) = result {
                tracing::warn!(%pr, %err, "viewed state update failed");
                if let Some(diff) = state.diffs.get_mut(&pr) {
                    diff.doc.set_viewed(file, previous);
                }
                state.error(format!("GitHub didn't save “viewed”: {err}"));
            }
            Vec::new()
        }
        DiffMsg::ReviewLoaded(Ok(saved)) => {
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
        DiffMsg::ReviewLoaded(Err(err)) => {
            tracing::warn!(%pr, %err, "reading review state failed");
            state.problems.insert(
                Problem::Drafts,
                format!("Drafts for {pr} are not being saved: {err}"),
            );
            Vec::new()
        }
        DiffMsg::ThreadsLoaded(result) => {
            match result {
                Ok(threads) => {
                    if let Some(diff) = state.diffs.get_mut(&pr) {
                        diff.set_threads(threads);
                    }
                }
                Err(err) => {
                    tracing::warn!(%pr, %err, "review threads fetch failed");
                    state.error(format!("Couldn't load review threads: {err}"));
                }
            }
            state.map_outdated(&pr)
        }
        DiffMsg::PatchesLoaded(result) => {
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
        DiffMsg::OutdatedMapped(mapped) => {
            if let Some(diff) = state.diffs.get_mut(&pr) {
                diff.mapped.extend(mapped);
                diff.refresh_annotations();
            }
            Vec::new()
        }
        DiffMsg::ResolvedSet {
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
                state.error(format!("GitHub didn't save that: {err}"));
            }
            Vec::new()
        }
        DiffMsg::ReviewSubmitted(outcome) => on_submitted(state, &pr, &outcome),
    }
}
