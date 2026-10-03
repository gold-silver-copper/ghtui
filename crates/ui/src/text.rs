//! Width-aware text helpers.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
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
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > max - 1 {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
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
                for c in word.chars() {
                    let cw = c.width().unwrap_or(0);
                    if used + cw > max {
                        lines.push(std::mem::take(&mut line));
                        used = 0;
                    }
                    line.push(c);
                    used += cw;
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
