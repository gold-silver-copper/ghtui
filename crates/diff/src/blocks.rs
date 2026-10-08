//! Change blocks: maximal runs of changed lines in an alignment.
//!
//! A block's hash covers the file path and the block's removed and added
//! text, not line numbers, so the same change hashes the same wherever it
//! sits. That makes it the unit for "reviewed" marks and for "changes since
//! my last review" (a block whose hash appeared in the earlier diff isn't
//! new, however the lines around it moved or the base was rebased).

use std::ops::Range;

use crate::file::TextDiff;
use crate::hunks::DiffLine;
use crate::sat_u32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeBlock {
    /// Alignment entries of the block.
    pub entries: Range<u32>,
    pub hash: String,
    /// The removed and added text differ only in whitespace and line breaks.
    pub formatting_only: bool,
}

pub fn change_blocks(path: &str, text: &TextDiff, lines: &[DiffLine]) -> Vec<ChangeBlock> {
    let is_context = |l: &DiffLine| !l.is_change();
    let mut out = Vec::new();
    let mut end = 0;
    for run in lines.chunk_by(|a, b| is_context(a) == is_context(b)) {
        let start = end;
        end += run.len();
        if run.first().is_none_or(is_context) {
            continue;
        }
        let mut hasher = Fnv::new();
        hasher.write(path.as_bytes());
        let (mut removed, mut added) = (String::new(), String::new());
        let (mut has_removed, mut has_added) = (false, false);
        for l in run {
            let line = text.line(l.shown());
            let (sign, squeezed): (&[u8], &mut String) = if let DiffLine::Added(_) = l {
                has_added = true;
                (b"\n+", &mut added)
            } else {
                has_removed = true;
                (b"\n-", &mut removed)
            };
            hasher.write(sign);
            hasher.write(line.trim_end().as_bytes());
            squeezed.extend(line.chars().filter(|c| !c.is_whitespace()));
        }
        // Gaining or losing the final newline is a real change (and has
        // its own marker), not formatting.
        let touches_end = text.final_newline_changed()
            && run.iter().flat_map(|l| l.lines()).any(|p| text.is_last(p));
        out.push(ChangeBlock {
            entries: sat_u32(start)..sat_u32(end),
            hash: format!("{:016x}", hasher.0),
            formatting_only: has_removed && has_added && removed == added && !touches_end,
        });
    }
    out
}

/// FNV-1a: stable across builds and platforms (unlike `DefaultHasher`),
/// which matters because block hashes are persisted.
pub(crate) struct Fnv(pub u64);

impl Fnv {
    pub fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    pub fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= u64::from(*b);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file::tests::text_diff;
    use crate::hunks::Whitespace;

    fn blocks(path: &str, old: &str, new: &str) -> Vec<ChangeBlock> {
        let text = text_diff(path, old, new);
        change_blocks(path, &text, text.lines(Whitespace::Exact))
    }

    #[test]
    fn hashes_follow_content_not_position() {
        let a = blocks("x.rs", "1\n2\nold\n", "1\n2\nnew\n");
        let b = blocks("x.rs", "0\n1\n2\nold\n", "0\n1\n2\nnew\n");
        assert_eq!(a[0].hash, b[0].hash);
        assert_ne!(
            a[0].hash,
            blocks("y.rs", "1\n2\nold\n", "1\n2\nnew\n")[0].hash
        );
        assert_ne!(
            a[0].hash,
            blocks("x.rs", "1\n2\nold\n", "1\n2\nnewer\n")[0].hash
        );
    }

    #[test]
    fn detects_formatting_only_blocks() {
        let joined = blocks(
            "x.rs",
            "a\nfoo(\n    x,\n    y\n);\nb\n",
            "a\nfoo(x, y);\nb\n",
        );
        assert_eq!(joined.len(), 1);
        assert!(joined[0].formatting_only);
        let real = blocks("x.rs", "a\nfoo(x, y);\nb\n", "a\nfoo(x, z);\nb\n");
        assert!(!real[0].formatting_only);
        let pure_add = blocks("x.rs", "a\nb\n", "a\n\nb\n");
        assert!(
            !pure_add[0].formatting_only,
            "an added blank line is a change"
        );
    }
}
