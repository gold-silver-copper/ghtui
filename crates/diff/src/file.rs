//! A computed file diff, ready to render.

use crate::highlight::{Language, Span, highlight};
use crate::hunks::{Algorithm, DiffLine, LineKind, Whitespace, align};
use crate::text::{Text, is_binary};

/// Context lines around changes.
pub const CONTEXT: u32 = 3;
/// Blobs larger than this aren't diffed line by line.
pub const MAX_DIFF_BYTES: usize = 16 << 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    Text(TextDiff),
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
    pub old: Text,
    pub new: Text,
    /// Per-line spans; empty when unhighlighted.
    pub old_spans: Vec<Vec<Span>>,
    pub new_spans: Vec<Vec<Span>>,
    /// Full alignment comparing lines exactly.
    pub lines: Vec<DiffLine>,
    /// Full alignment ignoring whitespace.
    pub lines_ignoring_whitespace: Vec<DiffLine>,
}

impl TextDiff {
    pub fn lines(&self, whitespace: Whitespace) -> &[DiffLine] {
        match whitespace {
            Whitespace::Exact => &self.lines,
            Whitespace::Ignore => &self.lines_ignoring_whitespace,
        }
    }

    pub fn has_changes(&self, whitespace: Whitespace) -> bool {
        self.lines(whitespace)
            .iter()
            .any(|l| l.kind != LineKind::Context)
    }

    /// The text of an alignment entry (the old side for removed lines).
    pub fn text(&self, line: &DiffLine) -> &str {
        match (line.kind, line.old, line.new) {
            (LineKind::Removed, Some(o), _) => self.old.line(o as usize - 1),
            (_, _, Some(n)) => self.new.line(n as usize - 1),
            _ => "",
        }
    }

    pub fn old_spans(&self, line: u32) -> &[Span] {
        spans_at(&self.old_spans, line)
    }

    pub fn new_spans(&self, line: u32) -> &[Span] {
        spans_at(&self.new_spans, line)
    }
}

fn spans_at(spans: &[Vec<Span>], line: u32) -> &[Span] {
    line.checked_sub(1)
        .and_then(|i| spans.get(i as usize))
        .map_or(&[], Vec::as_slice)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    pub content: Content,
    pub additions: u32,
    pub deletions: u32,
}

/// Added and removed line counts of an alignment.
pub fn counts(lines: &[DiffLine]) -> (u32, u32) {
    lines.iter().fold((0, 0), |(a, d), l| match l.kind {
        LineKind::Added => (a + 1, d),
        LineKind::Removed => (a, d + 1),
        LineKind::Context => (a, d),
    })
}

impl FileDiff {
    pub fn submodule(old: Option<String>, new: Option<String>) -> Self {
        Self {
            content: Content::Submodule { old, new },
            additions: 0,
            deletions: 0,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: Content::Error(message.into()),
            additions: 0,
            deletions: 0,
        }
    }

    /// Diffs two blob contents (`None` for an absent side). `path` picks the
    /// highlighting language.
    pub fn compute(path: &str, old: Option<&[u8]>, new: Option<&[u8]>) -> Self {
        let sizes = (old.map(<[u8]>::len), new.map(<[u8]>::len));
        if old.is_some_and(|b| b.len() > MAX_DIFF_BYTES)
            || new.is_some_and(|b| b.len() > MAX_DIFF_BYTES)
        {
            return Self {
                content: Content::TooLarge {
                    old_size: sizes.0,
                    new_size: sizes.1,
                },
                additions: 0,
                deletions: 0,
            };
        }
        if old.is_some_and(is_binary) || new.is_some_and(is_binary) {
            return Self {
                content: Content::Binary {
                    old_size: sizes.0,
                    new_size: sizes.1,
                },
                additions: 0,
                deletions: 0,
            };
        }
        let old = Text::new(old.unwrap_or_default());
        let new = Text::new(new.unwrap_or_default());
        let lines = align(&old, &new, Algorithm::Histogram, Whitespace::Exact);
        let (additions, deletions) = counts(&lines);
        let lines_ignoring_whitespace = if additions + deletions == 0 {
            lines.clone()
        } else {
            align(&old, &new, Algorithm::Histogram, Whitespace::Ignore)
        };
        let lang = Language::from_path(path);
        // Only highlight files with changes to show.
        let (old_spans, new_spans) = if additions + deletions == 0 {
            (Vec::new(), Vec::new())
        } else {
            (highlight(lang, &old), highlight(lang, &new))
        };
        Self {
            content: Content::Text(TextDiff {
                old,
                new,
                old_spans,
                new_spans,
                lines,
                lines_ignoring_whitespace,
            }),
            additions,
            deletions,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(!text.new_spans(1).is_empty());
        assert!(text.new_spans(99).is_empty());
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
