//! Tonal chips: short labels on filled container backgrounds.

use crate::theme::Bg;
use ratatui::style::Style;
use ratatui::text::Span;

use crate::ui::Ctx;

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

/// Adds a chip filled with `fill` to `out`, and the space after it.
pub fn push(out: &mut Vec<Span<'_>>, ctx: Ctx<'_>, text: impl Into<String>, fill: Bg, parent: Bg) {
    let theme = ctx.theme;
    out.extend(chip(ctx, text, theme.fill(fill), parent));
    out.push(Span::styled(" ", theme.body(parent)));
}

/// A reaction's chip: the ones you gave stand out, as on the website.
pub fn reaction_fill(mine: bool) -> Bg {
    if mine {
        Bg::PrimaryContainer
    } else {
        Bg::ContainerHighest
    }
}
