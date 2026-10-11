//! Diff computation and syntax highlighting. Pure logic, no UI, no I/O.
//!
//! Also comment anchoring ([`anchor`]), change blocks ([`blocks`]),
//! intra-line token diffs ([`intraline`]) and moved-code detection
//! ([`moves`]).

pub mod anchor;
pub mod blocks;
pub mod file;
pub mod highlight;
pub mod hunks;
pub mod intraline;
pub mod moves;
pub mod scope;
pub mod text;

pub use file::{Alignment, CONTEXT, Content, FileDiff, TextDiff, counts};
pub use highlight::{Language, Span, TokenKind, highlight};
pub use hunks::{
    Algorithm, DiffLine, Hunk, Whitespace, align, diff_lines, hunks, segments, segments_by,
    side_counts,
};
pub use text::{Text, is_binary};

/// `n` as a `u32`, saturating. Offsets and counts are `u32`, and [`Text`]
/// keeps its source within range, so this only clamps what can't happen.
pub(crate) fn sat_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}
