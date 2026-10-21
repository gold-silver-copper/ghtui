//! The diff screen: file tree plus diff, and its key handling.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::api::model::{NodeId, PatchFile, PrRef, RepoId, ReviewThread, ViewedFiles};
use crate::diff::anchor::{Commentable, LinePos, Side};
use crate::diff::{FileDiff, Whitespace};
use crate::git::Oid;
use crate::git::repo::{Commit, PrRefs};
use crate::store::ReviewState;
use crate::ui::annotations::Annotation;
use crate::ui::bars::Notice;
use crate::ui::diff_doc::{Doc, DocFile, DocInputs, Note, Pos, Row, ViewOptions, Viewed};
use crate::ui::diff_view::code_room;
use crate::ui::file_tree::{self, TreeRow, row_of_file, tree_rows, unfolded};
use crate::ui::pages::url as links;
use crate::ui::text::{self, short_sha};
use ratatui::layout::Rect;

use crate::diff_job::{DiffFiles, JobId, JobMsg, next_job};
use crate::join;
use crate::keymap::Action;
use crate::notice::Notices;
use crate::picker;
use crate::review::annotations;
use crate::state::{Cmd, Cmds, DiffMsg, Git, Overlay, Problem, Remote, State};

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
    /// A commit against its first parent.
    Commit(RepoId, Oid),
    /// From one commit to another, as `a..b`: a comparison's
    /// files, from the merge base its page found.
    Range(RepoId, Oid, Oid),
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

    /// Its files on GitHub: the one place a diff's link is written, and
    /// one that parses back to it.
    pub fn url(&self) -> String {
        match self {
            DiffOf::Pr(_) => self.page(),
            DiffOf::Commit(..) | DiffOf::Range(..) => links::files(&self.page()),
        }
    }

    /// Its file `path` on GitHub, at `lines` (the first and the last) if
    /// given, as GitHub anchors them (`#diff-<hash>R10-R12`): what
    /// [`DiffScreen::anchor`] goes back to.
    pub fn file_url(&self, path: &str, lines: Option<(LinePos, LinePos)>) -> String {
        let line = |l: LinePos| match l.side {
            Side::Left => format!("L{}", l.line),
            Side::Right => format!("R{}", l.line),
        };
        let lines = match lines {
            Some((start, end)) if start == end => line(start),
            Some((start, end)) => format!("{}-{}", line(start), line(end)),
            None => String::new(),
        };
        links::diff_file(&self.page(), path, &lines)
    }

    /// The page its files are on.
    fn page(&self) -> String {
        match self {
            DiffOf::Pr(pr) => format!("{}/files", pr.url()),
            DiffOf::Commit(repo, oid) => links::commit(repo, oid),
            DiffOf::Range(repo, from, to) => links::compare(repo, from, to),
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

/// Columns `h`/`l` scroll code sideways, and the most it scrolls.
const SHIFT: usize = 8;
const MAX_SHIFT: usize = 400;

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
    /// The file tree's width as set with keys; `None` fits its names.
    pub tree_width: Option<u16>,
    /// `None` picks split or unified by width.
    pub split_override: Option<bool>,
    pub ignore_whitespace: bool,
    /// Compare by syntax, where a file's language allows, not by lines.
    pub by_syntax: bool,
    /// Long code lines go on in rows under them, rather than off the edge.
    pub wrap: bool,
}

impl DiffPrefs {
    /// How diffs are compared, for the status bar.
    pub fn label(&self) -> String {
        let by = if self.by_syntax {
            "By syntax"
        } else {
            "By lines"
        };
        if self.ignore_whitespace {
            format!("{by}, ignoring whitespace")
        } else {
            by.to_owned()
        }
    }

    /// What a new diff screen `width` wide starts with.
    pub fn fit(width: u16) -> Self {
        Self {
            tree_visible: width >= 100,
            tree_width: None,
            split_override: None,
            ignore_whitespace: false,
            by_syntax: true,
            wrap: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffScreen {
    pub of: DiffOf,
    pub cursor: Pos,
    /// The half of split rows the cursor is on (the other when that one
    /// is empty); it follows the line held across view changes.
    pub half: Side,
    pub top: Pos,
    pub prefs: DiffPrefs,
    pub focus: Pane,
    pub tree_selected: usize,
    pub tree_scroll: usize,
    /// The last search query, for `n` / `N`.
    pub search: Option<String>,
    /// Where a visual line selection started.
    pub selection: Option<Pos>,
    /// The file tree's folded folders, by path.
    pub folded: HashSet<String>,
    /// Columns code is scrolled left by.
    pub shift: usize,
    /// The word cursor, by its column in the code of the cursor's line:
    /// `None` until `w` or `b` picks one.
    pub word: Option<usize>,
    /// What a link pointed at, to go to once it's loaded: a file
    /// (`diff-<hash>`, GitHub's SHA-256 of its path), maybe at lines
    /// (`diff-<hash>L3-R5`), or a review comment (`discussion_r<id>`,
    /// `r<id>`).
    pub anchor: Option<String>,
    /// The cursor jumped (to a search's match, a link's thread): the view
    /// is to show it with the threads under it.
    pub reveal: bool,
}

impl DiffScreen {
    pub fn new(of: DiffOf, prefs: DiffPrefs) -> Self {
        Self {
            of,
            cursor: Pos::default(),
            half: Side::Right,
            top: Pos::default(),
            prefs,
            focus: Pane::Diff,
            tree_selected: 0,
            tree_scroll: 0,
            search: None,
            selection: None,
            folded: HashSet::new(),
            shift: 0,
            word: None,
            anchor: None,
            reveal: false,
        }
    }

    /// Starts over at the top, as for a new range, keeping the view prefs.
    pub fn restart(&mut self) {
        *self = Self::new(self.of.clone(), self.prefs);
    }

    /// The view options this screen wants in a diff pane `width` wide.
    pub fn options(&self, Rect { width, .. }: Rect) -> ViewOptions {
        ViewOptions {
            split: self
                .prefs
                .split_override
                .unwrap_or(width >= SPLIT_MIN_WIDTH),
            whitespace: if self.prefs.ignore_whitespace {
                Whitespace::Ignore
            } else {
                Whitespace::Exact
            },
            structural: self.prefs.by_syntax,
            // Thread cards start after the gutter; keep comments readable.
            wrap: width.saturating_sub(24).clamp(20, 100),
            wrap_code: self.prefs.wrap.then_some(width),
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
    pub threads: Remote<Vec<ReviewThread>>,
    /// Commentable ranges from GitHub's patches of the whole PR, by path.
    pub patches: Arc<HashMap<String, Commentable>>,
    /// Your last submitted review on GitHub.
    pub last_review: LastReview,
    /// The PR's commits, oldest first.
    pub commits: Vec<Commit>,
    /// The draft deleted last, for undo.
    pub deleted: Option<crate::store::DraftComment>,
    /// Threads and comments that came in since you started looking.
    pub fresh: HashSet<NodeId>,
    /// What the head's check runs say about its lines, by check.
    pub checks: Vec<(String, crate::api::workflows::Annotation)>,
}

impl DiffInputs {
    /// Notes `path`'s viewed state, once GitHub's are in.
    pub fn set_viewed(&mut self, path: String, viewed: Viewed) {
        if let Some(files) = &mut self.viewed {
            files.states.insert(path, viewed);
        }
    }

    /// Marks thread `id` resolved or not. GitHub says only whether you may
    /// do what applies now; who may resolve a thread may unresolve it.
    pub fn set_resolved(&mut self, id: &NodeId, resolved: bool) {
        let mut threads = self.threads.data.iter_mut().flatten();
        if let Some(t) = threads.find(|t| t.id == *id) {
            let may = t.can_resolve || t.can_unresolve;
            t.resolved = resolved;
            (t.can_resolve, t.can_unresolve) = (may, may);
        }
    }

    /// The review threads, or none while they load.
    pub fn thread_list(&self) -> &[ReviewThread] {
        self.threads.data.as_deref().unwrap_or_default()
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
    /// What "since" compares with, when it isn't your last review: a
    /// review's commit, opened from the review.
    pub since_commit: Option<String>,
    /// Showing a sub-range of commits instead of the whole PR.
    range: Option<RangeView>,
    /// Files we've already asked the job to prioritize.
    requested: HashSet<usize>,
    /// The diff job whose results this shows; older jobs' are ignored.
    pub job: JobId,
    /// Which state of the inputs this is: new with each change, and never
    /// another diff's, so what's built from them knows when it's old.
    version: u64,
}

/// A version no diff's inputs have had.
fn next_version() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
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
            since_commit: None,
            range: None,
            requested: HashSet::new(),
            job: next_job(),
            version: next_version(),
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

    /// Which state of the inputs this is.
    pub fn version(&self) -> u64 {
        self.version
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
            annotations: (annotations(i.thread_list(), &self.mapped, &i.review.pending, &i.fresh))
                .into_iter()
                .chain(crate::workflows::in_diff(&i.checks))
                .collect(),
        }
    }

    /// Changes the inputs with `f` and shows the change.
    pub fn edit<R>(&mut self, f: impl FnOnce(&mut DiffInputs) -> R) -> R {
        let r = f(&mut self.inputs);
        self.version = next_version();
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
    pub fn edit_review(&mut self, pr: &PrRef, f: impl FnOnce(&mut DiffInputs)) -> Cmds {
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
            let files = self.doc.files().len();
            Some(format!(
                "Diffing {}/{files} {}",
                self.doc.ready_count(),
                crate::ui::text::noun(files as u64, "file")
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

/// Splits the content area between `tree` and the diff; shared by the
/// view and key handling so both agree on viewport sizes.
pub fn layout(content: Rect, prefs: &DiffPrefs, tree: &[TreeRow]) -> Layout {
    if !prefs.tree_visible || content.width < 60 {
        return Layout {
            tree: None,
            diff: content,
        };
    }
    // Its names fit up to a third of the width; keys widen it to half.
    let width = match prefs.tree_width {
        Some(w) => w.clamp(MIN_TREE, content.width / 2),
        None => file_tree::fit_width(tree).clamp(FIT_TREE, (content.width / 3).max(FIT_TREE)),
    };
    Layout {
        tree: Some(Rect { width, ..content }),
        diff: Rect {
            x: content.x + width,
            width: content.width - width,
            ..content
        },
    }
}

/// The narrowest the file tree gets, the narrowest it starts, and what a
/// key widens or narrows it by.
const MIN_TREE: u16 = 16;
const FIT_TREE: u16 = 24;
const TREE_STEP: u16 = 4;

/// Rows of the tree list below its title.
fn tree_list_height(tree: Rect) -> usize {
    usize::from(tree.height.saturating_sub(3))
}

/// Runs `change` on the diff, keeping the cursor, the top of the view and
/// the selection on the same lines and threads.
pub fn preserving_position<R>(
    screen: &mut DiffScreen,
    diff: &mut DiffState,
    change: impl FnOnce(&mut DiffState) -> R,
) -> R {
    let doc = &diff.doc;
    let [cursor, top] = [screen.cursor, screen.top].map(|p| doc.anchor(p, screen.half));
    let selection = screen.selection.map(|p| doc.anchor(p, screen.half));
    let r = change(diff);
    let doc = &diff.doc;
    screen.half = cursor.side().unwrap_or(screen.half);
    screen.cursor = doc.locate(cursor);
    screen.top = doc.locate(top);
    screen.selection = selection.map(|a| doc.locate(a));
    r
}

/// Handles `action` on the diff screen. Those that don't work here say
/// where they do.
pub fn apply(
    screen: &mut DiffScreen,
    state: &mut DiffState,
    action: Action,
    content: Rect,
    notice: &mut Notices,
) -> Cmds {
    let lay = layout(content, &screen.prefs, &state.tree);
    let page = usize::from(lay.diff.height)
        .saturating_sub(2)
        .max(1)
        .cast_signed();
    let tree_focused = screen.focus == Pane::Tree && lay.tree.is_some();
    let before = (screen.top, screen.cursor, screen.shift);
    let scrolls = matches!(action, Action::ScrollLeft | Action::ScrollRight);
    let mut cmds = Cmds::new();
    match action {
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
            screen.prefs.split_override = Some(!screen.options(lay.diff).split);
        }
        Action::IgnoreWhitespace => flip(
            &mut screen.prefs.ignore_whitespace,
            notice,
            ["Ignoring whitespace changes", "Showing whitespace changes"],
        ),
        Action::WrapLines => {
            screen.shift = 0;
            let said = ["Long lines wrap", "Long lines are cut at the edge"];
            flip(&mut screen.prefs.wrap, notice, said);
        }
        Action::ScrollLeft | Action::ScrollRight if screen.prefs.wrap => {
            notice.info("Long lines wrap: there's nothing to the side");
            return Cmds::new();
        }
        Action::BySyntax => flip(
            &mut screen.prefs.by_syntax,
            notice,
            [
                "Comparing by syntax: only the code that changed stands out",
                "Comparing by lines",
            ],
        ),
        Action::TreeNarrower | Action::TreeWider => match lay.tree {
            Some(tree) => {
                let wider = action == Action::TreeWider;
                let width = if wider {
                    tree.width.saturating_add(TREE_STEP)
                } else {
                    tree.width.saturating_sub(TREE_STEP)
                };
                screen.prefs.tree_width = Some(width);
                // What it came to, within its limits.
                let now = layout(content, &screen.prefs, &state.tree).tree;
                let now = now.map_or(width, |t| t.width);
                screen.prefs.tree_width = Some(now);
                if now == tree.width {
                    notice.info(if wider {
                        "The file tree is as wide as it gets"
                    } else {
                        "The file tree is as narrow as it gets"
                    });
                }
            }
            None => notice.info("The file tree is hidden"),
        },
        // Moving in the file tree; everything else acts on the diff.
        Action::Down
        | Action::Up
        | Action::PageDown
        | Action::PageUp
        | Action::Top
        | Action::Bottom
            if tree_focused =>
        {
            let tree_page = tree_list_height(lay.tree.unwrap_or_default())
                .saturating_sub(1)
                .max(1)
                .cast_signed();
            let delta = match action {
                Action::Down => 1,
                Action::Up => -1,
                Action::PageDown => tree_page,
                Action::PageUp => -tree_page,
                Action::Top => isize::MIN,
                _ => isize::MAX,
            };
            let row = screen.tree_selected.saturating_add_signed(delta);
            select_tree_row(screen, state, row);
        }
        Action::Fold | Action::Unfold | Action::Open | Action::FoldToggle if tree_focused => {
            let tree = unfolded(&state.tree, &screen.folded);
            let at = screen.tree_selected;
            let row = tree.get(at);
            // The folder the row is in.
            let parent = row.and_then(|row| {
                (0..at)
                    .rev()
                    .find(|&i| tree.get(i).is_some_and(|r| r.depth() < row.depth()))
            });
            match (action, row) {
                (_, None) => notice.info("No files"),
                (Action::Open | Action::Unfold, Some(TreeRow::File { .. })) => {
                    screen.focus = Pane::Diff;
                }
                // Into an open folder: its first row.
                (Action::Unfold, Some(TreeRow::Dir { folded: false, .. })) => {
                    select_tree_row(screen, state, at + 1);
                }
                // From a file or a folded folder, out to the folder it's in.
                (Action::Fold, Some(TreeRow::File { .. } | TreeRow::Dir { folded: true, .. })) => {
                    match parent {
                        Some(i) => select_tree_row(screen, state, i),
                        None => notice.info("Already at the top"),
                    }
                }
                // Folding a file folds the folder it's in.
                (_, Some(TreeRow::File { .. })) => {
                    match parent.and_then(|i| Some((i, tree.get(i)?))) {
                        Some((i, TreeRow::Dir { path, .. })) => {
                            screen.folded.insert(path.clone());
                            select_tree_row(screen, state, i);
                        }
                        _ => notice.info("Not in a folder"),
                    }
                }
                (_, Some(TreeRow::Dir { path, folded, .. })) => {
                    let fold = match action {
                        Action::Fold => true,
                        Action::Unfold => false,
                        _ => !folded,
                    };
                    if fold == *folded {
                        notice.info(if fold {
                            "Already folded"
                        } else {
                            "Already unfolded"
                        });
                    } else if fold {
                        screen.folded.insert(path.clone());
                    } else {
                        screen.folded.remove(path);
                    }
                }
            }
        }
        Action::FoldToggle => {
            let (doc, file) = (&state.doc, screen.cursor.file);
            let thread = doc.annotation_under(screen.cursor, screen.half);
            let want = match thread {
                Some(t) => !doc.thread_open(t),
                None => doc.files().get(file).is_some_and(DocFile::collapsed),
            };
            match thread {
                Some(t) => preserving_position(screen, state, |d| d.doc.toggle_thread(t)),
                None => {
                    preserving_position(screen, state, |d| d.doc.set_open(file, want));
                    // Files shown again are diffed on demand.
                    state.requested.remove(&file);
                }
            }
        }
        Action::FoldAll | Action::UnfoldAll => {
            let open = action == Action::UnfoldAll;
            preserving_position(screen, state, |d| d.doc.set_all_open(open));
            if open {
                state.requested.clear();
            }
            notice.info(if open {
                "Unfolded every file"
            } else {
                "Folded every file"
            });
        }
        Action::WordNext | Action::WordPrev => {
            let forward = action == Action::WordNext;
            let (cursor, half) = (screen.cursor, screen.half);
            let code = state.doc.code_at(cursor, half);
            let Some(word) = code.and_then(|code| text::step_word(code, screen.word, forward))
            else {
                notice.info(match code {
                    None => "No code on this row",
                    Some(_) if forward => "No more words on this line",
                    Some(_) => "No earlier word on this line",
                });
                return Cmds::new();
            };
            screen.word = Some(word.cols.start);
            // The word stays on screen: in split view, in its half.
            let split = state.doc.opts().split;
            let width = if split {
                lay.diff.width.saturating_sub(1) / 2
            } else {
                lay.diff.width
            };
            let file = state.doc.files().get(cursor.file);
            let room = file.map_or(0, |f| code_room(f, width, split));
            screen.shift = text::reveal(screen.shift, &word.cols, room);
        }
        Action::ScrollLeft => screen.shift = screen.shift.saturating_sub(SHIFT),
        Action::ScrollRight => screen.shift = screen.shift.saturating_add(SHIFT).min(MAX_SHIFT),
        // The tree's sections are its folders.
        Action::NextSection | Action::PrevSection if tree_focused => {
            let tree = unfolded(&state.tree, &screen.folded);
            let mut dirs = (tree.iter().enumerate())
                .filter(|(_, r)| matches!(r, TreeRow::Dir { .. }))
                .map(|(i, _)| i);
            let at = screen.tree_selected;
            let to = if action == Action::NextSection {
                dirs.find(|&i| i > at)
            } else {
                dirs.rfind(|&i| i < at)
            };
            match to {
                Some(i) => select_tree_row(screen, state, i),
                None => notice.info("No more folders that way"),
            }
        }
        Action::Down => screen.cursor = state.doc.step(screen.cursor, 1),
        Action::Up => screen.cursor = state.doc.step(screen.cursor, -1),
        // The view moves a page and the cursor with it, keeping its place
        // on screen.
        Action::PageDown | Action::PageUp => {
            let by = if action == Action::PageDown {
                page
            } else {
                -page
            };
            screen.cursor = state.doc.step(screen.cursor, by);
            screen.top = state.doc.offset(screen.top, by);
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
                Some(pos) => {
                    screen.cursor = pos;
                    face_thread(screen, state);
                }
                None => notice.info(none),
            }
        }
        Action::VisualLines => {
            let (selection, what) = match screen.selection {
                Some(_) => (None, "Selection cleared"),
                None => (Some(screen.cursor), "Selecting lines"),
            };
            screen.selection = selection;
            notice.info(what);
        }
        Action::NextUnviewed => match state.doc.next_unviewed(screen.cursor) {
            Some(pos) => jump(screen, pos),
            None => notice.info("Every file is viewed"),
        },
        Action::ExpandContext => {
            let pos = screen.cursor;
            let rows = |doc: &Doc| doc.files().get(pos.file).map(|f| f.rows().len());
            let before = rows(&state.doc);
            preserving_position(screen, state, |d| d.doc.expand(pos));
            if rows(&state.doc) == before {
                notice.info("No more context here");
            }
        }
        Action::FullFile => {
            let file = screen.cursor.file;
            preserving_position(screen, state, |d| d.doc.toggle_full(file));
        }
        Action::OldSide | Action::NewSide if !state.doc.opts().split => {
            notice.info("Sides are split view's: switch to it in the menu (Split or unified view)");
        }
        Action::OldSide => screen.half = Side::Left,
        Action::NewSide => screen.half = Side::Right,
        Action::JumpMove => match state.doc.move_at(screen.cursor, screen.half) {
            Some((mv, from)) => {
                if let Some(pos) = state.doc.move_target(mv, from) {
                    screen.cursor = pos;
                    // From removed lines to where they were added, and back.
                    screen.half = if from { Side::Right } else { Side::Left };
                }
            }
            None => notice.info("Not on moved code"),
        },
        Action::Open => match state.doc.row(screen.cursor) {
            Some(Row::Fold { .. }) => {
                let pos = screen.cursor;
                preserving_position(screen, state, |d| d.doc.unfold(pos));
            }
            Some(Row::Gap { .. }) => {
                let pos = screen.cursor;
                preserving_position(screen, state, |d| d.doc.expand(pos));
            }
            // Moved code: to its other end.
            Some(Row::Moved { .. }) => {
                return apply(screen, state, Action::JumpMove, content, notice);
            }
            // A file's header folds and unfolds it.
            Some(Row::Header | Row::Note(Note::Collapsed | Note::Viewed)) => {
                let file = screen.cursor.file;
                let open = state.doc.files().get(file).is_some_and(DocFile::collapsed);
                state.doc.set_open(file, open);
                // Expanded files are diffed on demand.
                state.requested.remove(&file);
                screen.cursor.row = 0;
            }
            _ => notice.info("Nothing to open here"),
        },
        Action::ToggleViewed => cmds.extend(toggle_viewed(screen, state, notice)),
        Action::MarkReviewed => cmds.extend(toggle_reviewed(screen, state, notice)),
        Action::SearchNext | Action::SearchPrev => {
            let Some(query) = screen.search.clone() else {
                notice.info("No search yet");
                return Cmds::new();
            };
            match state
                .doc
                .search(&query, screen.cursor, action == Action::SearchNext)
            {
                Some(pos) => go_to_match(screen, &state.doc, pos, &query),
                None => notice.error(format!("No match for “{query}”")),
            }
        }
        _ => notice.info(action.not_here()),
    }
    if scrolls && (screen.top, screen.cursor, screen.shift) == before {
        notice.info(match action {
            Action::ScrollLeft => "Already at the left edge",
            Action::ScrollRight => "Already at the right edge",
            _ => "Already there",
        });
    }
    cmds
}

/// Flips `flag` and says which way it went: `said` is what's said on,
/// then off.
fn flip(flag: &mut bool, notice: &mut Notices, said: [&'static str; 2]) {
    *flag = !*flag;
    let [on, off] = said;
    notice.info(if *flag { on } else { off });
}

/// Selects row `row` of the file tree as shown (or its last), and shows
/// the file on it in the diff: every move in the tree comes here.
pub fn select_tree_row(screen: &mut DiffScreen, state: &DiffState, row: usize) {
    let tree = unfolded(&state.tree, &screen.folded);
    screen.tree_selected = row.min(tree.len().saturating_sub(1));
    if let Some(&TreeRow::File { index, .. }) = tree.get(screen.tree_selected)
        && index != screen.cursor.file
    {
        jump(
            screen,
            Pos {
                file: index,
                row: 0,
            },
        );
    }
}

/// A click at `x`, `y`: in the file tree, selects its row; a second click
/// on a folder folds or unfolds it.
pub fn click(screen: &mut DiffScreen, state: &DiffState, content: Rect, x: u16, y: u16) {
    let Some(tree) = layout(content, &screen.prefs, &state.tree).tree else {
        return;
    };
    let rows = unfolded(&state.tree, &screen.folded);
    let Some(row) = (file_tree::row_at(tree, screen.tree_scroll, y))
        .filter(|&row| x < tree.right() && row < rows.len())
    else {
        return;
    };
    screen.focus = Pane::Tree;
    if let (true, Some(TreeRow::Dir { path, .. })) = (row == screen.tree_selected, rows.get(row))
        && !screen.folded.remove(path)
    {
        screen.folded.insert(path.clone());
    }
    select_tree_row(screen, state, row);
}

/// Puts the cursor on the half of a split row where the thread under it
/// is, so `c` replies to it.
fn face_thread(screen: &mut DiffScreen, state: &DiffState) {
    let doc = &state.doc;
    let line = doc
        .annotation_at(screen.cursor)
        .and_then(|i| doc.annotations().get(i as usize))
        .and_then(Annotation::on_line);
    if let Some(line) = line {
        screen.half = line.side;
    }
}

/// Unless the cursor's row and the thread rows under it are all in view,
/// scrolls them into it: the cursor in the middle, as vim's `n` puts a
/// match off screen, or higher when the threads would run off the bottom.
fn reveal(screen: &mut DiffScreen, doc: &Doc, height: usize) {
    let cursor = doc.to_global(screen.cursor);
    let threads = (cursor + 1..doc.total_rows())
        .take_while(|&g| matches!(doc.row(doc.to_pos(g)), Some(Row::Thread(_))))
        .count();
    let rows = (threads + 1).min(height);
    // Row 0 of the view is the sticky header's, but on a file's header.
    let first = doc.to_global(screen.top) + usize::from(screen.cursor.row != 0);
    if cursor >= first && cursor + rows <= doc.to_global(screen.top) + height {
        return;
    }
    let above = (height / 2).min(height - rows).max(1);
    screen.top = doc.to_pos(cursor.saturating_sub(above));
}

/// Puts `pos` at the top of the view with the cursor on it.
fn jump(screen: &mut DiffScreen, pos: Pos) {
    screen.cursor = pos;
    screen.top = pos;
}

/// Starts a search from the cursor, down or (`?`) up.
pub fn search(screen: &mut DiffScreen, state: &DiffState, query: String, forward: bool) -> Notice {
    let notice = match state.doc.search_counted(&query, screen.cursor, forward) {
        (Some(pos), n) => {
            go_to_match(screen, &state.doc, pos, &query);
            Notice::Info(format!(
                "{n} line{} match “{query}”",
                if n == 1 { "" } else { "s" }
            ))
        }
        (None, _) => Notice::Error(format!("No match for “{query}”")),
    };
    screen.search = Some(query);
    notice
}

/// Puts the cursor on a search match at `pos`, and the word cursor, if
/// there's one, on the first word there that has `query`.
fn go_to_match(screen: &mut DiffScreen, doc: &Doc, pos: Pos, query: &str) {
    screen.cursor = pos;
    screen.reveal = true;
    let Some(code) = doc
        .code_at(pos, screen.half)
        .filter(|_| screen.word.is_some())
    else {
        return;
    };
    let query = query.to_lowercase();
    let has = |w: &text::Word| {
        (code.get(w.bytes.clone())).is_some_and(|t| text::has(&t.to_lowercase(), &query))
    };
    if let Some(word) = text::words(code).into_iter().find(has) {
        screen.word = Some(word.cols.start);
    }
}

fn toggle_viewed(screen: &mut DiffScreen, state: &mut DiffState, notice: &mut Notices) -> Cmds {
    let Some(pr) = screen.of.pr().cloned() else {
        notice.info("Viewed files are a pull request's");
        return Cmds::new();
    };
    let Some(viewed) = &state.inputs().viewed else {
        notice.error("Viewed state hasn't loaded from GitHub yet");
        return Cmds::new();
    };
    let index = screen.cursor.file;
    let Some(file) = state.doc.files().get(index) else {
        return Cmds::new();
    };
    let (previous, path) = (file.viewed, file.meta.path().to_owned());
    let now = if previous == Viewed::Viewed {
        Viewed::Unviewed
    } else {
        Viewed::Viewed
    };
    let job = crate::review::SetViewed {
        pr,
        pull_request_id: viewed.pull_request_id.clone(),
        path: path.clone(),
        viewed: now == Viewed::Viewed,
        previous,
    };
    // Optimistic: show it now, roll back if GitHub refuses.
    state.edit(|i| i.set_viewed(path, now));
    screen.cursor = Pos {
        file: index,
        row: 0,
    };
    crate::work::start(job)
}

fn toggle_reviewed(screen: &DiffScreen, state: &mut DiffState, notice: &mut Notices) -> Cmds {
    let Some(pr) = screen.of.pr().cloned() else {
        notice.info("Reviewed marks are a pull request's");
        return Cmds::new();
    };
    let Some(block) = state.doc.block_at(screen.cursor) else {
        notice.info("Move to a changed line to mark it reviewed");
        return Cmds::new();
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

/// The link to what's under the cursor: the lines selected, or the line
/// under it, or else its file.
pub fn here_url(screen: &DiffScreen, state: &DiffState) -> String {
    let (doc, cursor) = (&state.doc, screen.cursor);
    let Some(file) = doc.files().get(cursor.file) else {
        return screen.of.url();
    };
    let from = (screen.selection)
        .filter(|s| s.file == cursor.file)
        .unwrap_or(cursor);
    let lines = (screen.focus == Pane::Diff)
        .then(|| doc.line_range(from, cursor, screen.half))
        .flatten();
    screen.of.file_url(file.meta.path(), lines)
}

/// Lines as GitHub anchors them: `R12`, `L3-R5`.
fn parse_lines(s: &str) -> Option<(LinePos, LinePos)> {
    let line = |s: &str| {
        let side = match s.get(..1)? {
            "L" => Side::Left,
            "R" => Side::Right,
            _ => return None,
        };
        let line = s.get(1..)?.parse().ok()?;
        Some(LinePos { side, line })
    };
    let (start, end) = s.split_once('-').unwrap_or((s, s));
    Some((line(start)?, line(end)?))
}

/// Where a link's anchor puts the cursor, and the selection it starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Landing {
    cursor: Pos,
    /// For lines: the half of split rows they're on, and the first.
    lines: Option<(Side, Pos)>,
}

impl From<Pos> for Landing {
    fn from(cursor: Pos) -> Self {
        Self {
            cursor,
            lines: None,
        }
    }
}

/// Where a link's anchor is in the diff: `Some(None)` when it isn't there,
/// `None` while the diff or review threads it needs are on their way.
fn anchor_pos(anchor: &str, state: &DiffState, pr: bool) -> Option<Option<Landing>> {
    let doc = &state.doc;
    if let Some(rest) = anchor.strip_prefix("diff-") {
        let (hash, lines) = rest.split_at_checked(64).unwrap_or((rest, ""));
        let Some((file, f)) =
            (doc.files().iter().enumerate()).find(|(_, f)| links::path_hash(f.meta.path()) == hash)
        else {
            return Some(None);
        };
        let top = Pos { file, row: 0 };
        let Some((start, end)) = parse_lines(lines) else {
            return Some(Some(top.into()));
        };
        // Lines wait for the file's diff; a folded file shows none.
        if f.diff.is_none() && !f.collapsed() {
            return None;
        }
        let row = |l| doc.line_row(file, l).unwrap_or(top);
        return Some(Some(Landing {
            cursor: row(end),
            lines: Some((end.side, row(start))),
        }));
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
    let threads = state.inputs().threads.data.as_ref()?;
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
        return Some(Some(pos.into()));
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
    diffed.then_some(Some(Pos { file, row: 0 }.into()))
}

/// Applies view options, clamps positions, keeps the cursor visible (clear
/// of the sticky header), syncs the tree with the diff, and asks the job to
/// prioritize files that are on screen but not diffed yet.
pub fn settle(screen: &mut DiffScreen, state: &mut DiffState, content: Rect) -> Cmds {
    let lay = layout(content, &screen.prefs, &state.tree);
    if lay.tree.is_none() {
        screen.focus = Pane::Diff;
    }
    let opts = screen.options(lay.diff);
    if state.doc.opts() != opts {
        preserving_position(screen, state, |d| d.doc.set_options(opts));
    }
    let doc = &state.doc;
    if doc.is_empty() {
        return Cmds::new();
    }
    if let Some(anchor) = screen.anchor.take() {
        match anchor_pos(&anchor, state, screen.of.pr().is_some()) {
            Some(Some(Landing { cursor, lines })) => {
                screen.cursor = cursor;
                match lines {
                    // Lines are selected, from a little above the first.
                    Some((half, from)) => {
                        screen.half = half;
                        screen.selection = (from != cursor).then_some(from);
                        screen.top = state.doc.offset(from, -2);
                    }
                    None => {
                        face_thread(screen, state);
                        screen.reveal = true;
                    }
                }
            }
            // A review comment waits for the threads.
            None => screen.anchor = Some(anchor),
            Some(None) => {}
        }
    }
    let doc = &state.doc;
    screen.cursor = doc.line_start(doc.clamp(screen.cursor));
    let height = usize::from(lay.diff.height).max(1);
    if std::mem::take(&mut screen.reveal) {
        reveal(screen, doc, height);
    }
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
    // All of the cursor's line, wrapped, if it fits.
    let rows = doc.line_end(screen.cursor).row - screen.cursor.row + 1;
    let end = cursor + rows.min(height.saturating_sub(1)).max(1);
    if end > top + height {
        top = end - height;
    }
    screen.top = doc.to_pos(top);

    if screen.focus == Pane::Diff
        && let Some(row) = row_of_file(&unfolded(&state.tree, &screen.folded), screen.cursor.file)
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
        return Cmds::new();
    }
    state.requested.extend(wanted.iter().copied());
    Cmd::Git(Git::Prioritize(screen.of.clone(), wanted)).into()
}

/// A message from a diff's current job.
pub(crate) fn on_job(state: &mut State, of: &DiffOf, msg: JobMsg) -> Cmds {
    match msg {
        JobMsg::Progress(line) => {
            if let Some(diff) = state.diffs.get_mut(of)
                && !diff.listed()
            {
                diff.progress = Some(line);
            }
        }
        // Only the file is kept: the new one has no lines yet.
        JobMsg::Files(files) => {
            change(state, of, |d| d.set_files(*files));
        }
        JobMsg::File(index, file) => {
            if let Some(diff) = state.diffs.get_mut(of) {
                diff.set_file(index, file);
            }
        }
        JobMsg::Moves(moves) => {
            change(state, of, |d| d.doc.set_moves(moves));
        }
        JobMsg::Mapped(mapped) => {
            change(state, of, |d| d.add_mapped(mapped));
        }
        JobMsg::Failed(error) => {
            if let Some(diff) = state.diffs.get_mut(of) {
                diff.progress = None;
                diff.error = Some(error.to_string());
            }
        }
        JobMsg::Since(old_head, Ok(hashes)) => {
            if let Some((screen, diff)) = state.diff_parts()
                && screen.of == *of
            {
                preserving_position(screen, diff, |d| d.doc.set_since(Some(hashes), true));
                let whose = if diff.since_commit.is_some() {
                    ""
                } else {
                    "your review of "
                };
                state.info(format!(
                    "Showing changes since {whose}{}",
                    short_sha(&old_head)
                ));
            }
        }
        JobMsg::Since(_, Err(err)) => {
            tracing::warn!(%of, ?err, "comparing with the last review failed");
            state.error(format!("Couldn't compare with your last review: {err}"));
        }
    }
    Cmds::new()
}

/// Changes `of`'s diff, if it's still open; on screen, keeping the cursor
/// where it is.
fn change<R>(state: &mut State, of: &DiffOf, f: impl FnOnce(&mut DiffState) -> R) -> Option<R> {
    match state.diff_parts() {
        Some((screen, diff)) if screen.of == *of => Some(preserving_position(screen, diff, f)),
        _ => state.diffs.get_mut(of).map(f),
    }
}

/// Changes the inputs of `of`'s diff, if it's still open.
pub(crate) fn edit(state: &mut State, of: &DiffOf, f: impl FnOnce(&mut DiffInputs)) {
    change(state, of, |d| d.edit(f));
}

/// A message for a diff screen.
pub(crate) fn update(state: &mut State, of: &DiffOf, msg: DiffMsg) -> Cmds {
    if let DiffMsg::Job(job, msg) = msg {
        if state.diffs.get(of).is_some_and(|d| d.job == job) {
            return on_job(state, of, msg);
        }
        return Cmds::new();
    }
    // The rest is about reviewing a pull request.
    let DiffOf::Pr(pr) = of.clone() else {
        return Cmds::new();
    };
    match msg {
        DiffMsg::LastReview(result) => {
            if let Some(diff) = state.diffs.get_mut(of) {
                let last = match result {
                    Ok(None) => LastReview::None,
                    Ok(Some(commit)) => Oid::parse(&commit)
                        .map_or_else(|e| LastReview::Failed(e.to_string()), LastReview::At),
                    Err(err) => LastReview::Failed(err.to_string()),
                };
                diff.edit(|i| i.last_review = last);
            }
        }
        DiffMsg::CommitsListed(Ok(commits)) => {
            edit(state, of, |i| i.commits = commits);
            state.notices.dismiss();
            return state.open_picker(picker::Kind::Commits { mark: None });
        }
        DiffMsg::CommitsListed(Err(err)) => {
            tracing::warn!(%pr, ?err, "listing commits failed");
            state.error(format!("Couldn't list commits: {err}"));
        }
        DiffMsg::ViewedLoaded(result) => match *result {
            Ok(viewed) => {
                edit(state, of, |i| i.viewed = Some(viewed));
            }
            Err(err) => {
                tracing::warn!(%pr, %err, "viewed state fetch failed");
                state.error(format!("Couldn't load viewed files: {err}"));
            }
        },
        // Approving from the pull request's page: its drafts go with it.
        DiffMsg::ReviewLoaded(Ok(saved)) if !state.diffs.contains_key(of) => {
            if let Some(Overlay::Submit(dialog)) = &mut state.overlay
                && dialog.pr == pr
            {
                dialog.saved = Some(saved);
            }
        }
        DiffMsg::ReviewLoaded(Ok(saved)) => {
            // Anything done before it arrived is kept, and saved.
            let mut changed = false;
            let save = change(state, of, |d| {
                d.edit_review(&pr, |i| changed = i.merge_saved_review(saved))
            });
            return save.filter(|_| changed).unwrap_or_default();
        }
        DiffMsg::ReviewLoaded(Err(err)) => {
            tracing::warn!(%pr, ?err, "reading review state failed");
            state.problems.insert(
                Problem::Drafts,
                format!("Drafts for {pr} are not being saved: {err}"),
            );
        }
        DiffMsg::ThreadsLoaded(result) => {
            if let Err(err) = &result {
                tracing::warn!(%pr, %err, "review threads fetch failed");
                state.error(format!("Couldn't load review threads: {err}"));
            }
            edit(state, of, |i| i.threads.finish(result));
            crate::live::threads_loaded(state, &pr);
        }
        DiffMsg::PatchesLoaded(Ok(patches)) => {
            let commentable = |f: PatchFile| Some((f.filename, Commentable::from_patch(&f.patch?)));
            let patches = Arc::new(patches.into_iter().filter_map(commentable).collect());
            edit(state, of, |i| i.patches = patches);
        }
        // The local fallback covers commenting; just note it.
        DiffMsg::PatchesLoaded(Err(err)) => {
            tracing::warn!(%pr, %err, "GitHub patches unavailable; using local hunks");
        }
        // Jobs are handled above.
        DiffMsg::Job(..) => {}
        DiffMsg::PendingReview(result) => match (result, &mut state.overlay) {
            (Ok(n), Some(Overlay::Submit(dialog))) if dialog.pr == pr => {
                dialog.on_github = Some(n);
            }
            (Ok(_), _) => {}
            // Submitting checks again, and says what it finds.
            (Err(err), _) => tracing::warn!(%pr, %err, "couldn't read the pending review"),
        },
    }
    Cmds::new()
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
