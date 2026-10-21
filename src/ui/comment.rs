//! A review comment's Markdown as the lines of a thread in the diff:
//! rendered as the conversation renders it, then wrapped, code and all,
//! to the thread's width, each line with the links it has.

use crate::ui::markdown;
use crate::ui::page::{Page, Role, Seg, Tone, wrap_segs};
use crate::ui::{cols, idx};

/// One line of a rendered comment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommentLine {
    pub segs: Vec<Seg>,
    /// Plain text, code, or a suggested change's line.
    pub tone: Tone,
    /// Where its segments' links lead, by their `link`.
    pub links: Vec<String>,
}

impl CommentLine {
    pub fn text(&self) -> String {
        self.segs.iter().map(Seg::text).collect()
    }
}

/// `body` rendered, `width` columns wide. A suggested change shows as a
/// diff of `original` (the lines it replaces, when known) and its lines.
pub fn lines(body: &str, width: usize, original: Option<&[String]>) -> Vec<CommentLine> {
    let mut page = Page::new(cols(width));
    markdown::render_comment(&mut page, body, original);
    let width = width.max(1);
    let mut out = Vec::new();
    for line in std::mem::take(&mut page.lines) {
        let mut segs = Vec::new();
        if line.indent > 0 {
            segs.push(Seg::new(" ".repeat(usize::from(line.indent)), Role::Body));
        }
        segs.extend(line.segs);
        // Code, tables and a page's rules can be wider than a thread.
        for mut segs in wrap_segs(segs, width) {
            let mut links: Vec<String> = Vec::new();
            for seg in &mut segs {
                let url = seg.link.and_then(|l| page.target(l)).and_then(|l| l.url());
                seg.link = url.map(|url| match links.iter().position(|u| u == url) {
                    Some(i) => idx(i),
                    None => {
                        links.push(url.to_owned());
                        idx(links.len().saturating_sub(1))
                    }
                });
            }
            out.push(CommentLine {
                segs,
                tone: line.tone,
                links,
            });
        }
    }
    let blank = |l: &CommentLine| l.tone == Tone::Plain && l.text().trim().is_empty();
    while out.last().is_some_and(blank) {
        out.pop();
    }
    let lead = out.iter().take_while(|l| blank(l)).count();
    out.drain(..lead);
    if out.is_empty() {
        out.push(CommentLine {
            segs: vec![Seg::new("(no text)", Role::Meta)],
            ..CommentLine::default()
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(lines: &[CommentLine]) -> Vec<String> {
        lines.iter().map(CommentLine::text).collect()
    }

    /// Code wraps to the thread rather than running off it, and a link
    /// keeps where it leads.
    #[test]
    fn code_wraps_and_links_lead_somewhere() {
        let body = "See [the docs](https://x.dev/a).\n\n```\nlet a_long_name = 1;\n```";
        let out = lines(body, 12, None);
        assert!(
            out.iter().all(|l| crate::ui::text::width(&l.text()) <= 12),
            "{:?}",
            texts(&out)
        );
        let linked = out.iter().find(|l| !l.links.is_empty()).unwrap();
        assert_eq!(linked.links, ["https://x.dev/a"]);
        assert!(out.iter().any(|l| l.tone == Tone::Code));
    }

    #[test]
    fn a_suggestion_is_a_small_diff() {
        let original = ["let x = 1;".to_owned()];
        let out = lines("```suggestion\nlet x = 2;\n```", 40, Some(&original));
        let tones: Vec<Tone> = out.iter().map(|l| l.tone).collect();
        assert_eq!(
            tones,
            [Tone::Plain, Tone::Removed, Tone::Added],
            "{:?}",
            texts(&out)
        );
        assert_eq!(
            texts(&out),
            ["Suggested change", "- let x = 1;", "+ let x = 2;"]
        );
    }
}
