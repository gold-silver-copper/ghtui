//! The fixed chrome around pages, after GitHub's: a header with where you
//! are, a search field and your account; a tab bar with an underlined active
//! tab; a sticky title; the search dropdown; key panels.

#![deny(clippy::arithmetic_side_effects)]

use crate::theme::Bg;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use ratatui_textarea::TextArea;

use crate::ui::{Ctx, PAD_X, cols, fill, fit_spans, inset, list_row, put, render_split, row, text};

const BAR: Bg = Bg::Container;
const FIELD: Bg = Bg::ContainerHighest;
const PANEL: Bg = Bg::ContainerHigh;

// ---- header -------------------------------------------------------------------------

/// One part of the location in the header (`owner`, `repo`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crumb {
    pub text: String,
    /// The last, current part is bold.
    pub current: bool,
    /// An open tab in the tab strip (spaced apart, not a path).
    pub tab: bool,
}

impl Crumb {
    /// What goes before it, after the first.
    fn sep(&self) -> &'static str {
        if self.tab { TAB_SEP } else { SEP }
    }
}

const LOGO: &str = "◆ ghtui";
const SEP: &str = " / ";
const TAB_SEP: &str = "   ";

/// Where the header's parts are, for drawing and for clicks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HeaderLayout {
    pub logo: Rect,
    pub crumbs: Vec<Rect>,
    pub search: Rect,
    pub right: Vec<Rect>,
}

pub fn header_layout(area: Rect, crumbs: &[Crumb], right: &[String]) -> HeaderLayout {
    let y = area.y;
    let one = |x: u16, w: u16| Rect::new(x, y, w, 1);
    let start = area.x.saturating_add(PAD_X.min(area.width));
    let logo = one(start, cols(text::width(LOGO)).min(area.width));
    // Right items, from the right edge, while where you are and the search
    // field still fit; the rest are dropped (zero width).
    // The search field: a third of the width, between 24 and 52 columns.
    let field_w = (area.width / 3)
        .clamp(24, 52)
        .min(area.width.saturating_sub(24));
    let crumbs_w = crumbs
        .iter()
        .map(|c| cols(text::width(&c.text)).saturating_add(3))
        .fold(0, u16::saturating_add);
    let keep = logo
        .right()
        .saturating_add(crumbs_w.min(32))
        .saturating_add(5)
        .saturating_add(field_w);
    // Earlier items matter more: you know who you are.
    let mut room = area
        .right()
        .saturating_sub(PAD_X)
        .saturating_add(3)
        .saturating_sub(keep);
    let widths: Vec<u16> = right
        .iter()
        .map(|item| {
            let w = cols(text::width(item));
            let Some(rest) = room.checked_sub(w.saturating_add(3)) else { return 0 };
            room = rest;
            w
        })
        .collect();
    let mut rx = area.right().saturating_sub(PAD_X);
    let mut right_rects = Vec::new();
    for &w in widths.iter().rev() {
        rx = rx.saturating_sub(w);
        right_rects.push(one(rx, w));
        if w > 0 {
            rx = rx.saturating_sub(3);
        }
    }
    right_rects.reverse();
    let field_x = rx.saturating_sub(field_w);
    let search = one(field_x, field_w);
    let mut x = logo.right().saturating_add(3);
    let mut crumb_rects = Vec::new();
    for (i, c) in crumbs.iter().enumerate() {
        if i > 0 {
            x = x.saturating_add(cols(c.sep().len()));
        }
        let w = cols(text::width(&c.text)).min(field_x.saturating_sub(x.saturating_add(2)));
        crumb_rects.push(one(x, w));
        x = x.saturating_add(w);
    }
    HeaderLayout {
        logo,
        crumbs: crumb_rects,
        search,
        right: right_rects,
    }
}

pub struct Header<'a> {
    pub ctx: Ctx<'a>,
    pub crumbs: &'a [Crumb],
    /// Right-side items (`⇄ 3`, `@login`).
    pub right: &'a [String],
    /// The search input when searching; otherwise a placeholder shows.
    pub input: Option<&'a TextArea<'static>>,
    pub placeholder: &'a str,
}

impl Widget for Header<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = self.ctx.theme;
        fill(buf, area, theme, BAR);
        let lay = header_layout(area, self.crumbs, self.right);
        Span::styled(LOGO, theme.accent(BAR).add_modifier(Modifier::BOLD)).render(lay.logo, buf);
        for (i, (c, r)) in self.crumbs.iter().zip(&lay.crumbs).enumerate() {
            if i > 0 {
                // A crumb with no room left is an ellipsis after the last.
                let sep = if r.width == 0 { " …" } else { c.sep() };
                put(
                    buf,
                    r.x.saturating_sub(cols(c.sep().len())),
                    r.y,
                    sep,
                    theme.meta(BAR),
                );
            }
            let style = if c.current {
                theme.title(BAR)
            } else {
                theme.body(BAR)
            };
            Span::styled(text::fit(&c.text, usize::from(r.width)), style).render(*r, buf);
        }
        match self.input {
            Some(input) => SearchField {
                ctx: self.ctx,
                input,
            }
            .render(lay.search, buf),
            None => {
                fill(buf, lay.search, theme, FIELD);
                let inner = inner(lay.search);
                Span::styled(
                    text::truncate(&format!("⌕ {}", self.placeholder), usize::from(inner.width)),
                    theme.meta(FIELD),
                )
                .render(inner, buf);
            }
        }
        for (item, r) in self.right.iter().zip(&lay.right) {
            Span::styled(item.as_str(), theme.meta(BAR)).render(*r, buf);
        }
    }
}

/// A search field being typed into: the header's, or a list's own filter
/// field edited in place.
pub struct SearchField<'a> {
    pub ctx: Ctx<'a>,
    pub input: &'a TextArea<'static>,
}

impl Widget for SearchField<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        fill(buf, area, self.ctx.theme, FIELD);
        let inner = inner(area);
        Span::styled("⌕ ", self.ctx.theme.accent(FIELD)).render(inner, buf);
        self.input.render(
            Rect {
                x: inner.x.saturating_add(2),
                width: inner.width.saturating_sub(2),
                ..inner
            },
            buf,
        );
    }
}

/// A field's text, a column in from each side.
fn inner(field: Rect) -> Rect {
    Rect {
        x: field.x.saturating_add(1),
        width: field.width.saturating_sub(2),
        ..field
    }
}

// ---- tabs ------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageTab {
    pub icon: &'static str,
    pub label: String,
    pub count: Option<u64>,
}

/// How much of each tab fits: everything; no counts on inactive tabs;
/// inactive tabs as icons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Fit {
    Full,
    NoCounts,
    Icons,
}

/// How much each tab shows: everything if it fits; else no counts; else
/// the inactive tabs from the right shrink to their icons until it fits.
fn fit(area: Rect, tabs: &[PageTab], active: Option<usize>) -> Vec<Fit> {
    let room = area.width.saturating_sub(PAD_X);
    let width = |i: usize, tab, level| tab_width(tab, level, active == Some(i)).saturating_add(1);
    let total = |level| {
        (tabs.iter().enumerate())
            .map(|(i, tab)| width(i, tab, level))
            .fold(0, u16::saturating_add)
    };
    if total(Fit::Full) <= room {
        return vec![Fit::Full; tabs.len()];
    }
    let mut levels = vec![Fit::NoCounts; tabs.len()];
    let mut used = total(Fit::NoCounts);
    for (i, tab) in tabs.iter().enumerate().rev() {
        if used <= room {
            break;
        }
        if active != Some(i)
            && let Some(level) = levels.get_mut(i)
        {
            *level = Fit::Icons;
            used = (used.saturating_sub(width(i, tab, Fit::NoCounts))).saturating_add(width(
                i,
                tab,
                Fit::Icons,
            ));
        }
    }
    levels
}

/// Each tab's rectangle on the label row.
pub fn tab_layout(area: Rect, tabs: &[PageTab], active: Option<usize>) -> Vec<Rect> {
    tab_rects(area, tabs, active, &fit(area, tabs, active))
}

/// [`tab_layout`], with how much each tab shows.
fn tab_rects(area: Rect, tabs: &[PageTab], active: Option<usize>, levels: &[Fit]) -> Vec<Rect> {
    let mut x = area.x.saturating_add(PAD_X.min(area.width));
    let mut out = Vec::new();
    for (i, (tab, &level)) in tabs.iter().zip(levels).enumerate() {
        let w = tab_width(tab, level, active == Some(i));
        let w = w.min(area.right().saturating_sub(x));
        out.push(Rect::new(x, area.y, w, 1));
        x = x.saturating_add(w).saturating_add(1);
    }
    out
}

fn tab_width(tab: &PageTab, level: Fit, active: bool) -> u16 {
    let shown = |at: Fit| active || level <= at;
    let count = if shown(Fit::Full) {
        tab.count.map_or(0, |n| {
            text::width(&crate::ui::pages::compact(n)).saturating_add(3)
        })
    } else {
        0
    };
    let label = if shown(Fit::NoCounts) {
        text::width(&tab.label).saturating_add(1)
    } else {
        0
    };
    cols(
        // ` 3 ◷ `: its number, for `3gt`, and its icon.
        [4, text::width(tab.icon), label, count]
            .into_iter()
            .fold(0, usize::saturating_add),
    )
}

/// Tabs on one row, the active one underlined on the row below (2 rows).
pub struct TabBar<'a> {
    pub ctx: Ctx<'a>,
    pub tabs: &'a [PageTab],
    pub active: Option<usize>,
}

impl Widget for TabBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = self.ctx.theme;
        let labels = row(area, area.y);
        fill(buf, labels, theme, BAR);
        let under = row(area, area.y.saturating_add(1));
        if area.height > 1 {
            fill(buf, under, theme, BAR);
            let rule = "─".repeat(usize::from(under.width));
            put(buf, under.x, under.y, &rule, theme.separator(BAR));
        }
        let levels = fit(labels, self.tabs, self.active);
        for (i, ((tab, r), &level)) in self
            .tabs
            .iter()
            .zip(tab_rects(labels, self.tabs, self.active, &levels))
            .zip(&levels)
            .enumerate()
        {
            let active = self.active == Some(i);
            let shown = |at: Fit| active || level <= at;
            let label_style = if active {
                theme.title(BAR)
            } else {
                theme.body(BAR)
            };
            // `1` to `9`, for `3gt`; none past them.
            let number = (u32::try_from(i).ok())
                .and_then(|i| char::from_digit(i.saturating_add(1), 10))
                .unwrap_or(' ');
            let mut spans = vec![
                Span::styled(
                    format!(" {number}"),
                    theme.meta(BAR).add_modifier(Modifier::DIM),
                ),
                Span::styled(format!(" {} ", tab.icon), theme.meta(BAR)),
            ];
            if shown(Fit::NoCounts) {
                spans.push(Span::styled(tab.label.as_str(), label_style));
            }
            if let Some(n) = tab.count.filter(|_| shown(Fit::Full)) {
                spans.push(Span::styled(" ", theme.body(BAR)));
                spans.push(Span::styled(
                    format!(" {} ", crate::ui::pages::compact(n)),
                    theme.fill(Bg::ContainerHighest),
                ));
            }
            spans.push(Span::styled(" ", theme.body(BAR)));
            Line::from(spans).render(r, buf);
            if active && area.height > 1 {
                let rule = "━".repeat(usize::from(r.width));
                put(buf, r.x, under.y, &rule, theme.accent(BAR));
            }
        }
    }
}

/// A one-row title strip under the header (a pull request's sticky title).
pub struct TitleBar<'a> {
    pub ctx: Ctx<'a>,
    pub line: Line<'a>,
}

impl Widget for TitleBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        fill(buf, area, self.ctx.theme, BAR);
        let inner = inset(area, PAD_X, 0);
        let spans = fit_spans(&self.line.spans, inner.width);
        Line { spans, ..self.line }.render(inner, buf);
    }
}

// ---- search dropdown -----------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuggestRow {
    Heading(String),
    Item {
        icon: String,
        label: String,
        /// Dim text after the label.
        detail: String,
        /// Right-aligned hint.
        hint: String,
    },
}

/// The dropdown under the search field being typed into.
pub struct SearchPanel<'a> {
    pub ctx: Ctx<'a>,
    pub rows: &'a [SuggestRow],
    /// Index among the item rows.
    pub selected: usize,
    pub field: Rect,
}

impl SearchPanel<'_> {
    pub fn area(&self, screen: Rect) -> Rect {
        let width = self.field.width.max(64).min(screen.width);
        let x = self
            .field
            .x
            .min(screen.right().saturating_sub(width))
            .max(screen.x);
        // Down to the status bar.
        let y = self.field.y.saturating_add(1);
        let height = cols(self.rows.len())
            .saturating_add(2)
            .min(screen.bottom().saturating_sub(y).saturating_sub(1));
        Rect::new(x, y, width, height)
    }

    /// The first row shown, keeping the selection in view, and the
    /// selection's row.
    fn scroll(&self, area: Rect) -> (usize, usize) {
        let rows = usize::from(area.height.saturating_sub(2));
        let selected_row = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| matches!(r, SuggestRow::Item { .. }))
            .nth(self.selected)
            .map_or(0, |(i, _)| i);
        (
            selected_row.saturating_add(1).saturating_sub(rows),
            selected_row,
        )
    }

    /// The row drawn at screen row `y`, if any.
    pub fn row_at(&self, screen: Rect, y: u16) -> Option<usize> {
        let area = self.area(screen);
        let n = usize::from(y.checked_sub(area.y.saturating_add(1))?);
        (n < usize::from(area.height.saturating_sub(2)))
            .then(|| self.scroll(area).0.saturating_add(n))
    }
}

impl Widget for SearchPanel<'_> {
    fn render(self, screen: Rect, buf: &mut Buffer) {
        let theme = self.ctx.theme;
        let area = self.area(screen);
        fill(buf, area, theme, PANEL);
        let rows = usize::from(area.height.saturating_sub(2));
        let (skip, selected_row) = self.scroll(area);
        let shown = self.rows.iter().enumerate().skip(skip).take(rows);
        for ((n, row), y) in shown.zip(area.y.saturating_add(1)..area.bottom()) {
            let (line, bg) = list_row(buf, theme, area, y, n == selected_row, PANEL);
            match row {
                SuggestRow::Heading(h) => {
                    Span::styled(h.as_str(), heading(theme)).render(line, buf);
                }
                SuggestRow::Item {
                    icon,
                    label,
                    detail,
                    hint,
                } => {
                    let left = [
                        Span::styled(format!("{icon}  {label}"), theme.body(bg)),
                        Span::styled(format!("  {detail}"), theme.meta(bg)),
                    ];
                    let hint = [Span::styled(hint.as_str(), theme.meta(bg))];
                    render_split(line, buf, &left, &hint, 2);
                }
            }
        }
    }
}

/// A section heading in a popup list.
fn heading(theme: &crate::theme::Theme) -> ratatui::style::Style {
    theme.meta(PANEL).add_modifier(Modifier::BOLD)
}

// ---- key panels -------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRow {
    pub key: String,
    pub label: String,
    /// Can't run now.
    pub dim: bool,
    /// A section heading rather than a key.
    pub heading: bool,
}

impl KeyRow {
    pub fn key(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            dim: false,
            heading: false,
        }
    }

    pub fn heading(label: impl Into<String>) -> Self {
        Self {
            heading: true,
            ..Self::key("", label)
        }
    }
}

/// A panel of keys at the bottom right, above the status bar: what can be
/// done here, with a selection.
pub struct KeyPanel<'a> {
    pub ctx: Ctx<'a>,
    pub title: &'a str,
    pub rows: &'a [KeyRow],
    /// The highlighted row, if one is (which-key's hints have none).
    pub selected: Option<usize>,
    /// Where the selection is among the items (`3/40`), shown when they
    /// don't all fit.
    pub position: (usize, usize),
}

impl KeyPanel<'_> {
    fn key_width(&self) -> usize {
        self.rows
            .iter()
            .map(|r| text::width(&r.key))
            .max()
            .unwrap_or(0)
    }

    pub fn area(&self, screen: Rect) -> Rect {
        let label_w = self
            .rows
            .iter()
            .map(|r| text::width(&r.label))
            .max()
            .unwrap_or(0);
        let width = self
            .key_width()
            .saturating_add(label_w)
            .saturating_add(3 + 4)
            .max(text::width(self.title).saturating_add(4));
        let width = cols(width).min(screen.width);
        let height = cols(self.rows.len())
            .saturating_add(3)
            .min(screen.height.saturating_sub(1));
        Rect::new(
            screen.right().saturating_sub(width.saturating_add(1)),
            screen.bottom().saturating_sub(height.saturating_add(1)),
            width,
            height,
        )
    }
}

impl Widget for KeyPanel<'_> {
    fn render(self, screen: Rect, buf: &mut Buffer) {
        let theme = self.ctx.theme;
        let area = self.area(screen);
        fill(buf, area, theme, PANEL);
        let key_w = self.key_width();
        let title_row = Rect::new(
            area.x.saturating_add(2),
            area.y,
            area.width.saturating_sub(4),
            1,
        );
        let rows = usize::from(area.height.saturating_sub(3));
        let (at, of) = self.position;
        let position = if self.rows.len() > rows {
            format!("{at}/{of}")
        } else {
            String::new()
        };
        let title = vec![Span::styled(self.title, theme.title(PANEL))];
        let position = vec![Span::styled(position, theme.meta(PANEL))];
        render_split(title_row, buf, &title, &position, 1);
        let skip = self
            .selected
            .map_or(0, |s| s.saturating_add(1).saturating_sub(rows));
        let shown = self.rows.iter().enumerate().skip(skip).take(rows);
        for ((n, row), y) in shown.zip(area.y.saturating_add(2)..area.bottom()) {
            let (line, bg) = list_row(buf, theme, area, y, self.selected == Some(n), PANEL);
            if row.heading {
                Span::styled(row.label.as_str(), heading(theme)).render(line, buf);
                continue;
            }
            let (key_style, label_style) = if row.dim {
                (theme.meta(bg), theme.meta(bg))
            } else {
                (
                    theme.accent(bg).add_modifier(Modifier::BOLD),
                    theme.body(bg),
                )
            };
            let pad = key_w.saturating_sub(text::width(&row.key));
            let left = [
                Span::styled(format!("{}{}   ", " ".repeat(pad), row.key), key_style),
                Span::styled(row.label.as_str(), label_style),
            ];
            render_split(line, buf, &left, &[], 0);
        }
    }
}
