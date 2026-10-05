//! Line diffs with context, as hunks.

pub use imara_diff::Algorithm;
use imara_diff::{Diff, InternedInput};

use crate::sat_u32;
use crate::text::Text;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Removed,
    Added,
}

/// One line of a hunk. Line numbers are 1-based, as in files and in
/// GitHub's comment API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old: Option<u32>,
    pub new: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    /// 1-based start and length, as in `@@ -a,b +c,d @@`.
    pub old_start: u32,
    pub old_len: u32,
    pub new_start: u32,
    pub new_len: u32,
    pub lines: Vec<DiffLine>,
}

impl Hunk {
    pub fn header(&self) -> String {
        format!(
            "@@ -{},{} +{},{} @@",
            self.old_start, self.old_len, self.new_start, self.new_len
        )
    }
}

/// How lines are compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Whitespace {
    /// Byte-exact, including line endings (like `git diff`).
    #[default]
    Exact,
    /// All whitespace ignored, including CR (like `git diff -w`).
    Ignore,
}

/// The full alignment of `old` and `new`: every line of both files, in
/// order, as context (present in both), removed or added. Within a change,
/// removed lines come before added ones.
pub fn align(
    old: &Text,
    new: &Text,
    algorithm: Algorithm,
    whitespace: Whitespace,
) -> Vec<DiffLine> {
    let key = |text: &Text, i: usize| -> String {
        match whitespace {
            Whitespace::Exact => text.token(i),
            Whitespace::Ignore => text
                .line(i)
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect(),
        }
    };
    let before: Vec<String> = (0..old.len()).map(|i| key(old, i)).collect();
    let after: Vec<String> = (0..new.len()).map(|i| key(new, i)).collect();
    let mut input = InternedInput::default();
    input.update_before(before.iter().map(String::as_str));
    input.update_after(after.iter().map(String::as_str));
    let mut diff = Diff::compute(algorithm, &input);
    diff.postprocess_lines(&input);

    let mut lines = Vec::with_capacity(old.len().max(new.len()));
    let (mut o, mut n) = (0u32, 0u32);
    for change in diff.hunks() {
        while o < change.before.start {
            lines.push(context_line(o, n));
            o += 1;
            n += 1;
        }
        lines.extend(change.before.clone().map(|k| DiffLine {
            kind: LineKind::Removed,
            old: Some(k + 1),
            new: None,
        }));
        lines.extend(change.after.clone().map(|k| DiffLine {
            kind: LineKind::Added,
            old: None,
            new: Some(k + 1),
        }));
        o = change.before.end;
        n = change.after.end;
    }
    while (o as usize) < old.len() {
        lines.push(context_line(o, n));
        o += 1;
        n += 1;
    }
    lines
}

/// Ranges of `lines` to show: everything within `context` lines of a
/// change, plus any `windows`, or everything when `full`. Adjacent visible
/// lines form one range, so changes closer than `2 * context` share a hunk.
pub fn segments(
    lines: &[DiffLine],
    context: u32,
    windows: &[std::ops::Range<u32>],
    full: bool,
) -> Vec<std::ops::Range<usize>> {
    segments_by(
        lines.len(),
        |i| lines.get(i).is_some_and(|l| l.kind != LineKind::Context),
        context,
        windows,
        full,
    )
}

/// [`segments`] with the caller deciding which lines count as changes
/// (e.g. only changes made since a previous review).
#[expect(clippy::single_range_in_vec_init, reason = "a list of one range")]
pub fn segments_by(
    len: usize,
    is_change: impl Fn(usize) -> bool,
    context: u32,
    windows: &[std::ops::Range<u32>],
    full: bool,
) -> Vec<std::ops::Range<usize>> {
    if len == 0 {
        return Vec::new();
    }
    if full {
        return vec![0..len];
    }
    let context = context as usize;
    let mut visible = vec![false; len];
    let mut since_change = usize::MAX;
    for (i, v) in visible.iter_mut().enumerate() {
        since_change = if is_change(i) {
            0
        } else {
            since_change.saturating_add(1)
        };
        *v = since_change <= context;
    }
    let mut until_change = usize::MAX;
    for (i, v) in visible.iter_mut().enumerate().rev() {
        until_change = if is_change(i) {
            0
        } else {
            until_change.saturating_add(1)
        };
        *v |= until_change <= context;
    }
    for window in windows {
        let (start, end) = (window.start as usize, window.end as usize);
        for v in visible.iter_mut().take(end).skip(start) {
            *v = true;
        }
    }
    let mut out = Vec::new();
    let mut start = None;
    for (i, v) in visible.iter().enumerate() {
        match (v, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                out.push(s..i);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push(s..len);
    }
    out
}

/// Old and new lines that come before `index` in the alignment.
pub fn counts_before(lines: &[DiffLine], index: usize) -> (u32, u32) {
    lines.iter().take(index).fold((0, 0), |(o, n), l| {
        (
            o + u32::from(l.old.is_some()),
            n + u32::from(l.new.is_some()),
        )
    })
}

/// The hunk for `range`, given how many old/new lines precede it.
pub fn hunk(lines: &[DiffLine], range: std::ops::Range<usize>, before: (u32, u32)) -> Hunk {
    let lines = lines.get(range).unwrap_or_default().to_vec();
    let old_len = sat_u32(lines.iter().filter(|l| l.old.is_some()).count());
    let new_len = sat_u32(lines.iter().filter(|l| l.new.is_some()).count());
    let start = |seen: u32, len: u32| if len > 0 { seen + 1 } else { seen };
    Hunk {
        old_start: start(before.0, old_len),
        old_len,
        new_start: start(before.1, new_len),
        new_len,
        lines,
    }
}

/// Diffs `old` against `new` and groups changes into hunks with `context`
/// lines around them, as `git diff -U<context>` does.
pub fn diff_lines(old: &Text, new: &Text, algorithm: Algorithm, context: u32) -> Vec<Hunk> {
    let lines = align(old, new, algorithm, Whitespace::Exact);
    let mut seen = 0;
    let mut counts = (0, 0);
    segments(&lines, context, &[], false)
        .into_iter()
        .map(|range| {
            let gap = lines.get(seen..range.start).unwrap_or_default();
            let (o, n) = counts_before(gap, gap.len());
            counts = (counts.0 + o, counts.1 + n);
            seen = range.start;
            hunk(&lines, range, counts)
        })
        .collect()
}

fn context_line(o: u32, n: u32) -> DiffLine {
    DiffLine {
        kind: LineKind::Context,
        old: Some(o + 1),
        new: Some(n + 1),
    }
}

#[cfg(test)]
#[expect(clippy::single_range_in_vec_init, reason = "lists of ranges")]
mod tests {
    use super::*;

    fn render(old: &str, new: &str) -> String {
        let (old, new) = (Text::new(old.as_bytes()), Text::new(new.as_bytes()));
        let mut out = String::new();
        for hunk in diff_lines(&old, &new, Algorithm::Histogram, 3) {
            out.push_str(&hunk.header());
            out.push('\n');
            for line in &hunk.lines {
                let (sign, text) = match line.kind {
                    LineKind::Context => (' ', new.line(line.new.unwrap() as usize - 1)),
                    LineKind::Added => ('+', new.line(line.new.unwrap() as usize - 1)),
                    LineKind::Removed => ('-', old.line(line.old.unwrap() as usize - 1)),
                };
                out.push(sign);
                out.push_str(text);
                out.push('\n');
            }
        }
        out
    }

    fn numbered(n: u32) -> String {
        (1..=n).map(|i| format!("line {i}\n")).collect()
    }

    #[test]
    fn single_change_with_context() {
        let old = numbered(10);
        let new = old.replace("line 5\n", "line five\n");
        assert_eq!(
            render(&old, &new),
            "@@ -2,7 +2,7 @@\n line 2\n line 3\n line 4\n-line 5\n+line five\n line 6\n line 7\n line 8\n"
        );
    }

    #[test]
    fn close_changes_merge_far_ones_split() {
        let old = numbered(30);
        let near = old.replace("line 5\n", "x\n").replace("line 9\n", "y\n");
        assert_eq!(
            diff_lines(
                &Text::new(old.as_bytes()),
                &Text::new(near.as_bytes()),
                Algorithm::Histogram,
                3
            )
            .len(),
            1
        );
        let far = old.replace("line 5\n", "x\n").replace("line 25\n", "y\n");
        assert_eq!(
            diff_lines(
                &Text::new(old.as_bytes()),
                &Text::new(far.as_bytes()),
                Algorithm::Histogram,
                3
            )
            .len(),
            2
        );
    }

    #[test]
    fn added_and_deleted_files() {
        assert_eq!(render("", "a\nb\n"), "@@ -0,0 +1,2 @@\n+a\n+b\n");
        assert_eq!(render("a\nb\n", ""), "@@ -1,2 +0,0 @@\n-a\n-b\n");
        assert_eq!(render("same\n", "same\n"), "");
    }

    #[test]
    fn insertion_at_start_and_end() {
        assert_eq!(
            render("a\nb\n", "z\na\nb\n"),
            "@@ -1,2 +1,3 @@\n+z\n a\n b\n"
        );
        assert_eq!(
            render("a\nb\n", "a\nb\nc\n"),
            "@@ -1,2 +1,3 @@\n a\n b\n+c\n"
        );
    }

    #[test]
    fn missing_final_newline_is_a_change() {
        assert_eq!(render("a\nb\n", "a\nb"), "@@ -1,2 +1,2 @@\n a\n-b\n+b\n");
    }

    #[test]
    fn crlf_change_is_a_change() {
        assert_eq!(
            render("a\nb\n", "a\r\nb\n"),
            "@@ -1,2 +1,2 @@\n-a\n+a\n b\n"
        );
    }

    /// Hunk headers must match `git diff` exactly: GitHub's commentable
    /// ranges and our rendering are compared by line numbers.
    #[test]
    fn hunk_headers_match_git() {
        let base = numbered(40);
        let cases = [
            (base.clone(), base.replace("line 2\n", "")),
            (
                base.clone(),
                base.replace("line 10\n", "line 10\nnew a\nnew b\n"),
            ),
            (
                base.clone(),
                base.replace("line 1\n", "first\n")
                    .replace("line 40\n", "last\n"),
            ),
            (
                base.clone(),
                base.replace("line 5\n", "x\n")
                    .replace("line 12\n", "y\n")
                    .replace("line 30\n", ""),
            ),
            (base.clone(), format!("{base}tail\n")),
            (format!("head\n{base}"), base.clone()),
        ];
        let dir = std::env::temp_dir().join(format!("ghtui-hunks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (i, (old, new)) in cases.iter().enumerate() {
            let (a, b) = (dir.join(format!("{i}.old")), dir.join(format!("{i}.new")));
            std::fs::write(&a, old).unwrap();
            std::fs::write(&b, new).unwrap();
            let out = std::process::Command::new("git")
                .args(["diff", "--no-index", "-U3", "--diff-algorithm=myers"])
                .arg(&a)
                .arg(&b)
                .output()
                .unwrap();
            let expected: Vec<String> = String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter(|l| l.starts_with("@@"))
                .map(|l| {
                    // Normalize git's `-a` (len 1 omitted) to `-a,1`.
                    let inner = l.trim_start_matches("@@ ").split(" @@").next().unwrap();
                    let parts: Vec<String> = inner
                        .split(' ')
                        .map(|p| {
                            if p.contains(',') {
                                p.to_owned()
                            } else {
                                format!("{p},1")
                            }
                        })
                        .collect();
                    format!("@@ {} @@", parts.join(" "))
                })
                .collect();
            let actual: Vec<String> = diff_lines(
                &Text::new(old.as_bytes()),
                &Text::new(new.as_bytes()),
                Algorithm::Myers,
                3,
            )
            .iter()
            .map(Hunk::header)
            .collect();
            assert_eq!(actual, expected, "case {i}");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    fn kinds(lines: &[DiffLine]) -> String {
        lines
            .iter()
            .map(|l| match l.kind {
                LineKind::Context => ' ',
                LineKind::Removed => '-',
                LineKind::Added => '+',
            })
            .collect()
    }

    #[test]
    fn alignment_covers_every_line() {
        let old = Text::new(b"a\nb\nc\n");
        let new = Text::new(b"a\nB\nc\nd\n");
        let lines = align(&old, &new, Algorithm::Histogram, Whitespace::Exact);
        assert_eq!(kinds(&lines), " -+ +");
        assert_eq!(lines.iter().filter(|l| l.old.is_some()).count(), 3);
        assert_eq!(lines.iter().filter(|l| l.new.is_some()).count(), 4);
    }

    #[test]
    fn ignoring_whitespace() {
        let old = Text::new(b"fn f() {\n  x();\n}\nend\n");
        let new = Text::new(b"fn f() {\n\tx( );\r\n}\nEND\n");
        assert_eq!(
            kinds(&align(&old, &new, Algorithm::Histogram, Whitespace::Exact)),
            " -+ -+"
        );
        assert_eq!(
            kinds(&align(&old, &new, Algorithm::Histogram, Whitespace::Ignore)),
            "   -+"
        );
    }

    #[test]
    fn segments_with_windows_and_full() {
        let old = numbered(40);
        let new = old.replace("line 20\n", "x\n");
        let lines = align(
            &Text::new(old.as_bytes()),
            &Text::new(new.as_bytes()),
            Algorithm::Histogram,
            Whitespace::Exact,
        );
        assert_eq!(segments(&lines, 3, &[], false), [16..24]);
        assert_eq!(segments(&lines, 3, &[0..2], false), [0..2, 16..24]);
        assert_eq!(segments(&lines, 3, &[10..16], false), [10..24]);
        assert_eq!(segments(&lines, 3, &[], true), [0..41]);
        assert!(segments(&[], 3, &[], false).is_empty());
    }
}
