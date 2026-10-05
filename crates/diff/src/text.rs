//! Blob contents as lines.

use std::sync::Arc;

use crate::sat_u32;

/// Bytes git checks for NUL to call a file binary.
const BINARY_SNIFF: usize = 8000;

/// Stands in for a source too large for `u32` line offsets.
const TOO_LARGE: &str = "(file too large to show)";

/// Same rule as git: a NUL byte in the first 8000 bytes means binary.
pub fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(BINARY_SNIFF).any(|&b| b == 0)
}

/// A text blob split into lines, keeping what the diff needs to know about
/// line endings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Text {
    /// The decoded source (invalid UTF-8 replaced).
    pub source: Arc<str>,
    /// Byte range of each line in `source`, excluding the `\n` (and a `\r`
    /// before it, which is reported through `crlf`).
    pub lines: Vec<(u32, u32)>,
    /// Lines that ended in `\r\n`.
    pub crlf: Vec<bool>,
    /// The last line has no terminating newline.
    pub missing_final_newline: bool,
}

impl Text {
    /// Splits `bytes` into lines. A source over `u32::MAX` bytes becomes one
    /// line saying so, as offsets are `u32`.
    pub fn new(bytes: &[u8]) -> Self {
        let source: Arc<str> = match String::from_utf8_lossy(bytes) {
            s if u32::try_from(s.len()).is_ok() => s.into(),
            _ => TOO_LARGE.into(),
        };
        let mut lines = Vec::new();
        let mut crlf = Vec::new();
        let mut start = 0;
        for chunk in source.split_inclusive('\n') {
            let (line, is_crlf) = match chunk.strip_suffix('\n') {
                Some(l) => l.strip_suffix('\r').map_or((l, false), |l| (l, true)),
                None => (chunk, false),
            };
            lines.push((sat_u32(start), sat_u32(start + line.len())));
            crlf.push(is_crlf);
            start += chunk.len();
        }
        let missing_final_newline = !source.is_empty() && !source.ends_with('\n');
        Self {
            source,
            lines,
            crlf,
            missing_final_newline,
        }
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Line `i` (0-based) without its terminator; empty past the end.
    pub fn line(&self, i: usize) -> &str {
        self.lines
            .get(i)
            .and_then(|&(a, b)| self.source.get(a as usize..b as usize))
            .unwrap_or_default()
    }

    /// Line number `n` (1-based, as in [`DiffLine`](crate::DiffLine));
    /// empty for 0 or past the end.
    pub fn line_no(&self, n: u32) -> &str {
        n.checked_sub(1).map_or("", |i| self.line(i as usize))
    }

    /// Line `i` as the diff compares it: content plus terminator, so a
    /// CRLF↔LF or missing-final-newline change counts as a change, as in git.
    pub(crate) fn token(&self, i: usize) -> String {
        let mut t = self.line(i).to_owned();
        if self.crlf.get(i) == Some(&true) {
            t.push('\r');
        }
        if i + 1 < self.len() || !self.missing_final_newline {
            t.push('\n');
        }
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_lines() {
        let t = Text::new(b"a\nb\n");
        assert_eq!(t.len(), 2);
        assert_eq!(t.line(1), "b");
        assert_eq!(t.line_no(2), "b");
        assert_eq!((t.line(2), t.line_no(0)), ("", ""));
        assert!(!t.missing_final_newline);
    }

    #[test]
    fn tracks_missing_final_newline() {
        let t = Text::new(b"a\nb");
        assert_eq!(t.len(), 2);
        assert!(t.missing_final_newline);
        assert_eq!(t.token(1), "b");
        assert_eq!(t.token(0), "a\n");
    }

    #[test]
    fn strips_and_records_crlf() {
        let t = Text::new(b"a\r\nb\n");
        assert_eq!(t.line(0), "a");
        assert_eq!(t.crlf, [true, false]);
        assert_eq!(t.token(0), "a\r\n");
    }

    #[test]
    fn empty_and_invalid_utf8() {
        assert!(Text::new(b"").is_empty());
        let t = Text::new(b"caf\xe9\n");
        assert_eq!(t.line(0), "caf\u{fffd}");
    }

    #[test]
    fn detects_binary() {
        assert!(is_binary(b"PNG\0\x01"));
        assert!(!is_binary(b"plain text"));
        let mut late_nul = vec![b'a'; 9000];
        late_nul.push(0);
        assert!(!is_binary(&late_nul));
    }
}
