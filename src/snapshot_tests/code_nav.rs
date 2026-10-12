use crate::api::browse::Blob;
use crate::browse::Data;
use crate::code_nav::{Ask, Definition, Found};
use crate::diff::anchor::Side;
use crate::diff_screen::Pane;
use crate::route::Route;
use crate::state::{Api, Cmd, Msg, Overlay, Screen, State, update};
use crate::theme::{ColorDepth, Mode};
use crate::ui::diff_doc::Pos;

use super::diff::screen_state;
use super::{ghtui, open, press, with_repo};

const LIB_RS: &str = "use std::io;\n\nfn main() {\n    helper(io::stdin());\n    other();\n}\n\nfn helper(input: io::Stdin) {}\n\nfn ioctl() {}\n";

/// `src/lib.rs` open at `main`.
fn with_code() -> State {
    let mut s = with_repo(Mode::Dark, ColorDepth::TrueColor);
    let route = Route::blob(ghtui(), "main".into(), "src/lib.rs".into());
    let blob = Blob {
        path: "src/lib.rs".into(),
        text: Some(LIB_RS.into()),
        size: 100,
        truncated: false,
    };
    open(&mut s, route, Data::Blob(Box::new(blob)));
    s
}

/// The selected line of the file, and the word under the cursor.
fn at(s: &State) -> (Option<u32>, Option<String>) {
    let Screen::Page(p) = s.screen() else {
        panic!("not on a page")
    };
    let word = (p.word_cursor()).map(|(_, code, w)| code.get(w.bytes).unwrap().to_owned());
    (p.selected.and_then(|i| p.line_of(i)), word)
}

fn said(s: &State) -> String {
    format!("{:?}", s.notices.shown())
}

/// `gO` lists the file's symbols; choosing one goes to its line.
#[test]
fn symbols_go_to_their_line() {
    let mut s = with_code();
    press(&mut s, "gO");
    let Some(Overlay::Picker(p)) = &s.overlay else {
        panic!("no picker")
    };
    let labels: Vec<String> = (s.picker_rows(p).into_iter())
        .map(|(item, _)| item.label)
        .collect();
    assert_eq!(labels, ["fn main", "fn helper", "fn ioctl"]);
    press(&mut s, "help<Enter>");
    assert_eq!(at(&s).0, Some(8));
}

/// `gd` goes to the definition of the word under the cursor in the file.
#[test]
fn gd_finds_a_definition_in_the_file() {
    let mut s = with_code();
    press(&mut s, "4gg");
    assert_eq!(at(&s), (Some(4), Some("helper".into())));
    press(&mut s, "gd");
    assert_eq!(at(&s).0, Some(8), "{}", said(&s));
    // Not a name: says so.
    press(&mut s, "1ggwww");
    assert_eq!(at(&s).1.as_deref(), Some("::"));
    press(&mut s, "gd");
    assert!(
        said(&s).contains("No name under the cursor"),
        "{}",
        said(&s)
    );
}

/// A name the file doesn't define is asked of GitHub's code search; the
/// one definition it finds opens at its line, at the file's revision.
#[test]
fn gd_asks_github_for_one_outside_the_file() {
    let mut s = with_code();
    press(&mut s, "5gg");
    let ask = Ask {
        repo: ghtui(),
        rev: "main".into(),
        path: "src/lib.rs".into(),
        word: "other".into(),
        language: "Rust",
    };
    assert_eq!(
        ask.query(),
        "other repo:gold-silver-copper/ghtui language:Rust"
    );
    assert_eq!(
        press(&mut s, "gd"),
        [Cmd::Api(Api::FindDefinition(ask.clone()))]
    );
    let found = Found {
        definitions: vec![Definition {
            path: "src/other.rs".into(),
            line: 12,
            name: "fn other".into(),
        }],
        total: 2,
        read: 2,
        incomplete: false,
    };
    update(&mut s, Msg::Definitions(ask, Ok(found)));
    assert_eq!(
        s.route(),
        Some(&Route::Blob {
            repo: ghtui(),
            rev: "main".into(),
            path: "src/other.rs".into(),
            lines: Some((12, 12)),
        })
    );
}

/// `*` searches a whole word: `io` isn't found in `ioctl`.
#[test]
fn star_matches_whole_words() {
    let mut s = with_code();
    press(&mut s, "1ggwwww*");
    assert_eq!(at(&s), (Some(4), Some("io".into())));
    press(&mut s, "**");
    assert_eq!(at(&s), (Some(1), Some("io".into())), "around, past ioctl");
    assert!(said(&s).contains("Match 1 of 3"), "{}", said(&s));
}

/// In Files changed, `gO` and `gd` work on the file under the cursor,
/// at the new side's lines.
#[test]
fn symbols_and_definitions_in_the_diff() {
    let at = Pos { file: 0, row: 0 };
    let mut s = screen_state(Mode::Dark, ColorDepth::TrueColor, at, Pane::Diff);
    let line = |s: &State| {
        let Screen::Diff(screen) = s.screen() else {
            panic!("not the diff")
        };
        let doc = &s.diff().unwrap().doc;
        doc.line_at(screen.cursor, Side::Right).map(|l| l.line)
    };
    press(&mut s, "gOorigin<Enter>");
    assert_eq!(line(&s), Some(14));
    // `Self::new(…)`: its third word.
    press(&mut s, "jwwwgd");
    assert_eq!(line(&s), Some(10), "{}", said(&s));
}

/// A `/` match carries the word cursor, so `gd` reads the word found.
#[test]
fn a_search_match_is_the_word_gd_reads() {
    let mut s = with_code();
    press(&mut s, "/other<Enter>");
    assert_eq!(at(&s), (Some(5), Some("other".into())), "{}", said(&s));
    press(&mut s, "gg/helper<Enter>");
    assert_eq!(at(&s), (Some(4), Some("helper".into())), "{}", said(&s));
    let mut long = "fn pad() {}\n".repeat(1500);
    long.push_str("fn last() {\n    valid_login(login).then(|| x);\n}\n");
    let mut s = with_repo(Mode::Dark, ColorDepth::TrueColor);
    let route = Route::blob(ghtui(), "main".into(), "src/lib.rs".into());
    let blob = Blob {
        path: "src/lib.rs".into(),
        text: Some(long),
        size: 100,
        truncated: false,
    };
    open(&mut s, route, Data::Blob(Box::new(blob)));
    press(&mut s, "gOlast<Enter>ww/valid_login<Enter>");
    assert_eq!(
        at(&s),
        (Some(1502), Some("valid_login".into())),
        "{}",
        said(&s)
    );
}
