//! Line diffs with context, as hunks.

use imara_diff::{Algorithm as ImaraAlgorithm, Diff, InternedInput};

use crate::text::Text;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Algorithm {
    #[default]
    Histogram,
    Myers,
}

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

/// Diffs `old` against `new` and groups changes into hunks with `context`
/// lines around them (hunks whose context would overlap are merged).
pub fn diff_lines(old: &Text, new: &Text, algorithm: Algorithm, context: u32) -> Vec<Hunk> {
    let before: Vec<String> = (0..old.len()).map(|i| old.token(i)).collect();
    let after: Vec<String> = (0..new.len()).map(|i| new.token(i)).collect();
    let mut input = InternedInput::default();
    input.update_before(before.iter().map(String::as_str));
    input.update_after(after.iter().map(String::as_str));
    let algorithm = match algorithm {
        Algorithm::Histogram => ImaraAlgorithm::Histogram,
        Algorithm::Myers => ImaraAlgorithm::Myers,
    };
    let mut diff = Diff::compute(algorithm, &input);
    diff.postprocess_lines(&input);

    let changes: Vec<imara_diff::Hunk> = diff.hunks().collect();
    let old_total = old.len() as u32;
    let new_total = new.len() as u32;

    let mut hunks = Vec::new();
    let mut i = 0;
    while i < changes.len() {
        // Extend the group while the next change is within 2*context lines.
        let mut j = i;
        while j + 1 < changes.len()
            && changes[j + 1].before.start - changes[j].before.end <= 2 * context
        {
            j += 1;
        }
        let first = &changes[i];
        let last = &changes[j];
        let old_from = first.before.start.saturating_sub(context);
        let new_from = first.after.start - (first.before.start - old_from);
        let old_to = (last.before.end + context).min(old_total);
        let new_to = (last.after.end + (old_to - last.before.end)).min(new_total);

        let mut lines = Vec::new();
        let (mut o, mut n) = (old_from, new_from);
        for change in &changes[i..=j] {
            while o < change.before.start {
                lines.push(context_line(o, n));
                o += 1;
                n += 1;
            }
            for k in change.before.clone() {
                lines.push(DiffLine {
                    kind: LineKind::Removed,
                    old: Some(k + 1),
                    new: None,
                });
            }
            for k in change.after.clone() {
                lines.push(DiffLine {
                    kind: LineKind::Added,
                    old: None,
                    new: Some(k + 1),
                });
            }
            o = change.before.end;
            n = change.after.end;
        }
        while o < old_to && n < new_to {
            lines.push(context_line(o, n));
            o += 1;
            n += 1;
        }
        hunks.push(Hunk {
            old_start: if old_to > old_from {
                old_from + 1
            } else {
                old_from
            },
            old_len: old_to - old_from,
            new_start: if new_to > new_from {
                new_from + 1
            } else {
                new_from
            },
            new_len: new_to - new_from,
            lines,
        });
        i = j + 1;
    }
    hunks
}

fn context_line(o: u32, n: u32) -> DiffLine {
    DiffLine {
        kind: LineKind::Context,
        old: Some(o + 1),
        new: Some(n + 1),
    }
}

#[cfg(test)]
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
}
