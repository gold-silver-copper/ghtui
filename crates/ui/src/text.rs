//! Width-aware text helpers.

use ratatui::buffer::CellWidth;
use ratatui::text::Span;
use unicode_segmentation::UnicodeSegmentation;

/// Columns `s` takes on screen, measured by ratatui's own rule a grapheme
/// at a time, so what fits is what's drawn (an emoji sequence joined to a
/// flag is two; a halfwidth sound mark takes a cell of its own). Characters
/// that are never drawn don't count: ratatui drops control characters, tabs
/// included (see `untab`), and anything of zero width, bidi overrides
/// included, so GitHub text can't reorder what's shown.
pub fn width(s: &str) -> usize {
    if s.is_ascii() {
        return s.bytes().filter(|b| !b.is_ascii_control()).count();
    }
    s.graphemes(true)
        .filter(|g| !g.contains(char::is_control))
        .map(|g| usize::from(g.cell_width()))
        .sum()
}

/// Columns `spans` take on screen.
pub fn spans_width<'a, 'b: 'a>(spans: impl IntoIterator<Item = &'a Span<'b>>) -> usize {
    spans.into_iter().map(|s| width(&s.content)).sum()
}

/// A grapheme from `graphemes` as drawn at column `col`: a tab is the
/// spaces to the next stop of four.
pub fn untab((g, w): (&str, usize), col: usize) -> (&str, usize) {
    const TAB: &str = "    ";
    match g {
        "\t" => TAB
            .get(col % TAB.len()..)
            .map_or((TAB, 4), |s| (s, s.len())),
        _ => (g, w),
    }
}

/// Characters that change or hide what code means without showing
/// themselves: control characters, bidi controls ("Trojan Source") and
/// zero-width spaces. Code views draw a marker in their place, as GitHub
/// warns about them. Joiners (emoji sequences) aren't included.
pub fn is_hidden(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{061C}'
                | '\u{200B}'
                | '\u{200E}'
                | '\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'
                | '\u{2066}'..='\u{2069}'
                | '\u{FEFF}'
        )
}

/// `s` with each tab turned into the spaces to the next stop of four,
/// counted from the start of its line.
pub fn expand_tabs(s: impl Into<String>) -> String {
    let s = s.into();
    if !s.contains('\t') {
        return s;
    }
    let mut col = 0;
    graphemes(&s)
        .map(|g| {
            let (g, w) = untab(g, col);
            col = if g.contains('\n') { 0 } else { col + w };
            g
        })
        .collect()
}

/// What a terminal draws as one character (an emoji with its modifiers,
/// a letter with its accents), with its width.
pub fn graphemes(s: &str) -> impl Iterator<Item = (&str, usize)> {
    s.graphemes(true).map(|g| (g, width(g)))
}

/// Cuts `s` to at most `max` columns, ending in `…` when shortened.
pub fn truncate(s: &str, max: usize) -> String {
    if width(s) <= max {
        return s.to_owned();
    }
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for (g, w) in graphemes(s) {
        if used + w > max - 1 {
            break;
        }
        out.push_str(g);
        used += w;
    }
    out.push('…');
    out
}

/// A commit id cut to GitHub's seven characters.
pub fn short_sha(oid: &str) -> &str {
    oid.get(..7).unwrap_or(oid)
}

/// 1536 → "1.5 KB".
pub fn size(bytes: u64) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{:.1} KB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

/// Greedy word wrap to `max` columns. Keeps blank lines (paragraph breaks)
/// and hard-breaks words longer than a line.
pub fn wrap(text: &str, max: usize) -> Vec<String> {
    let max = max.max(1);
    let mut lines = Vec::new();
    for raw in text.lines() {
        let raw = raw.trim_end();
        if raw.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut line = String::new();
        let mut used = 0;
        for word in raw.split(' ') {
            let w = width(word);
            if used > 0 && used + 1 + w > max {
                lines.push(std::mem::take(&mut line));
                used = 0;
            }
            if w > max {
                for (g, gw) in graphemes(word) {
                    if used + gw > max {
                        lines.push(std::mem::take(&mut line));
                        used = 0;
                    }
                    line.push_str(g);
                    used += gw;
                }
                continue;
            }
            if used > 0 {
                line.push(' ');
                used += 1;
            }
            line.push_str(word);
            used += w;
        }
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_counts_only_what_is_drawn() {
        assert_eq!(width("a\u{1b}[31mb"), 6);
        assert_eq!(width("a\u{1b}b"), 2);
        assert_eq!(width("a\u{202e}b"), 2);
        assert_eq!(width("漢字"), 4);
        assert!(is_hidden('\u{202e}') && is_hidden('\u{200b}') && !is_hidden('\u{200d}'));
    }

    #[test]
    fn truncates_with_ellipsis() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 6), "hello…");
        assert_eq!(truncate("日本語テキスト", 5), "日本…");
        assert_eq!(truncate("abc", 0), "");
        // Emoji are cut whole and measured as drawn.
        assert_eq!(truncate("⚠️ Fix warning in build", 5), "⚠️ F…");
        assert_eq!(truncate("👨‍🍳👨‍🍳👨‍🍳", 5), "👨‍🍳👨‍🍳…");
        assert!(width(&truncate("❤️❤️❤️abcdef", 5)) <= 5);
    }

    #[test]
    fn wraps_words() {
        assert_eq!(wrap("one two three", 7), ["one two", "three"]);
        assert_eq!(wrap("a\n\nb", 10), ["a", "", "b"]);
        assert_eq!(wrap("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert!(wrap("", 4).is_empty());
    }

    #[test]
    fn wrapped_lines_fit() {
        let text = "The quick brown fox jumps over the lazy dog, then keeps running far away.";
        for max in 1..30 {
            for line in wrap(text, max) {
                assert!(width(&line) <= max, "{max}: {line:?}");
            }
        }
    }
}
