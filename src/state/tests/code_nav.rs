use crate::code_nav::{Answer, Ask, Place, Want};
use crate::diff::anchor::Side;
use crate::route::code;
use crate::ui::diff_doc::Pos;

use super::*;

const LIB_RS: &str = "use std::io;\n\nfn main() {\n    helper(io::stdin());\n    other();\n}\n\nfn helper(input: io::Stdin) {}\n\nfn ioctl() {}\n";

/// `src/lib.rs` open at `main`.
fn with_code() -> State {
    let mut s = with_repo();
    let route = Route::blob(ghtui(), "main".into(), "src/lib.rs".into());
    let blob = crate::fixtures::file("src/lib.rs", LIB_RS);
    open(&mut s, route, Data::from(blob));
    s
}

/// The selected line of the file, and the word under the cursor.
fn at(s: &State) -> (Option<u32>, Option<String>) {
    let p = page(s);
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
    act(&mut s, Action::Symbols);
    assert_eq!(picked_labels(&s), ["fn main", "fn helper", "fn ioctl"]);
    press(&mut s, "help<Enter>");
    assert_eq!(at(&s).0, Some(8));
}

/// `gd` goes to the definition of the word under the cursor in the file.
#[test]
fn gd_finds_a_definition_in_the_file() {
    let mut s = with_code();
    press(&mut s, ":4<Enter>");
    assert_eq!(at(&s), (Some(4), Some("helper".into())));
    press(&mut s, "g");
    assert_eq!(at(&s), (Some(8), Some("helper".into())), "{}", said(&s));
    // Not a name: says so.
    let mut s = with_code();
    press(&mut s, "<Home><C-Right><C-Right><C-Right>");
    assert_eq!(at(&s).1.as_deref(), Some("::"));
    press(&mut s, "g");
    assert!(
        said(&s).contains("No name under the cursor"),
        "{}",
        said(&s)
    );
}

/// A name the file doesn't define is asked of the repository, at the
/// file's revision; the one definition found opens at its line there.
#[test]
fn gd_asks_the_repository_for_one_outside_the_file() {
    let mut s = with_code();
    press(&mut s, ":5<Enter>");
    let ask = Ask {
        repo: ghtui(),
        rev: "main".into(),
        path: "src/lib.rs".into(),
        want: Want::Definition("other".into()),
    };
    assert_eq!(press(&mut s, "g"), [Cmd::Git(Git::Code(ask.clone()))]);
    let answer = Answer {
        places: vec![Place {
            path: "src/other.rs".into(),
            line: 12,
            label: "fn other".into(),
        }],
        note: Some("Still indexing".into()),
        none: String::new(),
    };
    update(&mut s, Msg::Code(ask, Ok(answer)));
    assert_eq!(
        s.route(),
        Some(&Route::Code(code::Blob {
            repo: ghtui(),
            rev: "main".into(),
            path: "src/other.rs".into(),
            lines: Some((12, 12)),
        }))
    );
    assert!(said(&s).contains("Still indexing"), "{}", said(&s));
}

/// `gS` lists the repository's symbols and `gR` the lines using the word;
/// a place in this file goes to its line, one elsewhere opens its file.
#[test]
fn repository_symbols_and_references_are_listed() {
    let mut s = with_code();
    press(&mut s, ":4<Enter>");
    let ask = |want| Ask {
        repo: ghtui(),
        rev: "main".into(),
        path: "src/lib.rs".into(),
        want,
    };
    let place = |path: &str, line, label: &str| Place {
        path: path.into(),
        line,
        label: label.into(),
    };
    let symbols = ask(Want::Symbols);
    assert_eq!(
        act(&mut s, Action::RepoSymbols),
        [Cmd::Git(Git::Code(symbols.clone()))]
    );
    let answer = Answer {
        places: vec![
            place("src/other.rs", 12, "fn other"),
            place("src/lib.rs", 10, "fn ioctl"),
        ],
        ..Answer::default()
    };
    update(&mut s, Msg::Code(symbols, Ok(answer)));
    press(&mut s, "ioctl<Enter>");
    assert_eq!(at(&s).0, Some(10));

    press(&mut s, ":4<Enter>");
    let refs = ask(Want::References("helper".into()));
    assert_eq!(
        act(&mut s, Action::References),
        [Cmd::Git(Git::Code(refs.clone()))]
    );
    let answer = Answer {
        places: vec![
            place("src/lib.rs", 4, "helper(io::stdin());"),
            place("src/main.rs", 2, "lib::helper(x)"),
        ],
        ..Answer::default()
    };
    update(&mut s, Msg::Code(refs.clone(), Ok(answer)));
    assert_eq!(
        picked(&s)[1],
        ("lib::helper(x)".into(), "src/main.rs:2".into())
    );
    press(&mut s, "lib::<Enter>");
    assert_eq!(
        s.route(),
        Some(&Route::Code(code::Blob {
            repo: ghtui(),
            rev: "main".into(),
            path: "src/main.rs".into(),
            lines: Some((2, 2)),
        }))
    );

    // None found: says so, and what the index doesn't hold yet.
    let mut s = with_code();
    let none = Answer {
        note: Some("Still indexing".into()),
        none: "“helper” isn't used".into(),
        ..Answer::default()
    };
    update(&mut s, Msg::Code(refs, Ok(none)));
    assert!(
        said(&s).contains("isn't used · Still indexing"),
        "{}",
        said(&s)
    );
}

/// Searching the word under the cursor finds the whole word: `io` isn't found in `ioctl`.
#[test]
fn star_matches_whole_words() {
    let mut s = with_code();
    press(&mut s, "<Home><C-Right><C-Right><C-Right><C-Right>");
    act(&mut s, Action::SearchWord);
    assert_eq!(at(&s), (Some(4), Some("io".into())));
    act(&mut s, Action::SearchWord);
    act(&mut s, Action::SearchWord);
    assert_eq!(at(&s), (Some(1), Some("io".into())), "around, past ioctl");
    assert!(said(&s).contains("Match 1 of 3"), "{}", said(&s));
}

/// In Files changed, symbols and `g` work on the file under the cursor,
/// at the new side's lines.
#[test]
fn symbols_and_definitions_in_the_diff() {
    let at = Pos { file: 0, row: 0 };
    let mut s = screen_state(at, Pane::Diff);
    let line = |s: &State| {
        let screen = on_diff(s);
        let doc = &s.diff().unwrap().doc;
        doc.line_at(screen.cursor, Side::Right).map(|l| l.line)
    };
    act(&mut s, Action::Symbols);
    press(&mut s, "origin<Enter>");
    assert_eq!(line(&s), Some(14));
    // `Self::new(…)`: its third word.
    press(&mut s, "<Down><C-Right><C-Right><C-Right>g");
    assert_eq!(line(&s), Some(10), "{}", said(&s));
}

/// A `/` match carries the word cursor, so `gd` reads the word found.
#[test]
fn a_search_match_is_the_word_gd_reads() {
    let mut s = with_code();
    press(&mut s, "/other<Enter>");
    assert_eq!(at(&s), (Some(5), Some("other".into())), "{}", said(&s));
    press(&mut s, "<Home>/helper<Enter>");
    assert_eq!(at(&s), (Some(4), Some("helper".into())), "{}", said(&s));
    let mut long = "fn pad() {}\n".repeat(1500);
    long.push_str("fn last() {\n    valid_login(login).then(|| x);\n}\n");
    let mut s = with_repo();
    let route = Route::blob(ghtui(), "main".into(), "src/lib.rs".into());
    let blob = crate::fixtures::file("src/lib.rs", &long);
    open(&mut s, route, Data::from(blob));
    press(&mut s, "gOlast<Enter>ww/valid_login<Enter>");
    assert_eq!(
        at(&s),
        (Some(1502), Some("valid_login".into())),
        "{}",
        said(&s)
    );
}
