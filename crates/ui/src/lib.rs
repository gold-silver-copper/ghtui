//! Ratatui widgets. Every widget is a pure function of the props it's given
//! (the app maps its state to props); all colors come from theme roles.

pub mod annotations;
pub mod bars;
pub mod chips;
pub mod chrome;
pub mod diff_doc;
pub mod diff_view;
pub mod fetched;
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
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Widget};

pub use fetched::Fetched;
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

/// `n` columns (or rows) as a `u16`, saturating: for `text::width` and
/// friends.
pub fn cols(n: usize) -> u16 {
    u16::try_from(n).unwrap_or(u16::MAX)
}

/// An index or count as a `u32`, saturating.
pub fn idx(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// `text` at `(x, y)`, clipped to the buffer: nothing when the position is
/// outside it, and cut at its right edge.
pub fn put(buf: &mut Buffer, x: u16, y: u16, text: &str, style: Style) {
    if !buf.area.contains(Position { x, y }) {
        return;
    }
    let room = usize::from(buf.area.right().saturating_sub(x));
    #[expect(
        clippy::disallowed_methods,
        reason = "the one sanctioned unclipped write: the checks above clip"
    )]
    buf.set_stringn(x, y, text, room, style);
}

/// Paints `area` with a surface tone, clearing whatever was drawn there
/// (modifiers included).
pub fn fill(buf: &mut Buffer, area: Rect, theme: &Theme, bg: Bg) {
    Clear.render(area, buf);
    buf.set_style(area, theme.fill(bg));
}

/// Renders `left` and `right` on one line, at least `gap` columns apart,
/// truncating the left side first when they don't fit.
#[deny(clippy::arithmetic_side_effects)]
fn render_split(area: Rect, buf: &mut Buffer, left: Vec<Span<'_>>, right: Vec<Span<'_>>, gap: u16) {
    let right_width = cols(right.iter().map(Span::width).sum()).min(area.width);
    let left_area = Rect {
        width: area.width.saturating_sub(right_width.saturating_add(gap)),
        ..area
    };
    let right_area = Rect {
        x: area.right().saturating_sub(right_width),
        width: right_width,
        ..area
    };
    Line::from(left).render(left_area, buf);
    Line::from(right).render(right_area, buf);
}

/// A popup list's row `y`: a band with a stripe when selected, and the
/// padded content area with its tone. Empty when `y` is outside `panel`.
fn list_row(
    buf: &mut Buffer,
    theme: &Theme,
    panel: Rect,
    y: u16,
    selected: bool,
    base: Bg,
) -> (Rect, Bg) {
    let bg = if selected { Bg::SelectedHigh } else { base };
    let row = Rect::new(panel.x, y, panel.width, 1).intersection(panel);
    if selected && !row.is_empty() {
        fill(buf, row, theme, bg);
        Span::styled("▌", theme.accent(bg)).render(row, buf);
    }
    (inset(row, PAD_X, 0), bg)
}

/// `left` cut to fit (with `…`) on the left of `row`, `hint` on its right.
#[deny(clippy::arithmetic_side_effects)]
fn label_hint(buf: &mut Buffer, row: Rect, left: &[Span<'_>], hint: Span<'_>) {
    let hint_w = cols(hint.width()).min(row.width);
    let gap = if hint_w > 0 {
        hint_w.saturating_add(2)
    } else {
        0
    };
    let mut room = usize::from(row.width.saturating_sub(gap));
    let left: Vec<Span<'_>> = left
        .iter()
        .map(|s| {
            let shown = text::truncate(&s.content, room);
            room = room.saturating_sub(text::width(&shown));
            Span::styled(shown, s.style)
        })
        .collect();
    Line::from(left).render(row, buf);
    let hint_area = Rect {
        x: row.right().saturating_sub(hint_w),
        width: hint_w,
        ..row
    };
    hint.render(hint_area, buf);
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

#[deny(clippy::arithmetic_side_effects)]
pub fn inset(area: Rect, x: u16, y: u16) -> Rect {
    let x = x.min(area.width / 2);
    let y = y.min(area.height / 2);
    Rect {
        x: area.x.saturating_add(x),
        y: area.y.saturating_add(y),
        width: area.width.saturating_sub(x).saturating_sub(x),
        height: area.height.saturating_sub(y).saturating_sub(y),
    }
}

/// A rectangle of at most `width`×`height` centered horizontally in `area`,
/// `top` rows below its top edge.
#[deny(clippy::arithmetic_side_effects)]
pub fn centered(area: Rect, width: u16, height: u16, top: u16) -> Rect {
    let width = width.min(area.width);
    let top = top.min(area.height.saturating_sub(1));
    let height = height.min(area.height.saturating_sub(top));
    Rect {
        x: area.x.saturating_add(area.width.saturating_sub(width) / 2),
        y: area.y.saturating_add(top),
        width,
        height,
    }
}
