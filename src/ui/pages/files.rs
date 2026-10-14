//! A file, and its blame.

use super::*;

/// A file: highlighted with line numbers, or Markdown rendered.
/// A file at a revision, and the lines a link points at.
#[derive(Clone, Copy)]
pub struct FileAt<'a> {
    pub repo: &'a RepoId,
    pub rev: &'a str,
    pub path: &'a str,
    /// Marked, and where the page opens.
    pub lines: Option<(u32, u32)>,
}

pub fn file(page: &mut Page, at: FileAt<'_>, blob: &Blob, keys: Keys<'_>) {
    let FileAt {
        repo,
        rev,
        path,
        lines: marked,
    } = at;
    code_toolbar(page, repo, rev, path, true, None, keys);
    let size = crate::ui::text::size(blob.size);
    let Some(text) = &blob.text else {
        let title = Seg::new(size, Role::Meta);
        empty_box(
            page,
            title,
            "Binary file not shown here: open it on GitHub.",
        );
        return;
    };
    let lines = text.lines().count() as u64;
    let mut info = format!("{} · {size}", plural(lines, "line"));
    if blob.truncated {
        info.push_str(" · only the beginning is shown");
    }
    let blame = url::blame(repo, rev, path);
    let blame = link_seg(page, "Blame", blame, Role::Link);
    let raw = link_seg(page, "Raw", url::raw(repo, rev, path), Role::Link);
    let right = vec![blame, Seg::new("  ", Role::Meta), raw];
    page.box_top(vec![Seg::new(info, Role::Meta)], right);
    if is_markdown(path) {
        let base = LinkBase::new(repo, rev, path);
        markdown::render(page, text, Some(&base), Frame::Body);
        page.box_bottom();
        return;
    }
    let lines = markdown::highlighted_file(text, crate::diff::Language::from_path(path));
    let width = lines.len().to_string().len();
    let is_marked = |n: usize| marked.is_some_and(|(a, b)| (a as usize..=b as usize).contains(&n));
    // Each line is a row: selected, `y` copies its link.
    let file_url = url::blob(repo, rev, path);
    let line_url = |n: usize| format!("{file_url}#L{n}");
    for (i, mut segs) in lines.into_iter().enumerate() {
        let n = i + 1;
        let number = format!("{n:>width$}  ");
        segs.insert(0, Seg::new(number, Role::Syntax(Syntax::Comment)));
        if is_marked(n) && page.jump.is_none() {
            page.jump = Some(page.lines.len());
            page.selects = Some(page.items.len());
        }
        let start = page.lines.len();
        page.push(PageLine {
            segs,
            frame: Frame::Body,
            tone: if is_marked(n) {
                Tone::Marked
            } else {
                Tone::Code
            },
            fixed: 1,
            ..PageLine::default()
        });
        let link = page.link(line_url(n));
        page.numbered.push(page.items.len());
        page.item(start, link);
    }
    page.box_bottom();
}

/// A file with, beside each run of lines, the commit that last changed
/// them: its ID, author and age.
pub fn blame(
    page: &mut Page,
    at: FileAt<'_>,
    blob: Fetched<'_, Blob>,
    blame: Fetched<'_, Blame>,
    keys: Keys<'_>,
    now: u64,
) {
    let FileAt {
        repo,
        rev,
        path,
        lines: marked,
    } = at;
    code_toolbar(page, repo, rev, path, true, None, keys);
    let Some(blob) = blob.show(page, "the file") else {
        return;
    };
    let Some(blame) = blame.show(page, "the blame") else {
        return;
    };
    let Some(text) = &blob.text else {
        empty_box(
            page,
            Seg::new("Blame", Role::Meta),
            "Binary file not shown.",
        );
        return;
    };
    let file = link_seg(page, "File", url::blob(repo, rev, path), Role::Link);
    let commits = unique(blame.ranges.iter().map(|r| &r.commit.oid)).len() as u64;
    let info = format!("Blame · {}", plural(commits, "commit"));
    page.box_top(vec![Seg::new(info, Role::Meta)], vec![file]);
    let lines = markdown::highlighted_file(text, crate::diff::Language::from_path(path));
    let width = lines.len().to_string().len();
    let is_marked = |n: usize| marked.is_some_and(|(a, b)| (a as usize..=b as usize).contains(&n));
    // Beside each run of lines: its commit, author, age and message, in
    // a column that takes what the page can spare.
    let author_w = (blame.ranges.iter())
        .map(|r| crate::ui::text::width(r.commit.author.name()))
        .max()
        .unwrap_or(0)
        .min(20);
    let gutter_w = (usize::from(page.width) * 2 / 5).clamp(32, 90);
    let fixed_w = 7 + 1 + author_w + 1 + 8 + 2;
    let message_w = gutter_w.saturating_sub(fixed_w + 2);
    let gutter_w = if message_w >= 8 { gutter_w } else { fixed_w };
    // Each run is a row opening its commit.
    let mut run_start = 0;
    for (i, mut segs) in lines.into_iter().enumerate() {
        let n = i + 1;
        let range = blame
            .ranges
            .iter()
            .find(|r| (r.start as usize..=r.end as usize).contains(&n));
        if range.is_some_and(|r| r.start as usize == n && n > 1) {
            page.box_rule();
        }
        let mut gutter = match range {
            Some(r) if r.start as usize == n => {
                run_start = page.lines.len();
                let c = &r.commit;
                let author = crate::ui::text::truncate(c.author.name(), author_w);
                let mut gutter = vec![
                    link_seg(
                        page,
                        crate::ui::text::short_sha(&c.oid),
                        url::commit(repo, &c.oid),
                        Role::Code,
                    ),
                    Seg::new(
                        format!(" {author:<author_w$} {:>8}  ", time::ago_iso(&c.date, now)),
                        Role::Meta,
                    ),
                ];
                if message_w >= 8 {
                    let message = crate::ui::text::truncate(&c.headline, message_w);
                    gutter.push(Seg::new(format!("{message:<message_w$}  "), Role::Body));
                }
                gutter
            }
            _ => vec![Seg::new(" ".repeat(gutter_w), Role::Meta)],
        };
        let fixed = gutter.len() + 1;
        gutter.push(Seg::new(
            format!("{n:>width$}  "),
            Role::Syntax(Syntax::Comment),
        ));
        gutter.append(&mut segs);
        if is_marked(n) && page.jump.is_none() {
            page.jump = Some(page.lines.len());
            // The run it's in, pushed once the run ends.
            page.selects = Some(page.items.len());
        }
        page.push(PageLine {
            segs: gutter,
            frame: Frame::Body,
            tone: if is_marked(n) {
                Tone::Marked
            } else {
                Tone::Code
            },
            fixed: u8::try_from(fixed).unwrap_or(u8::MAX),
            ..PageLine::default()
        });
        page.numbered.push(page.items.len());
        if let Some(r) = range.filter(|r| r.end as usize == n) {
            let link = page.link(url::commit(repo, &r.commit.oid));
            page.item(run_start, link);
        }
    }
    page.box_bottom();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A blame scrolls to the linked line, not the rule above its run.
    #[test]
    fn a_blame_jumps_to_its_line() {
        let range = |start, end, oid: &str| crate::api::browse::BlameRange {
            start,
            end,
            age: 1,
            commit: CommitInfo {
                oid: oid.into(),
                headline: String::new(),
                author: Person::User(crate::api::browse::Login::unchecked("octocat")),
                date: "2026-10-01T00:00:00Z".into(),
            },
        };
        let blame = Blame {
            ranges: vec![range(1, 2, "aaaaaaa"), range(3, 3, "bbbbbbb")],
        };
        let blob = crate::fixtures::file("a.txt", "one\ntwo\nthree\n");
        let repo = RepoId::new("o", "r");
        let at = FileAt {
            repo: &repo,
            rev: "main",
            path: "a.txt",
            lines: Some((3, 3)),
        };
        let mut page = Page::new(100);
        let (blob, blame) = (Fetched::ready(&blob), Fetched::ready(&blame));
        super::blame(&mut page, at, blob, blame, Keys::default(), 0);
        let line = page
            .jump
            .and_then(|j| page.lines.get(j))
            .map(PageLine::text);
        assert!(
            line.as_deref().is_some_and(|l| l.contains("three")),
            "{line:?}"
        );
    }
}
