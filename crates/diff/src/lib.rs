//! Diff computation and syntax highlighting. Pure logic, no UI, no I/O.
//!
//! Also comment anchoring ([`anchor`]). Moved-code detection and intra-line
//! token diffs arrive in M4.

pub mod anchor;
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
