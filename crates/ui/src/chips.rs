//! Tonal chips: short labels on filled container backgrounds.

use ghtui_theme::Bg;
use ratatui::style::Style;
use ratatui::text::Span;

use crate::Ctx;

/// A chip's spans: text padded by one cell, with rounded caps when the
/// Nerd Font is enabled. `parent` is the background the chip sits on.
pub fn chip<'a>(ctx: Ctx<'_>, text: impl Into<String>, style: Style, parent: Bg) -> Vec<Span<'a>> {
    let text = text.into();
    match (ctx.icons.chip_caps(), style.bg) {
        (Some((left, right)), Some(chip_bg)) => {
            let cap = Style::new().fg(chip_bg).bg(ctx.theme.bg_color(parent));
            vec![
                Span::styled(left, cap),
                Span::styled(text, style),
                Span::styled(right, cap),
            ]
        }
        _ => vec![Span::styled(format!(" {text} "), style)],
    }
}
