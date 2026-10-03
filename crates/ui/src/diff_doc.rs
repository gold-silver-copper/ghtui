//! The diff document: every changed file as a sequence of display rows.
//!
//! Positions are `(file, row)` pairs rather than global row numbers, so the
//! cursor stays put when an earlier file's diff arrives and its row count
//! changes. Global indices are derived on demand from per-file offsets.

use std::collections::HashSet;
use std::sync::Arc;

use ghtui_diff::{Content, FileDiff, LineKind};
use ghtui_git::files::{ChangedFile, is_lockfile};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Note {
    Loading,
    Collapsed,
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
    Hunk(u32),
    Line {
        hunk: u32,
        line: u32,
    },
    /// "No newline at end of file" after the preceding line.
    NoNewline,
    Spacer,
}

#[derive(Debug, Clone)]
pub struct DocFile {
    pub meta: ChangedFile,
    /// `linguist-generated`, `linguist-vendored` or a lockfile.
    pub generated: bool,
    pub expanded: bool,
    pub diff: Option<Arc<FileDiff>>,
    rows: Vec<Row>,
}

impl DocFile {
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn collapsed(&self) -> bool {
        self.generated && !self.expanded
    }

    fn rebuild(&mut self) {
        let mut rows = vec![Row::Header];
        if self.collapsed() {
            rows.push(Row::Note(Note::Collapsed));
        } else {
            match self.diff.as_deref().map(|d| &d.content) {
                None => rows.push(Row::Note(Note::Loading)),
                Some(Content::Binary { .. }) => rows.push(Row::Note(Note::Binary)),
                Some(Content::TooLarge { .. }) => rows.push(Row::Note(Note::TooLarge)),
                Some(Content::Submodule { .. }) => rows.push(Row::Note(Note::Submodule)),
                Some(Content::Error(_)) => rows.push(Row::Note(Note::Error)),
                Some(Content::Text(text)) if text.hunks.is_empty() => {
                    rows.push(Row::Note(Note::NoChanges))
                }
                Some(Content::Text(text)) => {
                    let (old_len, new_len) = (text.old.len() as u32, text.new.len() as u32);
                    for (h, hunk) in text.hunks.iter().enumerate() {
                        rows.push(Row::Hunk(h as u32));
                        for (l, line) in hunk.lines.iter().enumerate() {
                            rows.push(Row::Line {
                                hunk: h as u32,
                                line: l as u32,
                            });
                            let old_end = line.old == Some(old_len)
                                && text.old.missing_final_newline
                                && line.kind != LineKind::Added;
                            let new_end = line.new == Some(new_len)
                                && text.new.missing_final_newline
                                && line.kind != LineKind::Removed;
                            if old_end || new_end {
                                rows.push(Row::NoNewline);
                            }
                        }
                    }
                }
            }
        }
        rows.push(Row::Spacer);
        self.rows = rows;
    }

    /// Columns for line numbers in this file (at least 3).
    pub fn number_width(&self) -> usize {
        let max = match self.diff.as_deref().map(|d| &d.content) {
            Some(Content::Text(t)) => t.old.len().max(t.new.len()),
            _ => 0,
        };
        max.to_string().len().max(3)
    }
}

/// A position in the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Pos {
    pub file: usize,
    pub row: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Doc {
    pub files: Vec<DocFile>,
    starts: Vec<usize>,
    total: usize,
}

impl Doc {
    pub fn new(files: Vec<ChangedFile>, generated: &HashSet<String>) -> Self {
        let files = files
            .into_iter()
            .map(|meta| {
                let path = meta.path().to_owned();
                let mut file = DocFile {
                    generated: generated.contains(&path) || is_lockfile(&path),
                    meta,
                    expanded: false,
                    diff: None,
                    rows: Vec::new(),
                };
                file.rebuild();
                file
            })
            .collect();
        let mut doc = Self {
            files,
            starts: Vec::new(),
            total: 0,
        };
        doc.reindex();
        doc
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
            file.rebuild();
            self.reindex();
        }
    }

    pub fn toggle_expanded(&mut self, index: usize) {
        if let Some(file) = self.files.get_mut(index)
            && file.generated
        {
            file.expanded = !file.expanded;
            file.rebuild();
            self.reindex();
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

    /// Next row after `pos` matching `pred`.
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
        self.find_after(pos, |r| matches!(r, Row::Hunk(_)))
    }

    pub fn prev_hunk(&self, pos: Pos) -> Option<Pos> {
        self.find_before(pos, |r| matches!(r, Row::Hunk(_)))
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

    /// Files whose diffs haven't arrived, among `count` rows from `top`.
    pub fn loading_in_view(&self, top: Pos, count: usize) -> Vec<usize> {
        let first = self.to_global(top);
        let mut out: Vec<usize> = Vec::new();
        for g in first..(first + count).min(self.total) {
            let pos = self.to_pos(g);
            if self.files[pos.file].diff.is_none()
                && !self.files[pos.file].collapsed()
                && !out.contains(&pos.file)
            {
                out.push(pos.file);
            }
        }
        out
    }

    pub fn totals(&self) -> (u32, u32) {
        self.files
            .iter()
            .filter_map(|f| f.diff.as_deref())
            .fold((0, 0), |(a, d), f| (a + f.additions, d + f.deletions))
    }

    pub fn ready_count(&self) -> usize {
        self.files.iter().filter(|f| f.diff.is_some()).count()
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

    fn doc() -> Doc {
        let mut doc = Doc::new(
            vec![changed("a.txt"), changed("Cargo.lock"), changed("b.txt")],
            &HashSet::new(),
        );
        let old = numbered(30);
        let new = old
            .replace("line 5\n", "five\n")
            .replace("line 25\n", "twenty-five\n");
        doc.set_diff(
            0,
            Arc::new(FileDiff::compute(
                "a.txt",
                Some(old.as_bytes()),
                Some(new.as_bytes()),
            )),
        );
        doc
    }

    #[test]
    fn rows_per_file() {
        let doc = doc();
        let a = doc.files[0].rows();
        assert_eq!(a[0], Row::Header);
        assert_eq!(a[1], Row::Hunk(0));
        assert_eq!(a.iter().filter(|r| matches!(r, Row::Hunk(_))).count(), 2);
        assert_eq!(*a.last().unwrap(), Row::Spacer);
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
        doc.set_diff(
            1,
            Arc::new(FileDiff::compute(
                "Cargo.lock",
                Some(b"a\n"),
                Some(b"b\nc\n"),
            )),
        );
        assert_eq!(doc.row(cursor), Some(Row::Note(Note::Loading)));
        assert_ne!(
            doc.to_global(cursor),
            before,
            "global index moved; the position didn't"
        );
    }

    #[test]
    fn hunk_and_file_navigation() {
        let doc = doc();
        let start = Pos::default();
        let h1 = doc.next_hunk(start).unwrap();
        let h2 = doc.next_hunk(h1).unwrap();
        assert_eq!(doc.row(h1), Some(Row::Hunk(0)));
        assert_eq!(doc.row(h2), Some(Row::Hunk(1)));
        assert_eq!(doc.next_hunk(h2), None);
        assert_eq!(doc.prev_hunk(h2), Some(h1));
        assert_eq!(doc.next_file(h2), Some(Pos { file: 1, row: 0 }));
        assert_eq!(doc.prev_file(h2), Some(Pos { file: 0, row: 0 }));
        assert_eq!(
            doc.prev_file(Pos { file: 1, row: 0 }),
            Some(Pos { file: 0, row: 0 })
        );
    }

    #[test]
    fn missing_newline_rows() {
        let mut doc = Doc::new(vec![changed("x.txt")], &HashSet::new());
        doc.set_diff(
            0,
            Arc::new(FileDiff::compute("x.txt", Some(b"a\nb\n"), Some(b"a\nb"))),
        );
        let rows = doc.files[0].rows();
        assert_eq!(rows.iter().filter(|r| **r == Row::NoNewline).count(), 1);
    }

    #[test]
    fn loading_files_in_view() {
        let doc = doc();
        assert_eq!(doc.loading_in_view(Pos::default(), 1000), [2]);
        assert!(doc.loading_in_view(Pos::default(), 3).is_empty());
    }
}
