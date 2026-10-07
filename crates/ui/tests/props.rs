//! Properties over arbitrary input from GitHub: Markdown and diffs lay out
//! and render without panicking, and stay inside their bounds, whatever
//! the text: wide (CJK), emoji, ZWJ sequences, flags, stacked combining
//! marks.

use std::collections::HashSet;
use std::sync::Arc;

use ghtui_diff::FileDiff;
use ghtui_git::files::{ChangedFile, FileStatus};
use ghtui_theme::{ColorDepth, DEFAULT_SEED, Mode, Theme};
use ghtui_ui::diff_doc::{Doc, Pos, ViewOptions};
use ghtui_ui::diff_view::{DiffView, Keys};
use ghtui_ui::markdown::{LinkBase, render};
use ghtui_ui::page::{Frame, Page, Tone};
use ghtui_ui::{Ctx, Icons};
use proptest::prelude::*;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;

/// Lines of code-ish text with the awkward parts: tabs, CRLF, control
/// characters, wide and combining characters, bidi overrides.
fn source() -> impl Strategy<Value = String> {
    let line = prop::collection::vec(
        prop_oneof![
            8 => "[a-z{}();=\" ]",
            1 => Just("\t".to_owned()),
            1 => Just("\r".to_owned()),
            1 => Just("\u{1b}[31m".to_owned()),
            1 => Just("漢字".to_owned()),
            1 => Just("e\u{301}".to_owned()),
            1 => Just("\u{202e}".to_owned()),
            1 => Just("👩‍👩‍👧‍👦".to_owned()),
            1 => Just("🇯🇵".to_owned()),
            1 => Just("a\u{301}\u{302}\u{303}\u{304}".to_owned()),
            1 => Just("Ｆｕｌｌ、".to_owned()),
            1 => Just("\u{200d}".to_owned()),
            1 => any::<char>().prop_map(String::from),
        ],
        0..30,
    )
    .prop_map(|parts| parts.concat());
    prop::collection::vec(line, 0..25).prop_map(|lines| lines.join("\n"))
}

fn modified(path: &str) -> ChangedFile {
    ChangedFile {
        status: FileStatus::Modified,
        old_path: Some(path.into()),
        new_path: Some(path.into()),
        old_mode: 0o100644,
        new_mode: 0o100644,
        old_oid: ghtui_git::Oid::new("1".repeat(40)),
        new_oid: ghtui_git::Oid::new("2".repeat(40)),
        similarity: None,
    }
}

fn render_doc(doc: &Doc, cursor: Pos, width: u16, height: u16) {
    let theme = Theme::new(DEFAULT_SEED, Mode::Dark, ColorDepth::TrueColor);
    let ctx = Ctx {
        theme: &theme,
        icons: Icons::default(),
        now: 0,
    };
    let keys = Keys {
        show: "↵",
        jump: "M",
        expand: "e",
        viewed: "v",
        reply: "c",
        resolve: "R",
        delete: "del",
        file_comment: "C",
    };
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(area);
    DiffView {
        ctx,
        doc,
        cursor,
        top: cursor,
        keys,
        selection: Some((Pos::default(), cursor)),
    }
    .render(area, &mut buf);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// Markdown never panics and no line is wider than the page (code
    /// isn't wrapped; it's clipped where it's drawn).
    #[test]
    fn markdown_fits_the_page(text in prop_oneof![any::<String>(), source()], width in 0u16..120) {
        let base = LinkBase::new("o/r", "main", "docs/README.md");
        let mut page = Page::new(width);
        render(&mut page, &text, Some(&base), Frame::None);
        for line in page.lines.iter().filter(|l| l.tone != Tone::Code) {
            let used = usize::from(line.indent) + ghtui_ui::text::width(&line.text());
            prop_assert!(used <= usize::from(page.width), "{used} > {}: {:?}", page.width, line.text());
        }
    }

    /// Cutting and wrapping text keep to the width asked for, in cells,
    /// and never split a grapheme (an emoji family, a flag, a letter and
    /// its marks).
    #[test]
    fn text_fits_its_width(text in prop_oneof![any::<String>(), source()], max in 1usize..40) {
        use ghtui_ui::text::{graphemes, truncate, width, wrap};
        let cut = truncate(&text, max);
        prop_assert!(width(&cut) <= max, "{} > {max}: {cut:?}", width(&cut));
        for line in wrap(&text, max) {
            // A grapheme wider than the line (a wide one in one cell) is
            // the one thing that may not fit.
            let widest = graphemes(&line).map(|(_, w)| w).max().unwrap_or(0);
            prop_assert!(width(&line) <= max.max(widest), "{} > {max}: {line:?}", width(&line));
            let whole: Vec<&str> = graphemes(&line).map(|(g, _)| g).collect();
            prop_assert_eq!(whole.concat(), line.clone());
        }
    }

    /// Any two versions of a file lay out into a document whose positions
    /// round-trip, that searches, and that renders at any size.
    #[test]
    fn diffs_lay_out_and_render(
        old in source(),
        new in source(),
        query in "[a-z]{1,3}",
        split in any::<bool>(),
        width in 1u16..160,
        height in 1u16..30,
        row in 0usize..200,
    ) {
        let diff = FileDiff::compute("src/lib.rs", Some(old.as_bytes()), Some(new.as_bytes()));
        let mut doc = Doc::new(vec![modified("src/lib.rs")], &HashSet::new(), Default::default());
        doc.set_diff(0, Arc::new(diff));
        doc.set_options(ViewOptions { split, ..ViewOptions::default() });
        doc.toggle_full(0);
        doc.toggle_full(0);
        let pos = doc.clamp(Pos { file: 0, row });
        prop_assert_eq!(doc.to_pos(doc.to_global(pos)), pos);
        // A row showing a line is found again at a row showing that line.
        if let Some(line) = doc.line_at(pos) {
            prop_assert_eq!(doc.line_at(doc.locate(doc.anchor(pos))), Some(line));
        }
        let _ = doc.search(&query, pos, true);
        let _ = doc.search(&query, pos, false);
        doc.expand(pos);
        render_doc(&doc, doc.clamp(pos), width, height);
    }
}
