//! Any pair of file versions diffs, highlights, lays out into a document
//! and renders without panicking.
#![no_main]

use std::collections::HashSet;
use std::sync::Arc;

use ghtui_diff::FileDiff;
use ghtui_git::files::{ChangedFile, FileStatus};
use ghtui_theme::{ColorDepth, DEFAULT_SEED, Mode, Theme};
use ghtui_ui::diff_doc::{Doc, Pos, ViewOptions};
use ghtui_ui::diff_view::{DiffView, Keys};
use ghtui_ui::{Ctx, Icons};
use libfuzzer_sys::fuzz_target;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;

fuzz_target!(|input: (u8, u8, &[u8], &[u8])| {
    let (flags, size, old, new) = input;
    let path = if flags & 1 == 0 { "src/lib.rs" } else { "notes.txt" };
    let diff = FileDiff::compute(path, Some(old), Some(new));
    let file = ChangedFile {
        status: FileStatus::Modified,
        old_path: Some(path.into()),
        new_path: Some(path.into()),
        old_mode: 0o100644,
        new_mode: 0o100644,
        old_oid: ghtui_git::Oid::new("1".repeat(40)),
        new_oid: ghtui_git::Oid::new("2".repeat(40)),
        similarity: None,
    };
    let mut doc = Doc::new(vec![file], &HashSet::new());
    doc.set_diff(0, Arc::new(diff));
    doc.set_options(ViewOptions {
        split: flags & 2 != 0,
        ..ViewOptions::default()
    });
    if flags & 4 != 0 {
        doc.toggle_full(0);
    }
    let cursor = doc.clamp(Pos {
        file: 0,
        row: usize::from(flags >> 3),
    });
    doc.expand(cursor);
    let theme = Theme::new(DEFAULT_SEED, Mode::Dark, ColorDepth::TrueColor);
    let area = Rect::new(0, 0, u16::from(size) + 1, u16::from(size % 40) + 1);
    let mut buf = Buffer::empty(area);
    DiffView {
        ctx: Ctx {
            theme: &theme,
            icons: Icons::default(),
            now: 0,
        },
        doc: &doc,
        cursor,
        top: cursor,
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
        selection: Some((Pos::default(), cursor)),
    }
    .render(area, &mut buf);
});
