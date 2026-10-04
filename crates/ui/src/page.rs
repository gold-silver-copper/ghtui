//! Pages: documents of styled lines with links, laid out like GitHub's.
//!
//! Builders produce lines from data with *semantic* roles (title, body,
//! link, code, ...), never colors; [`PageView`] maps roles to theme styles.
//! Lines can sit in boxes (rounded outlines, like GitHub's file list, README
//! and comments). Links are URLs: GitHub web URLs for anything ghtui can
//! show itself, so following one works like clicking it on the website.
//! *Items* are the rows you select and open (a file, an issue, a comment);
//! an optional *aside* is GitHub's right-hand sidebar.

use ghtui_theme::{Bg, DiffBg, Fg, Syntax};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use unicode_width::UnicodeWidthChar;

use crate::{Ctx, fill, text};

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

    pub fn linked(text: impl Into<String>, role: Role, link: u32) -> Self {
        Self {
            text: text.into(),
            role,
            link: Some(link),
        }
    }
}

/// Where a line sits in a box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Frame {
    #[default]
    None,
    /// `╭─ title ──── right ─╮`
    Top,
    /// `│ content │`
    Body,
    /// `├────────┤`
    Rule,
    /// `╰────────╯`
    Bottom,
}

/// The background inside a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tone {
    #[default]
    Plain,
    /// A code block.
    Code,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PageLine {
    pub segs: Vec<Seg>,
    /// Columns before the first segment (inside the box, if framed).
    pub indent: u16,
    /// Right-aligned segments (metadata in lists).
    pub right: Vec<Seg>,
    pub frame: Frame,
    pub tone: Tone,
}

impl PageLine {
    /// No text (a gap, or a box border).
    pub fn is_blank(&self) -> bool {
        self.segs.iter().all(|s| s.text.trim().is_empty()) && self.right.is_empty()
    }

    pub fn text(&self) -> String {
        self.segs.iter().map(|s| s.text.as_str()).collect()
    }
}

/// A selectable row: lines `start..end`, opened by following `link`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Item {
    pub start: usize,
    pub end: usize,
    pub link: u32,
}

/// Columns a box's border and padding take on each side.
const BOX_PAD: u16 = 2;
/// Columns between the page and its aside.
pub const ASIDE_GAP: u16 = 3;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Page {
    pub lines: Vec<PageLine>,
    /// Link targets (URLs), shared by the aside.
    pub links: Vec<String>,
    /// Columns of the main column.
    pub width: u16,
    pub items: Vec<Item>,
    /// The sidebar, drawn right of the main column and scrolled with it.
    pub aside: Vec<PageLine>,
    pub aside_width: u16,
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

    /// A gap line (none at the top, never two in a row).
    pub fn blank(&mut self) {
        if self
            .lines
            .last()
            .is_some_and(|l| !(l.is_blank() && l.frame == Frame::None))
        {
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
            segs: vec![Seg::linked(text, role, link)],
            indent,
            ..PageLine::default()
        });
        link
    }

    /// Columns for text on a line with this frame and indent.
    pub fn room(&self, frame: Frame, indent: u16) -> u16 {
        let inner = match frame {
            Frame::None => self.width,
            _ => self.width.saturating_sub(2 * BOX_PAD),
        };
        inner.saturating_sub(indent).max(8)
    }

    /// Word-wraps segments into lines `indent` columns in.
    pub fn wrapped(&mut self, segs: Vec<Seg>, indent: u16, frame: Frame) {
        let width = usize::from(self.room(frame, indent));
        for segs in wrap_segs(segs, width) {
            self.lines.push(PageLine {
                segs,
                indent,
                frame,
                ..PageLine::default()
            });
        }
    }

    /// A thin rule across the line (GitHub's header borders, Markdown's
    /// `---`).
    pub fn rule(&mut self, indent: u16, frame: Frame) {
        let width = usize::from(self.room(frame, indent));
        self.lines.push(PageLine {
            segs: vec![Seg::new("─".repeat(width), Role::Meta)],
            indent,
            frame,
            ..PageLine::default()
        });
    }

    /// Opens a box: `╭─ title ──── right ─╮`.
    pub fn box_top(&mut self, title: Vec<Seg>, right: Vec<Seg>) {
        self.blank_before_box();
        self.lines.push(PageLine {
            segs: title,
            right,
            frame: Frame::Top,
            ..PageLine::default()
        });
    }

    pub fn box_rule(&mut self) {
        self.lines.push(PageLine {
            frame: Frame::Rule,
            ..PageLine::default()
        });
    }

    pub fn box_bottom(&mut self) {
        // No empty line just inside the bottom edge.
        while self
            .lines
            .last()
            .is_some_and(|l| l.frame == Frame::Body && l.is_blank() && l.tone == Tone::Plain)
        {
            self.lines.pop();
        }
        self.lines.push(PageLine {
            frame: Frame::Bottom,
            ..PageLine::default()
        });
    }

    /// An empty line inside a box.
    pub fn box_gap(&mut self) {
        if self
            .lines
            .last()
            .is_some_and(|l| !(l.frame == Frame::Body && l.is_blank()) && l.frame != Frame::Top)
        {
            self.lines.push(PageLine {
                frame: Frame::Body,
                ..PageLine::default()
            });
        }
    }

    /// A line inside a box.
    pub fn box_line(&mut self, segs: Vec<Seg>, right: Vec<Seg>, indent: u16) {
        self.lines.push(PageLine {
            segs,
            right,
            indent,
            frame: Frame::Body,
            tone: Tone::Plain,
        });
    }

    fn blank_before_box(&mut self) {
        if self.lines.last().is_some_and(|l| l.frame == Frame::Bottom) {
            self.lines.push(PageLine::default());
            return;
        }
        self.blank();
    }

    /// Marks lines `start..` (to the end so far) as one item opening `link`.
    pub fn item(&mut self, start: usize, link: u32) {
        let end = self.lines.len();
        // Gaps before a box aren't part of it.
        let mut start = start;
        while start < end && self.lines[start].frame == Frame::None && self.lines[start].is_blank()
        {
            start += 1;
        }
        if end > start {
            self.items.push(Item { start, end, link });
        }
    }

    /// Builds the sidebar with `f`, which gets a page `width` wide that
    /// shares this page's links.
    pub fn build_aside(&mut self, width: u16, f: impl FnOnce(&mut Page)) {
        let mut aside = Page::new(width);
        aside.links = std::mem::take(&mut self.links);
        f(&mut aside);
        self.links = std::mem::take(&mut aside.links);
        self.aside = aside.lines;
        self.aside_width = width;
    }

    /// Rows the page takes (the longer of the column and the aside).
    pub fn height(&self) -> usize {
        self.lines.len().max(self.aside.len())
    }

    /// The item containing `line`.
    pub fn item_at(&self, line: usize) -> Option<usize> {
        self.items
            .iter()
            .position(|i| (i.start..i.end).contains(&line))
    }
}

/// Splits segments into lines of at most `width` columns, breaking at
/// spaces (or mid-word for words longer than a line). Newlines in segment
/// text force breaks.
pub(crate) fn wrap_segs(segs: Vec<Seg>, width: usize) -> Vec<Vec<Seg>> {
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

const PAGE_BG: Bg = Bg::ContainerLow;
const CODE_BG: Bg = Bg::Diff(DiffBg::Context);

/// Where a page's columns go in `area`: centered, as on GitHub.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Columns {
    pub main_x: u16,
    pub aside_x: Option<u16>,
}

pub fn columns(page: &Page, area: Rect) -> Columns {
    let aside = (!page.aside.is_empty()).then_some(ASIDE_GAP + page.aside_width);
    let total = page.width + aside.unwrap_or(0);
    let main_x = area.x + area.width.saturating_sub(total) / 2;
    Columns {
        main_x,
        aside_x: aside.map(|_| main_x + page.width + ASIDE_GAP),
    }
}

/// A link's place on screen (for letter hints).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spot {
    pub x: u16,
    pub y: u16,
    pub link: u32,
}

/// What's under a screen position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PageHit {
    /// A line of the main column.
    pub line: Option<usize>,
    pub link: Option<u32>,
}

/// Segments of a line placed on a row: `(x, visible text, seg)`.
fn place(line: &PageLine, x: u16, width: u16) -> Vec<(u16, String, &Seg)> {
    let (tx, tw) = match line.frame {
        Frame::None => (x + line.indent, width.saturating_sub(line.indent)),
        Frame::Body => (
            x + BOX_PAD + line.indent,
            width.saturating_sub(2 * BOX_PAD + line.indent),
        ),
        Frame::Top => (x + 3, width.saturating_sub(6)),
        Frame::Rule | Frame::Bottom => return Vec::new(),
    };
    let pad = u16::from(line.frame == Frame::Top);
    let right_w: u16 = line
        .right
        .iter()
        .map(|s| text::width(&s.text) as u16)
        .sum::<u16>()
        .min(tw);
    let left_w = if right_w > 0 {
        tw.saturating_sub(right_w + 2 + 2 * pad)
    } else {
        tw
    };
    let mut out = Vec::new();
    let mut at = tx;
    let mut room = usize::from(left_w);
    for seg in &line.segs {
        if room == 0 {
            break;
        }
        let shown = text::truncate(&seg.text.replace('\t', "    "), room);
        let w = text::width(&shown);
        room -= w.min(room);
        out.push((at, shown, seg));
        at += w as u16;
    }
    let mut rx = tx + tw - right_w;
    for seg in &line.right {
        let w = text::width(&seg.text) as u16;
        out.push((rx, seg.text.clone(), seg));
        rx += w;
    }
    out
}

/// Visible links' positions, main column then aside, top to bottom.
pub fn spots(page: &Page, area: Rect, scroll: usize) -> Vec<Spot> {
    let cols = columns(page, area);
    let mut out = Vec::new();
    let mut add = |lines: &[PageLine], x: u16, width: u16| {
        for (row, line) in lines
            .iter()
            .enumerate()
            .skip(scroll)
            .take(usize::from(area.height))
        {
            let y = area.y + (row - scroll) as u16;
            let mut seen = Vec::new();
            for (sx, _, seg) in place(line, x, width) {
                if let Some(link) = seg.link
                    && !seen.contains(&link)
                {
                    seen.push(link);
                    out.push(Spot { x: sx, y, link });
                }
            }
        }
    };
    add(&page.lines, cols.main_x, page.width);
    if let Some(ax) = cols.aside_x {
        add(&page.aside, ax, page.aside_width);
    }
    out.sort_by_key(|s| (s.y, s.x));
    out
}

/// What's at `(x, y)`.
pub fn hit(page: &Page, area: Rect, scroll: usize, x: u16, y: u16) -> PageHit {
    if !(area.y..area.bottom()).contains(&y) {
        return PageHit::default();
    }
    let row = scroll + usize::from(y - area.y);
    let cols = columns(page, area);
    let link_at = |line: &PageLine, lx: u16, width: u16| {
        place(line, lx, width)
            .into_iter()
            .find(|(sx, shown, _)| (*sx..*sx + text::width(shown) as u16).contains(&x))
            .and_then(|(_, _, seg)| seg.link)
    };
    if let Some(ax) = cols.aside_x
        && x >= ax
    {
        let link = page
            .aside
            .get(row)
            .and_then(|l| link_at(l, ax, page.aside_width));
        return PageHit { line: None, link };
    }
    let Some(line) = page.lines.get(row) else {
        return PageHit::default();
    };
    PageHit {
        line: Some(row),
        link: link_at(line, cols.main_x, page.width),
    }
}

/// A letter hint drawn over a link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HintLabel {
    pub x: u16,
    pub y: u16,
    pub label: String,
}

/// Renders a page scrolled to `scroll`, with `selected` item highlighted.
pub struct PageView<'a> {
    pub ctx: Ctx<'a>,
    pub page: &'a Page,
    pub scroll: usize,
    pub selected: Option<usize>,
    pub hints: &'a [HintLabel],
}

impl Widget for PageView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = self.ctx.theme;
        fill(buf, area, theme, PAGE_BG);
        let cols = columns(self.page, area);
        let selected = self.selected.and_then(|i| self.page.items.get(i)).copied();
        for row in 0..usize::from(area.height) {
            let i = self.scroll + row;
            let y = area.y + row as u16;
            if let Some(line) = self.page.lines.get(i) {
                let sel = selected.is_some_and(|s| (s.start..s.end).contains(&i));
                self.line(buf, line, cols.main_x, y, self.page.width, sel);
            }
            if let (Some(ax), Some(line)) = (cols.aside_x, self.page.aside.get(i)) {
                self.line(buf, line, ax, y, self.page.aside_width, false);
            }
        }
        self.scrollbar(area, buf);
        for hint in self.hints {
            let style = theme
                .fill(Bg::TertiaryContainer)
                .add_modifier(Modifier::BOLD);
            Span::styled(hint.label.clone(), style).render(
                Rect {
                    x: hint.x,
                    y: hint.y,
                    width: text::width(&hint.label) as u16,
                    height: 1,
                },
                buf,
            );
        }
    }
}

impl PageView<'_> {
    fn line(&self, buf: &mut Buffer, line: &PageLine, x: u16, y: u16, width: u16, sel: bool) {
        let theme = self.ctx.theme;
        let row = |x: u16, w: u16| Rect {
            x,
            y,
            width: w,
            height: 1,
        };
        let inner_bg = match (line.tone, sel) {
            (Tone::Code, _) => CODE_BG,
            (Tone::Plain, true) => Bg::Selected,
            (Tone::Plain, false) => PAGE_BG,
        };
        let framed = line.frame != Frame::None;
        // Backgrounds.
        if sel {
            // The stripe sits in the gutter, or just inside the box.
            let band = if framed {
                row(x + 1, width.saturating_sub(2))
            } else {
                row(x.saturating_sub(2), width + 2)
            };
            fill(buf, band, theme, Bg::Selected);
            let stripe_x = if framed { x + 1 } else { x.saturating_sub(2) };
            buf.set_string(stripe_x, y, "▌", theme.accent(Bg::Selected));
        } else if line.tone == Tone::Code {
            let band = if framed {
                row(x + 1, width.saturating_sub(2))
            } else {
                row(x, width)
            };
            fill(buf, band, theme, CODE_BG);
        }
        // Borders.
        let border = theme.separator(PAGE_BG);
        let rule = |buf: &mut Buffer, left: &str, right: &str| {
            buf.set_string(x, y, left, border);
            for cx in x + 1..x + width.saturating_sub(1) {
                buf.set_string(cx, y, "─", border);
            }
            buf.set_string(x + width.saturating_sub(1), y, right, border);
        };
        match line.frame {
            Frame::None => {}
            Frame::Top => rule(buf, "╭", "╮"),
            Frame::Rule => rule(buf, "├", "┤"),
            Frame::Bottom => rule(buf, "╰", "╯"),
            Frame::Body => {
                buf.set_string(x, y, "│", border);
                buf.set_string(x + width.saturating_sub(1), y, "│", border);
            }
        }
        // Text.
        let placed = place(line, x, width);
        if line.frame == Frame::Top {
            // Titles sit in gaps cut into the top edge.
            for (sx, shown, _) in &placed {
                let w = text::width(shown) as u16;
                fill(buf, row(sx.saturating_sub(1), w + 2), theme, PAGE_BG);
            }
        }
        for (sx, shown, seg) in placed {
            let w = text::width(&shown) as u16;
            Line::from(Span::styled(shown, self.style(&seg.role, inner_bg)))
                .render(row(sx, w), buf);
        }
    }

    fn scrollbar(&self, area: Rect, buf: &mut Buffer) {
        let total = self.page.height();
        let h = usize::from(area.height);
        if total <= h || h == 0 || area.width < 2 {
            return;
        }
        let thumb = (h * h / total).clamp(1, h);
        let top = (self.scroll * (h - thumb)) / total.saturating_sub(h).max(1);
        let x = area.right() - 1;
        for r in top..(top + thumb).min(h) {
            buf.set_string(x, area.y + r as u16, "▐", self.ctx.theme.separator(PAGE_BG));
        }
    }

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
            Role::Link => theme.accent(bg),
            Role::Code => match bg {
                Bg::Diff(_) | Bg::DiffSelected(_) => theme.style(Fg::Syntax(Syntax::String), bg),
                // GitHub's gray chip.
                _ => theme.fill(Bg::ContainerHigh),
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
}

/// Scrolls as little as possible to show rows `start..end` with `margin`
/// rows of context (less if the range is taller than the view).
pub fn reveal(
    start: usize,
    end: usize,
    scroll: usize,
    height: usize,
    total: usize,
    margin: usize,
) -> usize {
    let height = height.max(1);
    let max = total.saturating_sub(height);
    let margin = margin.min(height.saturating_sub(end - start) / 2);
    let mut scroll = scroll.min(max);
    if end + margin > scroll + height {
        scroll = (end + margin).saturating_sub(height);
    }
    if start < scroll + margin {
        scroll = start.saturating_sub(margin);
    }
    scroll.min(max)
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
    fn links_are_shared_and_items_mark_rows() {
        let mut page = Page::new(40);
        page.line(vec![Seg::new("title", Role::Title)]);
        page.blank();
        let start = page.lines.len();
        let a = page.link_line("one", "https://github.com/a", Role::Link, 0);
        page.line(vec![Seg::new("its second line", Role::Meta)]);
        page.item(start, a);
        let b = page.link_line("again", "https://github.com/a", Role::Link, 0);
        assert_eq!(a, b, "same URL, same link");
        assert_eq!(
            page.items,
            [Item {
                start: 2,
                end: 4,
                link: a
            }]
        );
        assert_eq!(page.item_at(3), Some(0));
        assert_eq!(page.item_at(4), None);
    }

    #[test]
    fn boxes_wrap_inside_their_borders_and_place_titles_on_the_edge() {
        let mut page = Page::new(24);
        page.box_top(
            vec![Seg::new("Title", Role::Strong)],
            vec![Seg::new("3", Role::Meta)],
        );
        page.wrapped(
            vec![Seg::new("one two three four five", Role::Body)],
            0,
            Frame::Body,
        );
        page.box_gap();
        page.box_bottom();
        let texts: Vec<String> = page.lines.iter().map(PageLine::text).collect();
        assert_eq!(texts, ["Title", "one two three four", "five", ""]);
        assert_eq!(
            page.lines.last().unwrap().frame,
            Frame::Bottom,
            "no gap above the edge"
        );

        let area = Rect::new(0, 0, 24, 10);
        let spots = spots(&page, area, 0);
        assert!(spots.is_empty());
        let placed = place(&page.lines[0], 0, 24);
        assert_eq!(placed[0].0, 3, "after `╭─ `");
        assert_eq!(placed[1].0, 24 - 3 - 1, "right title ends before ` ─╮`");
    }

    #[test]
    fn hits_and_spots_find_links_in_both_columns() {
        let mut page = Page::new(30);
        let link = page.link("https://github.com/a");
        page.line(vec![
            Seg::new("see ", Role::Body),
            Seg::linked("here", Role::Link, link),
        ]);
        page.build_aside(10, |aside| {
            aside.link_line("b", "https://github.com/b", Role::Link, 0);
        });
        let area = Rect::new(0, 0, 50, 5);
        let cols = columns(&page, area);
        assert_eq!(cols.main_x, (50 - 43) / 2);
        let spots = spots(&page, area, 0);
        assert_eq!(spots.len(), 2);
        assert_eq!(spots[0].x, cols.main_x + 4);
        assert_eq!(page.links[spots[1].link as usize], "https://github.com/b");
        let h = hit(&page, area, 0, cols.main_x + 5, 0);
        assert_eq!(
            h,
            PageHit {
                line: Some(0),
                link: Some(link)
            }
        );
        let h = hit(&page, area, 0, cols.main_x + 1, 0);
        assert_eq!(
            h,
            PageHit {
                line: Some(0),
                link: None
            }
        );
        let h = hit(&page, area, 0, cols.aside_x.unwrap(), 0);
        assert_eq!(h.link, Some(spots[1].link));
    }

    #[test]
    fn reveal_scrolls_as_little_as_possible() {
        assert_eq!(reveal(0, 1, 0, 10, 100, 2), 0);
        assert_eq!(reveal(20, 22, 0, 10, 100, 2), 14);
        assert_eq!(reveal(5, 6, 12, 10, 100, 2), 3);
        assert_eq!(reveal(99, 100, 0, 10, 100, 2), 90);
        // Taller than the view: its top shows.
        assert_eq!(reveal(10, 40, 0, 10, 100, 2), 10);
    }
}
