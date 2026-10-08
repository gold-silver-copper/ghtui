//! Text is measured the way it is drawn: a tab, a control character or a
//! halfwidth sound mark takes the same columns when wrapping or cutting a
//! line as when the line reaches the screen.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

fn rows(buf: &Buffer) -> Vec<String> {
    let area = buf.area;
    (area.top()..area.bottom())
        .map(|y| {
            (area.left()..area.right())
                .map(|x| buf[(x, y)].symbol())
                .collect()
        })
        .collect()
}

/// Halfwidth katakana with sound marks (ｶﾞ: each mark takes a cell of its
/// own on screen) is cut with its `…` showing, not clipped off the edge.
#[test]
fn halfwidth_katakana_is_cut_where_it_is_drawn() {
    let title = "ｶﾞｷﾞｸﾞｹﾞｺﾞｻﾞｼﾞｽﾞ";
    let cut = ghtui_ui::text::truncate(title, 8);
    let area = Rect::new(0, 0, 8, 1);
    let mut buf = Buffer::empty(area);
    ghtui_ui::put(&mut buf, 0, 0, &cut, ratatui::style::Style::default());
    let shown = rows(&buf).concat();
    assert!(
        shown.trim_end().ends_with('…'),
        "{cut:?} measures {} but draws as {shown:?}",
        ghtui_ui::text::width(&cut)
    );
}
