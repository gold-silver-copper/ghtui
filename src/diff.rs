//! Diff computation and syntax highlighting. Pure logic, no UI, no I/O.
//!
//! Also comment anchoring ([`anchor`]), change blocks ([`blocks`]),
//! intra-line token diffs ([`intraline`]), moved-code detection
//! ([`moves`]) and structural diffs ([`structure`]).

pub mod anchor;
pub mod blocks;
pub mod file;
pub mod highlight;
pub mod hunks;
pub mod intraline;
pub mod moves;
pub mod scope;
pub mod structure;
pub mod text;

pub use file::{Alignment, CONTEXT, Content, FileDiff, TextDiff, counts};
pub use highlight::{Language, Span, TokenKind};
pub use hunks::{DiffLine, Whitespace, hunks, segments_by};
pub use text::{Text, is_binary};

/// `n` as a `u32`, saturating. Offsets and counts are `u32`, and [`Text`]
/// keeps its source within range, so this only clamps what can't happen.
pub(crate) fn sat_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// `items` in the runs `same` keeps together, each with where it starts.
pub(crate) fn runs<T>(
    items: &[T],
    same: impl FnMut(&T, &T) -> bool,
) -> impl Iterator<Item = (usize, &[T])> {
    items.chunk_by(same).scan(0, |end, run| {
        let start = *end;
        *end += run.len();
        Some((start, run))
    })
}
