//! Width-aware text helpers.

use std::borrow::Cow;
use std::ops::Range;

use ratatui::buffer::CellWidth;
use ratatui::text::Span;
use unicode_segmentation::UnicodeSegmentation;

/// Columns `s` takes on screen, measured by ratatui's own rule a grapheme
/// at a time, so what fits is what's drawn (an emoji sequence joined to a
/// flag is two; a halfwidth sound mark takes a cell of its own). Characters
/// that are never drawn don't count: ratatui drops control characters, tabs
/// included (see `untab`), and anything of zero width, bidi overrides
/// included, so GitHub text can't reorder what's shown.
#[expect(
    clippy::disallowed_methods,
    reason = "ratatui's rule, a grapheme at a time"
)]
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
    fit(s, max).into_owned()
}

/// [`truncate`], without copying `s` when it fits: what drawing each
/// frame uses, where nearly everything does.
pub fn fit(s: &str, max: usize) -> Cow<'_, str> {
    if width(s) <= max {
        return Cow::Borrowed(s);
    }
    if max == 0 {
        return Cow::Borrowed("");
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
    Cow::Owned(out)
}

/// `word` as `n` of it: "file" or "files", "reply" or "replies", "branch"
/// or "branches".
pub fn noun(n: u64, word: &str) -> String {
    if n == 1 {
        return word.to_owned();
    }
    let consonant_y = word
        .strip_suffix('y')
        .filter(|stem| !stem.ends_with(['a', 'e', 'i', 'o', 'u']));
    match consonant_y {
        Some(stem) => format!("{stem}ies"),
        None if word.ends_with(['s', 'x']) || word.ends_with("ch") || word.ends_with("sh") => {
            format!("{word}es")
        }
        None => format!("{word}s"),
    }
}

/// A word of a line of code, as vim's `w` goes by them: a run of
/// identifier characters, or of other characters that aren't blank. It's
/// where it is in bytes, and in the columns it's drawn in (tabs to the
/// next stop of four, hidden characters as one marker, as code draws them).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Word {
    pub bytes: Range<usize>,
    pub cols: Range<usize>,
}

/// The words of `line`, in order.
pub fn words(line: &str) -> Vec<Word> {
    let mut out: Vec<Word> = Vec::new();
    let (mut byte, mut col) = (0, 0);
    let mut last = None;
    for (g, w) in graphemes(line) {
        let w = match untab((g, w), col) {
            (drawn, _) if drawn.contains(is_hidden) => 1,
            (_, w) => w,
        };
        // Blank, an identifier's, or punctuation's.
        let class = (g.chars().next())
            .filter(|c| !c.is_whitespace() && !is_hidden(*c))
            .map(is_ident);
        let (end, to) = (byte + g.len(), col + w);
        match (class, out.last_mut()) {
            (Some(_), Some(word)) if class == last => (word.bytes.end, word.cols.end) = (end, to),
            (Some(_), _) => out.push(Word {
                bytes: byte..end,
                cols: col..to,
            }),
            (None, _) => {}
        }
        (last, byte, col) = (class, end, to);
    }
    out
}

/// The word at column `col` of `words`, or the one nearest it (the left
/// one of two as near).
pub fn word_at(words: &[Word], col: usize) -> Option<&Word> {
    words.iter().min_by_key(|w| {
        w.cols
            .start
            .saturating_sub(col)
            .max((col + 1).saturating_sub(w.cols.end))
    })
}

/// `w` (or `b`) from the word at column `col` of `line`: the next word
/// (or the one before). With no word picked yet, the first.
pub fn step_word(line: &str, col: Option<usize>, forward: bool) -> Option<Word> {
    let words = words(line);
    let Some(col) = col else { return words.into_iter().next() };
    let at = word_at(&words, col)?.cols.start;
    if forward {
        words.into_iter().find(|w| w.cols.start > at)
    } else {
        words.into_iter().rev().find(|w| w.cols.start < at)
    }
}

/// How far to scroll sideways, from `shift`, so columns `cols` show in
/// `room` columns of code.
pub fn reveal(shift: usize, cols: &Range<usize>, room: usize) -> usize {
    if cols.start < shift {
        cols.start
    } else if cols.end > shift + room {
        cols.start.min(cols.end - room)
    } else {
        shift
    }
}

/// Whether `c` is part of an identifier, as [`words`] splits them.
fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `word` as `*` and `#` search it: a whole word, between vim's `\<` and
/// `\>` where its ends are an identifier's.
pub fn whole_word(word: &str) -> String {
    let start = if word.starts_with(is_ident) {
        "\\<"
    } else {
        ""
    };
    let end = if word.ends_with(is_ident) { "\\>" } else { "" };
    format!("{start}{word}{end}")
}

/// Where `query` is in `hay`, by bytes. Behind vim's `\<` it only starts
/// a word, and before `\>` only ends one: `\<io\>` isn't in `ioctl`.
pub fn hits<'a>(hay: &'a str, query: &'a str) -> impl Iterator<Item = Range<usize>> + 'a {
    let (start, needle) = match query.strip_prefix("\\<") {
        Some(rest) => (true, rest),
        None => (false, query),
    };
    let (end, needle) = match needle.strip_suffix("\\>") {
        Some(rest) => (true, rest),
        None => (false, needle),
    };
    let at_edge = move |r: &Range<usize>| {
        let before = hay.get(..r.start).and_then(|s| s.chars().next_back());
        let after = hay.get(r.end..).and_then(|s| s.chars().next());
        !(start && before.is_some_and(is_ident)) && !(end && after.is_some_and(is_ident))
    };
    (hay.match_indices(needle))
        .filter(move |_| !needle.is_empty())
        .map(|(at, m)| at..at + m.len())
        .filter(at_edge)
}

/// Whether `query` is in `hay`, as [`hits`] finds it.
pub fn has(hay: &str, query: &str) -> bool {
    hits(hay, query).next().is_some()
}

/// The web address in `line` that bytes `at` are part of: from `http://`
/// or `https://` to a blank or a quote, less punctuation that ends it.
pub fn url_at<'a>(line: &'a str, at: &Range<usize>) -> Option<&'a str> {
    let mut from = 0;
    while let Some(found) = line.get(from..)?.find("http") {
        let start = from + found;
        let rest = line.get(start..)?;
        let len = rest
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '<' | '>'))
            .unwrap_or(rest.len());
        let url = rest
            .get(..len)?
            .trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '}']);
        let scheme = ["https://", "http://"]
            .into_iter()
            .find(|s| url.starts_with(s));
        if scheme.is_some_and(|s| url.len() > s.len())
            && start < at.end
            && at.start < start + url.len()
        {
            return Some(url);
        }
        from = start + len.max(1);
    }
    None
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

    /// Words are vim's: identifiers, and runs of punctuation, by the
    /// columns they're drawn in; a web address is found from any of its
    /// words.
    #[test]
    fn words_go_as_vims_do() {
        let line = "\tlet x_1 = a.b(); // https://example.com/a?b=c.";
        let shown: Vec<&str> = (words(line).iter())
            .map(|w| line.get(w.bytes.clone()).unwrap())
            .collect();
        let expected = [
            "let", "x_1", "=", "a", ".", "b", "();", "//", "https", "://",
        ];
        assert_eq!(shown[..10], expected);
        assert_eq!(words(line)[0].cols, 4..7, "after the tab");
        let example = words(line)
            .into_iter()
            .find(|w| line.get(w.bytes.clone()).unwrap() == "example");
        let url = url_at(line, &example.unwrap().bytes);
        assert_eq!(url, Some("https://example.com/a?b=c"));
        assert_eq!(url_at(line, &(1..4)), None);
        assert_eq!(step_word(line, Some(5), true).unwrap().cols, 8..11);
        assert_eq!(step_word(line, Some(5), false), None);
    }

    /// `*` searches a whole word: not inside a longer one.
    #[test]
    fn whole_words_match_only_whole() {
        let io = whole_word("io");
        assert_eq!(io, "\\<io\\>");
        let found = |hay| hits(hay, &io).collect::<Vec<_>>();
        assert_eq!(found("use std::io;").first(), Some(&(9..11)));
        assert!(found("ioctl(fd); ratio").is_empty());
        assert!(has("ratio", "io"), "a plain search still finds it inside");
        assert_eq!(whole_word("->"), "->", "punctuation has no word edges");
        assert!(has("a->b", &whole_word("->")));
    }

    #[test]
    fn counts_say_one_or_many() {
        let say = |n, word| format!("{n} {}", noun(n, word));
        assert_eq!(say(1, "file"), "1 file");
        assert_eq!(say(0, "file"), "0 files");
        assert_eq!(say(2, "repository"), "2 repositories");
        assert_eq!(say(3, "branch"), "3 branches");
        assert_eq!(say(4, "day"), "4 days");
        assert_eq!(say(5, "match"), "5 matches");
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
}
