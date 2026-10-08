//! Comment anchoring.
//!
//! GitHub accepts line comments only inside the hunks of *its* diff, which
//! can differ from ours (different algorithm and context). Commentable ranges
//! therefore come from GitHub's per-file `patch` when available, and from a
//! local Myers diff with 3 lines of context otherwise (GitHub omits `patch`
//! for large and binary files, and caps the file list).
//!
//! Comments are anchored by real file line numbers and a side, never by
//! rendered row positions.

use crate::hunks::{Algorithm, DiffLine, Whitespace, align, diff_lines};
use crate::text::Text;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Side {
    /// The base (old) version: deleted lines.
    Left,
    /// The head (new) version: added and context lines.
    Right,
}

/// A line on one side of the diff (1-based).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LinePos {
    pub side: Side,
    pub line: u32,
}

/// One hunk's line ranges, 1-based and inclusive of `start`, exclusive of
/// `end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HunkRange {
    pub old: (u32, u32),
    pub new: (u32, u32),
}

impl HunkRange {
    fn contains(&self, pos: LinePos) -> bool {
        let (start, end) = match pos.side {
            Side::Left => self.old,
            Side::Right => self.new,
        };
        (start..end).contains(&pos.line)
    }
}

/// Where commentable ranges came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeSource {
    GitHubPatch,
    /// Local Myers diff, 3 lines of context.
    LocalFallback,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commentable {
    pub ranges: Vec<HunkRange>,
    pub source: RangeSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AnchorError {
    #[error("this line isn't part of GitHub's diff for the file")]
    NotCommentable,
    #[error("a multi-line comment must stay within one hunk of GitHub's diff")]
    CrossesHunks,
    #[error("a range must start before it ends")]
    Backwards,
}

impl Commentable {
    pub fn from_patch(patch: &str) -> Self {
        Self {
            ranges: parse_patch_headers(patch),
            source: RangeSource::GitHubPatch,
        }
    }

    /// GitHub's view reconstructed locally: Myers with 3 lines of context.
    pub fn local(old: &Text, new: &Text) -> Self {
        Self {
            ranges: diff_lines(old, new, Algorithm::Myers, 3)
                .iter()
                .map(|h| hunk_range((h.old_start, h.old_len), (h.new_start, h.new_len)))
                .collect(),
            source: RangeSource::LocalFallback,
        }
    }

    pub fn hunk_of(&self, pos: LinePos) -> Option<usize> {
        self.ranges.iter().position(|r| r.contains(pos))
    }

    pub fn is_commentable(&self, pos: LinePos) -> bool {
        self.hunk_of(pos).is_some()
    }

    /// Checks a single line (`start == end`) or a range. Both ends must be
    /// commentable and in the same hunk.
    pub fn validate(&self, start: LinePos, end: LinePos) -> Result<(), AnchorError> {
        if start.side == end.side && start.line > end.line {
            return Err(AnchorError::Backwards);
        }
        let a = self.hunk_of(start).ok_or(AnchorError::NotCommentable)?;
        let b = self.hunk_of(end).ok_or(AnchorError::NotCommentable)?;
        if a != b {
            return Err(AnchorError::CrossesHunks);
        }
        Ok(())
    }
}

/// The range of a hunk from its `(start, len)` on each side.
fn hunk_range(old: (u32, u32), new: (u32, u32)) -> HunkRange {
    // Saturating: patch headers come from GitHub, untrusted.
    let range = |(start, len): (u32, u32)| {
        // For an empty side git reports the line *before* the hunk; nothing
        // on that side is commentable.
        if len == 0 {
            (start.saturating_add(1), start.saturating_add(1))
        } else {
            (start, start.saturating_add(len))
        }
    };
    HunkRange {
        old: range(old),
        new: range(new),
    }
}

/// Hunk ranges from the `@@ -a,b +c,d @@` headers of a unified diff.
pub fn parse_patch_headers(patch: &str) -> Vec<HunkRange> {
    patch
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("@@ -")?;
            let (spec, _) = rest.split_once(" @@")?;
            let (old, new) = spec.split_once(" +")?;
            let parse = |s: &str| -> Option<(u32, u32)> {
                match s.split_once(',') {
                    Some((a, b)) => Some((a.parse().ok()?, b.parse().ok()?)),
                    None => Some((s.parse().ok()?, 1)),
                }
            };
            Some(hunk_range(parse(old)?, parse(new)?))
        })
        .collect()
}

/// Maps a line of `old` to the same line in `new`, if it survived
/// unchanged. `None` means it changed or was removed: the comment stays
/// outdated.
pub fn map_line(old: &Text, new: &Text, line: u32) -> Option<u32> {
    align(old, new, Algorithm::Histogram, Whitespace::Exact)
        .into_iter()
        .find_map(|l| match l {
            DiffLine::Context { old, new } if old == line => Some(new),
            _ => None,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn right(line: u32) -> LinePos {
        LinePos {
            side: Side::Right,
            line,
        }
    }

    fn left(line: u32) -> LinePos {
        LinePos {
            side: Side::Left,
            line,
        }
    }

    const PATCH: &str = "@@ -1,3 +1,4 @@\n a\n+b\n c\n d\n@@ -20,4 +21,3 @@ fn x()\n p\n-q\n r\n s\n@@ -40 +40 @@\n-old\n+new\n";

    #[test]
    fn parses_patch_headers() {
        let c = Commentable::from_patch(PATCH);
        assert_eq!(c.ranges.len(), 3);
        assert_eq!(c.ranges[0].new, (1, 5));
        assert_eq!(c.ranges[1].old, (20, 24));
        assert_eq!(c.ranges[2].old, (40, 41), "a missing length means 1");
        assert_eq!(c.source, RangeSource::GitHubPatch);
    }

    #[test]
    fn commentable_lines_and_sides() {
        let c = Commentable::from_patch(PATCH);
        assert!(c.is_commentable(right(1)));
        assert!(c.is_commentable(right(4)));
        assert!(!c.is_commentable(right(5)));
        assert!(!c.is_commentable(right(10)), "between hunks");
        assert!(c.is_commentable(left(21)));
        assert!(!c.is_commentable(left(10)));
    }

    #[test]
    fn ranges_must_stay_in_one_hunk() {
        let c = Commentable::from_patch(PATCH);
        assert_eq!(c.validate(right(1), right(4)), Ok(()));
        assert_eq!(
            c.validate(left(21), right(22)),
            Ok(()),
            "mixed sides in one hunk"
        );
        assert_eq!(
            c.validate(right(2), right(22)),
            Err(AnchorError::CrossesHunks)
        );
        assert_eq!(
            c.validate(right(2), right(10)),
            Err(AnchorError::NotCommentable)
        );
        assert_eq!(c.validate(right(4), right(2)), Err(AnchorError::Backwards));
    }

    #[test]
    fn pure_additions_have_no_left_side() {
        let c = Commentable::from_patch("@@ -0,0 +1,2 @@\n+a\n+b\n");
        assert!(c.is_commentable(right(2)));
        assert!(!c.is_commentable(left(0)));
        assert!(!c.is_commentable(left(1)));
    }

    #[test]
    fn huge_patch_line_numbers_saturate() {
        let max = u32::MAX;
        let c = Commentable::from_patch(&format!("@@ -{max},0 +{max},5 @@\n"));
        assert_eq!(c.ranges[0].old, (max, max));
        assert_eq!(c.ranges[0].new, (max, max));
    }

    #[test]
    fn local_fallback_matches_git_context() {
        let old: String = (1..=30).map(|i| format!("{i}\n")).collect();
        let new = old.replace("15\n", "fifteen\n");
        let c = Commentable::local(&Text::new(old.as_bytes()), &Text::new(new.as_bytes()));
        assert_eq!(c.source, RangeSource::LocalFallback);
        assert_eq!(c.ranges.len(), 1);
        assert!(c.is_commentable(right(12)));
        assert!(c.is_commentable(right(18)));
        assert!(!c.is_commentable(right(11)));
        assert!(!c.is_commentable(right(19)));
    }

    #[test]
    fn maps_lines_forward_when_unchanged() {
        let old = Text::new(b"a\nb\nc\nd\n");
        let new = Text::new(b"z\na\nb\nC\nd\n");
        assert_eq!(map_line(&old, &new, 1), Some(2));
        assert_eq!(map_line(&old, &new, 4), Some(5));
        assert_eq!(map_line(&old, &new, 3), None, "changed: stays outdated");
        assert_eq!(map_line(&old, &new, 9), None);
    }
}
