//! Popups on a raised surface tone: help and the command palette.

use ghtui_theme::Bg;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use ratatui_textarea::TextArea;

use crate::{Ctx, PAD_X, PAD_Y, centered, fill, padded, text};

const POPUP: Bg = Bg::ContainerHigh;

/// One help row: the keys bound to an action and what it does.
pub struct HelpEntry {
    pub keys: String,
    pub description: &'static str,
}

pub struct Help<'a> {
    pub ctx: Ctx<'a>,
    pub entries: &'a [HelpEntry],
}

/// Gap between help columns.
const HELP_GUTTER: u16 = 4;

impl Help<'_> {
    fn key_width(&self) -> usize {
        self.entries
            .iter()
            .map(|e| text::width(&e.keys))
            .max()
            .unwrap_or(0)
    }

    /// Width of the column holding `entries`.
    fn column_width(&self, entries: &[HelpEntry]) -> u16 {
        let desc = entries
            .iter()
            .map(|e| text::width(e.description))
            .max()
            .unwrap_or(0);
        (self.key_width() + 3 + desc) as u16
    }

    fn columns(&self, screen: Rect) -> Vec<&[HelpEntry]> {
        self.entries.chunks(self.rows_per_column(screen)).collect()
    }

    /// Entry rows per column: as many as fit, flowing into more columns
    /// when the screen is short.
    fn rows_per_column(&self, screen: Rect) -> usize {
        let room = usize::from(screen.height.saturating_sub(2 + 2 * PAD_Y + 2)).max(1);
        let columns = self.entries.len().div_ceil(room).max(1);
        self.entries.len().div_ceil(columns).max(1)
    }

    pub fn area(&self, screen: Rect) -> Rect {
        let rows = self.rows_per_column(screen);
        let columns = self.columns(screen);
        let width = columns.iter().map(|c| self.column_width(c)).sum::<u16>()
            + (columns.len().max(1) as u16 - 1) * HELP_GUTTER
            + 2 * PAD_X;
        let height = (rows as u16 + 2 + 2 * PAD_Y).min(screen.height.saturating_sub(2));
        let top = screen.height.saturating_sub(height) / 2;
        centered(screen, width.max(32), height, top)
    }
}

impl Widget for Help<'_> {
    fn render(self, screen: Rect, buf: &mut Buffer) {
        let theme = self.ctx.theme;
        let area = self.area(screen);
        fill(buf, area, theme, POPUP);
        let inner = padded(area);
        if inner.height == 0 {
            return;
        }
        Span::styled("Keyboard shortcuts", theme.title(POPUP))
            .render(Rect { height: 1, ..inner }, buf);
        let key_width = self.key_width();
        let mut x = inner.x;
        for column in self.columns(screen) {
            let column_width = self.column_width(column);
            for (row, entry) in column.iter().enumerate() {
                let y = inner.y + 2 + row as u16;
                if y >= inner.bottom() || x >= inner.right() {
                    continue;
                }
                let pad = key_width.saturating_sub(text::width(&entry.keys));
                Line::from(vec![
                    Span::styled(
                        format!("{}{}", entry.keys, " ".repeat(pad)),
                        theme.accent(POPUP).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("   ", theme.body(POPUP)),
                    Span::styled(entry.description, theme.body(POPUP)),
                ])
                .render(
                    Rect {
                        x,
                        y,
                        width: column_width.min(inner.right() - x),
                        height: 1,
                    },
                    buf,
                );
            }
            x = x.saturating_add(column_width + HELP_GUTTER);
        }
    }
}

/// A palette entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteItem {
    pub label: String,
    /// Right-aligned hint, e.g. the bound keys.
    pub hint: String,
}

pub struct Palette<'a> {
    pub ctx: Ctx<'a>,
    /// Shown before the input, e.g. ":".
    pub prompt: &'a str,
    pub input: &'a TextArea<'static>,
    pub items: &'a [PaletteItem],
    pub selected: usize,
}

pub const PALETTE_ROWS: usize = 10;

impl Widget for Palette<'_> {
    fn render(self, screen: Rect, buf: &mut Buffer) {
        let theme = self.ctx.theme;
        let rows = self.items.len().clamp(1, PALETTE_ROWS) as u16;
        let width = 72.min(screen.width.saturating_sub(4));
        let area = centered(screen, width, 1 + 1 + rows + 2 * PAD_Y, 2);
        fill(buf, area, theme, POPUP);
        let inner = padded(area);

        let prompt = Rect { height: 1, ..inner };
        Span::styled(
            self.prompt.to_owned(),
            theme.accent(POPUP).add_modifier(Modifier::BOLD),
        )
        .render(prompt, buf);
        let offset = (text::width(self.prompt) as u16 + 1).min(prompt.width);
        self.input.render(
            Rect {
                x: prompt.x + offset,
                width: prompt.width - offset,
                ..prompt
            },
            buf,
        );

        let list_top = inner.y + 2;
        if self.items.is_empty() {
            Span::styled("No matching commands", theme.meta(POPUP)).render(
                Rect {
                    y: list_top,
                    height: 1,
                    ..inner
                },
                buf,
            );
            return;
        }
        let first = self.selected.saturating_sub(PALETTE_ROWS - 1);
        for (i, item) in self.items.iter().enumerate().skip(first).take(PALETTE_ROWS) {
            let y = list_top + (i - first) as u16;
            if y >= area.bottom() {
                break;
            }
            let selected = i == self.selected;
            let bg = if selected { Bg::SelectedHigh } else { POPUP };
            let row = Rect {
                x: area.x,
                y,
                width: area.width,
                height: 1,
            };
            if selected {
                fill(buf, row, theme, bg);
            }
            let row = Rect {
                x: inner.x,
                width: inner.width,
                ..row
            };
            let hint_width = text::width(&item.hint) as u16;
            let label_room = usize::from(row.width.saturating_sub(hint_width + 2));
            Span::styled(text::truncate(&item.label, label_room), theme.body(bg)).render(row, buf);
            Span::styled(item.hint.clone(), theme.meta(bg)).render(
                Rect {
                    x: row.right().saturating_sub(hint_width),
                    width: hint_width.min(row.width),
                    ..row
                },
                buf,
            );
        }
    }
}
