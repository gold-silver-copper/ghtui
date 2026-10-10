//! Text is measured the way it is drawn: a tab, a control character or a
//! halfwidth sound mark takes the same columns when wrapping or cutting a
//! line as when the line reaches the screen.

use std::collections::HashSet;
use std::sync::Arc;

use ghtui_diff::FileDiff;
use ghtui_diff::anchor::Side;
use ghtui_git::files::{ChangedFile, FileStatus};
use ghtui_theme::{ColorDepth, DEFAULT_SEED, Mode, Theme};
use ghtui_ui::annotations::{Annotation, AnnotationComment, AnnotationKey, Place};
use ghtui_ui::diff_doc::{Doc, DocInputs, Pos};
use ghtui_ui::diff_view::{DiffView, Keys};
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

/// A paragraph with a tab in it wraps to show every word, the tab as the
/// spaces to the next stop, instead of being measured as nothing and cut
/// short or dropped when drawn.
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
        current: None,
        hints: &[],
    }
    .render(area, &mut buf);
    let shown = rows(&buf).join("\n");
    for word in ["aaaa    bbbb", "cccc", "dddd", "eeee"] {
        assert!(shown.contains(word), "{word} is shown:\n{shown}");
    }
    assert!(!shown.contains('…'), "nothing is cut:\n{shown}");
}

/// Tab-indented code in a review comment keeps its indentation.
#[test]
fn a_review_comment_keeps_its_tabs() {
    let path = "main.go";
    let file = ChangedFile {
        status: FileStatus::Modified,
        old_path: Some(path.into()),
        new_path: Some(path.into()),
        old_mode: 0o100644,
        new_mode: 0o100644,
        old_oid: ghtui_git::Oid::parse(&"1".repeat(40)).unwrap(),
        new_oid: ghtui_git::Oid::parse(&"2".repeat(40)).unwrap(),
        similarity: None,
    };
    let mut doc = Doc::new(vec![file], &HashSet::new(), DocInputs::default());
    let diff = FileDiff::compute(path, Some(b"a\n"), Some(b"b\n"));
    doc.set_diff(0, Arc::new(diff));
    doc.set_inputs(DocInputs {
        annotations: vec![Annotation {
            key: AnnotationKey::Draft(1),
            path: path.into(),
            place: Place::File { outdated: false },
            resolved: false,
            comments: vec![AnnotationComment {
                author: "alice".into(),
                body: "Try:\n\tx := 1".into(),
                created_at: String::new(),
                pending: false,
            }],
            left_out: 0,
            error: None,
            can_reply: true,
            can_resolve: false,
            can_unresolve: false,
        }],
        ..DocInputs::default()
    });
    let theme = theme();
    let area = Rect::new(0, 0, 60, 12);
    let mut buf = Buffer::empty(area);
    DiffView {
        ctx: Ctx {
            theme: &theme,
            icons: Icons::default(),
            now: 0,
        },
        doc: &doc,
        cursor: Pos::default(),
        half: Side::Right,
        top: Pos::default(),
        keys: Keys {
            show: "↵",
            jump: "M",
            expand: "e",
            viewed: "v",
            reply: "c",
            resolve: "R",
            delete: "del",
            file_comment: "C",
        },
        selection: None,
    }
    .render(area, &mut buf);
    let rows = rows(&buf);
    let code = rows
        .iter()
        .find(|r| r.contains("x := 1"))
        .unwrap_or_else(|| panic!("the code line is shown:\n{}", rows.join("\n")));
    let try_row = rows
        .iter()
        .find(|r| r.contains("Try:"))
        .unwrap_or_else(|| panic!("the first line is shown:\n{}", rows.join("\n")));
    assert!(
        code.find('x') > try_row.find('T'),
        "the tab indents the code past the comment's margin:\n{try_row}\n{code}"
    );
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
