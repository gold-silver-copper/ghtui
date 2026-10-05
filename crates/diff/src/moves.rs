//! Moved-code detection, in the spirit of `git diff --color-moved`.
//!
//! A run of removed lines that reappears as a run of added lines (in the
//! same file or another) is a move. Lines are compared with surrounding
//! whitespace trimmed, so re-indented code still counts. To avoid noise,
//! a move needs at least [`MIN_LINES`] lines and [`MIN_CHARS`] non-blank
//! characters, and can't start on a trivial line (`}`, blank, ...).

use std::collections::HashMap;
use std::ops::Range;

use crate::file::TextDiff;
use crate::hunks::{DiffLine, LineKind};
use crate::sat_u32;

pub const MIN_LINES: usize = 3;
pub const MIN_CHARS: usize = 30;

/// Where a moved block was removed and where it was added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Move {
    /// `(file index, alignment entries)` of the removed lines.
    pub from: (usize, Range<u32>),
    /// `(file index, alignment entries)` of the added lines.
    pub to: (usize, Range<u32>),
}

fn key(line: &str) -> &str {
    line.trim()
}

fn trivial(line: &str) -> bool {
    line.chars().filter(|c| c.is_alphanumeric()).count() < 2
}

/// Moves among `files`: `(file index, text, alignment)`.
pub fn detect_moves(files: &[(usize, &TextDiff, &[DiffLine])]) -> Vec<Move> {
    // Every added line, by content.
    let mut added: HashMap<&str, Vec<(usize, usize)>> = HashMap::new();
    for (fi, (_, text, lines)) in files.iter().enumerate() {
        for (e, line) in lines.iter().enumerate() {
            if line.kind == LineKind::Added {
                let content = key(text.text(line));
                if !content.is_empty() {
                    added.entry(content).or_default().push((fi, e));
                }
            }
        }
    }
    let mut used_added: Vec<Vec<bool>> =
        files.iter().map(|(_, _, l)| vec![false; l.len()]).collect();
    let mut moves = Vec::new();

    for (file_index, text, lines) in files.iter() {
        let mut e = 0;
        while let Some(line) = lines.get(e) {
            if line.kind != LineKind::Removed || trivial(text.text(line)) {
                e += 1;
                continue;
            }
            // Longest run of consecutive removed lines matching consecutive
            // added lines, over every candidate start.
            let mut best: Option<(usize, usize, usize)> = None; // (target file, target entry, length)
            for &(tf, te) in added
                .get(key(text.text(line)))
                .map_or(&[][..], Vec::as_slice)
            {
                let (Some((_, ttext, tlines)), Some(used)) = (files.get(tf), used_added.get(tf))
                else {
                    continue;
                };
                if used.get(te).is_none_or(|&u| u) {
                    continue;
                }
                let n = lines
                    .iter()
                    .skip(e)
                    .zip(tlines.iter().zip(used).skip(te))
                    .take_while(|(l, (t, used))| {
                        l.kind == LineKind::Removed
                            && t.kind == LineKind::Added
                            && !**used
                            && key(text.text(l)) == key(ttext.text(t))
                    })
                    .count();
                if best.is_none_or(|(_, _, len)| n > len) {
                    best = Some((tf, te, n));
                }
            }
            let chars = |n| -> usize {
                let run = lines.iter().skip(e).take(n);
                run.map(|l| key(text.text(l)).len()).sum()
            };
            let Some((tf, te, n)) =
                best.filter(|&(_, _, n)| n >= MIN_LINES && chars(n) >= MIN_CHARS)
            else {
                e += 1;
                continue;
            };
            let (Some((target, _, _)), Some(used)) = (files.get(tf), used_added.get_mut(tf)) else {
                e += 1;
                continue;
            };
            for used in used.iter_mut().skip(te).take(n) {
                *used = true;
            }
            moves.push(Move {
                from: (*file_index, sat_u32(e)..sat_u32(e + n)),
                to: (*target, sat_u32(te)..sat_u32(te + n)),
            });
            e += n;
        }
    }
    moves
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file::{Content, FileDiff};
    use crate::hunks::Whitespace;

    fn text(path: &str, old: &str, new: &str) -> Box<TextDiff> {
        match FileDiff::compute(path, Some(old.as_bytes()), Some(new.as_bytes())).content {
            Content::Text(t) => t,
            other => panic!("{other:?}"),
        }
    }

    const BLOCK: &str = "fn helper(x: u32) -> u32 {\n    let y = x * 2;\n    y + 1\n}\n";

    #[test]
    fn finds_moves_across_files() {
        let a = text(
            "a.rs",
            &format!("fn main() {{}}\n{BLOCK}"),
            "fn main() {}\n",
        );
        let b = text("b.rs", "// b\n", &format!("// b\n{BLOCK}"));
        let moves = detect_moves(&[
            (0, &a, a.lines(Whitespace::Exact)),
            (1, &b, b.lines(Whitespace::Exact)),
        ]);
        assert_eq!(moves.len(), 1, "{moves:?}");
        assert_eq!(moves[0].from.0, 0);
        assert_eq!(moves[0].to.0, 1);
        assert!(moves[0].from.1.len() >= 3);
    }

    #[test]
    fn finds_moves_within_a_file_even_reindented() {
        let indented: String = BLOCK.lines().map(|l| format!("    {l}\n")).collect();
        let old = format!("{BLOCK}mod inner {{\n}}\n");
        let new = format!("mod inner {{\n{indented}}}\n");
        let t = text("a.rs", &old, &new);
        let moves = detect_moves(&[(0, &t, t.lines(Whitespace::Exact))]);
        assert_eq!(moves.len(), 1, "{moves:?}");
    }

    #[test]
    fn ignores_small_or_trivial_matches() {
        let t = text("a.rs", "x\n}\n}\n}\ny\n", "y\n}\n}\n}\nx\n");
        assert!(detect_moves(&[(0, &t, t.lines(Whitespace::Exact))]).is_empty());
        let short = text("a.rs", "let a = 1;\nkeep\n", "keep\nlet a = 1;\n");
        assert!(detect_moves(&[(0, &short, short.lines(Whitespace::Exact))]).is_empty());
    }
}
