//! Intra-line diffs: which tokens changed between a removed line and the
//! added line it pairs with.
//!
//! Lines are split into tokens at word boundaries and at syntax-highlight
//! boundaries (tree-sitter tokens), then the token sequences are diffed.
//! When most of a line changed, nothing is highlighted: emphasis on every
//! token is just noise.

use std::collections::HashMap;

use imara_diff::{Algorithm, Diff, InternedInput};
use unicode_segmentation::UnicodeSegmentation;

use crate::file::TextDiff;
use crate::highlight::Span;
use crate::hunks::{DiffLine, LineKind};
use crate::sat_u32;

/// Above this share of changed characters, a pair isn't highlighted.
const MAX_CHANGED_SHARE: f64 = 0.75;

/// Byte ranges of changed tokens, per alignment entry.
pub type IntraLine = HashMap<u32, Vec<(u32, u32)>>;

/// Splits `line` into tokens: runs of word characters, runs of whitespace,
/// and single other characters (whole graphemes, so a letter keeps its
/// accent), further split at highlight boundaries. Together the tokens make
/// up the whole line.
fn tokens<'a>(line: &'a str, spans: &[Span]) -> Vec<&'a str> {
    let mut cuts: Vec<u32> = spans.iter().flat_map(|s| [s.start, s.end]).collect();
    cuts.sort_unstable();
    let mut out = Vec::new();
    let mut rest = line;
    let mut prev: Option<u8> = None; // 0 word, 1 space, 2 other
    for (i, g) in line.grapheme_indices(true) {
        let class = match g.chars().next() {
            Some(c) if c.is_alphanumeric() || c == '_' => 0,
            Some(c) if c.is_whitespace() => 1,
            _ => 2,
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

    // Where each token starts (and the line's end), in bytes.
    let offsets = |tokens: &[&str]| -> Vec<u32> {
        std::iter::once(0)
            .chain(tokens.iter().scan(0u32, |at, t| {
                *at = at.saturating_add(sat_u32(t.len()));
                Some(*at)
            }))
            .collect()
    };
    let (old_at, new_at) = (offsets(&old_tokens), offsets(&new_tokens));
    let span = |at: &[u32], r: std::ops::Range<u32>| {
        let get = |i: u32| at.get(i as usize).copied().unwrap_or_default();
        (get(r.start), get(r.end))
    };
    let (mut old_changed, mut new_changed) = (Vec::new(), Vec::new());
    for hunk in diff.hunks() {
        let (mut o, mut n) = (
            span(&old_at, hunk.before.clone()),
            span(&new_at, hunk.after.clone()),
        );
        // One token for another: emphasise only the characters that differ.
        if hunk.before.len() == 1 && hunk.after.len() == 1 {
            let (a, b) = (
                old.get(o.0 as usize..o.1 as usize).unwrap_or_default(),
                new.get(n.0 as usize..n.1 as usize).unwrap_or_default(),
            );
            if let Some((prefix, suffix)) = shared_ends(a, b) {
                o = (o.0 + prefix, o.1 - suffix);
                n = (n.0 + prefix, n.1 - suffix);
            }
        }
        old_changed.extend(Some(o).filter(|(a, b)| a < b));
        new_changed.extend(Some(n).filter(|(a, b)| a < b));
    }
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

/// The bytes two tokens share at their start and end (by grapheme, never
/// overlapping), when that's most of them: `count`/`counts` share `count`,
/// so only the `s` stands out. `None` for tokens that merely look alike
/// (`width`/`words`), which stay emphasised whole.
fn shared_ends(a: &str, b: &str) -> Option<(u32, u32)> {
    let (ga, gb): (Vec<&str>, Vec<&str>) =
        (a.graphemes(true).collect(), b.graphemes(true).collect());
    let shorter = ga.len().min(gb.len());
    let prefix = ga.iter().zip(&gb).take_while(|(x, y)| x == y).count();
    let suffix = ga
        .iter()
        .rev()
        .zip(gb.iter().rev())
        .take(shorter - prefix)
        .take_while(|(x, y)| x == y)
        .count();
    let kept = prefix + suffix;
    if kept < 2 || kept * 2 < shorter {
        return None;
    }
    let bytes = |gs: &[&str]| sat_u32(gs.iter().map(|g| g.len()).sum());
    Some((
        bytes(ga.get(..prefix).unwrap_or_default()),
        bytes(ga.get(ga.len() - suffix..).unwrap_or_default()),
    ))
}

/// Lines this similar or more (character-bigram Dice coefficient) pair up.
const MIN_SIMILARITY: f64 = 0.5;
/// Blocks with more lines than this on a side pair greedily instead.
const MAX_ALIGNED: usize = 200;

/// Character bigrams of the trimmed line, sorted: what `similarity` compares.
fn bigrams(line: &str) -> Vec<(char, char)> {
    let s = line.trim();
    let mut v: Vec<(char, char)> = s.chars().zip(s.chars().skip(1)).collect();
    v.sort_unstable();
    v
}

/// Dice coefficient of two lines' sorted bigrams.
fn dice(x: &[(char, char)], y: &[(char, char)]) -> f64 {
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
    2.0 * common as f64 / (x.len() + y.len()).max(1) as f64
}

/// Dice coefficient over character bigrams of the trimmed lines.
fn similarity(a: &str, b: &str) -> f64 {
    let (x, y) = (bigrams(a), bigrams(b));
    if x.is_empty() || y.is_empty() {
        return if a.trim() == b.trim() { 1.0 } else { 0.0 };
    }
    dice(&x, &y)
}

/// Which removed line pairs with which added line, in order: the pairing
/// with the greatest total similarity, lines too unlike anything left
/// unpaired. Big blocks pair each removed line with its best match ahead.
fn pair_lines(old: &[&str], new: &[&str]) -> Vec<(usize, usize)> {
    #[derive(Clone, Copy)]
    enum Step {
        Start,
        SkipOld,
        SkipNew,
        Pair,
    }
    let score = |a: &str, b: &str| Some(similarity(a, b)).filter(|s| *s >= MIN_SIMILARITY);
    if old.len() > MAX_ALIGNED || new.len() > MAX_ALIGNED {
        let mut pairs = Vec::new();
        let mut next = 0;
        for (i, a) in old.iter().enumerate() {
            let best = (next..)
                .zip(new.iter().skip(next).take(16))
                .filter_map(|(j, b)| Some((j, score(a, b)?)))
                .max_by(|x, y| x.1.total_cmp(&y.1));
            if let Some((j, _)) = best {
                pairs.push((i, j));
                next = j + 1;
            }
        }
        return pairs;
    }
    let new_grams: Vec<_> = new.iter().map(|b| bigrams(b)).collect();
    // `table[i][j]`: the best total for the first `i` old and `j` new lines.
    let mut table: Vec<Vec<(f64, Step)>> = vec![vec![(0.0, Step::Start); new.len() + 1]];
    for a in old {
        let grams = bigrams(a);
        let prev = table.last().cloned().unwrap_or_default();
        let mut row = vec![(0.0, Step::SkipOld)];
        for (j, b) in new.iter().enumerate() {
            let pair = match (grams.is_empty(), new_grams.get(j)) {
                (false, Some(g)) if !g.is_empty() => dice(&grams, g),
                _ => similarity(a, b),
            };
            let skip_old = (prev.get(j + 1).map_or(0.0, |c| c.0), Step::SkipOld);
            let skip_new = (row.last().map_or(0.0, |c: &(f64, Step)| c.0), Step::SkipNew);
            let mut best = if skip_old.0 >= skip_new.0 {
                skip_old
            } else {
                skip_new
            };
            if pair >= MIN_SIMILARITY {
                let paired = prev.get(j).map_or(0.0, |c| c.0) + pair;
                if paired > best.0 {
                    best = (paired, Step::Pair);
                }
            }
            row.push(best);
        }
        table.push(row);
    }
    // Walk back from the end.
    let mut pairs = Vec::new();
    let (mut i, mut j) = (old.len(), new.len());
    while let Some(&(_, step)) = table.get(i).and_then(|row| row.get(j)) {
        match step {
            Step::Start => break,
            Step::SkipOld => i = i.saturating_sub(1),
            Step::SkipNew => j = j.saturating_sub(1),
            Step::Pair => {
                i = i.saturating_sub(1);
                j = j.saturating_sub(1);
                pairs.push((i, j));
            }
        }
        if i == 0 && j == 0 {
            break;
        }
    }
    pairs.reverse();
    pairs
}

/// Intra-line changes for paired lines in `lines`. Within each change
/// block, removed and added lines are paired by similarity (see
/// [`pair_lines`]), so an edited line finds its counterpart even when lines
/// were inserted or deleted around it.
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
        let old: Vec<u32> = removed.iter().filter_map(|l| l.old).collect();
        let new: Vec<u32> = added.iter().filter_map(|l| l.new).collect();
        let old_text: Vec<&str> = old.iter().map(|&o| text.old.line_no(o)).collect();
        let new_text: Vec<&str> = new.iter().map(|&n| text.new.line_no(n)).collect();
        for (r, a) in pair_lines(&old_text, &new_text) {
            let (Some(&o), Some(&n)) = (old.get(r), new.get(a)) else {
                continue;
            };
            if let Some((old_changed, new_changed)) = diff_pair(
                text.old.line_no(o),
                text.new.line_no(n),
                text.old_spans(o),
                text.new_spans(n),
            ) {
                out.insert(sat_u32(removed_start + r), old_changed);
                out.insert(sat_u32(added_start + a), new_changed);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file::tests::text_diff;
    use crate::hunks::Whitespace;

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
    fn the_shared_part_of_a_word_stays_plain() {
        let (old, new) = ("value_count += 1", "value_total += 1");
        let (o, n) = diff_pair(old, new, &[], &[]).unwrap();
        assert_eq!(changed(old, &o), ["count"]);
        assert_eq!(changed(new, &n), ["total"]);
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
        // Two lines deleted before the edited one: position would pair the
        // edit with an unrelated line.
        let text = text_diff(
            "x.rs",
            "keep\nfn unrelated() {}\nstruct Gone;\nlet total = compute(alpha, beta);\nkeep\n",
            "keep\nlet total = compute(alpha, gamma);\nkeep\n",
        );
        let lines = text.lines(Whitespace::Exact);
        let map = intraline(&text, lines);
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
        let text = text_diff(
            "x.txt",
            "keep\nalpha beta\ngamma delta\nkeep\n",
            "keep\nalpha BETA\ngamma delta epsilon\nkeep\n",
        );
        let map = intraline(&text, text.lines(Whitespace::Exact));
        assert_eq!(map.len(), 4, "{map:?}");
    }
}
