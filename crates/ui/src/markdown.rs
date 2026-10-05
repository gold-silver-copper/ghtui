//! GitHub-flavored Markdown as page lines: headings, paragraphs, lists,
//! task lists, quotes, tables, highlighted code blocks, links and images.
//! HTML is reduced to its text (images to their alt text, links kept).

use ghtui_diff::highlight::{Language, highlight};
use ghtui_diff::text::Text;
use ghtui_theme::Syntax;
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::cols;
use crate::diff_view::syntax_role;
use crate::page::{Frame, Page, PageLine, Role, Seg, Tone};

/// Where relative links point: a repository, revision and directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkBase {
    pub repo: String,
    pub rev: String,
    /// Directory of the document, without a trailing slash ("" for root).
    pub dir: String,
}

impl LinkBase {
    /// The base for a document at `path` (or "" for none) in `repo` at `rev`.
    pub fn new(repo: &(impl ToString + ?Sized), rev: impl Into<String>, path: &str) -> Self {
        Self {
            repo: repo.to_string(),
            rev: rev.into(),
            dir: path.rsplit_once('/').map_or("", |(d, _)| d).to_owned(),
        }
    }
}

/// HTML tags that start or end a block.
#[rustfmt::skip]
const BLOCK_TAGS: &[&str] = &[
    "p", "div", "h1", "h2", "h3", "h4", "h5", "h6", "summary", "details", "ul", "ol", "li", "tr",
    "table", "blockquote", "pre", "hr",
];

/// The scheme of an absolute URL (`https` in `https://…`), lowercased.
pub fn scheme(url: &str) -> Option<String> {
    let (scheme, _) = url.split_once(':')?;
    let mut chars = scheme.chars();
    let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    valid.then(|| scheme.to_ascii_lowercase())
}

/// Links from content may lead only to the web or to email: never to
/// local files, other apps' URL handlers, or ghtui's own actions.
pub fn is_safe_link(url: &str) -> bool {
    scheme(url).is_none_or(|s| matches!(s.as_str(), "http" | "https" | "mailto"))
}

/// Resolves a link the way github.com does for a document in a repo.
/// `None` for links content mustn't have (see [`is_safe_link`]).
pub fn resolve(base: Option<&LinkBase>, url: &str) -> Option<String> {
    let url = url.trim();
    if !is_safe_link(url) {
        return None;
    }
    Some(resolve_safe(base, url))
}

fn resolve_safe(base: Option<&LinkBase>, url: &str) -> String {
    if scheme(url).is_some() || url.starts_with('#') {
        return url.to_owned();
    }
    if let Some(rest) = url.strip_prefix('/') {
        return match base {
            // Root-relative links are relative to the repository on GitHub.
            Some(b) => format!("https://github.com/{}/blob/{}/{}", b.repo, b.rev, rest),
            None => format!("https://github.com/{rest}"),
        };
    }
    let Some(base) = base else {
        return url.to_owned();
    };
    let (path, frag) = url.split_once('#').map_or((url, ""), |(p, f)| (p, f));
    let mut parts: Vec<&str> = base.dir.split('/').filter(|p| !p.is_empty()).collect();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    let kind = if path.ends_with('/') { "tree" } else { "blob" };
    let mut out = format!(
        "https://github.com/{}/{kind}/{}/{}",
        base.repo,
        base.rev,
        parts.join("/")
    );
    if !frag.is_empty() {
        out.push('#');
        out.push_str(frag);
    }
    out
}

fn language_for_fence(info: &str) -> Option<Language> {
    let name = info
        .split([',', ' '])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let ext = match name.as_str() {
        "rust" | "rs" => "rs",
        "python" | "py" => "py",
        "js" | "javascript" | "jsx" | "mjs" => "js",
        "ts" | "typescript" => "ts",
        "tsx" => "tsx",
        "go" | "golang" => "go",
        "json" | "jsonc" => "json",
        "yaml" | "yml" => "yml",
        "toml" => "toml",
        "sh" | "bash" | "shell" | "zsh" | "console" | "shell-session" => "sh",
        "md" | "markdown" => "md",
        _ => return None,
    };
    Language::from_path(&format!("x.{ext}"))
}

struct Renderer<'a> {
    page: &'a mut Page,
    base: Option<&'a LinkBase>,
    indent: u16,
    frame: Frame,
    /// Inline content of the current block.
    inline: Vec<Seg>,
    /// Style stack for inline content.
    roles: Vec<Role>,
    link: Option<u32>,
    /// Open lists: next number (`None` for bullets).
    lists: Vec<Option<u64>>,
    /// A list item's marker waits for its first line.
    pending_marker: Option<String>,
    quote_depth: u16,
    code: Option<(Option<Language>, String)>,
    table: Option<Vec<Vec<Vec<Seg>>>>,
    heading: Option<HeadingLevel>,
    /// Inside an image: its alt text so far, and the link outside it.
    image: Option<(String, Option<u32>)>,
    in_comment: bool,
    /// Roles pushed by HTML tags still open.
    html_roles: usize,
}

impl Renderer<'_> {
    fn role(&self) -> Role {
        self.roles.last().cloned().unwrap_or(Role::Body)
    }

    fn text(&mut self, text: &str) {
        if let Some((alt, _)) = &mut self.image {
            alt.push_str(text);
            return;
        }
        let role = self.role();
        self.inline.push(Seg {
            text: text.to_owned(),
            role,
            link: self.link,
        });
    }

    fn flush(&mut self) {
        let mut segs = std::mem::take(&mut self.inline);
        if segs.iter().all(|s| s.text.trim().is_empty()) && self.pending_marker.is_none() {
            return;
        }
        if let Some(marker) = self.pending_marker.take() {
            segs.insert(0, Seg::new(marker, Role::Meta));
        }
        if self.quote_depth > 0 {
            let quote = "│ ".repeat(usize::from(self.quote_depth));
            segs.insert(0, Seg::new(quote, Role::Meta));
        }
        self.page.wrapped(segs, self.indent, self.frame);
    }

    fn html(&mut self, html: &str) {
        // Comments (issue templates' instructions) are hidden, as on GitHub;
        // they may span several HTML events.
        let mut visible = String::new();
        let mut rest = html;
        loop {
            if self.in_comment {
                match rest.split_once("-->") {
                    Some((_, after)) => {
                        rest = after;
                        self.in_comment = false;
                    }
                    None => break,
                }
            } else {
                match rest.split_once("<!--") {
                    Some((before, after)) => {
                        visible.push_str(before);
                        rest = after;
                        self.in_comment = true;
                    }
                    None => {
                        visible.push_str(rest);
                        break;
                    }
                }
            }
        }
        // Keep text, images' alt text and links; drop the tags.
        let mut rest = visible.as_str();
        while let Some((before, tail)) = rest.split_once('<') {
            if !before.trim().is_empty() {
                self.text(&decode_entities(before));
            } else if !before.is_empty() && !before.contains('\n') {
                self.text(" ");
            }
            let Some((tag, after)) = tail.split_once('>') else {
                break;
            };
            let lower = tag.to_ascii_lowercase();
            let closing = lower.starts_with('/');
            let name: String = lower
                .trim_start_matches('/')
                .chars()
                .take_while(char::is_ascii_alphanumeric)
                .collect();
            let inline_role = match name.as_str() {
                "b" | "strong" | "summary" => Some(Role::Strong),
                "i" | "em" => Some(Role::Emph),
                "code" | "kbd" => Some(Role::Code),
                "s" | "del" | "strike" => Some(Role::Strike),
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => Some(Role::Heading),
                _ => None,
            };
            if BLOCK_TAGS.contains(&name.as_str()) {
                // An empty line keeps its list marker for the text to come.
                if self.inline.iter().all(|s| s.text.trim().is_empty()) {
                    self.inline.clear();
                } else {
                    self.flush();
                }
                if name == "li" && !closing {
                    self.pending_marker = Some("• ".into());
                }
            }
            if let Some(role) = inline_role {
                if closing {
                    if self.html_roles > 0 {
                        self.html_roles -= 1;
                        self.roles.pop();
                    }
                } else {
                    self.html_roles += 1;
                    self.roles.push(role);
                }
            } else if lower.starts_with("img") {
                // Images without alt text (badges, mostly) are left out.
                if let Some(alt) = attr(tag, "alt").filter(|a| !a.trim().is_empty()) {
                    let link = self.link.or_else(|| {
                        attr(tag, "src")
                            .and_then(|s| resolve(self.base, &s))
                            .map(|s| self.page.link(s))
                    });
                    self.inline.push(Seg {
                        text: format!("[image: {}]", decode_entities(&alt)),
                        role: Role::Meta,
                        link,
                    });
                }
            } else if lower.starts_with("a ") || lower == "a" {
                self.link = attr(tag, "href")
                    .and_then(|h| resolve(self.base, &h))
                    .map(|h| self.page.link(h));
                self.roles.push(Role::Link);
            } else if lower.starts_with("/a") {
                self.link = None;
                self.roles.pop();
            } else if lower.starts_with("br") {
                self.text("\n");
            }
            rest = after;
        }
        if !rest.trim().is_empty() {
            self.text(&decode_entities(rest));
        }
    }

    fn event(&mut self, event: Event<'_>) {
        if let Some((_, code)) = &mut self.code {
            match event {
                Event::Text(t) => code.push_str(&t),
                Event::End(TagEnd::CodeBlock) => self.end_code(),
                _ => {}
            }
            return;
        }
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => self.text(&t),
            Event::Code(c) => {
                let link = self.link;
                self.inline.push(Seg {
                    text: format!(" {c} "),
                    role: Role::Code,
                    link,
                });
            }
            Event::Html(h) | Event::InlineHtml(h) => self.html(&h),
            Event::SoftBreak => self.text(" "),
            Event::HardBreak => self.text("\n"),
            Event::Rule => {
                self.flush();
                self.page.rule(self.indent, self.frame);
            }
            Event::TaskListMarker(done) => {
                // A checkbox replaces the bullet, as on GitHub.
                if self.pending_marker.as_deref() == Some("• ") {
                    self.pending_marker = Some(String::new());
                }
                let marker = if done { "☑ " } else { "☐ " };
                self.inline.push(Seg::new(
                    marker,
                    if done { Role::Success } else { Role::Meta },
                ));
            }
            Event::FootnoteReference(name) => self.text(&format!("[{name}]")),
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Heading { level, .. } => {
                self.flush();
                self.page_blank();
                self.heading = Some(level);
                self.roles.push(if level <= HeadingLevel::H2 {
                    Role::Title
                } else {
                    Role::Heading
                });
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.quote_depth += 1;
            }
            Tag::CodeBlock(kind) => {
                self.flush();
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => language_for_fence(&info),
                    CodeBlockKind::Indented => None,
                };
                self.code = Some((lang, String::new()));
            }
            Tag::List(start) => {
                self.flush();
                if self.lists.is_empty() {
                    self.page_blank();
                }
                self.lists.push(start);
            }
            Tag::Item => {
                self.flush();
                let depth = cols(self.lists.len().saturating_sub(1));
                self.indent = depth.saturating_mul(2);
                let marker = match self.lists.last_mut() {
                    Some(Some(n)) => {
                        let m = format!("{n}. ");
                        *n += 1;
                        m
                    }
                    _ => "• ".to_owned(),
                };
                self.pending_marker = Some(marker);
            }
            Tag::Emphasis => self.roles.push(Role::Emph),
            Tag::Strong => self.roles.push(Role::Strong),
            Tag::Strikethrough => self.roles.push(Role::Strike),
            Tag::Link { dest_url, .. } => {
                self.link = resolve(self.base, &dest_url).map(|url| self.page.link(url));
                self.roles.push(Role::Link);
            }
            Tag::Image { dest_url, .. } => {
                let outer = self.link;
                // A linked image (a badge) leads where the link does.
                if outer.is_none() {
                    self.link = resolve(self.base, &dest_url).map(|url| self.page.link(url));
                }
                self.image = Some((String::new(), outer));
            }
            Tag::Table(_) => {
                self.flush();
                self.table = Some(Vec::new());
            }
            Tag::TableRow | Tag::TableHead => {
                if let Some(t) = &mut self.table {
                    t.push(Vec::new());
                }
            }
            Tag::TableCell => self.inline.clear(),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.flush();
                if self.lists.is_empty() {
                    self.page_blank();
                }
            }
            TagEnd::Heading(_) => {
                self.roles.pop();
                let level = self.heading.take();
                self.flush();
                // h1 and h2 are underlined, as on GitHub.
                if level.is_some_and(|l| l <= HeadingLevel::H2) {
                    self.page.rule(self.indent, self.frame);
                }
                self.page_blank();
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.quote_depth = self.quote_depth.saturating_sub(1);
                self.page_blank();
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
                self.indent = cols(self.lists.len().saturating_sub(1)).saturating_mul(2);
                if self.lists.is_empty() {
                    self.indent = 0;
                    self.page_blank();
                }
            }
            TagEnd::Item => self.flush(),
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.roles.pop();
            }
            TagEnd::Link => {
                self.link = None;
                self.roles.pop();
            }
            TagEnd::Image => {
                let Some((alt, outer)) = self.image.take() else {
                    return;
                };
                if !alt.trim().is_empty() {
                    self.inline.push(Seg {
                        text: format!("[image: {}]", alt.trim()),
                        role: Role::Meta,
                        link: self.link,
                    });
                }
                self.link = outer;
            }
            TagEnd::TableCell => {
                let cell = std::mem::take(&mut self.inline);
                if let Some(row) = self.table.as_mut().and_then(|t| t.last_mut()) {
                    row.push(cell);
                }
            }
            TagEnd::Table => self.end_table(),
            _ => {}
        }
    }

    fn page_blank(&mut self) {
        if self.quote_depth > 0 {
            return;
        }
        match self.frame {
            Frame::None => self.page.blank(),
            _ => self.page.box_gap(),
        }
    }

    fn end_code(&mut self) {
        let Some((lang, code)) = self.code.take() else {
            return;
        };
        let (indent, frame) = (self.indent, self.frame);
        let line = |segs| PageLine {
            segs,
            tone: Tone::Code,
            frame,
            indent,
            right: Vec::new(),
        };
        self.page.push(line(Vec::new()));
        for mut segs in highlighted(&code, lang) {
            segs.insert(0, Seg::new("  ", Role::Syntax(Syntax::Default)));
            self.page.push(line(segs));
        }
        self.page.push(line(Vec::new()));
        self.page_blank();
    }

    fn end_table(&mut self) {
        let Some(rows) = self.table.take() else {
            return;
        };
        let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
        let width = |cell: &Vec<Seg>| {
            cell.iter()
                .map(|s| crate::text::width(&s.text))
                .sum::<usize>()
        };
        let mut widths = vec![0usize; cols];
        for row in &rows {
            for (w, cell) in widths.iter_mut().zip(row) {
                *w = (*w).max(width(cell)).min(40);
            }
        }
        for (r, row) in rows.iter().enumerate() {
            let mut segs = Vec::new();
            for (c, (cell, w)) in row.iter().zip(&widths).enumerate() {
                if c > 0 {
                    segs.push(Seg::new(" │ ", Role::Meta));
                }
                let mut cell = cell.clone();
                if r == 0 {
                    for s in &mut cell {
                        if s.role == Role::Body {
                            s.role = Role::Strong;
                        }
                    }
                }
                let pad = w.saturating_sub(width(&cell));
                segs.extend(cell);
                segs.push(Seg::new(" ".repeat(pad), Role::Body));
            }
            self.page.push(PageLine {
                segs,
                indent: self.indent,
                frame: self.frame,
                ..PageLine::default()
            });
            if r == 0 {
                let rule: Vec<String> = widths.iter().map(|w| "─".repeat(*w)).collect();
                self.page.push(PageLine {
                    segs: vec![Seg::new(rule.join("─┼─"), Role::Meta)],
                    indent: self.indent,
                    frame: self.frame,
                    ..PageLine::default()
                });
            }
        }
        self.page_blank();
    }
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let at = lower.find(&format!("{name}="))?;
    let rest = tag.get(at + name.len() + 1..)?;
    let (quote, rest) = match rest.chars().next()? {
        q @ ('"' | '\'') => (q, rest.get(1..)?),
        _ => (' ', rest),
    };
    Some(
        rest.split_once(quote)
            .map_or(rest, |(value, _)| value)
            .to_owned(),
    )
}

/// `code`'s lines as highlighted segments.
pub(crate) fn highlighted(code: &str, lang: Option<Language>) -> Vec<Vec<Seg>> {
    let text = Text::new(code.as_bytes());
    let spans = highlight(lang, &text);
    let plain = |s: &str| Seg::new(s, Role::Syntax(Syntax::Default));
    (0..text.len())
        .map(|i| {
            let line = text.line(i);
            let mut segs = Vec::new();
            let mut at = 0usize;
            for s in spans.get(i).map_or(&[][..], Vec::as_slice) {
                let (a, b) = (s.start as usize, s.end as usize);
                if a > at {
                    segs.extend(line.get(at..a).map(plain));
                }
                let role = Role::Syntax(syntax_role(s.kind));
                segs.extend(line.get(a..b).map(|t| Seg::new(t, role)));
                at = b;
            }
            if at < line.len() {
                segs.extend(line.get(at..).map(plain));
            }
            segs
        })
        .collect()
}

fn decode_entities(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// Renders `markdown` into `page`, inside a box when `frame` is
/// [`Frame::Body`].
pub fn render(page: &mut Page, markdown: &str, base: Option<&LinkBase>, frame: Frame) {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES;
    let mut r = Renderer {
        page,
        base,
        indent: 0,
        frame,
        inline: Vec::new(),
        roles: Vec::new(),
        link: None,
        lists: Vec::new(),
        pending_marker: None,
        quote_depth: 0,
        code: None,
        table: None,
        heading: None,
        image: None,
        in_comment: false,
        html_roles: 0,
    };
    for event in Parser::new_ext(markdown, options) {
        r.event(event);
    }
    r.flush();
    // Trailing blank lines inside boxes waste space.
    while frame != Frame::None
        && r.page
            .lines
            .last()
            .is_some_and(|l| l.is_blank() && l.frame == frame && l.tone == Tone::Plain)
    {
        r.page.lines.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(md: &str) -> Vec<String> {
        let mut page = Page::new(40);
        render(&mut page, md, None, Frame::None);
        page.lines.iter().map(PageLine::text).collect()
    }

    #[test]
    fn headings_paragraphs_and_lists() {
        let out = lines("# Title\n\nSome *text* here.\n\n- one\n- two\n  1. nested\n");
        assert_eq!(out[0], "Title", "no `#`, as on GitHub");
        assert!(out[1].starts_with("────"), "h1 is underlined: {out:?}");
        assert!(out.contains(&"Some text here.".to_owned()), "{out:?}");
        assert!(out.contains(&"• one".to_owned()), "{out:?}");
        assert!(out.iter().any(|l| l == "1. nested"), "{out:?}");
    }

    #[test]
    fn code_blocks_are_highlighted() {
        let mut page = Page::new(40);
        render(&mut page, "```rust\nfn main() {}\n```\n", None, Frame::None);
        let code: Vec<&PageLine> = page.lines.iter().filter(|l| l.tone == Tone::Code).collect();
        assert!(code.iter().any(|l| l.text().contains("fn main()")));
        assert!(
            code.iter()
                .flat_map(|l| &l.segs)
                .any(|s| s.role == Role::Syntax(Syntax::Keyword))
        );
    }

    #[test]
    fn links_resolve_relative_to_the_repo() {
        let base = LinkBase::new("o/r", "main", "docs/README.md");
        let resolved = |url| resolve(Some(&base), url).unwrap();
        assert_eq!(
            resolved("guide.md"),
            "https://github.com/o/r/blob/main/docs/guide.md"
        );
        assert_eq!(resolved("../src/"), "https://github.com/o/r/tree/main/src");
        assert_eq!(
            resolved("/LICENSE"),
            "https://github.com/o/r/blob/main/LICENSE"
        );
        assert_eq!(resolved("https://x.dev/a"), "https://x.dev/a");
        assert_eq!(resolved("#install"), "#install");

        let mut page = Page::new(60);
        render(
            &mut page,
            "See [the guide](guide.md).",
            Some(&base),
            Frame::None,
        );
        assert_eq!(
            page.links,
            [crate::page::Link::from(
                "https://github.com/o/r/blob/main/docs/guide.md"
            )]
        );
        let line = &page.lines[0];
        assert!(
            line.segs
                .iter()
                .any(|s| s.role == Role::Link && s.link == Some(0))
        );
    }

    /// Content can't link to local files, other apps, or ghtui's actions.
    #[test]
    fn unsafe_links_are_text() {
        for url in [
            "file:///etc/passwd",
            "vscode://x",
            "javascript:alert(1)",
            "ghtui:star",
            "ghtui:quote:me\nhi",
            "GHTUI:more",
        ] {
            assert_eq!(resolve(None, url), None, "{url}");
            let mut page = Page::new(60);
            render(
                &mut page,
                &format!("[x]({url}) <a href=\"{url}\">y</a>"),
                None,
                Frame::None,
            );
            assert!(page.links.is_empty(), "{url}: {:?}", page.links);
        }
        assert!(resolve(None, "mailto:a@b.c").is_some());
        assert!(resolve(None, "HTTPS://x.dev").is_some());
    }

    #[test]
    fn html_blocks_break_lines() {
        let out = lines(
            "<details>\n<summary>Release notes</summary>\n<p><em>Sourced from</em> x.</p>\n<ul>\n<li><p>one</p></li>\n<li>two</li>\n</ul>\n</details>",
        );
        assert!(out.contains(&"Release notes".to_owned()), "{out:?}");
        assert!(out.contains(&"Sourced from x.".to_owned()), "{out:?}");
        assert!(
            out.contains(&"• one".to_owned()) && out.contains(&"• two".to_owned()),
            "{out:?}"
        );
    }

    #[test]
    fn html_comments_are_hidden() {
        let out = lines("<!--\nFill this in.\n-->\nReal text <!-- note --> here.");
        assert!(!out.join("|").contains("Fill"), "{out:?}");
        assert!(
            out.iter()
                .any(|l| l.contains("Real text") && l.contains("here")),
            "{out:?}"
        );
        assert!(!out.iter().any(|l| l.contains("note")), "{out:?}");
    }

    #[test]
    fn html_keeps_text_images_and_links() {
        let out = lines(
            "<p align=\"center\"><img src=\"logo.png\" alt=\"Logo\"></p>\n\n<a href=\"https://x.dev\">Site</a>",
        );
        assert!(out.iter().any(|l| l.contains("[image: Logo]")), "{out:?}");
        assert!(out.iter().any(|l| l.contains("Site")), "{out:?}");
    }

    #[test]
    fn tables_and_tasks() {
        let out = lines("| a | bb |\n|---|---|\n| 1 | 2 |\n\n- [x] done\n- [ ] todo\n");
        assert!(out.iter().any(|l| l.starts_with("a │ bb")), "{out:?}");
        assert!(out.iter().any(|l| l.starts_with("☑ done")), "{out:?}");
        assert!(out.iter().any(|l| l.contains("☑ done")), "{out:?}");
        assert!(out.iter().any(|l| l.contains("☐ todo")), "{out:?}");
    }
}
