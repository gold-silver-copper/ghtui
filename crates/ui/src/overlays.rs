//! Popups on a raised surface tone: the command palette and pickers.

use ghtui_theme::Bg;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::Span;
use ratatui::widgets::Widget;
use ratatui_textarea::TextArea;

use crate::{Ctx, PAD_Y, centered, fill, label_hint, list_row, padded, text};

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
        // Taller on tall screens (but never past the bottom), wide enough
        // for the longest row.
        let max_rows = PALETTE_ROWS.max(usize::from(screen.height / 2));
        let fits = usize::from(screen.height.saturating_sub(4 + 2 * PAD_Y)).max(1);
        let rows = self.items.len().clamp(1, max_rows).min(fits);
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
            Span::styled("No matches", theme.meta(POPUP)).render(
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
            let (row, bg) = list_row(buf, theme, area, y, i == self.selected, POPUP);
            let label = Span::styled(item.label.as_str(), theme.body(bg));
            label_hint(
                buf,
                row,
                &[label],
                Span::styled(item.hint.as_str(), theme.meta(bg)),
            );
        }
    }
}
