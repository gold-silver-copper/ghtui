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

/// Paints `area` with a surface tone.
pub fn fill(buf: &mut Buffer, area: Rect, theme: &Theme, bg: Bg) {
    buf.set_style(area, theme.fill(bg));
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ");
            }
        }
    }
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
