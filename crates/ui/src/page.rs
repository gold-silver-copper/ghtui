//! Pages: documents of styled lines with links, laid out like GitHub's.
//!
//! Builders produce lines from data with *semantic* roles (title, body,
//! link, code, ...), never colors; [`PageView`] maps roles to theme styles.
//! Lines can sit in boxes (rounded outlines, like GitHub's file list, README
//! and comments). Links are URLs: GitHub web URLs for anything ghtui can
//! show itself, so following one works like clicking it on the website.
//! *Items* are the rows you select and open (a file, an issue, a comment);
//! an optional *aside* is GitHub's right-hand sidebar.

#![deny(clippy::arithmetic_side_effects)]

use ghtui_theme::{Bg, DiffBg, Fg, Syntax};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use std::collections::HashMap;

use crate::{Ctx, cols, fill, idx, put, text};

/// Where following a link leads: a URL, or something the page does. Only
/// pages create the actions; Markdown from GitHub can only make URLs.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Link {
    Url(String),
    /// Load the next page of a list.
    More,
    /// Star or unstar the repository.
    Star,
    /// Write a comment.
    Comment,
    /// Edit the list's filter.
    Filter,
    /// Change the list's sort.
    Sort,
    /// Show the list's items in this state (`open`, `closed`...).
    State(String),
    /// Switch branches.
    Branch,
    /// Find a file in the repository.
    FindFile,
    /// Reply quoting comment `n` of [`Page::quotes`].
    Quote(u32),
}

impl Link {
    pub fn url(&self) -> Option<&str> {
        match self {
            Link::Url(url) => Some(url),
            _ => None,
        }
    }
}

impl From<String> for Link {
    fn from(url: String) -> Self {
        Link::Url(url)
    }
}

impl From<&str> for Link {
    fn from(url: &str) -> Self {
        Link::Url(url.to_owned())
    }
}

/// A comment a quote reply quotes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quote {
    pub author: String,
    pub body: String,
}

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
    /// A code line a link points at.
    Marked,
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
    /// Link targets, shared by the aside.
    pub links: Vec<Link>,
    /// Each link's index in `links`.
    link_ids: HashMap<Link, u32>,
    /// Comments quote replies quote, by [`Link::Quote`].
    pub quotes: Vec<Quote>,
    /// Columns of the main column.
    pub width: u16,
    pub items: Vec<Item>,
    /// The sidebar, drawn right of the main column and scrolled with it.
    pub aside: Vec<PageLine>,
    pub aside_width: u16,
    /// Lists take one row per item.
    pub compact: bool,
    /// The line the page opens scrolled to (a linked line), if not the top.
    pub jump: Option<usize>,
}

impl Page {
    pub fn new(width: u16) -> Self {
        Self {
            width: width.max(20),
            ..Self::default()
        }
    }

    /// Registers a link target, returning its index.
    pub fn link(&mut self, link: impl Into<Link>) -> u32 {
        let link = link.into();
        if let Some(&i) = self.link_ids.get(&link) {
            return i;
        }
        let i = idx(self.links.len());
        self.links.push(link.clone());
        self.link_ids.insert(link, i);
        i
    }

    /// A link that quote-replies to a comment.
    pub fn quote(&mut self, author: &str, body: &str) -> u32 {
        let n = idx(self.quotes.len());
        self.quotes.push(Quote {
            author: author.to_owned(),
            body: body.to_owned(),
        });
        self.link(Link::Quote(n))
    }

    /// What following link `i` does.
    pub fn target(&self, i: u32) -> Option<&Link> {
        self.links.get(i as usize)
    }

    pub fn push(&mut self, line: PageLine) {
        self.lines.push(line);
    }

    /// A line of `segs`, `indent` columns in, with `right` on its right.
    pub(crate) fn add(&mut self, frame: Frame, indent: u16, segs: Vec<Seg>, right: Vec<Seg>) {
        self.lines.push(PageLine {
            segs,
            indent,
            right,
            frame,
            tone: Tone::Plain,
        });
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
        self.add(Frame::None, 0, segs, Vec::new());
    }

    /// A line with a single link segment.
    pub fn link_line(
        &mut self,
        text: impl Into<String>,
        url: impl Into<Link>,
        role: Role,
        indent: u16,
    ) -> u32 {
        let link = self.link(url);
        self.add(
            Frame::None,
            indent,
            vec![Seg::linked(text, role, link)],
            Vec::new(),
        );
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
            self.add(frame, indent, segs, Vec::new());
        }
    }

    /// A thin rule across the line (GitHub's header borders, Markdown's
    /// `---`).
    pub fn rule(&mut self, indent: u16, frame: Frame) {
        let rule = "─".repeat(usize::from(self.room(frame, indent)));
        self.add(frame, indent, vec![Seg::new(rule, Role::Meta)], Vec::new());
    }

    /// Opens a box: `╭─ title ──── right ─╮`.
    pub fn box_top(&mut self, title: Vec<Seg>, right: Vec<Seg>) {
        self.blank_before_box();
        self.add(Frame::Top, 0, title, right);
    }

    pub fn box_rule(&mut self) {
        self.add(Frame::Rule, 0, Vec::new(), Vec::new());
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
        self.add(Frame::Bottom, 0, Vec::new(), Vec::new());
    }

    /// An empty line inside a box.
    pub fn box_gap(&mut self) {
        if self
            .lines
            .last()
            .is_some_and(|l| !(l.frame == Frame::Body && l.is_blank()) && l.frame != Frame::Top)
        {
            self.add(Frame::Body, 0, Vec::new(), Vec::new());
        }
    }

    /// A line inside a box.
    pub fn box_line(&mut self, segs: Vec<Seg>, right: Vec<Seg>, indent: u16) {
        self.add(Frame::Body, indent, segs, right);
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
        let gaps = self.lines.iter().skip(start);
        let start = start.saturating_add(
            gaps.take_while(|l| l.frame == Frame::None && l.is_blank())
                .count(),
        );
        if end > start {
            self.items.push(Item { start, end, link });
        }
    }

    /// Builds the sidebar with `f`, which gets a page `width` wide that
    /// shares this page's links.
    pub fn build_aside(&mut self, width: u16, f: impl FnOnce(&mut Page)) {
        let mut aside = Page::new(width);
        aside.links = std::mem::take(&mut self.links);
        aside.link_ids = std::mem::take(&mut self.link_ids);
        aside.quotes = std::mem::take(&mut self.quotes);
        f(&mut aside);
        self.links = std::mem::take(&mut aside.links);
        self.link_ids = std::mem::take(&mut aside.link_ids);
        self.quotes = std::mem::take(&mut aside.quotes);
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
        let Some(line) = lines.last_mut() else {
            return;
        };
        match line.last_mut() {
            Some(last) if last.role == seg.role && last.link == seg.link => {
                last.text.push_str(text);
            }
            _ => line.push(Seg {
                text: text.to_owned(),
                role: seg.role.clone(),
                link: seg.link,
            }),
        }
    };
    for seg in segs {
        for (n, part) in seg.text.split('\n').enumerate() {
            if n > 0 {
                lines.push(Vec::new());
                used = 0;
            }
            // Words with the spaces that follow them.
            for word in part.split_inclusive(' ') {
                let w = text::width(word.trim_end());
                if used > 0 && used.saturating_add(w) > width {
                    lines.push(Vec::new());
                    used = 0;
                    if word.trim().is_empty() {
                        continue;
                    }
                }
                if w > width {
                    // Hard-break a long word.
                    let mut chunk = String::new();
                    for (g, gw) in text::graphemes(word) {
                        if used.saturating_add(gw) > width {
                            push(&mut lines, &seg, &chunk);
                            chunk.clear();
                            lines.push(Vec::new());
                            used = 0;
                        }
                        chunk.push_str(g);
                        used = used.saturating_add(gw);
                    }
                    push(&mut lines, &seg, &chunk);
                    continue;
                }
                push(&mut lines, &seg, word);
                used = used.saturating_add(text::width(word));
            }
        }
    }
    // Drop trailing spaces.
    for last in lines.iter_mut().filter_map(|l| l.last_mut()) {
        last.text.truncate(last.text.trim_end().len());
    }
    lines
}

const PAGE_BG: Bg = Bg::ContainerLow;
const CODE_BG: Bg = Bg::Diff(DiffBg::Context);
const MARKED_BG: Bg = Bg::DiffSelected(DiffBg::Context);

/// Where a page's columns go in `area`: centered, as on GitHub.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Columns {
    pub main_x: u16,
    pub aside_x: Option<u16>,
}

pub fn columns(page: &Page, area: Rect) -> Columns {
    let aside = (!page.aside.is_empty()).then_some(ASIDE_GAP.saturating_add(page.aside_width));
    let total = page.width.saturating_add(aside.unwrap_or(0));
    let main_x = area.x.saturating_add(area.width.saturating_sub(total) / 2);
    Columns {
        main_x,
        aside_x: aside.map(|_| main_x.saturating_add(page.width).saturating_add(ASIDE_GAP)),
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
        Frame::None => (
            x.saturating_add(line.indent),
            width.saturating_sub(line.indent),
        ),
        Frame::Body => (
            x.saturating_add(BOX_PAD).saturating_add(line.indent),
            width.saturating_sub((2 * BOX_PAD).saturating_add(line.indent)),
        ),
        Frame::Top => (x.saturating_add(3), width.saturating_sub(6)),
        Frame::Rule | Frame::Bottom => return Vec::new(),
    };
    // Between the sides; a top edge keeps a column of rule on each.
    let gap = if line.frame == Frame::Top { 4 } else { 2 };
    // The right side gets at most half when there's a left side too.
    let right_cap = if line.segs.is_empty() { tw } else { tw / 2 };
    let right_w = line
        .right
        .iter()
        .map(|s| cols(text::width(&s.text)))
        .fold(0, u16::saturating_add)
        .min(right_cap);
    let left_w = if right_w > 0 {
        tw.saturating_sub(right_w.saturating_add(gap))
    } else {
        tw
    };
    let mut out = Vec::new();
    lay_out(&mut out, &line.segs, tx, left_w);
    let right_x = tx.saturating_add(tw.saturating_sub(right_w));
    lay_out(&mut out, &line.right, right_x, right_w);
    out
}

/// `segs` from `x` on, cut to `room` columns.
fn lay_out<'a>(out: &mut Vec<(u16, String, &'a Seg)>, segs: &'a [Seg], mut x: u16, room: u16) {
    let mut room = usize::from(room);
    for seg in segs {
        if room == 0 {
            break;
        }
        let shown = text::truncate(&seg.text.replace('\t', "    "), room);
        let w = text::width(&shown);
        room = room.saturating_sub(w);
        out.push((x, shown, seg));
        x = x.saturating_add(cols(w));
    }
}

/// Visible links' positions, main column then aside, top to bottom.
pub fn spots(page: &Page, area: Rect, scroll: usize) -> Vec<Spot> {
    let layout = columns(page, area);
    let mut out = Vec::new();
    let mut add = |lines: &[PageLine], x: u16, width: u16| {
        for (line, y) in lines.iter().skip(scroll).zip(area.top()..area.bottom()) {
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
    add(&page.lines, layout.main_x, page.width);
    if let Some(ax) = layout.aside_x {
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
    let row = scroll.saturating_add(usize::from(y.saturating_sub(area.y)));
    let layout = columns(page, area);
    let link_at = |line: &PageLine, lx: u16, width: u16| {
        place(line, lx, width)
            .into_iter()
            .find(|(sx, shown, _)| (*sx..sx.saturating_add(cols(text::width(shown)))).contains(&x))
            .and_then(|(_, _, seg)| seg.link)
    };
    if let Some(ax) = layout.aside_x
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
        link: link_at(line, layout.main_x, page.width),
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
        let layout = columns(self.page, area);
        let selected = self.selected.and_then(|i| self.page.items.get(i)).copied();
        for (row, y) in (area.top()..area.bottom()).enumerate() {
            let i = self.scroll.saturating_add(row);
            if let Some(line) = self.page.lines.get(i) {
                let sel = selected.is_some_and(|s| (s.start..s.end).contains(&i));
                self.line(buf, line, layout.main_x, y, self.page.width, sel);
            }
            if let (Some(ax), Some(line)) = (layout.aside_x, self.page.aside.get(i)) {
                self.line(buf, line, ax, y, self.page.aside_width, false);
            }
        }
        self.scrollbar(area, buf);
        for hint in self.hints {
            let style = theme
                .fill(Bg::TertiaryContainer)
                .add_modifier(Modifier::BOLD);
            // Just before the link (over the space, padding or icon there),
            // so the link's own text stays readable.
            let width = cols(text::width(&hint.label));
            let x = hint
                .x
                .checked_sub(width)
                .filter(|x| *x >= area.x)
                .unwrap_or(hint.x);
            Span::styled(hint.label.clone(), style).render(
                Rect {
                    x,
                    y: hint.y,
                    width,
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
            (Tone::Marked, _) => MARKED_BG,
            (Tone::Plain, true) => Bg::Selected,
            (Tone::Plain, false) => PAGE_BG,
        };
        let framed = line.frame != Frame::None;
        // Backgrounds.
        if sel {
            // The stripe sits in the gutter, or just inside the box.
            let band = if framed {
                row(x.saturating_add(1), width.saturating_sub(2))
            } else {
                row(x.saturating_sub(2), width.saturating_add(2))
            };
            fill(buf, band, theme, Bg::Selected);
            put(buf, band.x, y, "▌", theme.accent(Bg::Selected));
        } else if line.tone != Tone::Plain {
            let band = if framed {
                row(x.saturating_add(1), width.saturating_sub(2))
            } else {
                row(x, width)
            };
            fill(buf, band, theme, inner_bg);
        }
        // Borders.
        let border = theme.separator(PAGE_BG);
        let rule = |buf: &mut Buffer, left: &str, right: &str| {
            let middle = "─".repeat(usize::from(width.saturating_sub(2)));
            put(buf, x, y, &format!("{left}{middle}{right}"), border);
        };
        match line.frame {
            Frame::None => {}
            Frame::Top => rule(buf, "╭", "╮"),
            Frame::Rule => rule(buf, "├", "┤"),
            Frame::Bottom => rule(buf, "╰", "╯"),
            Frame::Body => {
                put(buf, x, y, "│", border);
                put(
                    buf,
                    x.saturating_add(width.saturating_sub(1)),
                    y,
                    "│",
                    border,
                );
            }
        }
        // Text.
        let placed = place(line, x, width);
        if line.frame == Frame::Top {
            // Titles sit in gaps cut into the top edge.
            for (sx, shown, _) in &placed {
                let w = cols(text::width(shown));
                fill(
                    buf,
                    row(sx.saturating_sub(1), w.saturating_add(2)),
                    theme,
                    PAGE_BG,
                );
            }
        }
        for (sx, shown, seg) in placed {
            let w = cols(text::width(&shown));
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
        let thumb = h
            .saturating_mul(h)
            .checked_div(total)
            .unwrap_or(h)
            .clamp(1, h);
        let top = self
            .scroll
            .saturating_mul(h.saturating_sub(thumb))
            .checked_div(total.saturating_sub(h))
            .unwrap_or(0);
        let x = area.right().saturating_sub(1);
        for r in top..top.saturating_add(thumb).min(h) {
            let y = area.y.saturating_add(cols(r));
            put(buf, x, y, "▐", self.ctx.theme.separator(PAGE_BG));
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
            Role::Link | Role::Accent => theme.accent(bg),
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
    let margin = margin.min(height.saturating_sub(end.saturating_sub(start)) / 2);
    let mut scroll = scroll.min(max);
    if end.saturating_add(margin) > scroll.saturating_add(height) {
        scroll = end.saturating_add(margin).saturating_sub(height);
    }
    if start < scroll.saturating_add(margin) {
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
        assert_eq!(
            page.links[spots[1].link as usize],
            Link::from("https://github.com/b")
        );
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
