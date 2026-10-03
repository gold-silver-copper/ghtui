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

impl Help<'_> {
    pub fn area(&self, screen: Rect) -> Rect {
        let keys_width = self
            .entries
            .iter()
            .map(|e| text::width(&e.keys))
            .max()
            .unwrap_or(0);
        let desc_width = self
            .entries
            .iter()
            .map(|e| text::width(e.description))
            .max()
            .unwrap_or(0);
        let width = (keys_width + 3 + desc_width) as u16 + 2 * PAD_X;
        let height = self.entries.len() as u16 + 2 + 2 * PAD_Y;
        let height = height.min(screen.height.saturating_sub(2));
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
        let keys_width = self
            .entries
            .iter()
            .map(|e| text::width(&e.keys))
            .max()
            .unwrap_or(0);
        let mut lines = vec![
            Line::from(Span::styled("Keyboard shortcuts", theme.title(POPUP))),
            Line::default(),
        ];
        for entry in self.entries {
            let pad = keys_width.saturating_sub(text::width(&entry.keys));
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{}{}", entry.keys, " ".repeat(pad)),
                    theme.accent(POPUP).add_modifier(Modifier::BOLD),
                ),
                Span::styled("   ", theme.body(POPUP)),
                Span::styled(entry.description, theme.body(POPUP)),
            ]));
        }
        for (i, line) in lines
            .into_iter()
            .take(usize::from(inner.height))
            .enumerate()
        {
            line.render(
                Rect {
                    y: inner.y + i as u16,
                    height: 1,
                    ..inner
                },
                buf,
            );
        }
    }
}

/// A palette entry.
pub struct PaletteItem {
    pub label: String,
    /// Right-aligned hint, e.g. the bound keys.
    pub hint: String,
}

pub struct Palette<'a> {
    pub ctx: Ctx<'a>,
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
        Span::styled(":", theme.accent(POPUP).add_modifier(Modifier::BOLD)).render(prompt, buf);
        self.input.render(
            Rect {
                x: prompt.x + 2,
                width: prompt.width.saturating_sub(2),
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
