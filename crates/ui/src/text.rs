//! Width-aware text helpers.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
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
