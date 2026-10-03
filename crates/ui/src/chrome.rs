//! The fixed chrome around pages, after GitHub's: a header with where you
//! are, a search field and your account; a tab bar with an underlined active
//! tab; a sticky title; the search dropdown; key panels.

use ghtui_theme::Bg;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use ratatui_textarea::TextArea;

use crate::{Ctx, PAD_X, fill, text};

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
}

const LOGO: &str = "◆ ghtui";
const SEP: &str = " / ";

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
    let start = area.x + PAD_X.min(area.width);
    let logo = one(start, (text::width(LOGO) as u16).min(area.width));
    // Right items, from the right edge.
    let mut rx = area.right().saturating_sub(PAD_X);
    let mut right_rects = Vec::new();
    for item in right.iter().rev() {
        let w = text::width(item) as u16;
        rx = rx.saturating_sub(w);
        right_rects.push(one(rx, w));
        rx = rx.saturating_sub(3);
    }
    right_rects.reverse();
    // The search field: a third of the width, between 24 and 52 columns.
    let field_w = (area.width / 3)
        .clamp(24, 52)
        .min(area.width.saturating_sub(24));
    let field_x = rx.saturating_sub(field_w);
    let search = one(field_x, field_w);
    let mut x = logo.right() + 3;
    let mut crumb_rects = Vec::new();
    for (i, c) in crumbs.iter().enumerate() {
        if i > 0 {
            x += SEP.len() as u16;
        }
        let w = (text::width(&c.text) as u16).min(field_x.saturating_sub(x + 2));
        crumb_rects.push(one(x, w));
        x += w;
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
                buf.set_string(r.x - SEP.len() as u16, r.y, SEP, theme.meta(BAR));
            }
            let style = if c.current {
                theme.title(BAR)
            } else {
                theme.body(BAR)
            };
            Span::styled(text::truncate(&c.text, usize::from(r.width)), style).render(*r, buf);
        }
        fill(buf, lay.search, theme, FIELD);
        let inner = Rect {
            x: lay.search.x + 1,
            width: lay.search.width.saturating_sub(2),
            ..lay.search
        };
        match self.input {
            Some(input) => {
                Span::styled("⌕ ", theme.accent(FIELD)).render(inner, buf);
                input.render(
                    Rect {
                        x: inner.x + 2,
                        width: inner.width.saturating_sub(2),
                        ..inner
                    },
                    buf,
                );
            }
            None => {
                Span::styled(
                    text::truncate(&format!("⌕ {}", self.placeholder), usize::from(inner.width)),
                    theme.meta(FIELD),
                )
                .render(inner, buf);
            }
        }
        for (item, r) in self.right.iter().zip(&lay.right) {
            Span::styled(item.clone(), theme.meta(BAR)).render(*r, buf);
        }
    }
}

// ---- tabs ------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    pub icon: &'static str,
    pub label: String,
    pub count: Option<u64>,
    /// Opens in the browser.
    pub external: bool,
}

/// Each tab's rectangle on the label row.
pub fn tab_layout(area: Rect, tabs: &[Tab]) -> Vec<Rect> {
    let mut x = area.x + PAD_X.min(area.width);
    let mut out = Vec::new();
    for tab in tabs {
        let w = tab_width(tab);
        let w = w.min(area.right().saturating_sub(x));
        out.push(Rect::new(x, area.y, w, 1));
        x += w + 1;
    }
    out
}

fn tab_width(tab: &Tab) -> u16 {
    let count = tab
        .count
        .map_or(0, |n| text::width(&crate::pages::compact(n)) + 3);
    let external = if tab.external { 2 } else { 0 };
    (2 + text::width(tab.icon) + 1 + text::width(&tab.label) + count + external) as u16
}

/// Tabs on one row, the active one underlined on the row below (2 rows).
pub struct TabBar<'a> {
    pub ctx: Ctx<'a>,
    pub tabs: &'a [Tab],
    pub active: Option<usize>,
}

impl Widget for TabBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = self.ctx.theme;
        let labels = Rect { height: 1, ..area };
        fill(buf, labels, theme, BAR);
        let under = Rect {
            y: area.y + 1,
            height: 1,
            ..area
        };
        if area.height > 1 {
            fill(buf, under, theme, BAR);
            for x in under.left()..under.right() {
                buf.set_string(x, under.y, "─", theme.separator(BAR));
            }
        }
        for (i, (tab, r)) in self
            .tabs
            .iter()
            .zip(tab_layout(labels, self.tabs))
            .enumerate()
        {
            let active = self.active == Some(i);
            let label_style = if active {
                theme.title(BAR)
            } else {
                theme.body(BAR)
            };
            let mut spans = vec![
                Span::styled(format!(" {} ", tab.icon), theme.meta(BAR)),
                Span::styled(tab.label.clone(), label_style),
            ];
            if let Some(n) = tab.count {
                spans.push(Span::styled(" ", theme.body(BAR)));
                spans.push(Span::styled(
                    format!(" {} ", crate::pages::compact(n)),
                    theme.fill(Bg::ContainerHighest),
                ));
            }
            if tab.external {
                spans.push(Span::styled(
                    format!(" {}", self.ctx.icons.external()),
                    theme.meta(BAR),
                ));
            }
            spans.push(Span::styled(" ", theme.body(BAR)));
            Line::from(spans).render(r, buf);
            if active && area.height > 1 {
                for x in r.left()..r.right() {
                    buf.set_string(x, under.y, "━", theme.accent(BAR));
                }
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
        self.line.render(
            Rect {
                x: area.x + PAD_X,
                width: area.width.saturating_sub(2 * PAD_X),
                ..area
            },
            buf,
        );
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

/// The dropdown under the header's search field.
pub struct SearchPanel<'a> {
    pub ctx: Ctx<'a>,
    pub rows: &'a [SuggestRow],
    /// Index among the item rows.
    pub selected: usize,
    pub field: Rect,
    /// A dim line at the bottom ("enter go · tab next · esc close").
    pub help: &'a str,
}

impl SearchPanel<'_> {
    pub fn area(&self, screen: Rect) -> Rect {
        let width = self.field.width.max(64).min(screen.width);
        let x = self
            .field
            .x
            .min(screen.right().saturating_sub(width))
            .max(screen.x);
        let height = (self.rows.len() as u16 + 3).min(screen.height.saturating_sub(2));
        Rect::new(x, self.field.y + 1, width, height)
    }
}

impl Widget for SearchPanel<'_> {
    fn render(self, screen: Rect, buf: &mut Buffer) {
        let theme = self.ctx.theme;
        let area = self.area(screen);
        fill(buf, area, theme, PANEL);
        let inner_w = area.width.saturating_sub(4);
        let rows = usize::from(area.height.saturating_sub(3));
        // Keep the selection in view.
        let selected_row = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| matches!(r, SuggestRow::Item { .. }))
            .nth(self.selected)
            .map_or(0, |(i, _)| i);
        let skip = (selected_row + 1).saturating_sub(rows);
        for (n, row) in self.rows.iter().enumerate().skip(skip).take(rows) {
            let y = area.y + 1 + (n - skip) as u16;
            let line = Rect::new(area.x + 2, y, inner_w, 1);
            match row {
                SuggestRow::Heading(h) => {
                    Span::styled(h.clone(), theme.meta(PANEL).add_modifier(Modifier::BOLD))
                        .render(line, buf);
                }
                SuggestRow::Item {
                    icon,
                    label,
                    detail,
                    hint,
                } => {
                    let sel = n == selected_row;
                    let bg = if sel { Bg::SelectedHigh } else { PANEL };
                    if sel {
                        fill(buf, Rect::new(area.x, y, area.width, 1), theme, bg);
                        buf.set_string(area.x, y, "▌", theme.accent(bg));
                    }
                    let hint_w = text::width(hint) as u16;
                    let room = usize::from(inner_w.saturating_sub(hint_w + 2));
                    let label = text::truncate(&format!("{icon}  {label}"), room);
                    let detail_room = room.saturating_sub(text::width(&label));
                    Line::from(vec![
                        Span::styled(label, theme.body(bg)),
                        Span::styled(
                            text::truncate(&format!("  {detail}"), detail_room),
                            theme.meta(bg),
                        ),
                    ])
                    .render(line, buf);
                    if hint_w > 0 {
                        Span::styled(hint.clone(), theme.meta(bg)).render(
                            Rect::new(line.right().saturating_sub(hint_w), y, hint_w, 1),
                            buf,
                        );
                    }
                }
            }
        }
        let help_y = area.bottom().saturating_sub(1);
        Span::styled(
            text::truncate(self.help, usize::from(inner_w)),
            theme.meta(PANEL),
        )
        .render(Rect::new(area.x + 2, help_y, inner_w, 1), buf);
    }
}

// ---- key panels -------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRow {
    pub key: String,
    pub label: String,
    /// Why it can't run now, if it can't (shown dim).
    pub unavailable: Option<String>,
}

/// A panel of keys at the bottom right, above the status bar: which keys
/// can follow a prefix, or what can be done here (with a selection).
pub struct KeyPanel<'a> {
    pub ctx: Ctx<'a>,
    pub title: &'a str,
    pub rows: &'a [KeyRow],
    pub selected: Option<usize>,
    pub help: &'a str,
}

impl KeyPanel<'_> {
    pub fn area(&self, screen: Rect) -> Rect {
        let key_w = self
            .rows
            .iter()
            .map(|r| text::width(&r.key))
            .max()
            .unwrap_or(0);
        let label_w = self
            .rows
            .iter()
            .map(|r| text::width(&r.label))
            .max()
            .unwrap_or(0);
        let width = ((key_w + 3 + label_w + 4)
            .max(text::width(self.help) + 4)
            .max(text::width(self.title) + 4) as u16)
            .min(screen.width);
        let height = (self.rows.len() as u16 + 4).min(screen.height.saturating_sub(1));
        Rect::new(
            screen.right().saturating_sub(width + 1),
            screen.bottom().saturating_sub(height + 1),
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
        let key_w = self
            .rows
            .iter()
            .map(|r| text::width(&r.key))
            .max()
            .unwrap_or(0);
        let inner_w = area.width.saturating_sub(4);
        Span::styled(self.title, theme.title(PANEL))
            .render(Rect::new(area.x + 2, area.y, inner_w, 1), buf);
        let rows = usize::from(area.height.saturating_sub(4));
        let skip = self.selected.map_or(0, |s| (s + 1).saturating_sub(rows));
        for (n, row) in self.rows.iter().enumerate().skip(skip).take(rows) {
            let y = area.y + 2 + (n - skip) as u16;
            let sel = self.selected == Some(n);
            let bg = if sel { Bg::SelectedHigh } else { PANEL };
            if sel {
                fill(buf, Rect::new(area.x, y, area.width, 1), theme, bg);
                buf.set_string(area.x, y, "▌", theme.accent(bg));
            }
            let (key_style, label_style) = if row.unavailable.is_some() {
                (theme.meta(bg), theme.meta(bg))
            } else {
                (
                    theme.accent(bg).add_modifier(Modifier::BOLD),
                    theme.body(bg),
                )
            };
            let pad = key_w.saturating_sub(text::width(&row.key));
            Line::from(vec![
                Span::styled(format!("{}{}", " ".repeat(pad), row.key), key_style),
                Span::styled("   ", theme.body(bg)),
                Span::styled(row.label.clone(), label_style),
            ])
            .render(Rect::new(area.x + 2, y, inner_w, 1), buf);
        }
        Span::styled(
            text::truncate(self.help, usize::from(inner_w)),
            theme.meta(PANEL),
        )
        .render(
            Rect::new(area.x + 2, area.bottom().saturating_sub(1), inner_w, 1),
            buf,
        );
    }
}
