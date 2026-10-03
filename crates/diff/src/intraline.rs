//! Intra-line diffs: which tokens changed between a removed line and the
//! added line it pairs with.
//!
//! Lines are split into tokens at word boundaries and at syntax-highlight
//! boundaries (tree-sitter tokens), then the token sequences are diffed.
//! When most of a line changed, nothing is highlighted: emphasis on every
//! token is just noise.

use std::collections::HashMap;

use imara_diff::{Algorithm, Diff, InternedInput};

use crate::file::TextDiff;
use crate::highlight::Span;
use crate::hunks::{DiffLine, LineKind};

/// Above this share of changed characters, a pair isn't highlighted.
const MAX_CHANGED_SHARE: f64 = 0.75;

/// Byte ranges of changed tokens, per alignment entry.
pub type IntraLine = HashMap<u32, Vec<(u32, u32)>>;

/// Splits `line` into tokens: runs of word characters, runs of whitespace,
/// and single other characters, further split at highlight boundaries.
fn tokens(line: &str, spans: &[Span]) -> Vec<(u32, u32)> {
    let mut cuts: Vec<u32> = spans.iter().flat_map(|s| [s.start, s.end]).collect();
    cuts.sort_unstable();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut prev: Option<u8> = None; // 0 word, 1 space, 2 other
    for (i, c) in line.char_indices() {
        let class = if c.is_alphanumeric() || c == '_' {
            0
        } else if c.is_whitespace() {
            1
        } else {
            2
        };
        let at_cut = cuts.binary_search(&(i as u32)).is_ok();
        let boundary = i > start && (at_cut || class == 2 || prev != Some(class));
        if boundary {
            out.push((start as u32, i as u32));
            start = i;
        }
        prev = Some(class);
    }
    if start < line.len() {
        out.push((start as u32, line.len() as u32));
    }
    out
}

/// Changed byte ranges of the old and the new line.
pub type ChangedRanges = (Vec<(u32, u32)>, Vec<(u32, u32)>);

/// Changed byte ranges in `old` and `new`, or `None` when too much changed
/// to be worth highlighting.
pub fn diff_pair(
    old: &str,
    new: &str,
    old_spans: &[Span],
    new_spans: &[Span],
) -> Option<ChangedRanges> {
    let (old_tokens, new_tokens) = (tokens(old, old_spans), tokens(new, new_spans));
    let text = |line: &str, t: &(u32, u32)| line[t.0 as usize..t.1 as usize].to_owned();
    let before: Vec<String> = old_tokens.iter().map(|t| text(old, t)).collect();
    let after: Vec<String> = new_tokens.iter().map(|t| text(new, t)).collect();
    let mut input = InternedInput::default();
    input.update_before(before.iter().map(String::as_str));
    input.update_after(after.iter().map(String::as_str));
    let diff = Diff::compute(Algorithm::Histogram, &input);

    let changed = |tokens: &[(u32, u32)], removed: bool| -> Vec<(u32, u32)> {
        let mut ranges: Vec<(u32, u32)> = Vec::new();
        for (i, t) in tokens.iter().enumerate() {
            let hit = if removed {
                diff.is_removed(i as u32)
            } else {
                diff.is_added(i as u32)
            };
            if !hit {
                continue;
            }
            match ranges.last_mut() {
                Some(last) if last.1 == t.0 => last.1 = t.1,
                _ => ranges.push(*t),
            }
        }
        ranges
    };
    let (old_changed, new_changed) = (changed(&old_tokens, true), changed(&new_tokens, false));
    let share = |ranges: &[(u32, u32)], line: &str| {
        let changed: u32 = ranges.iter().map(|(a, b)| b - a).sum();
        f64::from(changed) / line.trim().len().max(1) as f64
    };
    if old_changed.is_empty() && new_changed.is_empty() {
        return None;
    }
    if share(&old_changed, old) > MAX_CHANGED_SHARE || share(&new_changed, new) > MAX_CHANGED_SHARE
    {
        return None;
    }
    Some((old_changed, new_changed))
}

/// Lines this similar or more (character-bigram Dice coefficient) pair up.
const MIN_SIMILARITY: f64 = 0.5;
/// How far ahead to look for a line's partner.
const PAIR_WINDOW: usize = 16;

/// Dice coefficient over character bigrams of the trimmed lines.
fn similarity(a: &str, b: &str) -> f64 {
    let bigrams = |s: &str| -> Vec<(char, char)> {
        let chars: Vec<char> = s.trim().chars().collect();
        let mut v: Vec<(char, char)> = chars.windows(2).map(|w| (w[0], w[1])).collect();
        v.sort_unstable();
        v
    };
    let (x, y) = (bigrams(a), bigrams(b));
    if x.is_empty() || y.is_empty() {
        return if a.trim() == b.trim() { 1.0 } else { 0.0 };
    }
    // Count common bigrams (multiset intersection of sorted lists).
    let (mut i, mut j, mut common) = (0, 0, 0usize);
    while i < x.len() && j < y.len() {
        match x[i].cmp(&y[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                common += 1;
                i += 1;
                j += 1;
            }
        }
    }
    2.0 * common as f64 / (x.len() + y.len()) as f64
}

/// Intra-line changes for paired lines in `lines`. Within each change
/// block, removed and added lines are paired by similarity, in order (not
/// just by position), so an edited line finds its counterpart even when
/// lines were inserted or deleted around it.
pub fn intraline(text: &TextDiff, lines: &[DiffLine]) -> IntraLine {
    let mut out = IntraLine::new();
    let mut e = 0;
    while e < lines.len() {
        if lines[e].kind == LineKind::Context {
            e += 1;
            continue;
        }
        let removed_start = e;
        while e < lines.len() && lines[e].kind == LineKind::Removed {
            e += 1;
        }
        let added_start = e;
        while e < lines.len() && lines[e].kind == LineKind::Added {
            e += 1;
        }
        let old_of = |r: usize| lines[r].old.map(|o| (o, text.old.line(o as usize - 1)));
        let new_of = |a: usize| lines[a].new.map(|n| (n, text.new.line(n as usize - 1)));
        let mut next_added = added_start;
        for r in removed_start..added_start {
            let Some((o, old_line)) = old_of(r) else {
                continue;
            };
            let best = (next_added..e.min(next_added + PAIR_WINDOW))
                .filter_map(|a| new_of(a).map(|(n, l)| (a, n, similarity(old_line, l))))
                .filter(|(_, _, sim)| *sim >= MIN_SIMILARITY)
                .max_by(|x, y| x.2.total_cmp(&y.2));
            let Some((a, n, _)) = best else {
                continue;
            };
            next_added = a + 1;
            let new_line = text.new.line(n as usize - 1);
            if let Some((old_changed, new_changed)) =
                diff_pair(old_line, new_line, text.old_spans(o), text.new_spans(n))
            {
                out.insert(r as u32, old_changed);
                out.insert(a as u32, new_changed);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changed<'a>(line: &'a str, ranges: &[(u32, u32)]) -> Vec<&'a str> {
        ranges
            .iter()
            .map(|(a, b)| &line[*a as usize..*b as usize])
            .collect()
    }

    #[test]
    fn highlights_changed_tokens() {
        let (old, new) = ("let total = compute(a, b);", "let total = compute(a, c);");
        let (o, n) = diff_pair(old, new, &[], &[]).unwrap();
        assert_eq!(changed(old, &o), ["b"]);
        assert_eq!(changed(new, &n), ["c"]);
    }

    #[test]
    fn whole_word_granularity() {
        let (old, new) = ("value_count += 1", "value_total += 1");
        let (o, n) = diff_pair(old, new, &[], &[]).unwrap();
        assert_eq!(changed(old, &o), ["value_count"]);
        assert_eq!(changed(new, &n), ["value_total"]);
    }

    #[test]
    fn mostly_changed_lines_get_no_emphasis() {
        assert!(diff_pair("fn a() {}", "struct Totally { different: u8 }", &[], &[]).is_none());
        assert!(diff_pair("same", "same", &[], &[]).is_none());
    }

    #[test]
    fn tokens_split_at_highlight_boundaries() {
        // A string literal token boundary splits "x" from the quote marks.
        let spans = [Span {
            start: 4,
            end: 7,
            kind: crate::highlight::TokenKind::String,
        }];
        let t = tokens("say \"x\" now", &spans);
        assert!(t.contains(&(4, 5)), "{t:?}");
    }

    #[test]
    fn pairs_by_similarity_not_position() {
        use crate::file::{Content, FileDiff};
        use crate::hunks::Whitespace;
        // Two lines deleted before the edited one: position would pair the
        // edit with an unrelated line.
        let diff = FileDiff::compute(
            "x.rs",
            Some(
                b"keep\nfn unrelated() {}\nstruct Gone;\nlet total = compute(alpha, beta);\nkeep\n",
            ),
            Some(b"keep\nlet total = compute(alpha, gamma);\nkeep\n"),
        );
        let Content::Text(text) = &diff.content else {
            panic!()
        };
        let lines = text.lines(Whitespace::Exact);
        let map = intraline(text, lines);
        let added = lines
            .iter()
            .position(|l| l.kind == LineKind::Added)
            .unwrap() as u32;
        let new_line = text.new.line(1);
        assert_eq!(changed(new_line, &map[&added]), ["gamma"]);
        assert!(similarity("abc def", "abc deg") > 0.5);
        assert!(similarity("fn unrelated() {}", "let total = 1;") < 0.5);
    }

    #[test]
    fn pairs_lines_within_blocks() {
        use crate::file::{Content, FileDiff};
        use crate::hunks::Whitespace;
        let diff = FileDiff::compute(
            "x.txt",
            Some(b"keep\nalpha beta\ngamma delta\nkeep\n"),
            Some(b"keep\nalpha BETA\ngamma delta epsilon\nkeep\n"),
        );
        let Content::Text(text) = &diff.content else {
            panic!()
        };
        let map = intraline(text, text.lines(Whitespace::Exact));
        assert_eq!(map.len(), 4, "{map:?}");
    }
}
