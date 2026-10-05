//! Markdown from GitHub (READMEs, comments) renders without panicking.
#![no_main]

use ghtui_ui::markdown::{LinkBase, render};
use ghtui_ui::page::{Frame, Page};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: (u8, &str)| {
    let (width, text) = input;
    let width = u16::from(width % 120) + 1;
    let base = LinkBase::new("o/r", "main", "docs/README.md");
    let mut page = Page::new(width);
    render(&mut page, text, Some(&base), Frame::None);
});
