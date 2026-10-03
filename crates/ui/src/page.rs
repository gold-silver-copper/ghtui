//! Pages: documents of styled lines with links, like a text browser.
//!
//! Builders produce lines from data with *semantic* roles (title, body,
//! link, code, ...), never colors; [`PageView`] maps roles to theme styles.
//! Links are URLs (GitHub web URLs for anything ghtui can show itself, so
//! following one works like clicking it on the website).

use ghtui_theme::{Bg, DiffBg, Fg, Syntax};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use unicode_width::UnicodeWidthChar;

use crate::{Ctx, PAD_X, fill, text};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    Body,
    Title,
    Heading,
    Strong,
    Emph,
    Strike,
    Meta,
    Link,
    /// Inline code.
    Code,
    /// A token inside a code block.
    Syntax(Syntax),
    Success,
    Error,
    Accent,
    Added,
    Removed,
    /// A tonal chip.
    Chip(Bg),
    /// A GitHub label chip (`rrggbb`).
    Label(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seg {
    pub text: String,
    pub role: Role,
    /// Index into [`Page::links`].
    pub link: Option<u32>,
}

impl Seg {
    pub fn new(text: impl Into<String>, role: Role) -> Self {
        Self {
            text: text.into(),
            role,
            link: None,
        }
    }
}

/// What a line sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Surface {
    #[default]
    Page,
    /// A card (comments, the repo's about box).
    Card,
    /// A code block (highlighted tokens).
    Code,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PageLine {
    pub segs: Vec<Seg>,
    pub surface: Surface,
    /// Columns before the first segment.
    pub indent: u16,
    /// Right-aligned segments (metadata in lists).
    pub right: Vec<Seg>,
}

impl PageLine {
    pub fn links(&self) -> impl Iterator<Item = u32> + '_ {
        let mut seen = Vec::new();
        self.segs
            .iter()
            .chain(&self.right)
            .filter_map(|s| s.link)
            .filter(move |l| {
                let new = !seen.contains(l);
                seen.push(*l);
                new
            })
    }

    pub fn is_blank(&self) -> bool {
        self.segs.iter().all(|s| s.text.trim().is_empty()) && self.right.is_empty()
    }

    pub fn text(&self) -> String {
        self.segs.iter().map(|s| s.text.as_str()).collect()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Page {
    pub lines: Vec<PageLine>,
    /// Link targets (URLs).
    pub links: Vec<String>,
    /// Columns available to builders (wrapping width).
    pub width: u16,
}

impl Page {
    pub fn new(width: u16) -> Self {
        Self {
            width: width.max(20),
            ..Self::default()
        }
    }

    /// Registers a link target, returning its index.
    pub fn link(&mut self, url: impl Into<String>) -> u32 {
        let url = url.into();
        if let Some(i) = self.links.iter().position(|l| *l == url) {
            return i as u32;
        }
        self.links.push(url);
        (self.links.len() - 1) as u32
    }

    pub fn push(&mut self, line: PageLine) {
        self.lines.push(line);
    }

    pub fn blank(&mut self) {
        if self.lines.last().is_some_and(|l| !l.is_blank()) {
            self.lines.push(PageLine::default());
        }
    }

    /// A line of segments.
    pub fn line(&mut self, segs: Vec<Seg>) {
        self.lines.push(PageLine {
            segs,
            ..PageLine::default()
        });
    }

    /// A line with a single link segment.
    pub fn link_line(
        &mut self,
        text: impl Into<String>,
        url: impl Into<String>,
        role: Role,
        indent: u16,
    ) -> u32 {
        let link = self.link(url);
        self.lines.push(PageLine {
            segs: vec![Seg {
                text: text.into(),
                role,
                link: Some(link),
            }],
            indent,
            ..PageLine::default()
        });
        link
    }

    /// Word-wraps segments into lines `indent` columns in.
    pub fn wrapped(&mut self, segs: Vec<Seg>, indent: u16, surface: Surface) {
        let width = usize::from(self.width.saturating_sub(indent)).max(10);
        for segs in wrap_segs(segs, width) {
            self.lines.push(PageLine {
                segs,
                surface,
                indent,
                right: Vec::new(),
            });
        }
    }

    /// The nearest line at or after (or before) `from` with a link.
    pub fn link_line_from(&self, from: usize, forward: bool) -> Option<usize> {
        let has = |i: &usize| self.lines[*i].links().next().is_some();
        if forward {
            (from..self.lines.len()).find(has)
        } else {
            (0..=from.min(self.lines.len().saturating_sub(1)))
                .rev()
                .find(has)
        }
    }
}

/// Splits segments into lines of at most `width` columns, breaking at
/// spaces (or mid-word for words longer than a line). Newlines in segment
/// text force breaks.
pub fn wrap_segs(segs: Vec<Seg>, width: usize) -> Vec<Vec<Seg>> {
    let mut lines: Vec<Vec<Seg>> = vec![Vec::new()];
    let mut used = 0usize;
    let push = |lines: &mut Vec<Vec<Seg>>, seg: &Seg, text: &str| {
        let line = lines.last_mut().expect("at least one line");
        match line.last_mut() {
            Some(last) if last.role == seg.role && last.link == seg.link => {
                last.text.push_str(text)
            }
            _ => line.push(Seg {
                text: text.to_owned(),
                role: seg.role.clone(),
                link: seg.link,
            }),
        }
    };
    for seg in &segs {
        for (n, part) in seg.text.split('\n').enumerate() {
            if n > 0 {
                lines.push(Vec::new());
                used = 0;
            }
            // Words with the spaces that follow them.
            let mut rest = part;
            while !rest.is_empty() {
                let word_end = rest.find(' ').map_or(rest.len(), |i| i + 1);
                let word = &rest[..word_end];
                rest = &rest[word_end..];
                let w = text::width(word.trim_end());
                if used > 0 && used + w > width {
                    lines.push(Vec::new());
                    used = 0;
                    if word.trim().is_empty() {
                        continue;
                    }
                }
                if w > width {
                    // Hard-break a long word.
                    let mut chunk = String::new();
                    for c in word.chars() {
                        let cw = c.width().unwrap_or(0);
                        if used + cw > width {
                            push(&mut lines, seg, &chunk);
                            chunk.clear();
                            lines.push(Vec::new());
                            used = 0;
                        }
                        chunk.push(c);
                        used += cw;
                    }
                    push(&mut lines, seg, &chunk);
                    continue;
                }
                push(&mut lines, seg, word);
                used += text::width(word);
            }
        }
    }
    // Drop trailing spaces.
    for line in &mut lines {
        if let Some(last) = line.last_mut() {
            let trimmed = last.text.trim_end().to_owned();
            last.text = trimmed;
        }
    }
    lines
}

/// Renders a page: `cursor` line highlighted, scrolled to `scroll`.
pub struct PageView<'a> {
    pub ctx: Ctx<'a>,
    pub page: &'a Page,
    pub cursor: usize,
    pub scroll: usize,
}

const PAGE_BG: Bg = Bg::ContainerLow;
const CARD_BG: Bg = Bg::Container;

impl Widget for PageView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = self.ctx.theme;
        fill(buf, area, theme, PAGE_BG);
        for (i, line) in self
            .page
            .lines
            .iter()
            .enumerate()
            .skip(self.scroll)
            .take(usize::from(area.height))
        {
            let y = area.y + (i - self.scroll) as u16;
            let row = Rect {
                y,
                height: 1,
                ..area
            };
            let cursor = i == self.cursor;
            let bg = match (line.surface, cursor) {
                (Surface::Code, true) => Bg::DiffSelected(DiffBg::Context),
                (Surface::Code, false) => Bg::Diff(DiffBg::Context),
                (_, true) => Bg::Selected,
                (Surface::Card, false) => CARD_BG,
                (Surface::Page, false) => PAGE_BG,
            };
            let inner = Rect {
                x: area.x + PAD_X,
                width: area.width.saturating_sub(2 * PAD_X),
                ..row
            };
            if cursor || line.surface != Surface::Page {
                // Cards and code blocks span the content width.
                let band = if cursor { row } else { inner };
                fill(buf, band, theme, bg);
            }
            let text_area = Rect {
                x: inner.x + line.indent.min(inner.width),
                width: inner.width.saturating_sub(line.indent),
                ..inner
            };
            let right_width = line
                .right
                .iter()
                .map(|s| text::width(&s.text))
                .sum::<usize>() as u16;
            let left_width =
                text_area
                    .width
                    .saturating_sub(if right_width > 0 { right_width + 2 } else { 0 });
            Line::from(self.spans(&line.segs, bg, usize::from(left_width))).render(
                Rect {
                    width: left_width,
                    ..text_area
                },
                buf,
            );
            if right_width > 0 {
                let rw = right_width.min(text_area.width);
                Line::from(self.spans(&line.right, bg, usize::from(rw))).render(
                    Rect {
                        x: text_area.right() - rw,
                        width: rw,
                        ..text_area
                    },
                    buf,
                );
            }
        }
    }
}

impl PageView<'_> {
    fn style(&self, role: &Role, bg: Bg) -> Style {
        let theme = self.ctx.theme;
        match role {
            Role::Body => theme.body(bg),
            Role::Title => theme.title(bg),
            Role::Heading => theme.accent(bg).add_modifier(Modifier::BOLD),
            Role::Strong => theme.body(bg).add_modifier(Modifier::BOLD),
            Role::Emph => theme.body(bg).add_modifier(Modifier::ITALIC),
            Role::Strike => theme.meta(bg).add_modifier(Modifier::CROSSED_OUT),
            Role::Meta => theme.meta(bg),
            Role::Link => theme.accent(bg).add_modifier(Modifier::UNDERLINED),
            Role::Code => match bg {
                Bg::Diff(_) | Bg::DiffSelected(_) => theme.style(Fg::Syntax(Syntax::String), bg),
                _ => theme.style(Fg::Tertiary, bg),
            },
            Role::Syntax(s) => match bg {
                Bg::Diff(_) | Bg::DiffSelected(_) => theme.style(Fg::Syntax(*s), bg),
                _ => theme.body(bg),
            },
            Role::Success => theme.style(Fg::Success, bg),
            Role::Error => theme.error(bg),
            Role::Accent => theme.accent(bg),
            Role::Added => theme.style(Fg::DiffAddedSign, bg),
            Role::Removed => theme.style(Fg::DiffRemovedSign, bg),
            Role::Chip(chip) => theme.fill(*chip),
            Role::Label(color) => theme.label_chip(color),
        }
    }

    fn spans(&self, segs: &[Seg], bg: Bg, room: usize) -> Vec<Span<'static>> {
        let mut out = Vec::new();
        let mut used = 0;
        for seg in segs {
            let w = text::width(&seg.text);
            if used + w > room {
                let rest = room.saturating_sub(used);
                if rest > 0 {
                    out.push(Span::styled(
                        text::truncate(&seg.text, rest),
                        self.style(&seg.role, bg),
                    ));
                }
                break;
            }
            used += w;
            out.push(Span::styled(
                seg.text.replace('\t', "    "),
                self.style(&seg.role, bg),
            ));
        }
        out
    }
}

/// Keeps `cursor` visible with a line of context.
pub fn scroll_to(cursor: usize, scroll: usize, height: usize, total: usize) -> usize {
    let height = height.max(1);
    let mut scroll = scroll.min(total.saturating_sub(height));
    if cursor < scroll + 1 {
        scroll = cursor.saturating_sub(1);
    }
    if cursor + 2 > scroll + height {
        scroll = (cursor + 2).saturating_sub(height);
    }
    scroll.min(total.saturating_sub(height))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(lines: &[Vec<Seg>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.iter().map(|s| s.text.as_str()).collect())
            .collect()
    }

    #[test]
    fn wraps_across_segments() {
        let segs = vec![
            Seg::new("hello ", Role::Body),
            Seg::new("bold words ", Role::Strong),
            Seg::new("and more text", Role::Body),
        ];
        let lines = wrap_segs(segs, 12);
        assert_eq!(texts(&lines), ["hello bold", "words and", "more text"]);
        assert_eq!(lines[0][1].role, Role::Strong);
    }

    #[test]
    fn newlines_and_long_words() {
        let lines = wrap_segs(vec![Seg::new("a\nabcdefghij", Role::Body)], 4);
        assert_eq!(texts(&lines), ["a", "abcd", "efgh", "ij"]);
    }

    #[test]
    fn links_and_navigation() {
        let mut page = Page::new(40);
        page.line(vec![Seg::new("title", Role::Title)]);
        page.blank();
        let a = page.link_line("one", "https://github.com/a", Role::Link, 0);
        let b = page.link_line("again", "https://github.com/a", Role::Link, 0);
        assert_eq!(a, b, "same URL, same link");
        assert_eq!(page.link_line_from(0, true), Some(2));
        assert_eq!(page.link_line_from(3, false), Some(3));
        assert_eq!(page.link_line_from(1, false), None);
    }

    #[test]
    fn scrolling_keeps_cursor_visible() {
        assert_eq!(scroll_to(0, 0, 10, 100), 0);
        assert_eq!(scroll_to(20, 0, 10, 100), 12);
        assert_eq!(scroll_to(5, 12, 10, 100), 4);
        assert_eq!(scroll_to(99, 0, 10, 100), 90);
    }
}
