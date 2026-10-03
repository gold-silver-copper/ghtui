//! Diff computation and syntax highlighting. Pure logic, no UI, no I/O.
//!
//! Moved-code detection, intra-line token diffs and comment anchoring arrive
//! in later milestones.

pub mod file;
pub mod highlight;
pub mod hunks;
pub mod text;

pub use file::{CONTEXT, Content, FileDiff, TextDiff, counts};
pub use highlight::{Language, Span, TokenKind, highlight};
pub use hunks::{
    Algorithm, DiffLine, Hunk, LineKind, Whitespace, align, counts_before, diff_lines, hunk,
    segments,
};
pub use text::{Text, is_binary};
