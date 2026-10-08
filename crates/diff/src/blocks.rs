//! Change blocks: maximal runs of changed lines in an alignment.
//!
//! A block's hash covers the file path and the block's removed and added
//! text, not line numbers, so the same change hashes the same wherever it
//! sits. That makes it the unit for "reviewed" marks and for "changes since
//! my last review" (a block whose hash appeared in the earlier diff isn't
//! new, however the lines around it moved or the base was rebased).
//! It names an exact change: a whitespace-ignoring block that alone lies in
//! one exact block takes its hash, so marks hold in either mode.

use std::collections::HashMap;
use std::ops::Range;

use crate::file::{Alignment, TextDiff};
use crate::hunks::DiffLine;
use crate::sat_u32;

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ChangeBlock {
    /// Alignment entries of the block.
    pub entries: Range<u32>,
    pub hash: String,
    /// The removed and added text differ only in whitespace and line breaks.
    pub formatting_only: bool,
}

pub(crate) fn change_blocks(path: &str, text: &TextDiff, lines: &[DiffLine]) -> Vec<ChangeBlock> {
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

/// Names each of `ignoring`'s blocks that alone lies in one exact block (a
/// line is the same entry in both) after it; others stay new.
pub(crate) fn name_after(exact: &Alignment, ignoring: &mut Alignment) {
    let names: HashMap<DiffLine, &String> = (exact.blocks.iter())
        .flat_map(|b| b.entries.clone().map(move |e| (e, &b.hash)))
        .filter_map(|(e, hash)| Some((*exact.lines.get(e as usize)?, hash)))
        .collect();
    let name = |e: u32| ignoring.lines.get(e as usize).and_then(|l| names.get(l));
    let mut takers = HashMap::new();
    for (i, b) in ignoring.blocks.iter().enumerate() {
        let (first, mut rest) = (name(b.entries.start), b.entries.clone().map(name));
        if let Some(h) = first.filter(|_| rest.all(|n| n == first)) {
            takers.entry(h).and_modify(|t| *t = None).or_insert(Some(i));
        }
    }
    for (hash, i) in takers {
        if let Some(b) = i.and_then(|i| ignoring.blocks.get_mut(i)) {
            b.hash.clone_from(hash);
        }
    }
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
        text.alignment(Whitespace::Exact).blocks.clone()
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

    #[test]
    fn an_exact_change_split_by_ignoring_whitespace_names_no_part() {
        let text = text_diff("x.rs", "a\n  b\nc\n", "A\n    b\nC\n");
        let exact = &text.alignment(Whitespace::Exact).blocks;
        let parts = &text.alignment(Whitespace::Ignore).blocks;
        assert_eq!((exact.len(), parts.len()), (1, 2));
        assert_ne!(parts[0].hash, parts[1].hash);
        assert!(parts.iter().all(|p| p.hash != exact[0].hash));
    }
}
