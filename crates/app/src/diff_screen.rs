//! The diff screen: file tree plus diff, and its key handling.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use ghtui_api::model::{NodeId, PatchFile, PrRef, RepoId, ReviewThread, ViewedFiles};
use ghtui_diff::anchor::Commentable;
use ghtui_diff::{FileDiff, Whitespace};
use ghtui_git::Oid;
use ghtui_git::repo::{Commit, PrRefs};
use ghtui_store::ReviewState;
use ghtui_ui::bars::Notice;
use ghtui_ui::diff_doc::{Doc, DocInputs, Note, Pos, Row, ViewOptions, Viewed};
use ghtui_ui::file_tree::{TreeRow, row_of_file, tree_rows};
use ghtui_ui::text::short_sha;
use ratatui::layout::Rect;

use crate::diff_job::{DiffFiles, JobId, JobMsg, next_job};
use crate::join;
use crate::keymap::Action;
use crate::review::{annotations, on_submitted};
use crate::state::{Api, Cmd, DiffMsg, Git, Overlay, Problem, Screen, State};

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
    /// GitHub couldn't say, and why: asked again next time.
    Failed(String),
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

/// How a diff screen is asked to show diffs, whichever it shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiffPrefs {
    pub tree_visible: bool,
    /// `None` picks split or unified by width.
    pub split_override: Option<bool>,
    pub ignore_whitespace: bool,
}

impl DiffPrefs {
    /// What a new diff screen `width` wide starts with.
    pub fn fit(width: u16) -> Self {
        Self {
            tree_visible: width >= 100,
            split_override: None,
            ignore_whitespace: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffScreen {
    pub of: DiffOf,
    pub cursor: Pos,
    pub top: Pos,
    pub prefs: DiffPrefs,
    pub focus: Pane,
    pub tree_selected: usize,
    pub tree_scroll: usize,
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
    pub fn new(of: DiffOf, prefs: DiffPrefs) -> Self {
        Self {
            of,
            cursor: Pos::default(),
            top: Pos::default(),
            prefs,
            focus: Pane::Diff,
            tree_selected: 0,
            tree_scroll: 0,
            search: None,
            selection: None,
            anchor: None,
        }
    }

    /// Starts over at the top, as for a new range, keeping the view prefs.
    pub fn restart(&mut self) {
        *self = Self::new(self.of.clone(), self.prefs);
    }

    /// The view options this screen wants at `content` size.
    pub fn options(&self, content: Rect) -> ViewOptions {
        let lay = layout(content, self.prefs.tree_visible);
        ViewOptions {
            split: self
                .prefs
                .split_override
                .unwrap_or(lay.diff.width >= SPLIT_MIN_WIDTH),
            whitespace: if self.prefs.ignore_whitespace {
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

/// What GitHub and the saved review say of a PR's diff; it outlasts rebuilds.
#[derive(Debug, Default)]
pub struct DiffInputs {
    /// GitHub's viewed state and the PR's node ID.
    pub viewed: Option<ViewedFiles>,
    /// Locally persisted review marks and draft comments.
    pub review: ReviewState,
    /// The saved review state has been read (saving before that would
    /// overwrite it).
    pub review_loaded: bool,
    /// Review threads from GitHub.
    pub threads: Vec<ReviewThread>,
    /// Commentable ranges from GitHub's patches of the whole PR, by path.
    pub patches: Arc<HashMap<String, Commentable>>,
    /// Your last submitted review on GitHub.
    pub last_review: LastReview,
    /// The PR's commits, oldest first.
    pub commits: Vec<Commit>,
    /// The draft deleted last, for undo.
    pub deleted: Option<ghtui_store::DraftComment>,
}

impl DiffInputs {
    /// Notes `path`'s viewed state, once GitHub's are in.
    pub fn set_viewed(&mut self, path: String, viewed: Viewed) {
        if let Some(files) = &mut self.viewed {
            files.states.insert(path, viewed);
        }
    }

    /// Marks thread `id` resolved or not.
    pub fn set_resolved(&mut self, id: &NodeId, resolved: bool) {
        if let Some(t) = self.threads.iter_mut().find(|t| t.id == *id) {
            t.resolved = resolved;
        }
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
        changed
    }
}

/// Everything known about one PR's diff: its inputs, and what a job made.
#[derive(Debug)]
pub struct DiffState {
    /// Changed only through [`DiffState::edit`], which shows the change.
    inputs: DiffInputs,
    pub doc: Doc,
    pub tree: Vec<TreeRow>,
    pub refs: Option<PrRefs>,
    /// What the background job is doing, while it works.
    pub progress: Option<String>,
    pub error: Option<String>,
    /// Outdated threads mapped onto this job's head (`None`: can't be).
    /// Changed only through [`DiffState::add_mapped`], which shows it.
    mapped: HashMap<NodeId, Option<u32>>,
    /// What the work that waits on several inputs last ran for.
    pub(crate) joins: join::Joins,
    /// "Since my last review" was asked for and is waiting on data.
    pub since_requested: bool,
    /// Showing a sub-range of commits instead of the whole PR.
    range: Option<RangeView>,
    /// Files we've already asked the job to prioritize.
    requested: HashSet<usize>,
    /// The diff job whose results this shows; older jobs' are ignored.
    pub job: JobId,
}

impl DiffState {
    /// A new diff of the whole PR that knows nothing yet: all is fetched again.
    pub fn fresh() -> Self {
        Self {
            inputs: DiffInputs::default(),
            doc: Doc::default(),
            tree: Vec::new(),
            refs: None,
            progress: Some("Preparing".into()),
            error: None,
            mapped: HashMap::new(),
            joins: join::Joins::default(),
            since_requested: false,
            range: None,
            requested: HashSet::new(),
            job: next_job(),
        }
    }

    /// Starts over for a different commit range, with the same inputs.
    pub fn restart(&mut self, range: Option<RangeView>) {
        *self = Self {
            inputs: std::mem::take(&mut self.inputs),
            range,
            ..Self::fresh()
        };
    }

    /// The commit range shown, if not the whole PR.
    pub fn range(&self) -> Option<&RangeView> {
        self.range.as_ref()
    }

    pub fn inputs(&self) -> &DiffInputs {
        &self.inputs
    }

    /// What the doc shows of the inputs and this job's mapping; GitHub's
    /// patches only on the whole PR's diff, whose lines they number.
    fn doc_inputs(&self) -> DocInputs {
        let i = &self.inputs;
        DocInputs {
            patches: if self.range.is_none() {
                Arc::clone(&i.patches)
            } else {
                Arc::default()
            },
            viewed: i.viewed.iter().flat_map(|v| v.states.clone()).collect(),
            reviewed: i.review.reviewed_hunks.iter().cloned().collect(),
            annotations: annotations(&i.threads, &self.mapped, &i.review.pending),
        }
    }

    /// Changes the inputs with `f` and shows the change.
    pub fn edit<R>(&mut self, f: impl FnOnce(&mut DiffInputs) -> R) -> R {
        let r = f(&mut self.inputs);
        self.doc.set_inputs(self.doc_inputs());
        r
    }

    /// Places outdated threads as this job's mapping says, and shows them.
    pub fn add_mapped(&mut self, mapped: impl IntoIterator<Item = (NodeId, Option<u32>)>) {
        self.mapped.extend(mapped);
        self.doc.set_inputs(self.doc_inputs());
    }

    /// Changes the review with `f`, shows the change, and saves it once
    /// the saved one has been read (saving before would overwrite it).
    #[must_use]
    pub fn edit_review(&mut self, pr: &PrRef, f: impl FnOnce(&mut DiffInputs)) -> Vec<Cmd> {
        self.edit(f);
        let i = &self.inputs;
        let save = || Cmd::SaveReview(pr.clone(), i.review.clone());
        i.review_loaded.then(save).into_iter().collect()
    }

    /// The file list has arrived.
    pub fn listed(&self) -> bool {
        self.refs.is_some()
    }

    /// The head commit the diff was computed at.
    pub fn head(&self) -> Option<Oid> {
        self.refs.as_ref().map(|r| r.head.clone())
    }

    /// Lists the job's files. The doc shows the inputs from the start, so
    /// whatever arrived before the files (GitHub's patches, say) applies.
    pub fn set_files(&mut self, files: DiffFiles) {
        self.doc = Doc::new(files.files, &files.generated, self.doc_inputs());
        self.tree = tree_rows(&self.doc);
        self.refs = Some(files.refs);
        self.progress = (!self.doc.is_empty()).then(|| "Computing diffs".into());
    }

    pub fn set_file(&mut self, index: usize, diff: Arc<FileDiff>) {
        self.doc.set_diff(index, diff);
        if self.doc.ready_count() == self.doc.files().len() {
            self.progress = None;
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
    let lay = layout(content, screen.prefs.tree_visible);
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
            screen.prefs.tree_visible = !screen.prefs.tree_visible;
            if !screen.prefs.tree_visible {
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
            screen.prefs.split_override = Some(!screen.options(content).split);
        }
        Action::IgnoreWhitespace => {
            screen.prefs.ignore_whitespace = !screen.prefs.ignore_whitespace;
            *notice = Some(Notice::Info(
                if screen.prefs.ignore_whitespace {
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
    let Some(viewed) = &state.inputs().viewed else {
        *notice = Some(Notice::Error(
            "Viewed state hasn't loaded from GitHub yet".into(),
        ));
        return Vec::new();
    };
    let index = screen.cursor.file;
    let Some(file) = state.doc.files().get(index) else {
        return Vec::new();
    };
    let (previous, path) = (file.viewed, file.meta.path().to_owned());
    let now = if previous == Viewed::Viewed {
        Viewed::Unviewed
    } else {
        Viewed::Viewed
    };
    let cmd = Cmd::Api(Api::SetViewed {
        pr,
        pull_request_id: viewed.pull_request_id.clone(),
        path: path.clone(),
        viewed: now == Viewed::Viewed,
        previous,
    });
    // Optimistic: show it now, roll back if GitHub refuses.
    state.edit(|i| i.set_viewed(path, now));
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
    state.edit_review(&pr, |i| {
        let hunks = &mut i.review.reviewed_hunks;
        if let Some(i) = hunks.iter().position(|h| *h == hash) {
            hunks.remove(i);
        } else {
            hunks.push(hash);
            hunks.sort();
        }
    })
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
fn anchor_pos(anchor: &str, state: &DiffState, pr: bool) -> Option<Option<Pos>> {
    let doc = &state.doc;
    if let Some(hash) = anchor.strip_prefix("diff-") {
        let hash = hash.get(..64).unwrap_or(hash);
        let file = doc
            .files()
            .iter()
            .position(|f| path_hash(f.meta.path()) == hash);
        return Some(file.map(|file| Pos { file, row: 0 }));
    }
    let Some(id) = anchor
        .strip_prefix("discussion_r")
        .or_else(|| anchor.strip_prefix('r'))
        .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
    else {
        // Not something ghtui finds in a diff (`#files`).
        return Some(None);
    };
    // Only pull requests have review threads.
    if !pr {
        return Some(None);
    }
    let threads = &state.inputs().threads;
    if threads.is_empty() {
        return None;
    }
    let comment = format!("#discussion_r{id}");
    // Annotations list the threads first, in order.
    let Some((index, thread)) = threads
        .iter()
        .enumerate()
        .find(|(_, t)| t.comments.iter().any(|c| c.url.ends_with(&comment)))
    else {
        return Some(None);
    };
    if let Some(pos) = u32::try_from(index)
        .ok()
        .and_then(|i| doc.annotation_pos(i))
    {
        return Some(Some(pos));
    }
    // Its file shows no thread rows yet: wait while it's being diffed;
    // once it is (collapsed, say), go to the file.
    let Some(file) = doc
        .files()
        .iter()
        .position(|f| f.meta.path() == thread.path)
    else {
        return Some(None);
    };
    let diffed = doc.files().get(file).is_some_and(|f| f.diff.is_some());
    diffed.then_some(Some(Pos { file, row: 0 }))
}

/// Applies view options, clamps positions, keeps the cursor visible (clear
/// of the sticky header), syncs the tree with the diff, and asks the job to
/// prioritize files that are on screen but not diffed yet.
#[must_use]
pub fn settle(screen: &mut DiffScreen, state: &mut DiffState, content: Rect) -> Vec<Cmd> {
    let lay = layout(content, screen.prefs.tree_visible);
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
        match anchor_pos(&anchor, state, screen.of.pr().is_some()) {
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

/// A message from `diff`'s current job; `screen` is its screen, only
/// while it's on top.
fn on_job(diff: &mut DiffState, screen: Option<&mut DiffScreen>, msg: JobMsg) {
    match msg {
        JobMsg::Progress(line) if !diff.listed() => diff.progress = Some(line),
        JobMsg::Progress(_) => {}
        JobMsg::Files(files) => diff.set_files(*files),
        JobMsg::File(index, file) => diff.set_file(index, file),
        JobMsg::Moves(moves) => match screen {
            Some(screen) => preserving_position(screen, &mut diff.doc, |d| d.set_moves(moves)),
            None => diff.doc.set_moves(moves),
        },
        JobMsg::Mapped(mapped) => diff.add_mapped(mapped),
        JobMsg::Failed(error) => {
            diff.progress = None;
            diff.error = Some(error.to_string());
        }
    }
}

/// A message for a diff screen. The composer's and the Submit dialog's
/// replies answer that dialog; any other reaches only `of`'s diff, its
/// screen while on top, and the status bar.
#[must_use]
pub(crate) fn update(state: &mut State, of: &DiffOf, msg: DiffMsg) -> Vec<Cmd> {
    match (msg, of) {
        // The text stays in the composer, with GitHub's reason.
        (DiffMsg::Replied(Err(err)), _) => {
            if let Some(Overlay::Compose(compose)) = &mut state.overlay {
                compose.sending = false;
                compose.error = Some(err.to_string());
            }
            Vec::new()
        }
        (DiffMsg::Replied(Ok(())), DiffOf::Pr(pr)) => {
            state.overlay = None;
            state.info("Reply posted");
            vec![Cmd::Api(Api::FetchThreads(pr.clone()))]
        }
        (DiffMsg::ReviewSubmitted(outcome), DiffOf::Pr(pr)) => on_submitted(state, pr, &outcome),
        (msg, of) => {
            let State {
                screens,
                diffs,
                notice,
                problems,
                ..
            } = state;
            let screen = match screens.last_mut() {
                Screen::Diff(screen) if screen.of == *of => Some(&mut **screen),
                _ => None,
            };
            on_reply(of, diffs.get_mut(of), screen, notice, problems, msg)
        }
    }
}

/// A reply about `of`: it changes `diff` (if it's still open), `screen`
/// (only while `of` is on top), the status bar and the problems banner,
/// and nothing else.
fn on_reply(
    of: &DiffOf,
    diff: Option<&mut DiffState>,
    screen: Option<&mut DiffScreen>,
    notice: &mut Option<Notice>,
    problems: &mut BTreeMap<Problem, String>,
    msg: DiffMsg,
) -> Vec<Cmd> {
    let mut error = |text: String| *notice = Some(Notice::Error(text));
    match (msg, diff) {
        (DiffMsg::Job(job, msg), Some(diff)) if diff.job == job => on_job(diff, screen, msg),
        (DiffMsg::LastReview(result), Some(diff)) => {
            let last = match result {
                Ok(None) => LastReview::None,
                Ok(Some(commit)) => LastReview::At(Oid::new(commit)),
                Err(err) => LastReview::Failed(err.to_string()),
            };
            diff.edit(|i| i.last_review = last);
        }
        // Compared with the whole PR, so kept off a range; turned on only on screen.
        (DiffMsg::SinceReady(old_head, Ok(hashes)), Some(diff)) if diff.range().is_none() => {
            let Some(screen) = screen else {
                diff.doc.set_since(Some(hashes), false);
                return Vec::new();
            };
            preserving_position(screen, &mut diff.doc, |doc| {
                doc.set_since(Some(hashes), true);
            });
            let said = format!(
                "Showing changes since your review of {}",
                short_sha(&old_head)
            );
            *notice = Some(Notice::Info(said));
        }
        (DiffMsg::SinceReady(_, Err(err)), _) => {
            tracing::warn!(%of, ?err, "comparing with the last review failed");
            error(format!("Couldn't compare with your last review: {err}"));
        }
        (DiffMsg::CommitsListed(Ok(commits)), Some(diff)) => diff.edit(|i| i.commits = commits),
        (DiffMsg::CommitsListed(Err(err)), _) => {
            tracing::warn!(%of, ?err, "listing commits failed");
            error(format!("Couldn't list commits: {err}"));
        }
        (DiffMsg::ViewedLoaded(result), diff) => match (*result, diff) {
            (Ok(viewed), Some(diff)) => diff.edit(|i| i.viewed = Some(viewed)),
            (Ok(_), None) => {}
            (Err(err), _) => {
                tracing::warn!(%of, %err, "viewed state fetch failed");
                error(format!("Couldn't load viewed files: {err}"));
            }
        },
        (
            DiffMsg::ViewedSaved {
                path,
                previous,
                result: Err(err),
            },
            diff,
        ) => {
            tracing::warn!(%of, %err, "viewed state update failed");
            if let Some(diff) = diff {
                diff.edit(|i| i.set_viewed(path, previous));
            }
            error(format!("GitHub didn't save “viewed”: {err}"));
        }
        // Anything done before it arrived is kept, and saved.
        (DiffMsg::ReviewLoaded(Ok(saved)), Some(diff)) => {
            let DiffOf::Pr(pr) = of else {
                return Vec::new();
            };
            let mut changed = false;
            let save = diff.edit_review(pr, |i| changed = i.merge_saved_review(saved));
            return if changed { save } else { Vec::new() };
        }
        (DiffMsg::ReviewLoaded(Err(err)), _) => {
            tracing::warn!(%of, ?err, "reading review state failed");
            problems.insert(
                Problem::Drafts,
                format!("Drafts for {of} are not being saved: {err}"),
            );
        }
        (DiffMsg::ThreadsLoaded(Ok(threads)), Some(diff)) => diff.edit(|i| i.threads = threads),
        (DiffMsg::ThreadsLoaded(Err(err)), _) => {
            tracing::warn!(%of, %err, "review threads fetch failed");
            error(format!("Couldn't load review threads: {err}"));
        }
        (DiffMsg::PatchesLoaded(Ok(patches)), Some(diff)) => {
            let commentable = |f: PatchFile| Some((f.filename, Commentable::from_patch(&f.patch?)));
            let patches = Arc::new(patches.into_iter().filter_map(commentable).collect());
            diff.edit(|i| i.patches = patches);
        }
        // The local fallback covers commenting; just note it.
        (DiffMsg::PatchesLoaded(Err(err)), _) => {
            tracing::warn!(%of, %err, "GitHub patches unavailable; using local hunks");
        }
        (
            DiffMsg::ResolvedSet {
                thread_id,
                resolved,
                result: Err(err),
            },
            diff,
        ) => {
            if let Some(diff) = diff {
                diff.edit(|i| i.set_resolved(&thread_id, !resolved));
            }
            error(format!("GitHub didn't save that: {err}"));
        }
        // The dialogs' replies are answered in `update`; an old job's, and
        // what came for a diff that has closed, are dropped.
        (
            DiffMsg::ResolvedSet { .. }
            | DiffMsg::ViewedSaved { .. }
            | DiffMsg::SinceReady(..)
            | DiffMsg::Job(..)
            | DiffMsg::Replied(_)
            | DiffMsg::ReviewSubmitted(_),
            _,
        )
        | (_, None) => {}
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    impl DiffState {
        /// Where this job's outdated threads are.
        pub(crate) fn mapped(&self) -> &HashMap<NodeId, Option<u32>> {
            &self.mapped
        }

        /// Lists `doc`'s files at `refs` and diffs them as it has, as if the
        /// job had sent them.
        pub(crate) fn with_doc(mut self, refs: PrRefs, doc: &Doc) -> Self {
            let files = doc.files().iter().map(|f| f.meta.clone()).collect();
            let generated = HashSet::new();
            self.set_files(DiffFiles {
                refs,
                files,
                generated,
            });
            for (i, f) in doc.files().iter().enumerate() {
                if let Some(diff) = &f.diff {
                    self.doc.set_diff(i, Arc::clone(diff));
                }
            }
            self.doc.set_moves(doc.moves().to_vec());
            self
        }
    }
}
