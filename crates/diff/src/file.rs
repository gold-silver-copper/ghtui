//! A computed file diff, ready to render.

use crate::anchor::{Commentable, HunkRange, LinePos, Side};
use crate::highlight::{Language, Span, highlight};
use crate::hunks::{Algorithm, DiffLine, Whitespace, align};
use crate::intraline::{IntraLine, intraline};
use crate::scope::{self, Scope};
use crate::text::{Text, is_binary};

/// Context lines around changes.
pub const CONTEXT: u32 = 3;
/// Blobs larger than this aren't diffed line by line.
pub const MAX_DIFF_BYTES: usize = 16 << 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    Text(Box<TextDiff>),
    Binary {
        old_size: Option<usize>,
        new_size: Option<usize>,
    },
    TooLarge {
        old_size: Option<usize>,
        new_size: Option<usize>,
    },
    /// Gitlink change; commit IDs, not blobs.
    Submodule {
        old: Option<String>,
        new: Option<String>,
    },
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextDiff {
    // Read through a `LinePos`, so a line is always read from its own side.
    old: Text,
    new: Text,
    /// Per-line spans; empty when unhighlighted.
    old_spans: Vec<Vec<Span>>,
    new_spans: Vec<Vec<Span>>,
    /// Full alignment comparing lines exactly.
    pub lines: Vec<DiffLine>,
    /// Full alignment ignoring whitespace.
    pub lines_ignoring_whitespace: Vec<DiffLine>,
    /// GitHub-style commentable ranges (Myers, 3 lines of context), used when
    /// GitHub's own patch isn't available.
    pub local_ranges: Vec<HunkRange>,
    /// Named scopes of the new side (functions, types...), outermost first.
    pub scopes: Vec<Scope>,
    /// Changed token ranges on paired lines, for each alignment.
    pub intraline: IntraLine,
    pub intraline_ignoring_whitespace: IntraLine,
}

impl TextDiff {
    pub fn lines(&self, whitespace: Whitespace) -> &[DiffLine] {
        match whitespace {
            Whitespace::Exact => &self.lines,
            Whitespace::Ignore => &self.lines_ignoring_whitespace,
        }
    }

    /// The scopes containing new-side line `line`, outermost first:
    /// `["impl Doc", "fn offset"]`.
    pub fn scope(&self, line: u32) -> Vec<&str> {
        scope::path(&self.scopes, line)
    }

    pub fn intraline(&self, whitespace: Whitespace) -> &IntraLine {
        match whitespace {
            Whitespace::Exact => &self.intraline,
            Whitespace::Ignore => &self.intraline_ignoring_whitespace,
        }
    }

    pub fn has_changes(&self, whitespace: Whitespace) -> bool {
        self.lines(whitespace).iter().any(|l| l.is_change())
    }

    fn side(&self, side: Side) -> (&Text, &[Vec<Span>]) {
        match side {
            Side::Left => (&self.old, &self.old_spans),
            Side::Right => (&self.new, &self.new_spans),
        }
    }

    /// The text of a line, without its terminator.
    pub fn line(&self, pos: LinePos) -> &str {
        self.side(pos.side).0.line_no(pos.line)
    }

    /// A line's highlighting; empty when unhighlighted.
    pub fn spans(&self, pos: LinePos) -> &[Span] {
        nth(self.side(pos.side).1, pos.line).map_or(&[], Vec::as_slice)
    }

    /// The line ended in `\r\n`.
    pub fn crlf(&self, pos: LinePos) -> bool {
        nth(&self.side(pos.side).0.crlf, pos.line) == Some(&true)
    }

    /// The last line of its side.
    pub fn is_last(&self, pos: LinePos) -> bool {
        pos.line as usize == self.side(pos.side).0.len()
    }

    /// The last line of its side, with no newline after it.
    pub fn ends_without_newline(&self, pos: LinePos) -> bool {
        self.is_last(pos) && self.side(pos.side).0.missing_final_newline
    }

    /// One side gained or lost its final newline.
    pub fn final_newline_changed(&self) -> bool {
        self.old.missing_final_newline != self.new.missing_final_newline
    }

    /// Lines in the longer side.
    pub fn max_lines(&self) -> usize {
        self.old.len().max(self.new.len())
    }
}

/// Line `line`'s item (1-based) of a per-line list.
fn nth<T>(items: &[T], line: u32) -> Option<&T> {
    items.get(line.checked_sub(1)? as usize)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    pub content: Content,
    pub additions: u32,
    pub deletions: u32,
}

/// Added and removed line counts of an alignment.
pub fn counts(lines: &[DiffLine]) -> (u32, u32) {
    lines.iter().fold((0, 0), |(a, d), l| match l {
        DiffLine::Added(_) => (a + 1, d),
        DiffLine::Removed(_) => (a, d + 1),
        DiffLine::Context { .. } => (a, d),
    })
}

impl FileDiff {
    /// A diff without line changes.
    fn empty(content: Content) -> Self {
        Self {
            content,
            additions: 0,
            deletions: 0,
        }
    }

    pub fn submodule(old: Option<String>, new: Option<String>) -> Self {
        Self::empty(Content::Submodule { old, new })
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self::empty(Content::Error(message.into()))
    }

    /// Diffs two blob contents (`None` for an absent side). `path` picks the
    /// highlighting language.
    pub fn compute(path: &str, old: Option<&[u8]>, new: Option<&[u8]>) -> Self {
        Self::compute_with(path, old, new, true)
    }

    /// Alignments only: no highlighting or intra-line diffs (for comparing
    /// against an earlier version, never rendered).
    pub fn compute_plain(path: &str, old: Option<&[u8]>, new: Option<&[u8]>) -> Self {
        Self::compute_with(path, old, new, false)
    }

    fn compute_with(path: &str, old: Option<&[u8]>, new: Option<&[u8]>, rich: bool) -> Self {
        let (old_size, new_size) = (old.map(<[u8]>::len), new.map(<[u8]>::len));
        if old_size.max(new_size) > Some(MAX_DIFF_BYTES) {
            return Self::empty(Content::TooLarge { old_size, new_size });
        }
        if old.is_some_and(is_binary) || new.is_some_and(is_binary) {
            return Self::empty(Content::Binary { old_size, new_size });
        }
        let old = Text::new(old.unwrap_or_default());
        let new = Text::new(new.unwrap_or_default());
        let lines = align(&old, &new, Algorithm::Histogram, Whitespace::Exact);
        let (additions, deletions) = counts(&lines);
        let changed = additions + deletions > 0;
        let lines_ignoring_whitespace = if changed {
            align(&old, &new, Algorithm::Histogram, Whitespace::Ignore)
        } else {
            lines.clone()
        };
        // Only highlight files with changes to show.
        let (local_ranges, old_spans, new_spans, scopes) = if changed && rich {
            let lang = Language::from_path(path);
            let ranges = Commentable::local(&old, &new).ranges;
            let scopes = scope::scopes(lang, &new);
            (ranges, highlight(lang, &old), highlight(lang, &new), scopes)
        } else {
            Default::default()
        };
        let mut text = TextDiff {
            old,
            new,
            old_spans,
            new_spans,
            lines,
            lines_ignoring_whitespace,
            local_ranges,
            scopes,
            intraline: IntraLine::new(),
            intraline_ignoring_whitespace: IntraLine::new(),
        };
        if rich {
            text.intraline = intraline(&text, &text.lines);
            text.intraline_ignoring_whitespace = intraline(&text, &text.lines_ignoring_whitespace);
        }
        Self {
            content: Content::Text(Box::new(text)),
            additions,
            deletions,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The text diff of `old` to `new`.
    pub(crate) fn text_diff(path: &str, old: &str, new: &str) -> Box<TextDiff> {
        match FileDiff::compute(path, Some(old.as_bytes()), Some(new.as_bytes())).content {
            Content::Text(text) => text,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn text_diff_counts_and_highlights() {
        let diff = FileDiff::compute(
            "a.rs",
            Some(b"fn a() {}\n"),
            Some(b"fn b() {}\nfn c() {}\n"),
        );
        assert_eq!((diff.additions, diff.deletions), (2, 1));
        let Content::Text(text) = &diff.content else {
            panic!("{diff:?}")
        };
        assert!(text.has_changes(Whitespace::Exact));
        let new = |line| LinePos {
            side: Side::Right,
            line,
        };
        assert!(!text.spans(new(1)).is_empty());
        assert!(text.spans(new(99)).is_empty());
    }

    #[test]
    fn added_deleted_binary_and_large() {
        let added = FileDiff::compute("x.txt", None, Some(b"a\nb\n"));
        assert_eq!((added.additions, added.deletions), (2, 0));
        let deleted = FileDiff::compute("x.txt", Some(b"a\n"), None);
        assert_eq!((deleted.additions, deleted.deletions), (0, 1));
        let binary = FileDiff::compute("x.png", Some(b"\0\x01"), Some(b"\0\x02\x03"));
        assert_eq!(
            binary.content,
            Content::Binary {
                old_size: Some(2),
                new_size: Some(3)
            }
        );
        let big = vec![b'a'; MAX_DIFF_BYTES + 1];
        assert!(matches!(
            FileDiff::compute("x.txt", None, Some(&big)).content,
            Content::TooLarge { .. }
        ));
    }
}
