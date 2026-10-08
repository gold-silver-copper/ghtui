//! Text is measured the way it is drawn: a tab, a control character or a
//! halfwidth sound mark takes the same columns when wrapping or cutting a
//! line as when the line reaches the screen.

use ghtui_theme::{ColorDepth, DEFAULT_SEED, Mode, Theme};
use ghtui_ui::markdown::render;
use ghtui_ui::page::{Frame, Page, PageView};
use ghtui_ui::{Ctx, Icons};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;

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

fn theme() -> Theme {
    Theme::new(DEFAULT_SEED, Mode::Dark, ColorDepth::TrueColor)
}

/// A paragraph with a tab in it wraps to show every word, instead of
/// being measured with the tab as nothing and cut short when drawn.
#[test]
fn a_paragraph_with_tabs_wraps_instead_of_losing_its_end() {
    let mut page = Page::new(20);
    render(&mut page, "aaaa\tbbbb cccc dddd eeee", None, Frame::None);
    let theme = theme();
    let area = Rect::new(0, 0, 20, 4);
    let mut buf = Buffer::empty(area);
    PageView {
        ctx: Ctx {
            theme: &theme,
            icons: Icons::default(),
            now: 0,
        },
        page: &page,
        scroll: 0,
        selected: None,
        hints: &[],
    }
    .render(area, &mut buf);
    let shown = rows(&buf).join("\n");
    for word in ["aaaa", "bbbb", "cccc", "dddd", "eeee"] {
        assert!(shown.contains(word), "{word} is shown:\n{shown}");
    }
    assert!(!shown.contains('…'), "nothing is cut:\n{shown}");
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
