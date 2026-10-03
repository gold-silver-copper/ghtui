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

use std::collections::HashSet;
use std::ops::Range;
use std::sync::Arc;

use ghtui_diff::{
    CONTEXT, Content, DiffLine, FileDiff, LineKind, TextDiff, Whitespace, counts, hunk, segments,
};
use ghtui_git::files::{ChangedFile, is_lockfile};

/// Lines revealed per expansion step.
pub const EXPAND_STEP: u32 = 20;

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
    Error,
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
    Spacer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Viewed {
    #[default]
    Unviewed,
    Viewed,
    /// Viewed, but the file changed since.
    Dismissed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ViewOptions {
    pub split: bool,
    pub whitespace: Whitespace,
}

/// A maximal run of changed lines: the unit marked "reviewed".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub entries: Range<u32>,
    /// Content hash (path and changed lines), stable across line moves.
    pub hash: String,
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
    /// `@@` text per visible segment.
    headers: Vec<String>,
    blocks: Vec<Block>,
}

impl DocFile {
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn headers(&self) -> &[String] {
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

    fn rebuild(&mut self, opts: ViewOptions) {
        self.rows.clear();
        self.headers.clear();
        self.blocks.clear();
        self.rows.push(Row::Header);
        if self.collapsed() {
            let note = if self.viewed == Viewed::Viewed {
                Note::Viewed
            } else {
                Note::Collapsed
            };
            self.rows.push(Row::Note(note));
            self.rows.push(Row::Spacer);
            return;
        }
        let note = match self.diff.as_deref().map(|d| &d.content) {
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
        let diff = self.diff.clone().expect("text diff present");
        let Content::Text(text) = &diff.content else {
            unreachable!("checked above")
        };
        let lines = text.lines(opts.whitespace);
        self.blocks = blocks(self.meta.path(), text, lines);

        let segs = segments(lines, CONTEXT, &self.windows, self.full);
        let (mut seen, mut before) = (0usize, (0u32, 0u32));
        let mut next_hidden = 0u32;
        for (s, seg) in segs.iter().enumerate() {
            if seg.start as u32 > next_hidden {
                self.rows.push(Row::Gap {
                    start: next_hidden,
                    end: seg.start as u32,
                });
            }
            let skipped = counts_in(&lines[seen..seg.start]);
            before = (before.0 + skipped.0, before.1 + skipped.1);
            seen = seg.start;
            self.headers.push(hunk(lines, seg.clone(), before).header());
            self.rows.push(Row::Hunk { seg: s as u32 });
            if opts.split {
                push_split_rows(&mut self.rows, text, lines, seg.clone());
            } else {
                for e in seg.clone() {
                    self.rows.push(Row::Line(e as u32));
                    if ends_without_newline(text, &lines[e]) {
                        self.rows.push(Row::NoNewline);
                    }
                }
            }
            next_hidden = seg.end as u32;
        }
        if (next_hidden as usize) < lines.len() {
            self.rows.push(Row::Gap {
                start: next_hidden,
                end: lines.len() as u32,
            });
        }
        self.rows.push(Row::Spacer);
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

fn counts_in(lines: &[DiffLine]) -> (u32, u32) {
    lines.iter().fold((0, 0), |(o, n), l| {
        (
            o + u32::from(l.old.is_some()),
            n + u32::from(l.new.is_some()),
        )
    })
}

fn ends_without_newline(text: &TextDiff, line: &DiffLine) -> bool {
    let old_end = line.old == Some(text.old.len() as u32)
        && text.old.missing_final_newline
        && line.kind != LineKind::Added;
    let new_end = line.new == Some(text.new.len() as u32)
        && text.new.missing_final_newline
        && line.kind != LineKind::Removed;
    old_end || new_end
}

/// Split rows: context lines pair with themselves; within a change, the
/// removed and added runs are zipped side by side.
fn push_split_rows(rows: &mut Vec<Row>, text: &TextDiff, lines: &[DiffLine], seg: Range<usize>) {
    let mut e = seg.start;
    while e < seg.end {
        if lines[e].kind == LineKind::Context {
            rows.push(Row::Split {
                left: Some(e as u32),
                right: Some(e as u32),
            });
            if ends_without_newline(text, &lines[e]) {
                rows.push(Row::NoNewline);
            }
            e += 1;
            continue;
        }
        let removed_start = e;
        while e < seg.end && lines[e].kind == LineKind::Removed {
            e += 1;
        }
        let added_start = e;
        while e < seg.end && lines[e].kind == LineKind::Added {
            e += 1;
        }
        let (removed, added) = (added_start - removed_start, e - added_start);
        let mut missing_newline = false;
        for i in 0..removed.max(added) {
            let left = (i < removed).then_some((removed_start + i) as u32);
            let right = (i < added).then_some((added_start + i) as u32);
            missing_newline |= [left, right]
                .into_iter()
                .flatten()
                .any(|x| ends_without_newline(text, &lines[x as usize]));
            rows.push(Row::Split { left, right });
        }
        if missing_newline {
            rows.push(Row::NoNewline);
        }
    }
}

/// Change blocks with content hashes.
fn blocks(path: &str, text: &TextDiff, lines: &[DiffLine]) -> Vec<Block> {
    let mut out = Vec::new();
    let mut e = 0;
    while e < lines.len() {
        if lines[e].kind == LineKind::Context {
            e += 1;
            continue;
        }
        let start = e;
        let mut hasher = Fnv::new();
        hasher.write(path.as_bytes());
        while e < lines.len() && lines[e].kind != LineKind::Context {
            let sign: &[u8] = if lines[e].kind == LineKind::Added {
                b"\n+"
            } else {
                b"\n-"
            };
            hasher.write(sign);
            hasher.write(text.text(&lines[e]).trim_end().as_bytes());
            e += 1;
        }
        out.push(Block {
            entries: start as u32..e as u32,
            hash: format!("{:016x}", hasher.0),
        });
    }
    out
}

/// FNV-1a: stable across builds and platforms (unlike `DefaultHasher`),
/// which matters because these hashes are persisted.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= u64::from(*b);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
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

#[derive(Debug, Clone, Default)]
pub struct Doc {
    pub files: Vec<DocFile>,
    pub opts: ViewOptions,
    /// Hashes of blocks marked reviewed.
    pub reviewed: HashSet<String>,
    starts: Vec<usize>,
    total: usize,
}

impl Doc {
    pub fn new(files: Vec<ChangedFile>, generated: &HashSet<String>) -> Self {
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
                }
            })
            .collect();
        let mut doc = Self {
            files,
            ..Self::default()
        };
        doc.rebuild_all();
        doc
    }

    fn rebuild_all(&mut self) {
        let opts = self.opts;
        for file in &mut self.files {
            file.rebuild(opts);
        }
        self.reindex();
    }

    fn rebuild(&mut self, index: usize) {
        let opts = self.opts;
        if let Some(file) = self.files.get_mut(index) {
            file.rebuild(opts);
            self.reindex();
        }
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
        if let Some(file) = self.files.get_mut(index) {
            file.diff = Some(diff);
            self.rebuild(index);
        }
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
        if let Some(file) = self.files.get_mut(index) {
            file.expanded = !file.expanded;
            self.rebuild(index);
        }
    }

    pub fn set_viewed(&mut self, index: usize, viewed: Viewed) {
        if let Some(file) = self.files.get_mut(index) {
            file.viewed = viewed;
            file.expanded = false;
            self.rebuild(index);
        }
    }

    pub fn toggle_full(&mut self, index: usize) {
        if let Some(file) = self.files.get_mut(index) {
            file.full = !file.full;
            self.rebuild(index);
        }
    }

    /// Reveals more context at `pos`: a gap row opens up to [`EXPAND_STEP`]
    /// lines from each of its ends (all of it if small); anywhere in a
    /// segment widens that segment by [`EXPAND_STEP`] on both sides.
    #[allow(clippy::single_range_in_vec_init, reason = "a list of one window")]
    pub fn expand(&mut self, pos: Pos) {
        let Some(row) = self.row(pos) else { return };
        let whitespace = self.opts.whitespace;
        let Some(file) = self.files.get_mut(pos.file) else {
            return;
        };
        let Some(text) = file.text() else { return };
        let len = text.lines(whitespace).len() as u32;
        let step = EXPAND_STEP;
        let windows = match row {
            Row::Gap { start, end } if end - start <= 2 * step => vec![start..end],
            Row::Gap { start, end } => vec![start..start + step, end - step..end],
            _ => {
                let rows = &file.rows;
                let entry_of = |r: &Row| match r {
                    Row::Line(e) => Some(*e),
                    Row::Split { left, right } => left.or(*right),
                    _ => None,
                };
                // Rows of the segment around `pos`.
                let begin = rows[..=pos.row]
                    .iter()
                    .rposition(|r| matches!(r, Row::Hunk { .. }))
                    .unwrap_or(0);
                let end = rows[begin + 1..]
                    .iter()
                    .position(|r| matches!(r, Row::Gap { .. } | Row::Spacer | Row::Hunk { .. }))
                    .map_or(rows.len(), |p| p + begin + 1);
                let entries: Vec<u32> = rows[begin..end].iter().filter_map(entry_of).collect();
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

    /// Clamps a position to an existing row.
    pub fn clamp(&self, pos: Pos) -> Pos {
        let Some(last) = self.files.len().checked_sub(1) else {
            return Pos::default();
        };
        let file = pos.file.min(last);
        let row = pos.row.min(self.files[file].rows.len() - 1);
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
        let file = self.starts.partition_point(|&s| s <= global) - 1;
        Pos {
            file,
            row: global - self.starts[file],
        }
    }

    /// Moves by `delta` rows, clamped to the document.
    pub fn offset(&self, pos: Pos, delta: isize) -> Pos {
        let global = self.to_global(pos) as isize + delta;
        self.to_pos(global.max(0) as usize)
    }

    pub fn last(&self) -> Pos {
        self.to_pos(self.total.saturating_sub(1))
    }

    fn find_after(&self, pos: Pos, pred: impl Fn(Row) -> bool) -> Option<Pos> {
        let start = self.to_global(pos) + 1;
        (start..self.total)
            .map(|g| self.to_pos(g))
            .find(|p| self.row(*p).is_some_and(&pred))
    }

    fn find_before(&self, pos: Pos, pred: impl Fn(Row) -> bool) -> Option<Pos> {
        let start = self.to_global(pos);
        (0..start)
            .rev()
            .map(|g| self.to_pos(g))
            .find(|p| self.row(*p).is_some_and(&pred))
    }

    pub fn next_hunk(&self, pos: Pos) -> Option<Pos> {
        self.find_after(pos, |r| matches!(r, Row::Hunk { .. }))
    }

    pub fn prev_hunk(&self, pos: Pos) -> Option<Pos> {
        self.find_before(pos, |r| matches!(r, Row::Hunk { .. }))
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
            .find(|&f| self.files[f].viewed != Viewed::Viewed)
            .map(|file| Pos { file, row: 0 })
    }

    /// Files whose diffs haven't arrived, among `count` rows from `top`.
    pub fn loading_in_view(&self, top: Pos, count: usize) -> Vec<usize> {
        let first = self.to_global(top);
        let mut out: Vec<usize> = Vec::new();
        for g in first..(first + count).min(self.total) {
            let pos = self.to_pos(g);
            let file = &self.files[pos.file];
            if file.diff.is_none() && !file.collapsed() && !out.contains(&pos.file) {
                out.push(pos.file);
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
        let entry = match self.row(pos)? {
            Row::Line(e) => e,
            Row::Split { left, right } => left.or(right)?,
            _ => return None,
        };
        file.block_of(entry)
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

    fn matches(&self, pos: Pos, needle: &str, smart_case: bool) -> bool {
        let text = self.row_text(pos);
        if smart_case {
            text.contains(needle)
        } else {
            text.to_lowercase().contains(needle)
        }
    }

    /// The next row (after `from`, wrapping) whose text contains `query`.
    /// Case-insensitive unless the query has uppercase letters.
    pub fn search(&self, query: &str, from: Pos, forward: bool) -> Option<Pos> {
        if query.is_empty() || self.total == 0 {
            return None;
        }
        let smart_case = query.chars().any(char::is_uppercase);
        let needle = if smart_case {
            query.to_owned()
        } else {
            query.to_lowercase()
        };
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
            .find(|p| self.matches(*p, &needle, smart_case))
    }

    /// How many rows match `query`.
    pub fn count_matches(&self, query: &str) -> usize {
        if query.is_empty() {
            return 0;
        }
        let smart_case = query.chars().any(char::is_uppercase);
        let needle = if smart_case {
            query.to_owned()
        } else {
            query.to_lowercase()
        };
        (0..self.total)
            .filter(|g| self.matches(self.to_pos(*g), &needle, smart_case))
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghtui_git::files::{FileStatus, ZERO_OID};

    fn changed(path: &str) -> ChangedFile {
        ChangedFile {
            status: FileStatus::Modified,
            old_path: Some(path.into()),
            new_path: Some(path.into()),
            old_mode: 0o100644,
            new_mode: 0o100644,
            old_oid: ZERO_OID.into(),
            new_oid: ZERO_OID.into(),
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
        );
        let old = numbered(60);
        let new = old
            .replace("line 5\n", "five\n")
            .replace("line 45\n", "forty-five\n");
        doc.set_diff(0, compute("a.txt", &old, &new));
        doc
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
        assert_eq!(a.headers(), ["@@ -2,7 +2,7 @@", "@@ -42,7 +42,7 @@"]);
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

        doc.set_viewed(1, Viewed::Viewed);
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
        let mut doc = Doc::new(vec![changed("big.txt")], &HashSet::new());
        let old = numbered(200);
        let new = old
            .replace("line 2\n", "two\n")
            .replace("line 150\n", "x\n");
        doc.set_diff(0, compute("big.txt", &old, &new));
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
        let mut doc = Doc::new(vec![changed("x.rs")], &HashSet::new());
        doc.set_diff(0, compute("x.rs", "a\nb\nc\nd\n", "a\nB\nC\nX\nd\n"));
        doc.set_options(ViewOptions {
            split: true,
            whitespace: Whitespace::Exact,
        });
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
        let mut doc = Doc::new(vec![changed("x.rs")], &HashSet::new());
        doc.set_diff(
            0,
            compute("x.rs", "fn a() {\n  x();\n}\n", "fn a() {\n    x();\n}\n"),
        );
        assert_eq!(doc.totals(), (1, 1));
        doc.set_options(ViewOptions {
            split: false,
            whitespace: Whitespace::Ignore,
        });
        assert_eq!(doc.totals(), (0, 0));
        assert_eq!(doc.files[0].rows()[1], Row::Note(Note::NoChanges));
    }

    #[test]
    fn anchors_keep_the_line_across_view_changes() {
        let mut doc = doc();
        let text = doc.files[0].text().unwrap().clone();
        let row = doc.files[0]
            .rows()
            .iter()
            .position(|r| matches!(r, Row::Line(e) if text.lines[*e as usize].new == Some(44)))
            .unwrap();
        let anchor = doc.anchor(Pos { file: 0, row });
        doc.set_options(ViewOptions {
            split: true,
            whitespace: Whitespace::Exact,
        });
        let pos = doc.locate(anchor);
        assert!(
            doc.row_text(pos).contains("line 44"),
            "{}",
            doc.row_text(pos)
        );
    }

    #[test]
    fn reviewed_blocks_hash_content_not_position() {
        let hash = |path: &str, old: &str, new: &str| {
            let mut doc = Doc::new(vec![changed(path)], &HashSet::new());
            doc.set_diff(0, compute(path, old, new));
            doc.files[0].blocks()[0].hash.clone()
        };
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
        let mut doc = Doc::new(vec![changed("x.txt")], &HashSet::new());
        doc.set_diff(0, compute("x.txt", "a\nb\n", "a\nb"));
        let count = |doc: &Doc| {
            doc.files[0]
                .rows()
                .iter()
                .filter(|r| **r == Row::NoNewline)
                .count()
        };
        assert_eq!(count(&doc), 1);
        doc.set_options(ViewOptions {
            split: true,
            whitespace: Whitespace::Exact,
        });
        assert_eq!(count(&doc), 1);
    }

    #[test]
    fn loading_files_in_view() {
        let doc = doc();
        assert_eq!(doc.loading_in_view(Pos::default(), 1000), [2]);
        assert!(doc.loading_in_view(Pos::default(), 3).is_empty());
    }
}
