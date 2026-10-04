//! Ratatui widgets. Every widget is a pure function of the props it's given
//! (the app maps its state to props); all colors come from theme roles.

pub mod annotations;
pub mod bars;
pub mod chips;
pub mod chrome;
pub mod diff_doc;
pub mod diff_view;
pub mod file_tree;
pub mod icons;
pub mod markdown;
pub mod overlays;
pub mod page;
pub mod pages;
pub mod review_sheets;
pub mod text;
pub mod time;

use ghtui_theme::{Bg, Theme};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Widget};

pub use icons::Icons;

/// Base spacing unit: 2 cells horizontally, 1 vertically.
pub const PAD_X: u16 = 2;
pub const PAD_Y: u16 = 1;

/// What every widget needs besides its own props.
#[derive(Clone, Copy)]
pub struct Ctx<'a> {
    pub theme: &'a Theme,
    pub icons: Icons,
    /// Unix seconds, for relative times. Passed in so rendering is pure.
    pub now: u64,
}

/// Paints `area` with a surface tone, clearing whatever was drawn there
/// (modifiers included).
pub fn fill(buf: &mut Buffer, area: Rect, theme: &Theme, bg: Bg) {
    Clear.render(area, buf);
    buf.set_style(area, theme.fill(bg));
}

/// Renders `left` and `right` on one line, at least `gap` columns apart,
/// truncating the left side first when they don't fit.
fn render_split(area: Rect, buf: &mut Buffer, left: Vec<Span<'_>>, right: Vec<Span<'_>>, gap: u16) {
    let right_width = (right.iter().map(Span::width).sum::<usize>() as u16).min(area.width);
    let left_area = Rect {
        width: area.width.saturating_sub(right_width + gap),
        ..area
    };
    let right_area = Rect {
        x: area.right() - right_width,
        width: right_width,
        ..area
    };
    Line::from(left).render(left_area, buf);
    Line::from(right).render(right_area, buf);
}

/// `key what · key what`: keys in bold accent, what they do in meta.
fn key_hints(theme: &Theme, bg: Bg, hints: &[(&str, &str)]) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (i, (key, what)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", theme.meta(bg)));
        }
        spans.push(Span::styled(
            key.to_string(),
            theme.accent(bg).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(format!(" {what}"), theme.meta(bg)));
    }
    spans
}

/// `area` shrunk by the standard padding.
pub fn padded(area: Rect) -> Rect {
    inset(area, PAD_X, PAD_Y)
}

pub fn inset(area: Rect, x: u16, y: u16) -> Rect {
    let x = x.min(area.width / 2);
    let y = y.min(area.height / 2);
    Rect {
        x: area.x + x,
        y: area.y + y,
        width: area.width - 2 * x,
        height: area.height - 2 * y,
    }
}

/// A rectangle of at most `width`×`height` centered horizontally in `area`,
/// `top` rows below its top edge.
pub fn centered(area: Rect, width: u16, height: u16, top: u16) -> Rect {
    let width = width.min(area.width);
    let top = top.min(area.height.saturating_sub(1));
    let height = height.min(area.height - top);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + top,
        width,
        height,
    }
}
