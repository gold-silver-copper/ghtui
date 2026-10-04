//! Popups on a raised surface tone: the command palette and pickers.

use ghtui_theme::Bg;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::Span;
use ratatui::widgets::Widget;
use ratatui_textarea::TextArea;

use crate::{Ctx, PAD_Y, centered, fill, padded, text};

const POPUP: Bg = Bg::ContainerHigh;

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
        // Taller on tall screens, wide enough for the longest row.
        let max_rows = PALETTE_ROWS.max(usize::from(screen.height / 2));
        let rows = self.items.len().clamp(1, max_rows);
        let widest = self
            .items
            .iter()
            .map(|i| text::width(&i.label) + text::width(&i.hint) + 6)
            .max()
            .unwrap_or(0);
        let width = (widest.clamp(72, 120) as u16).min(screen.width.saturating_sub(4));
        let area = centered(screen, width, 1 + 1 + rows as u16 + 2 * PAD_Y, 2);
        fill(buf, area, theme, POPUP);
        let inner = padded(area);

        let prompt = Rect { height: 1, ..inner };
        Span::styled(
            self.prompt.to_owned(),
            theme.accent(POPUP).add_modifier(Modifier::BOLD),
        )
        .render(prompt, buf);
        // With more rows than fit, where you are among them.
        let count = if self.items.len() > rows {
            format!("{}/{}", self.selected + 1, self.items.len())
        } else {
            String::new()
        };
        let count_w = (text::width(&count) as u16).min(prompt.width);
        Span::styled(count, theme.meta(POPUP)).render(
            Rect {
                x: prompt.right() - count_w,
                width: count_w,
                ..prompt
            },
            buf,
        );
        let offset = (text::width(self.prompt) as u16 + 1).min(prompt.width);
        self.input.render(
            Rect {
                x: prompt.x + offset,
                width: (prompt.width - offset).saturating_sub(count_w + 1),
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
        let first = (self.selected + 1).saturating_sub(rows);
        for (i, item) in self.items.iter().enumerate().skip(first).take(rows) {
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
                buf.set_string(row.x, y, "▌", theme.accent(bg));
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
