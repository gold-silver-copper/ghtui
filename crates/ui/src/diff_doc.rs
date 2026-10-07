//! The diff document: every changed file as a sequence of display rows.
//!
//! Rows are built from each file's full line alignment and the view options:
//! unified or split, exact or whitespace-insensitive comparison, and per-file
//! context (expansion windows, full-file mode). Hidden stretches become gap
//! rows that can be expanded.
//!
//! Positions are `(file, row)` pairs rather than global row numbers, so the
//! cursor stays put when an earlier file's diff arrives. When a file's rows
//! are rebuilt (view changes), [`Doc::anchor`] and [`Doc::locate`] keep the
//! cursor on the same source line.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use ghtui_diff::anchor::{Commentable, LinePos, RangeSource, Side};
use ghtui_diff::blocks::{ChangeBlock, change_blocks};
use ghtui_diff::moves::Move;
use ghtui_diff::{
    CONTEXT, Content, DiffLine, FileDiff, LineKind, TextDiff, Whitespace, counts, hunk, segments_by,
};
use ghtui_git::files::{ChangedFile, is_lockfile};

/// A file's viewed state, as GitHub keeps it.
pub use ghtui_api::model::ViewedState as Viewed;

use crate::annotations::{Annotation, AnnotationKey, ThreadRow, ThreadRowKind};
use crate::{idx, text};

/// Thread text wraps at this width unless the view says otherwise.
const DEFAULT_WRAP: u16 = 72;

/// Lines revealed per expansion step.
const EXPAND_STEP: u32 = 20;

/// Where a line sits on each side: its old number on the left, its new
/// number on the right.
pub(crate) fn sides(line: &DiffLine) -> (Option<LinePos>, Option<LinePos>) {
    let at = |side, n: Option<u32>| n.map(|line| LinePos { side, line });
    (at(Side::Left, line.old), at(Side::Right, line.new))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Note {
    Loading,
    /// Generated or vendored file, or a lockfile.
    Collapsed,
    /// Marked viewed.
    Viewed,
    Binary,
    TooLarge,
    Submodule,
    NoChanges,
    /// "Since my last review" is on and nothing in the file is new.
    NothingNew,
    Error,
}

/// Why a change block is folded into one row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoldReason {
    /// Differs only in whitespace and line breaks.
    Formatting,
    /// Already in the diff at your last review.
    Seen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Header,
    Note(Note),
    /// Hidden alignment entries `start..end`.
    Gap {
        start: u32,
        end: u32,
    },
    /// Header of visible segment `seg`.
    Hunk {
        seg: u32,
    },
    /// One alignment entry (unified view).
    Line(u32),
    /// Old and new entries side by side (split view).
    Split {
        left: Option<u32>,
        right: Option<u32>,
    },
    /// "No newline at end of file" after the preceding line.
    NoNewline,
    /// A line of a review thread or draft (index into the file's thread
    /// rows).
    Thread(u32),
    /// A folded change block (index into the file's blocks).
    Fold {
        block: u32,
        reason: FoldReason,
    },
    /// "Moved from/to …" after the first line of a moved block (index into
    /// the document's moves).
    Moved {
        mv: u32,
        from: bool,
    },
    Spacer,
}

/// A hunk's header: its `@@` range and the scopes its first change is in,
/// outermost first (`["impl Doc", "fn offset"]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HunkHeader {
    pub range: String,
    pub scope: Vec<String>,
}

/// The new-side line at alignment entry `e`, or the nearest before it (a
/// removed line has none of its own), or after it.
fn new_line_near(lines: &[DiffLine], e: usize) -> Option<u32> {
    let (before, after) = lines
        .split_at_checked(e.saturating_add(1))
        .unwrap_or((lines, &[]));
    before
        .iter()
        .rev()
        .find_map(|l| l.new)
        .or_else(|| after.iter().find_map(|l| l.new))
}

impl Row {
    /// The alignment entries a line row shows (none for other rows).
    pub fn entries(self) -> impl Iterator<Item = u32> {
        let (a, b) = match self {
            Row::Line(e) => (Some(e), None),
            Row::Split { left, right } => (left, right),
            _ => (None, None),
        };
        a.into_iter().chain(b)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ViewOptions {
    pub split: bool,
    pub whitespace: Whitespace,
    /// Columns for comment text; 0 for the default.
    pub wrap: u16,
}

/// A maximal run of changed lines: the unit marked "reviewed", folded when
/// formatting-only, and compared for "since my last review".
pub type Block = ChangeBlock;

/// Inputs for rebuilding one file's rows besides the file itself.
struct Extras<'a> {
    anns: &'a [(u32, &'a Annotation)],
    open: &'a dyn Fn(&Annotation) -> bool,
    /// Block hashes from the diff at your last review, when that mode is on.
    since: Option<&'a HashSet<String>>,
    /// `(move index, entries, is the removed side)` in this file.
    moves: &'a [(u32, Range<u32>, bool)],
}

#[derive(Debug, Clone)]
pub struct DocFile {
    pub meta: ChangedFile,
    /// `linguist-generated`, `linguist-vendored` or a lockfile.
    pub generated: bool,
    pub viewed: Viewed,
    /// Shown despite being generated or viewed.
    pub expanded: bool,
    pub diff: Option<Arc<FileDiff>>,
    /// Show every line.
    pub full: bool,
    /// Extra visible alignment ranges from expanding.
    pub windows: Vec<Range<u32>>,
    rows: Vec<Row>,
    /// The header of each visible segment.
    headers: Vec<HunkHeader>,
    blocks: Vec<Block>,
    /// Folded blocks the user opened (by first entry).
    pub unfolded: HashSet<u32>,
    thread_rows: Vec<ThreadRow>,
    /// Line annotations by anchor.
    by_line: HashMap<LinePos, Vec<u32>>,
    /// Commentable ranges reconstructed locally.
    local_commentable: Option<Commentable>,
}

impl DocFile {
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn headers(&self) -> &[HunkHeader] {
        &self.headers
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    pub fn collapsed(&self) -> bool {
        !self.expanded && (self.generated || self.viewed == Viewed::Viewed)
    }

    pub fn text(&self) -> Option<&TextDiff> {
        match self.diff.as_deref().map(|d| &d.content) {
            Some(Content::Text(text)) => Some(text),
            _ => None,
        }
    }

    /// The block containing alignment entry `entry`.
    pub fn block_of(&self, entry: u32) -> Option<&Block> {
        self.blocks.iter().find(|b| b.entries.contains(&entry))
    }

    pub fn thread_row(&self, i: u32) -> Option<&ThreadRow> {
        self.thread_rows.get(i as usize)
    }

    /// Annotations (indices into `Doc::annotations`) anchored at a line.
    pub fn annotations_at(&self, pos: LinePos) -> &[u32] {
        self.by_line.get(&pos).map_or(&[], Vec::as_slice)
    }

    /// The lines an alignment entry shows: removed lines are on the left,
    /// added on the right, context on both.
    pub fn entry_lines(
        &self,
        entry: u32,
        whitespace: Whitespace,
    ) -> impl Iterator<Item = LinePos> + use<> {
        let line = self
            .text()
            .and_then(|t| t.lines(whitespace).get(entry as usize));
        let (left, right) = line.map(sides).unwrap_or_default();
        let kind = line.map(|l| l.kind);
        let left = left.filter(|_| kind != Some(LineKind::Added));
        let right = right.filter(|_| kind != Some(LineKind::Removed));
        left.into_iter().chain(right)
    }

    fn push_annotation(&mut self, index: u32, ann: &Annotation, open: bool, wrap: usize) {
        if !open {
            let first = ann
                .comments
                .first()
                .map(|c| {
                    format!(
                        "{}: {}",
                        c.author,
                        c.body.lines().next().unwrap_or_default()
                    )
                })
                .unwrap_or_default();
            self.push_thread_row(index, ThreadRowKind::Summary, first);
            return;
        }
        for (c, comment) in ann.comments.iter().enumerate() {
            self.push_thread_row(
                index,
                ThreadRowKind::Head {
                    comment: c,
                    first: c == 0,
                },
                comment.author.clone(),
            );
            let body = if comment.body.trim().is_empty() {
                "(no text)".to_owned()
            } else {
                comment.body.replace("\r\n", "\n")
            };
            for line in text::wrap(&body, wrap) {
                self.push_thread_row(index, ThreadRowKind::Body, line);
            }
            if c == 0 && ann.left_out > 0 {
                let gap = format!("… {} more replies on GitHub (o)", ann.left_out);
                self.push_thread_row(index, ThreadRowKind::Gap, gap);
            }
        }
        if let Some(error) = &ann.error {
            for line in text::wrap(&format!("GitHub rejected this: {error}"), wrap) {
                self.push_thread_row(index, ThreadRowKind::Error, line);
            }
        }
        self.push_thread_row(index, ThreadRowKind::Footer, String::new());
    }

    fn push_thread_row(&mut self, ann: u32, kind: ThreadRowKind, text: String) {
        self.rows.push(Row::Thread(idx(self.thread_rows.len())));
        self.thread_rows.push(ThreadRow { ann, kind, text });
    }

    fn rebuild(&mut self, opts: ViewOptions, extras: &Extras<'_>) {
        let (anns, open) = (extras.anns, extras.open);
        self.rows.clear();
        self.headers.clear();
        self.blocks.clear();
        self.thread_rows.clear();
        self.by_line.clear();
        let wrap = usize::from(if opts.wrap == 0 {
            DEFAULT_WRAP
        } else {
            opts.wrap
        });
        for (i, ann) in anns {
            if let Some(line) = ann.on_line() {
                let pos = LinePos {
                    side: ann.side,
                    line,
                };
                self.by_line.entry(pos).or_default().push(*i);
            }
        }
        self.rows.push(Row::Header);
        // File-level comments, and outdated threads that can't be placed,
        // sit under the header.
        for (i, ann) in anns {
            if ann.on_line().is_none() {
                self.push_annotation(*i, ann, open(ann), wrap);
            }
        }
        let note = match self.diff.as_deref().map(|d| &d.content) {
            _ if self.collapsed() && self.viewed == Viewed::Viewed => Some(Note::Viewed),
            _ if self.collapsed() => Some(Note::Collapsed),
            None => Some(Note::Loading),
            Some(Content::Binary { .. }) => Some(Note::Binary),
            Some(Content::TooLarge { .. }) => Some(Note::TooLarge),
            Some(Content::Submodule { .. }) => Some(Note::Submodule),
            Some(Content::Error(_)) => Some(Note::Error),
            Some(Content::Text(text)) if !text.has_changes(opts.whitespace) && !self.full => {
                Some(Note::NoChanges)
            }
            Some(Content::Text(_)) => None,
        };
        if let Some(note) = note {
            self.rows.push(Row::Note(note));
            self.rows.push(Row::Spacer);
            return;
        }
        let diff = self.diff.clone();
        let Some(Content::Text(text)) = diff.as_deref().map(|d| &d.content) else {
            return;
        };
        let lines = text.lines(opts.whitespace);
        self.blocks = change_blocks(self.meta.path(), text, lines);
        let fold_of: Vec<Option<FoldReason>> = self
            .blocks
            .iter()
            .map(|b| {
                if self.unfolded.contains(&b.entries.start) {
                    None
                } else if extras.since.is_some_and(|s| s.contains(&b.hash)) {
                    Some(FoldReason::Seen)
                } else if b.formatting_only {
                    Some(FoldReason::Formatting)
                } else {
                    None
                }
            })
            .collect();
        // With "since my last review", changes you'd already seen don't
        // anchor context; they only show (folded) when near new ones.
        let mut seen_entry = vec![false; lines.len()];
        if extras.since.is_some() {
            for (b, fold) in self.blocks.iter().zip(&fold_of) {
                if *fold == Some(FoldReason::Seen) {
                    for e in b.entries.clone() {
                        if let Some(seen) = seen_entry.get_mut(e as usize) {
                            *seen = true;
                        }
                    }
                }
            }
            let anything_new = lines
                .iter()
                .zip(&seen_entry)
                .any(|(l, seen)| l.kind != LineKind::Context && !seen);
            if !anything_new && !self.full {
                self.rows.push(Row::Note(Note::NothingNew));
                self.rows.push(Row::Spacer);
                return;
            }
        }

        // Commented lines are always visible.
        let mut windows = self.windows.clone();
        for (e, line) in lines.iter().enumerate() {
            let (left, right) = sides(line);
            if left
                .into_iter()
                .chain(right)
                .any(|p| self.by_line.contains_key(&p))
            {
                let e = idx(e);
                windows.push(e..e.saturating_add(1));
            }
        }
        let segs = segments_by(
            lines.len(),
            |e| {
                lines.get(e).is_some_and(|l| l.kind != LineKind::Context)
                    && seen_entry.get(e) == Some(&false)
            },
            CONTEXT,
            &windows,
            self.full,
        );
        let (mut seen, mut before) = (0usize, (0u32, 0u32));
        let mut next_hidden = 0u32;
        for (s, seg) in segs.iter().enumerate() {
            if idx(seg.start) > next_hidden {
                self.rows.push(Row::Gap {
                    start: next_hidden,
                    end: idx(seg.start),
                });
            }
            let skipped = counts_in(lines.get(seen..seg.start).unwrap_or_default());
            before = (before.0 + skipped.0, before.1 + skipped.1);
            seen = seg.start;
            // Named by where its first change is.
            let first_change = seg
                .clone()
                .find(|&e| lines.get(e).is_some_and(|l| l.kind != LineKind::Context))
                .unwrap_or(seg.start);
            self.headers.push(HunkHeader {
                range: hunk(lines, seg.clone(), before).header(),
                scope: new_line_near(lines, first_change)
                    .map(|n| text.scope(n).into_iter().map(str::to_owned).collect())
                    .unwrap_or_default(),
            });
            self.rows.push(Row::Hunk { seg: idx(s) });
            let first_new_row = self.rows.len();
            // Lines, with folded blocks as one row each.
            let push =
                |rows: &mut Vec<Row>, range| push_lines(rows, text, lines, range, opts.split);
            let mut at = seg.start;
            for (i, (b, fold)) in self.blocks.iter().zip(&fold_of).enumerate() {
                let (start, end) = (b.entries.start as usize, b.entries.end as usize);
                let Some(reason) = fold.filter(|_| start < seg.end && end > seg.start) else {
                    continue;
                };
                push(&mut self.rows, at..start.max(seg.start));
                self.rows.push(Row::Fold {
                    block: idx(i),
                    reason,
                });
                at = end.min(seg.end);
            }
            push(&mut self.rows, at..seg.end);
            if !self.by_line.is_empty() || !extras.moves.is_empty() {
                self.insert_extras(first_new_row, extras, wrap, opts.whitespace);
            }
            next_hidden = idx(seg.end);
        }
        if (next_hidden as usize) < lines.len() {
            self.rows.push(Row::Gap {
                start: next_hidden,
                end: idx(lines.len()),
            });
        }
        self.rows.push(Row::Spacer);
    }

    /// After the rows from `from`: inserts "moved from/to" rows after the
    /// first line of each moved block, and open line annotations after the
    /// rows showing their lines.
    fn insert_extras(
        &mut self,
        from: usize,
        extras: &Extras<'_>,
        wrap: usize,
        whitespace: Whitespace,
    ) {
        let (anns, open) = (extras.anns, extras.open);
        let tail = self.rows.split_off(from);
        for row in tail {
            self.rows.push(row);
            let entries: Vec<u32> = row.entries().collect();
            for (mv, range, is_from) in extras.moves {
                if entries.contains(&range.start) {
                    self.rows.push(Row::Moved {
                        mv: *mv,
                        from: *is_from,
                    });
                }
            }
            let mut here: Vec<u32> = Vec::new();
            for e in entries {
                for pos in self.entry_lines(e, whitespace) {
                    for i in self.annotations_at(pos) {
                        if !here.contains(i) {
                            here.push(*i);
                        }
                    }
                }
            }
            for i in here {
                if let Some((_, ann)) = anns.iter().find(|(j, _)| *j == i)
                    && open(ann)
                {
                    self.push_annotation(i, ann, true, wrap);
                }
            }
        }
    }

    /// Columns for line numbers in this file (at least 3).
    pub fn number_width(&self) -> usize {
        let max = self.text().map_or(0, |t| t.old.len().max(t.new.len()));
        max.to_string().len().max(3)
    }

    /// The source line a row shows: `(is_new_side, line)`.
    fn row_line(&self, row: Row, whitespace: Whitespace) -> Option<(bool, u32)> {
        let text = self.text()?;
        let lines = text.lines(whitespace);
        let entry = |e: u32| lines.get(e as usize);
        let side = |l: &DiffLine| l.new.map(|n| (true, n)).or(l.old.map(|o| (false, o)));
        match row {
            Row::Line(e) | Row::Gap { start: e, .. } => entry(e).and_then(side),
            Row::Split { left, right } => right.or(left).and_then(entry).and_then(side),
            Row::Hunk { seg } => {
                // The first line after the header.
                let pos = self.rows.iter().position(|r| *r == Row::Hunk { seg })?;
                self.rows
                    .get(pos + 1)
                    .and_then(|r| self.row_line(*r, whitespace))
            }
            _ => None,
        }
    }
}

/// Whether a thread is expanded: as the user left it, or by default.
fn is_open(thread_open: &HashMap<AnnotationKey, bool>, ann: &Annotation) -> bool {
    thread_open
        .get(&ann.key)
        .copied()
        .unwrap_or_else(|| ann.open_by_default())
}

fn counts_in(lines: &[DiffLine]) -> (u32, u32) {
    lines.iter().fold((0, 0), |(o, n), l| {
        (
            o + u32::from(l.old.is_some()),
            n + u32::from(l.new.is_some()),
        )
    })
}

fn ends_without_newline(text: &TextDiff, line: &DiffLine) -> bool {
    let old_end = line.old == Some(idx(text.old.len()))
        && text.old.missing_final_newline
        && line.kind != LineKind::Added;
    let new_end = line.new == Some(idx(text.new.len()))
        && text.new.missing_final_newline
        && line.kind != LineKind::Removed;
    old_end || new_end
}

/// Rows for entries `range`: one each, or split.
fn push_lines(
    rows: &mut Vec<Row>,
    text: &TextDiff,
    lines: &[DiffLine],
    range: Range<usize>,
    split: bool,
) {
    if split {
        return push_split_rows(rows, text, lines, range);
    }
    for e in range {
        rows.push(Row::Line(idx(e)));
        if lines.get(e).is_some_and(|l| ends_without_newline(text, l)) {
            rows.push(Row::NoNewline);
        }
    }
}

/// Split rows: context lines pair with themselves; within a change, the
/// removed and added runs are zipped side by side.
fn push_split_rows(rows: &mut Vec<Row>, text: &TextDiff, lines: &[DiffLine], seg: Range<usize>) {
    let is = |e: usize, kind: LineKind| lines.get(e).is_some_and(|l| l.kind == kind);
    let mut e = seg.start;
    while e < seg.end {
        let Some(line) = lines.get(e) else { break };
        if line.kind == LineKind::Context {
            rows.push(Row::Split {
                left: Some(idx(e)),
                right: Some(idx(e)),
            });
            if ends_without_newline(text, line) {
                rows.push(Row::NoNewline);
            }
            e += 1;
            continue;
        }
        let removed_start = e;
        while e < seg.end && is(e, LineKind::Removed) {
            e += 1;
        }
        let added_start = e;
        while e < seg.end && is(e, LineKind::Added) {
            e += 1;
        }
        let (removed, added) = (added_start - removed_start, e - added_start);
        let mut missing_newline = false;
        for i in 0..removed.max(added) {
            let left = (i < removed).then_some(idx(removed_start + i));
            let right = (i < added).then_some(idx(added_start + i));
            missing_newline |= [left, right].into_iter().flatten().any(|x| {
                lines
                    .get(x as usize)
                    .is_some_and(|l| ends_without_newline(text, l))
            });
            rows.push(Row::Split { left, right });
        }
        if missing_newline {
            rows.push(Row::NoNewline);
        }
    }
}

/// A position in the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Pos {
    pub file: usize,
    pub row: usize,
}

/// A position described by content, to find again after rows change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Anchor {
    file: usize,
    line: Option<(bool, u32)>,
    row: usize,
}

/// What a diff shows that isn't the diff; a [`Doc`] takes it only whole.
#[derive(Debug, Clone, Default)]
pub struct DocInputs {
    /// Commentable ranges from GitHub's patches, by path.
    pub patches: Arc<HashMap<String, Commentable>>,
    /// Viewed state by path; unviewed when missing.
    pub viewed: HashMap<String, Viewed>,
    /// Hashes of blocks marked reviewed.
    pub reviewed: HashSet<String>,
    /// Review threads and drafts.
    pub annotations: Vec<Annotation>,
}

#[derive(Debug, Clone, Default)]
pub struct Doc {
    // Rows are derived from these: change them through the setters, which
    // rebuild, so rows never go stale.
    pub(crate) files: Vec<DocFile>,
    pub(crate) opts: ViewOptions,
    pub(crate) inputs: DocInputs,
    /// Threads opened or closed by the user (others use their default).
    thread_open: HashMap<AnnotationKey, bool>,
    /// Moved blocks (computed on the exact alignment).
    pub(crate) moves: Vec<Move>,
    /// Block hashes of the diff at your last review.
    pub(crate) since: Option<HashSet<String>>,
    /// Show only what changed since your last review.
    pub(crate) since_active: bool,
    starts: Vec<usize>,
    total: usize,
}

impl Doc {
    pub fn files(&self) -> &[DocFile] {
        &self.files
    }

    pub fn opts(&self) -> ViewOptions {
        self.opts
    }

    pub fn annotations(&self) -> &[Annotation] {
        &self.inputs.annotations
    }

    pub fn moves(&self) -> &[Move] {
        &self.moves
    }

    /// Showing only what changed since your last review.
    pub fn since_active(&self) -> bool {
        self.since_active
    }

    pub fn new(files: Vec<ChangedFile>, generated: &HashSet<String>, inputs: DocInputs) -> Self {
        let files = files
            .into_iter()
            .map(|meta| {
                let path = meta.path().to_owned();
                DocFile {
                    generated: generated.contains(&path) || is_lockfile(&path),
                    meta,
                    viewed: Viewed::Unviewed,
                    expanded: false,
                    diff: None,
                    full: false,
                    windows: Vec::new(),
                    rows: Vec::new(),
                    headers: Vec::new(),
                    blocks: Vec::new(),
                    unfolded: HashSet::new(),
                    thread_rows: Vec::new(),
                    by_line: HashMap::new(),
                    local_commentable: None,
                }
            })
            .collect();
        let mut doc = Self {
            files,
            ..Self::default()
        };
        doc.set_inputs(inputs);
        doc.rebuild_all();
        doc
    }

    /// Shows new inputs, rebuilding files whose viewed state changed
    /// (collapsing them), or everything if the threads or drafts did.
    pub fn set_inputs(&mut self, inputs: DocInputs) {
        let old = std::mem::replace(&mut self.inputs, inputs);
        let viewed = &self.inputs.viewed;
        let mut changed = Vec::new();
        for (i, f) in self.files.iter_mut().enumerate() {
            let now = viewed.get(f.meta.path()).copied().unwrap_or_default();
            if f.viewed != now {
                (f.viewed, f.expanded) = (now, false);
                changed.push(i);
            }
        }
        if old.annotations == self.inputs.annotations {
            changed.into_iter().for_each(|i| self.rebuild(i));
        } else {
            self.rebuild_all();
        }
    }

    fn rebuild_all(&mut self) {
        for index in 0..self.files.len() {
            self.rebuild_file(index);
        }
        self.reindex();
    }

    fn rebuild(&mut self, index: usize) {
        if index < self.files.len() {
            self.rebuild_file(index);
            self.reindex();
        }
    }

    /// Changes file `index` with `f` and rebuilds its rows.
    fn update(&mut self, index: usize, f: impl FnOnce(&mut DocFile)) {
        if let Some(file) = self.files.get_mut(index) {
            f(file);
            self.rebuild(index);
        }
    }

    fn rebuild_file(&mut self, index: usize) {
        let Doc {
            files,
            opts,
            inputs: DocInputs { annotations, .. },
            thread_open,
            moves,
            since,
            since_active,
            ..
        } = self;
        let Some(file) = files.get_mut(index) else {
            return;
        };
        let path = file.meta.path().to_owned();
        // Moves index the exact alignment.
        let file_moves: Vec<(u32, Range<u32>, bool)> = if opts.whitespace == Whitespace::Exact {
            moves
                .iter()
                .enumerate()
                .flat_map(|(i, m)| {
                    let from = (m.from.0 == index).then(|| (idx(i), m.from.1.clone(), true));
                    let to = (m.to.0 == index).then(|| (idx(i), m.to.1.clone(), false));
                    from.into_iter().chain(to)
                })
                .collect()
        } else {
            Vec::new()
        };
        let anns: Vec<(u32, &Annotation)> = annotations
            .iter()
            .enumerate()
            .filter(|(_, a)| a.path == path)
            .map(|(i, a)| (idx(i), a))
            .collect();
        let open = |a: &Annotation| is_open(thread_open, a);
        let extras = Extras {
            anns: &anns,
            open: &open,
            since: since.as_ref().filter(|_| *since_active),
            moves: &file_moves,
        };
        file.rebuild(*opts, &extras);
    }

    pub fn set_moves(&mut self, moves: Vec<Move>) {
        self.moves = moves;
        self.rebuild_all();
    }

    /// The move covering alignment entry `e` of `file`: `(index, removed side)`.
    pub fn move_at_entry(&self, file: usize, e: u32) -> Option<(u32, bool)> {
        if self.opts.whitespace != Whitespace::Exact {
            return None;
        }
        self.moves.iter().enumerate().find_map(|(i, m)| {
            if m.from.0 == file && m.from.1.contains(&e) {
                Some((idx(i), true))
            } else if m.to.0 == file && m.to.1.contains(&e) {
                Some((idx(i), false))
            } else {
                None
            }
        })
    }

    /// The move under the cursor: on a "moved" row or inside a moved block.
    pub fn move_at(&self, pos: Pos) -> Option<(u32, bool)> {
        match self.row(pos)? {
            Row::Moved { mv, from } => Some((mv, from)),
            row => row.entries().find_map(|e| self.move_at_entry(pos.file, e)),
        }
    }

    /// The first row of the other end of move `mv`.
    pub fn move_target(&self, mv: u32, from_side: bool) -> Option<Pos> {
        let m = self.moves.get(mv as usize)?;
        let (file, entries) = if from_side { &m.to } else { &m.from };
        let rows = self.files.get(*file)?.rows();
        let row = rows
            .iter()
            .position(|r| r.entries().any(|e| e == entries.start))?;
        Some(Pos { file: *file, row })
    }

    /// Enables or disables "since my last review" (once hashes are known).
    pub fn set_since(&mut self, hashes: Option<HashSet<String>>, active: bool) {
        self.since = hashes;
        self.since_active = active && self.since.is_some();
        self.rebuild_all();
    }

    /// Shows or hides only what's new, with the hashes already known.
    /// False if they aren't known yet.
    pub fn show_since(&mut self, active: bool) -> bool {
        if self.since.is_none() {
            return false;
        }
        self.since_active = active;
        self.rebuild_all();
        true
    }

    /// Opens a folded block (formatting-only or seen).
    pub fn unfold(&mut self, pos: Pos) -> bool {
        let Some(Row::Fold { block, .. }) = self.row(pos) else {
            return false;
        };
        let Some(file) = self.files.get_mut(pos.file) else {
            return false;
        };
        let Some(start) = file.blocks.get(block as usize).map(|b| b.entries.start) else {
            return false;
        };
        file.unfolded.insert(start);
        self.rebuild(pos.file);
        true
    }

    /// Whether the file has changes not in the diff at your last review.
    pub fn has_new_changes(&self, file: usize) -> bool {
        let Some(since) = &self.since else {
            return true;
        };
        self.files
            .get(file)
            .is_some_and(|f| f.blocks.iter().any(|b| !since.contains(&b.hash)))
    }

    /// Opens or closes a thread.
    pub fn toggle_thread(&mut self, index: u32) {
        let Some(ann) = self.inputs.annotations.get(index as usize) else {
            return;
        };
        let open = is_open(&self.thread_open, ann);
        self.thread_open.insert(ann.key.clone(), !open);
        if let Some(file) = self.files.iter().position(|f| f.meta.path() == ann.path) {
            self.rebuild(file);
        }
    }

    /// Where comments on `file` can go: GitHub's patch if we have it,
    /// otherwise the local reconstruction.
    pub fn commentable(&self, file: usize) -> Option<&Commentable> {
        let f = self.files.get(file)?;
        self.inputs
            .patches
            .get(f.meta.path())
            .or(f.local_commentable.as_ref())
    }

    /// The line a row comments on: removed lines on the left, everything
    /// else on the right (in split view, the right half unless empty).
    pub fn line_at(&self, pos: Pos) -> Option<LinePos> {
        let file = self.files.get(pos.file)?;
        let text = file.text()?;
        let lines = text.lines(self.opts.whitespace);
        let at = |e: u32| {
            let l = lines.get(e as usize)?;
            let (left, right) = sides(l);
            if l.kind == LineKind::Removed {
                left
            } else {
                right
            }
        };
        match self.row(pos)? {
            Row::Line(e) => at(e),
            Row::Split { left, right } => right.or(left).and_then(at),
            _ => None,
        }
    }

    /// The annotation a row belongs to: a thread row's own, or the first
    /// one anchored on a line row.
    pub fn annotation_at(&self, pos: Pos) -> Option<u32> {
        let file = self.files.get(pos.file)?;
        match self.row(pos)? {
            Row::Thread(t) => file.thread_row(t).map(|r| r.ann),
            row => row
                .entries()
                .flat_map(|e| file.entry_lines(e, self.opts.whitespace))
                .find_map(|pos| file.annotations_at(pos).first().copied()),
        }
    }

    /// Where annotation `ann` (an index into the annotations) first shows:
    /// its line, or its first thread row (`None` while its file isn't
    /// diffed, or is collapsed).
    pub fn annotation_pos(&self, ann: u32) -> Option<Pos> {
        (0..self.total)
            .map(|g| self.to_pos(g))
            .find(|p| self.annotation_at(*p) == Some(ann))
    }

    /// Whether a row is where an unresolved thread starts: its first row,
    /// or the line of a collapsed one.
    fn is_open_thread_start(&self, pos: Pos) -> bool {
        let Some(file) = self.files.get(pos.file) else {
            return false;
        };
        let unresolved = |i: u32| {
            self.annotations()
                .get(i as usize)
                .is_some_and(|a| !a.resolved)
        };
        match self.row(pos) {
            Some(Row::Thread(t)) => file.thread_row(t).is_some_and(|r| {
                matches!(
                    r.kind,
                    ThreadRowKind::Summary | ThreadRowKind::Head { first: true, .. }
                ) && unresolved(r.ann)
            }),
            Some(Row::Line(_) | Row::Split { .. }) => {
                // Collapsed threads only show as a gutter marker.
                let next_is_thread = matches!(
                    self.row(Pos {
                        file: pos.file,
                        row: pos.row + 1
                    }),
                    Some(Row::Thread(_))
                );
                !next_is_thread && self.annotation_at(pos).is_some_and(unresolved)
            }
            _ => false,
        }
    }

    pub fn next_thread(&self, pos: Pos) -> Option<Pos> {
        self.find_after(pos, |p| self.is_open_thread_start(p))
    }

    pub fn prev_thread(&self, pos: Pos) -> Option<Pos> {
        self.find_before(pos, |p| self.is_open_thread_start(p))
    }

    fn reindex(&mut self) {
        self.starts.clear();
        let mut total = 0;
        for file in &self.files {
            self.starts.push(total);
            total += file.rows.len();
        }
        self.total = total;
    }

    pub fn set_diff(&mut self, index: usize, diff: Arc<FileDiff>) {
        self.update(index, |file| {
            file.local_commentable = match &diff.content {
                Content::Text(t) => Some(Commentable {
                    ranges: t.local_ranges.clone(),
                    source: RangeSource::LocalFallback,
                }),
                _ => None,
            };
            file.diff = Some(diff);
        });
    }

    /// Changes view options. Expansion windows index the alignment, so they
    /// reset when the whitespace mode changes.
    pub fn set_options(&mut self, opts: ViewOptions) {
        if opts.whitespace != self.opts.whitespace {
            for file in &mut self.files {
                file.windows.clear();
            }
        }
        self.opts = opts;
        self.rebuild_all();
    }

    /// Shows or hides a collapsed (generated or viewed) file.
    pub fn toggle_expanded(&mut self, index: usize) {
        self.update(index, |file| file.expanded = !file.expanded);
    }

    pub fn toggle_full(&mut self, index: usize) {
        self.update(index, |file| file.full = !file.full);
    }

    /// Reveals more context at `pos`: a gap row opens up to [`EXPAND_STEP`]
    /// lines from each of its ends (all of it if small); anywhere in a
    /// segment widens that segment by [`EXPAND_STEP`] on both sides.
    #[expect(clippy::single_range_in_vec_init, reason = "a list of one window")]
    pub fn expand(&mut self, pos: Pos) {
        let Some(row) = self.row(pos) else { return };
        let whitespace = self.opts.whitespace;
        let Some(file) = self.files.get_mut(pos.file) else {
            return;
        };
        let Some(text) = file.text() else { return };
        let len = idx(text.lines(whitespace).len());
        let step = EXPAND_STEP;
        let windows = match row {
            Row::Gap { start, end } if end - start <= 2 * step => vec![start..end],
            Row::Gap { start, end } => vec![start..start + step, end - step..end],
            _ => {
                let rows = &file.rows;
                // Rows of the segment around `pos`.
                let begin = rows
                    .get(..=pos.row)
                    .unwrap_or_default()
                    .iter()
                    .rposition(|r| matches!(r, Row::Hunk { .. }))
                    .unwrap_or(0);
                let end = rows
                    .get(begin + 1..)
                    .unwrap_or_default()
                    .iter()
                    .position(|r| matches!(r, Row::Gap { .. } | Row::Spacer | Row::Hunk { .. }))
                    .map_or(rows.len(), |p| p + begin + 1);
                let entries: Vec<u32> = rows
                    .get(begin..end)
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|r| r.entries().next())
                    .collect();
                let (Some(first), Some(last)) = (entries.iter().min(), entries.iter().max()) else {
                    return;
                };
                vec![
                    first.saturating_sub(step)..*first,
                    last + 1..(last + 1 + step).min(len),
                ]
            }
        };
        file.windows
            .extend(windows.into_iter().filter(|w| w.start < w.end));
        self.rebuild(pos.file);
    }

    pub fn anchor(&self, pos: Pos) -> Anchor {
        let pos = self.clamp(pos);
        let line = self
            .files
            .get(pos.file)
            .zip(self.row(pos))
            .and_then(|(f, r)| f.row_line(r, self.opts.whitespace));
        Anchor {
            file: pos.file,
            line,
            row: pos.row,
        }
    }

    /// The row showing the anchored line, or the closest one after it.
    pub fn locate(&self, anchor: Anchor) -> Pos {
        let Some(file) = self.files.get(anchor.file) else {
            return self.clamp(Pos::default());
        };
        let Some((new_side, line)) = anchor.line else {
            return self.clamp(Pos {
                file: anchor.file,
                row: anchor.row.min(1),
            });
        };
        let mut best: Option<(u32, usize)> = None;
        for (i, row) in file.rows.iter().enumerate() {
            if matches!(row, Row::Hunk { .. }) {
                continue;
            }
            let Some((side, l)) = file.row_line(*row, self.opts.whitespace) else {
                continue;
            };
            if side == new_side && l == line {
                return Pos {
                    file: anchor.file,
                    row: i,
                };
            }
            if side == new_side && l > line && best.is_none_or(|(b, _)| l < b) {
                best = Some((l, i));
            }
        }
        Pos {
            file: anchor.file,
            row: best.map_or(0, |(_, i)| i),
        }
    }

    pub fn total_rows(&self) -> usize {
        self.total
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn row(&self, pos: Pos) -> Option<Row> {
        self.files.get(pos.file)?.rows.get(pos.row).copied()
    }

    /// The scopes the line at `pos` (or the next line below it) is in,
    /// outermost first: what the sticky header names.
    pub fn scope_at(&self, pos: Pos) -> Vec<&str> {
        let Some(file) = self.files.get(pos.file) else {
            return Vec::new();
        };
        let (Some(text), Some(e)) = (
            file.text(),
            file.rows
                .iter()
                .skip(pos.row)
                .find_map(|r| r.entries().next()),
        ) else {
            return Vec::new();
        };
        new_line_near(text.lines(self.opts.whitespace), e as usize)
            .map(|n| text.scope(n))
            .unwrap_or_default()
    }

    /// Clamps a position to an existing row.
    pub fn clamp(&self, pos: Pos) -> Pos {
        let Some(last) = self.files.len().checked_sub(1) else {
            return Pos::default();
        };
        let file = pos.file.min(last);
        let rows = self.files.get(file).map_or(0, |f| f.rows.len());
        let row = pos.row.min(rows.saturating_sub(1));
        Pos { file, row }
    }

    pub fn to_global(&self, pos: Pos) -> usize {
        let pos = self.clamp(pos);
        self.starts.get(pos.file).map_or(0, |s| s + pos.row)
    }

    pub fn to_pos(&self, global: usize) -> Pos {
        if self.files.is_empty() {
            return Pos::default();
        }
        let global = global.min(self.total.saturating_sub(1));
        let Some(file) = self.starts.partition_point(|&s| s <= global).checked_sub(1) else {
            return Pos::default();
        };
        let start = self.starts.get(file).copied().unwrap_or_default();
        Pos {
            file,
            row: global - start,
        }
    }

    /// Moves by `delta` rows, clamped to the document.
    pub fn offset(&self, pos: Pos, delta: isize) -> Pos {
        self.to_pos(self.to_global(pos).saturating_add_signed(delta))
    }

    pub fn last(&self) -> Pos {
        self.to_pos(self.total.saturating_sub(1))
    }

    fn find_after(&self, pos: Pos, pred: impl Fn(Pos) -> bool) -> Option<Pos> {
        let start = self.to_global(pos) + 1;
        (start..self.total)
            .map(|g| self.to_pos(g))
            .find(|p| pred(*p))
    }

    fn find_before(&self, pos: Pos, pred: impl Fn(Pos) -> bool) -> Option<Pos> {
        let start = self.to_global(pos);
        (0..start).rev().map(|g| self.to_pos(g)).find(|p| pred(*p))
    }

    pub fn next_hunk(&self, pos: Pos) -> Option<Pos> {
        self.find_after(pos, |p| matches!(self.row(p), Some(Row::Hunk { .. })))
    }

    pub fn prev_hunk(&self, pos: Pos) -> Option<Pos> {
        self.find_before(pos, |p| matches!(self.row(p), Some(Row::Hunk { .. })))
    }

    pub fn next_file(&self, pos: Pos) -> Option<Pos> {
        (pos.file + 1 < self.files.len()).then(|| Pos {
            file: pos.file + 1,
            row: 0,
        })
    }

    pub fn prev_file(&self, pos: Pos) -> Option<Pos> {
        if pos.row > 0 {
            return Some(Pos {
                file: pos.file,
                row: 0,
            });
        }
        pos.file.checked_sub(1).map(|file| Pos { file, row: 0 })
    }

    /// The next file (wrapping) not marked viewed.
    pub fn next_unviewed(&self, pos: Pos) -> Option<Pos> {
        let n = self.files.len();
        (1..=n)
            .map(|k| (pos.file + k) % n)
            .find(|&f| {
                self.files
                    .get(f)
                    .is_some_and(|f| f.viewed != Viewed::Viewed)
            })
            .map(|file| Pos { file, row: 0 })
    }

    /// Files whose diffs haven't arrived, among `count` rows from `top`.
    pub fn loading_in_view(&self, top: Pos, count: usize) -> Vec<usize> {
        let first = self.to_global(top);
        let end = (first + count).min(self.total);
        let mut out = Vec::new();
        for (i, (file, &start)) in self.files.iter().zip(&self.starts).enumerate() {
            let shown = start < end && start + file.rows.len() > first;
            if shown && file.diff.is_none() && !file.collapsed() {
                out.push(i);
            }
        }
        out
    }

    /// Added and removed lines across loaded files, under the current
    /// whitespace mode.
    pub fn totals(&self) -> (u32, u32) {
        self.files
            .iter()
            .map(|f| self.file_counts(f))
            .fold((0, 0), |(a, d), (fa, fd)| (a + fa, d + fd))
    }

    pub fn file_counts(&self, file: &DocFile) -> (u32, u32) {
        match (file.text(), file.diff.as_deref()) {
            (Some(text), _) => counts(text.lines(self.opts.whitespace)),
            (None, Some(d)) => (d.additions, d.deletions),
            (None, None) => (0, 0),
        }
    }

    pub fn ready_count(&self) -> usize {
        self.files.iter().filter(|f| f.diff.is_some()).count()
    }

    pub fn viewed_count(&self) -> usize {
        self.files
            .iter()
            .filter(|f| f.viewed == Viewed::Viewed)
            .count()
    }

    /// The block under `pos`, if it's on a changed line.
    pub fn block_at(&self, pos: Pos) -> Option<&Block> {
        let file = self.files.get(pos.file)?;
        file.block_of(self.row(pos)?.entries().next()?)
    }

    /// Searchable text of a row.
    pub fn row_text(&self, pos: Pos) -> String {
        let Some(file) = self.files.get(pos.file) else {
            return String::new();
        };
        let lines = file.text().map(|t| (t, t.lines(self.opts.whitespace)));
        let entry = |e: u32| {
            lines
                .and_then(|(t, l)| l.get(e as usize).map(|line| t.text(line)))
                .unwrap_or_default()
        };
        match self.row(pos) {
            Some(Row::Header) => file.meta.path().to_owned(),
            Some(Row::Line(e)) => entry(e).to_owned(),
            Some(Row::Split { left, right }) => {
                let left = left.map(entry).unwrap_or_default();
                let right = right.map(entry).unwrap_or_default();
                format!("{left}\n{right}")
            }
            _ => String::new(),
        }
    }

    /// Whether a row's text contains `query`, as [`Doc::search`] matches.
    fn matcher(&self, query: &str) -> impl Fn(Pos) -> bool {
        let smart_case = query.chars().any(char::is_uppercase);
        let needle = if smart_case {
            query.to_owned()
        } else {
            query.to_lowercase()
        };
        move |pos| {
            let text = self.row_text(pos);
            if smart_case {
                text.contains(&needle)
            } else {
                text.to_lowercase().contains(&needle)
            }
        }
    }

    /// The next row (after `from`, wrapping) whose text contains `query`.
    /// Case-insensitive unless the query has uppercase letters.
    pub fn search(&self, query: &str, from: Pos, forward: bool) -> Option<Pos> {
        if query.is_empty() || self.total == 0 {
            return None;
        }
        let matches = self.matcher(query);
        let start = self.to_global(from);
        let n = self.total;
        (1..=n)
            .map(|k| {
                if forward {
                    (start + k) % n
                } else {
                    (start + n - k % n) % n
                }
            })
            .map(|g| self.to_pos(g))
            .find(|p| matches(*p))
    }

    /// How many rows match `query`.
    pub fn count_matches(&self, query: &str) -> usize {
        if query.is_empty() {
            return 0;
        }
        let matches = self.matcher(query);
        (0..self.total).filter(|g| matches(self.to_pos(*g))).count()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ghtui_git::files::{FileStatus, ZERO_OID};

    pub(crate) fn changed(path: &str) -> ChangedFile {
        ChangedFile {
            status: FileStatus::Modified,
            old_path: Some(path.into()),
            new_path: Some(path.into()),
            old_mode: 0o100644,
            new_mode: 0o100644,
            old_oid: ghtui_git::Oid::new(ZERO_OID),
            new_oid: ghtui_git::Oid::new(ZERO_OID),
            similarity: None,
        }
    }

    fn numbered(n: u32) -> String {
        (1..=n).map(|i| format!("line {i}\n")).collect()
    }

    fn compute(path: &str, old: &str, new: &str) -> Arc<FileDiff> {
        Arc::new(FileDiff::compute(
            path,
            Some(old.as_bytes()),
            Some(new.as_bytes()),
        ))
    }

    fn doc() -> Doc {
        let mut doc = Doc::new(
            vec![changed("a.txt"), changed("Cargo.lock"), changed("b.txt")],
            &HashSet::new(),
            Default::default(),
        );
        let old = numbered(60);
        let new = old
            .replace("line 5\n", "five\n")
            .replace("line 45\n", "forty-five\n");
        doc.set_diff(0, compute("a.txt", &old, &new));
        doc
    }

    /// A document of one file, `old` changed to `new`.
    fn one(path: &str, old: &str, new: &str) -> Doc {
        let mut doc = Doc::new(vec![changed(path)], &HashSet::new(), Default::default());
        doc.set_diff(0, compute(path, old, new));
        doc
    }

    fn view(split: bool, whitespace: Whitespace) -> ViewOptions {
        ViewOptions {
            split,
            whitespace,
            wrap: 0,
        }
    }

    /// The first row whose (first) line matches.
    fn line_row(file: &DocFile, matches: impl Fn(&DiffLine) -> bool) -> Option<usize> {
        let lines = &file.text()?.lines;
        file.rows().iter().position(|r| {
            r.entries()
                .next()
                .is_some_and(|e| matches(&lines[e as usize]))
        })
    }

    fn kinds(file: &DocFile) -> String {
        file.rows()
            .iter()
            .map(|r| match r {
                Row::Header => 'H',
                Row::Note(_) => 'N',
                Row::Gap { .. } => 'G',
                Row::Hunk { .. } => '@',
                Row::Line(_) => 'L',
                Row::Split { .. } => 'S',
                Row::NoNewline => '\\',
                Row::Thread(_) => 'T',
                Row::Fold { .. } => 'F',
                Row::Moved { .. } => 'M',
                Row::Spacer => '_',
            })
            .collect()
    }

    #[test]
    fn rows_with_gaps_between_hunks() {
        let doc = doc();
        let a = &doc.files[0];
        // Leading gap (line 1), hunk 2–8, gap, hunk 42–48, trailing gap.
        assert_eq!(kinds(a), "HG@LLLLLLLLG@LLLLLLLLG_");
        let ranges: Vec<&str> = a.headers().iter().map(|h| h.range.as_str()).collect();
        assert_eq!(ranges, ["@@ -2,7 +2,7 @@", "@@ -42,7 +42,7 @@"]);
        assert!(doc.files[1].generated, "lockfiles are collapsed");
        assert_eq!(doc.files[1].rows()[1], Row::Note(Note::Collapsed));
        assert_eq!(doc.files[2].rows()[1], Row::Note(Note::Loading));
    }

    #[test]
    fn global_and_local_positions_round_trip() {
        let doc = doc();
        for g in 0..doc.total_rows() {
            assert_eq!(doc.to_global(doc.to_pos(g)), g);
        }
        assert_eq!(doc.to_pos(usize::MAX), doc.last());
    }

    #[test]
    fn positions_survive_earlier_files_arriving() {
        let mut doc = doc();
        let cursor = Pos { file: 2, row: 1 };
        let before = doc.to_global(cursor);
        doc.toggle_expanded(1);
        doc.set_diff(1, compute("Cargo.lock", "a\n", "b\nc\n"));
        assert_eq!(doc.row(cursor), Some(Row::Note(Note::Loading)));
        assert_ne!(doc.to_global(cursor), before);
    }

    #[test]
    fn hunk_file_and_unviewed_navigation() {
        let mut doc = doc();
        let h1 = doc.next_hunk(Pos::default()).unwrap();
        let h2 = doc.next_hunk(h1).unwrap();
        assert_eq!(doc.row(h1), Some(Row::Hunk { seg: 0 }));
        assert_eq!(doc.row(h2), Some(Row::Hunk { seg: 1 }));
        assert_eq!(doc.next_hunk(h2), None);
        assert_eq!(doc.prev_hunk(h2), Some(h1));
        assert_eq!(doc.next_file(h2), Some(Pos { file: 1, row: 0 }));
        assert_eq!(doc.prev_file(h2), Some(Pos { file: 0, row: 0 }));

        let viewed = [("Cargo.lock".to_owned(), Viewed::Viewed)].into();
        doc.set_inputs(DocInputs {
            viewed,
            ..DocInputs::default()
        });
        assert_eq!(doc.files[1].rows()[1], Row::Note(Note::Viewed));
        assert_eq!(
            doc.next_unviewed(Pos::default()),
            Some(Pos { file: 2, row: 0 })
        );
        assert_eq!(
            doc.next_unviewed(Pos { file: 2, row: 0 }),
            Some(Pos { file: 0, row: 0 })
        );
    }

    #[test]
    fn expanding_gaps_and_segments() {
        let old = numbered(200);
        let new = old
            .replace("line 2\n", "two\n")
            .replace("line 150\n", "x\n");
        let mut doc = one("big.txt", &old, &new);
        // The leading one-line gap opens completely.
        doc.expand(Pos { file: 0, row: 1 });
        assert!(kinds(&doc.files[0]).starts_with("H@L"));
        // A long gap opens from both ends, leaving a smaller gap.
        let middle = doc.files[0]
            .rows()
            .iter()
            .position(|r| matches!(r, Row::Gap { start, end } if end - start > 2 * EXPAND_STEP))
            .unwrap();
        let before = doc.files[0].rows().len();
        doc.expand(Pos {
            file: 0,
            row: middle,
        });
        assert_eq!(doc.files[0].rows().len(), before + 2 * EXPAND_STEP as usize);
        // Full file shows every line in one segment.
        doc.toggle_full(0);
        assert_eq!(doc.files[0].headers().len(), 1);
        assert!(
            !doc.files[0]
                .rows()
                .iter()
                .any(|r| matches!(r, Row::Gap { .. }))
        );
    }

    #[test]
    fn expanding_from_inside_a_segment_widens_it() {
        let mut doc = doc();
        let before = doc.files[0].rows().len();
        doc.expand(Pos { file: 0, row: 4 });
        assert!(doc.files[0].rows().len() > before);
    }

    #[test]
    fn split_rows_pair_changes() {
        let mut doc = one("x.rs", "a\nb\nc\nd\n", "a\nB\nC\nX\nd\n");
        doc.set_options(view(true, Whitespace::Exact));
        let rows: Vec<Row> = doc.files[0]
            .rows()
            .iter()
            .copied()
            .filter(|r| matches!(r, Row::Split { .. }))
            .collect();
        // a|a, b|B, c|C, _|X, d|d.
        assert_eq!(rows.len(), 5);
        assert!(matches!(
            rows[1],
            Row::Split {
                left: Some(_),
                right: Some(_)
            }
        ));
        assert!(matches!(
            rows[3],
            Row::Split {
                left: None,
                right: Some(_)
            }
        ));
    }

    #[test]
    fn whitespace_mode_hides_whitespace_only_changes() {
        let mut doc = one("x.rs", "fn a() {\n  x();\n}\n", "fn a() {\n    x();\n}\n");
        assert_eq!(doc.totals(), (1, 1));
        doc.set_options(view(false, Whitespace::Ignore));
        assert_eq!(doc.totals(), (0, 0));
        assert_eq!(doc.files[0].rows()[1], Row::Note(Note::NoChanges));
    }

    #[test]
    fn anchors_keep_the_line_across_view_changes() {
        let mut doc = doc();
        let row = line_row(&doc.files[0], |l| l.new == Some(44)).unwrap();
        let anchor = doc.anchor(Pos { file: 0, row });
        doc.set_options(view(true, Whitespace::Exact));
        let pos = doc.locate(anchor);
        assert!(
            doc.row_text(pos).contains("line 44"),
            "{}",
            doc.row_text(pos)
        );
    }

    #[test]
    fn reviewed_blocks_hash_content_not_position() {
        let hash = |path, old, new| one(path, old, new).files[0].blocks()[0].hash.clone();
        let base = hash("x.rs", "1\n2\n3\nold\n", "1\n2\n3\nnew\n");
        assert_eq!(
            base,
            hash("x.rs", "0\n1\n2\n3\nold\n", "0\n1\n2\n3\nnew\n"),
            "moved"
        );
        assert_ne!(
            base,
            hash("x.rs", "1\n2\n3\nold\n", "1\n2\n3\nnewer\n"),
            "edited"
        );
        assert_ne!(
            base,
            hash("y.rs", "1\n2\n3\nold\n", "1\n2\n3\nnew\n"),
            "other file"
        );
    }

    #[test]
    fn search_wraps_and_respects_smart_case() {
        let doc = doc();
        let first = doc.search("five", Pos::default(), true).unwrap();
        assert!(doc.row_text(first).contains("five"));
        let second = doc.search("five", first, true).unwrap();
        assert!(doc.row_text(second).contains("forty-five"));
        assert_eq!(doc.search("five", second, true), Some(first), "wraps");
        assert_eq!(doc.search("five", first, false), Some(second), "backwards");
        assert_eq!(doc.search("FIVE", Pos::default(), true), None);
        assert_eq!(doc.count_matches("five"), 2);
    }

    #[test]
    fn missing_newline_rows() {
        let mut doc = one("x.txt", "a\nb\n", "a\nb");
        let count = |doc: &Doc| {
            doc.files[0]
                .rows()
                .iter()
                .filter(|r| **r == Row::NoNewline)
                .count()
        };
        assert_eq!(count(&doc), 1);
        doc.set_options(view(true, Whitespace::Exact));
        assert_eq!(count(&doc), 1);
    }

    #[test]
    fn loading_files_in_view() {
        let doc = doc();
        assert_eq!(doc.loading_in_view(Pos::default(), 1000), [2]);
        assert!(doc.loading_in_view(Pos::default(), 3).is_empty());
    }

    mod annotations {
        use super::*;
        use crate::annotations::{Annotation, AnnotationComment, AnnotationKey};

        fn ann(key: &str, side: Side, line: Option<u32>) -> Annotation {
            Annotation {
                key: AnnotationKey::Thread(ghtui_api::model::NodeId::new(key)),
                path: "a.txt".into(),
                side,
                line,
                start_line: None,
                original_line: line,
                resolved: false,
                outdated: false,
                moved: false,
                file_level: line.is_none(),
                comments: vec![AnnotationComment {
                    author: "alice".into(),
                    body: "Is this right?\nSecond line.".into(),
                    created_at: String::new(),
                    pending: false,
                }],
                left_out: 0,
                error: None,
                can_reply: true,
                can_resolve: true,
                can_unresolve: false,
            }
        }

        fn set_annotations(doc: &mut Doc, annotations: Vec<Annotation>) {
            doc.set_inputs(DocInputs {
                annotations,
                ..DocInputs::default()
            });
        }

        fn rows_of(doc: &Doc) -> String {
            kinds(&doc.files[0])
        }

        #[test]
        fn threads_sit_under_their_line_or_the_header() {
            let mut doc = doc();
            set_annotations(
                &mut doc,
                vec![
                    ann("line", Side::Right, Some(5)),
                    ann("file", Side::Right, None),
                ],
            );
            let file = &doc.files[0];
            // File-level thread right after the header (head, 2 body, footer).
            assert!(rows_of(&doc).starts_with("HTTTT"), "{}", rows_of(&doc));
            // The line thread follows the row showing new line 5.
            let row = line_row(file, |l| l.new == Some(5)).unwrap();
            assert!(matches!(file.rows()[row + 1], Row::Thread(_)));
            assert_eq!(doc.annotation_at(Pos { file: 0, row }), Some(0));
        }

        /// A long thread says how many replies between its first comment
        /// and its newest weren't fetched.
        #[test]
        fn a_long_threads_left_out_replies_show() {
            let mut doc = doc();
            let mut long = ann("file", Side::Right, None);
            long.left_out = 120;
            set_annotations(&mut doc, vec![long]);
            let file = &doc.files[0];
            let texts: Vec<(ThreadRowKind, String)> = file
                .rows()
                .iter()
                .filter_map(|r| match r {
                    Row::Thread(t) => file
                        .thread_row(*t)
                        .map(|t| (t.kind.clone(), t.text.clone())),
                    _ => None,
                })
                .collect();
            // After the first comment's body, before the footer.
            assert_eq!(
                texts.get(3),
                Some(&(
                    ThreadRowKind::Gap,
                    "… 120 more replies on GitHub (o)".to_owned()
                )),
                "{texts:?}"
            );
        }

        #[test]
        fn commented_lines_are_always_visible() {
            let mut doc = doc();
            // Line 30 is far from both changes (5 and 45): normally hidden.
            let visible = |doc: &Doc| line_row(&doc.files[0], |l| l.new == Some(30)).is_some();
            assert!(!visible(&doc));
            set_annotations(&mut doc, vec![ann("far", Side::Right, Some(30))]);
            assert!(visible(&doc));
        }

        #[test]
        fn resolved_threads_start_collapsed_and_toggle() {
            let mut doc = doc();
            let mut resolved = ann("r", Side::Right, Some(5));
            resolved.resolved = true;
            set_annotations(&mut doc, vec![resolved]);
            assert!(!rows_of(&doc).contains('T'), "collapsed: marker only");
            doc.toggle_thread(0);
            assert!(rows_of(&doc).contains("TTTT"));
            doc.toggle_thread(0);
            assert!(!rows_of(&doc).contains('T'));
        }

        #[test]
        fn next_thread_skips_resolved_ones() {
            let mut doc = doc();
            let mut resolved = ann("r", Side::Right, Some(5));
            resolved.resolved = true;
            set_annotations(&mut doc, vec![resolved, ann("open", Side::Right, Some(45))]);
            let first = doc.next_thread(Pos::default()).unwrap();
            assert_eq!(
                doc.files[0]
                    .thread_row(match doc.row(first) {
                        Some(Row::Thread(t)) => t,
                        other => panic!("{other:?}"),
                    })
                    .unwrap()
                    .ann,
                1
            );
            assert_eq!(doc.next_thread(first), None);
            assert_eq!(doc.prev_thread(doc.last()), Some(first));
        }

        #[test]
        fn left_side_threads_attach_to_removed_lines_in_split_view() {
            let mut doc = doc();
            doc.set_options(ViewOptions {
                wrap: 40,
                ..view(true, Whitespace::Exact)
            });
            set_annotations(&mut doc, vec![ann("old", Side::Left, Some(5))]);
            let file = &doc.files[0];
            let at = line_row(file, |l| l.old == Some(5)).unwrap();
            assert!(matches!(file.rows()[at + 1], Row::Thread(_)));
        }

        #[test]
        fn line_positions_for_comments() {
            let doc = doc();
            let file = &doc.files[0];
            let removed = line_row(file, |l| l.kind == LineKind::Removed).unwrap();
            let pos = doc
                .line_at(Pos {
                    file: 0,
                    row: removed,
                })
                .unwrap();
            assert_eq!((pos.side, pos.line), (Side::Left, 5));
            assert_eq!(
                doc.line_at(Pos {
                    file: 0,
                    row: removed + 1
                })
                .unwrap()
                .side,
                Side::Right
            );
            assert_eq!(
                doc.line_at(Pos { file: 0, row: 0 }),
                None,
                "headers aren't lines"
            );
            let c = doc.commentable(0).unwrap();
            assert!(c.is_commentable(pos));
        }
    }

    mod better {
        use super::*;
        use ghtui_diff::moves::detect_moves;

        #[test]
        fn formatting_only_blocks_fold_and_unfold() {
            let mut doc = one(
                "x.rs",
                "a\nfoo(\n    x,\n    y\n);\nb\n",
                "a\nfoo(x, y);\nb\n",
            );
            assert_eq!(kinds(&doc.files[0]), "H@LFL_");
            let formatting = Row::Fold {
                block: 0,
                reason: FoldReason::Formatting,
            };
            let fold = doc.files[0]
                .rows()
                .iter()
                .position(|r| *r == formatting)
                .unwrap();
            assert!(doc.unfold(Pos { file: 0, row: fold }));
            assert!(!kinds(&doc.files[0]).contains('F'));
        }

        #[test]
        fn since_review_shows_only_new_changes() {
            let mut doc = doc();
            // At the last review, only the change at line 5 existed.
            let base = numbered(60);
            let mut old = one("a.txt", &base, &base.replace("line 5\n", "five\n"));
            let seen: HashSet<String> = old.files[0]
                .blocks()
                .iter()
                .map(|b| b.hash.clone())
                .collect();

            doc.set_since(Some(seen.clone()), true);
            let rows = kinds(&doc.files[0]);
            // The old change (line 5) is gone from view; the new one (45) shows.
            let shows = |n: u32| line_row(&doc.files[0], |l| l.new == Some(n)).is_some();
            assert!(shows(45), "{rows}");
            assert!(!shows(5), "{rows}");
            assert!(doc.has_new_changes(0));

            // A file with only seen changes says so.
            old.set_since(Some(seen), true);
            assert_eq!(old.files[0].rows()[1], Row::Note(Note::NothingNew));
            old.set_since(old.since.clone(), false);
            assert_ne!(old.files[0].rows()[1], Row::Note(Note::NothingNew));
        }

        #[test]
        fn moved_blocks_get_rows_and_jump_targets() {
            let block = "fn helper(x: u32) -> u32 {\n    let y = x * 2;\n    y + 1\n}\n";
            let mut doc = Doc::new(
                vec![changed("a.rs"), changed("b.rs")],
                &HashSet::new(),
                Default::default(),
            );
            doc.set_diff(
                0,
                compute(
                    "a.rs",
                    &format!("fn main() {{}}\n{block}"),
                    "fn main() {}\n",
                ),
            );
            doc.set_diff(1, compute("b.rs", "// b\n", &format!("// b\n{block}")));
            let moves = {
                let texts: Vec<_> = doc
                    .files
                    .iter()
                    .map(|f| f.text().unwrap().clone())
                    .collect();
                detect_moves(&[
                    (0, &texts[0], texts[0].lines(Whitespace::Exact)),
                    (1, &texts[1], texts[1].lines(Whitespace::Exact)),
                ])
            };
            assert_eq!(moves.len(), 1);
            doc.set_moves(moves);
            let moved_from = doc.files[0]
                .rows()
                .iter()
                .position(|r| matches!(r, Row::Moved { from: true, .. }))
                .unwrap();
            let moved_to = doc.files[1]
                .rows()
                .iter()
                .position(|r| matches!(r, Row::Moved { from: false, .. }))
                .unwrap();
            let (mv, from) = doc
                .move_at(Pos {
                    file: 0,
                    row: moved_from,
                })
                .unwrap();
            let target = doc.move_target(mv, from).unwrap();
            assert_eq!(target.file, 1);
            assert_eq!(target.row + 1, moved_to, "lands on the block's first line");
            // In whitespace-insensitive mode moves don't apply.
            doc.set_options(view(false, Whitespace::Ignore));
            assert!(!kinds(&doc.files[0]).contains('M'));
        }
    }
}
