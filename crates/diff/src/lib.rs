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
pub mod text;

pub use file::{CONTEXT, Content, FileDiff, TextDiff, counts};
pub use highlight::{Language, Span, TokenKind, highlight};
pub use hunks::{
    Algorithm, DiffLine, Hunk, LineKind, Whitespace, align, counts_before, diff_lines, hunk,
    segments, segments_by,
};
pub use text::{Text, is_binary};
