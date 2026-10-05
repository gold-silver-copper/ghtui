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
use crate::sat_u32;

/// Above this share of changed characters, a pair isn't highlighted.
const MAX_CHANGED_SHARE: f64 = 0.75;

/// Byte ranges of changed tokens, per alignment entry.
pub type IntraLine = HashMap<u32, Vec<(u32, u32)>>;

/// Splits `line` into tokens: runs of word characters, runs of whitespace,
/// and single other characters, further split at highlight boundaries.
/// Together the tokens make up the whole line.
fn tokens<'a>(line: &'a str, spans: &[Span]) -> Vec<&'a str> {
    let mut cuts: Vec<u32> = spans.iter().flat_map(|s| [s.start, s.end]).collect();
    cuts.sort_unstable();
    let mut out = Vec::new();
    let mut rest = line;
    let mut prev: Option<u8> = None; // 0 word, 1 space, 2 other
    for (i, c) in line.char_indices() {
        let class = if c.is_alphanumeric() || c == '_' {
            0
        } else if c.is_whitespace() {
            1
        } else {
            2
        };
        let at_cut = cuts.binary_search(&sat_u32(i)).is_ok();
        let start = line.len() - rest.len();
        if i > start
            && (at_cut || class == 2 || prev != Some(class))
            && let Some((token, tail)) = rest.split_at_checked(i - start)
        {
            out.push(token);
            rest = tail;
        }
        prev = Some(class);
    }
    if !rest.is_empty() {
        out.push(rest);
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
    let mut input = InternedInput::default();
    input.update_before(old_tokens.iter().copied());
    input.update_after(new_tokens.iter().copied());
    let diff = Diff::compute(Algorithm::Histogram, &input);

    let changed = |tokens: &[&str], removed: bool| -> Vec<(u32, u32)> {
        let mut ranges: Vec<(u32, u32)> = Vec::new();
        let mut start = 0u32;
        for (i, t) in (0u32..).zip(tokens) {
            let end = start.saturating_add(sat_u32(t.len()));
            let hit = if removed {
                diff.is_removed(i)
            } else {
                diff.is_added(i)
            };
            if hit {
                match ranges.last_mut() {
                    Some(last) if last.1 == start => last.1 = end,
                    _ => ranges.push((start, end)),
                }
            }
            start = end;
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
        let s = s.trim();
        let mut v: Vec<(char, char)> = s.chars().zip(s.chars().skip(1)).collect();
        v.sort_unstable();
        v
    };
    let (x, y) = (bigrams(a), bigrams(b));
    if x.is_empty() || y.is_empty() {
        return if a.trim() == b.trim() { 1.0 } else { 0.0 };
    }
    // Count common bigrams (multiset intersection of sorted lists).
    let (mut i, mut j, mut common) = (0, 0, 0usize);
    while let (Some(p), Some(q)) = (x.get(i), y.get(j)) {
        match p.cmp(q) {
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
    let starts = |run: &[DiffLine], kind| run.first().is_some_and(|l| l.kind == kind);
    // Runs of one kind, with the alignment entry each starts at.
    let mut runs = lines
        .chunk_by(|a, b| a.kind == b.kind)
        .scan(0, |end, run| {
            let start = *end;
            *end += run.len();
            Some((start, run))
        })
        .peekable();
    let mut out = IntraLine::new();
    while let Some((removed_start, removed)) = runs.next() {
        if !starts(removed, LineKind::Removed) {
            continue;
        }
        let Some((added_start, added)) = runs.next_if(|(_, run)| starts(run, LineKind::Added))
        else {
            continue;
        };
        let mut next_added = 0;
        for (r, line) in (removed_start..).zip(removed) {
            let Some(o) = line.old else {
                continue;
            };
            let old_line = text.old.line_no(o);
            let best = (next_added..)
                .zip(added.iter().skip(next_added).take(PAIR_WINDOW))
                .filter_map(|(a, l)| {
                    l.new
                        .map(|n| (a, n, similarity(old_line, text.new.line_no(n))))
                })
                .filter(|(_, _, sim)| *sim >= MIN_SIMILARITY)
                .max_by(|x, y| x.2.total_cmp(&y.2));
            let Some((a, n, _)) = best else {
                continue;
            };
            next_added = a + 1;
            if let Some((old_changed, new_changed)) = diff_pair(
                old_line,
                text.new.line_no(n),
                text.old_spans(o),
                text.new_spans(n),
            ) {
                out.insert(sat_u32(r), old_changed);
                out.insert(sat_u32(added_start + a), new_changed);
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
            .map(|(a, b)| line.get(*a as usize..*b as usize).unwrap())
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
        assert_eq!(t, ["say", " ", "\"", "x", "\"", " ", "now"]);
        assert_eq!(tokens("naïve über", &[]), ["naïve", " ", "über"]);
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
            .map(sat_u32)
            .unwrap();
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
