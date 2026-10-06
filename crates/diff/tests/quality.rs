//! Diff quality on small, realistic cases: what a reviewer sees. Each case
//! renders the changed lines with emphasis marked `⟦like this⟧`.

#![allow(
    clippy::panic,
    clippy::unwrap_used,
    reason = "the rendering helper isn't a #[test] function, but a panic in it is still a test failure"
)]

use ghtui_diff::{Content, FileDiff, LineKind, Whitespace};

/// The changed lines of `old` → `new` (as `path`), `-`/`+` prefixed, with
/// emphasised ranges in `⟦⟧`.
fn show(path: &str, old: &str, new: &str) -> String {
    let diff = FileDiff::compute(path, Some(old.as_bytes()), Some(new.as_bytes()));
    let Content::Text(text) = &diff.content else {
        panic!("not a text diff");
    };
    let mut out = String::new();
    for (i, line) in (0u32..).zip(text.lines(Whitespace::Exact)) {
        let sign = match line.kind {
            LineKind::Context => continue,
            LineKind::Removed => '-',
            LineKind::Added => '+',
        };
        let content = text.text(line);
        let mut shown = String::new();
        let mut at = 0;
        for &(a, b) in text
            .intraline(Whitespace::Exact)
            .get(&i)
            .into_iter()
            .flatten()
        {
            let (a, b) = (a as usize, b as usize);
            shown.push_str(content.get(at..a).unwrap());
            shown.push('⟦');
            shown.push_str(content.get(a..b).unwrap());
            shown.push('⟧');
            at = b;
        }
        shown.push_str(content.get(at..).unwrap());
        out.push_str(&format!("{sign}{shown}\n"));
    }
    out
}

// ---- pairing lines within a change block ----------------------------------

/// Pairing the first removed line with its single best match would take the
/// second line's partner and leave that edit unexplained; the best pairing
/// overall matches both.
#[test]
fn pairs_for_the_best_total_not_greedily() {
    let old = "let total = price * count;\nlet total = price * count + tax + fee;\n";
    let new = "let total = price * qty;\nlet total = price * count + tax;\n";
    assert_eq!(
        show("a.rs", old, new),
        "-let total = price * ⟦count⟧;\n\
         -let total = price * count + tax⟦ + fee⟧;\n\
         +let total = price * ⟦qty⟧;\n\
         +let total = price * count + tax;\n"
    );
}

/// Swapped lines can't both pair in order; one does, the other shows plain.
#[test]
fn swapped_edits_pair_one_in_order() {
    let old = "let a = one(left);\nlet b = two(right);\n";
    let new = "let b = two(down);\nlet a = one(up);\n";
    let shown = show("a.rs", old, new);
    let emphasised = shown.lines().filter(|l| l.contains('⟦')).count();
    assert_eq!(emphasised, 2, "{shown}");
}

/// More additions than removals: the edited line still finds its partner
/// among the new ones.
#[test]
fn pairs_among_extra_additions() {
    let old = "fn area(w: u32, h: u32) -> u32 {\n";
    let new = "/// The area.\n#[must_use]\nfn area(w: u64, h: u64) -> u64 {\n";
    assert_eq!(
        show("a.rs", old, new),
        "-fn area(w: ⟦u32⟧, h: ⟦u32⟧) -> ⟦u32⟧ {\n\
         +/// The area.\n\
         +#[must_use]\n\
         +fn area(w: ⟦u64⟧, h: ⟦u64⟧) -> ⟦u64⟧ {\n"
    );
}

// ---- characters within a token -------------------------------------------

#[test]
fn only_the_differing_characters_of_a_token_stand_out() {
    assert_eq!(
        show("a.rs", "let n = count;\n", "let n = counts;\n"),
        "-let n = count;\n+let n = count⟦s⟧;\n"
    );
    assert_eq!(
        show("a.rs", "let share = 0.75;\n", "let share = 0.8;\n"),
        "-let share = 0.⟦75⟧;\n+let share = 0.⟦8⟧;\n"
    );
    assert_eq!(
        show("a.rs", "foo_bar(x);\n", "foo_baz(x);\n"),
        "-foo_ba⟦r⟧(x);\n+foo_ba⟦z⟧(x);\n"
    );
    assert_eq!(
        show("a.rs", "say(\"Hello\");\n", "say(\"Hello!\");\n"),
        "-say(\"Hello\");\n+say(\"Hello⟦!⟧\");\n"
    );
}

/// Unrelated words that happen to share a letter stay emphasised whole.
#[test]
fn look_alike_words_stay_whole() {
    assert_eq!(
        show(
            "a.rs",
            "let r = area.width + 1;\n",
            "let r = area.words + 1;\n"
        ),
        "-let r = area.⟦width⟧ + 1;\n+let r = area.⟦words⟧ + 1;\n"
    );
}

/// Narrowing never splits a letter from its accent.
#[test]
fn characters_are_whole_graphemes() {
    assert_eq!(
        show("a.txt", "le cafe\u{301} noir\n", "le cafe\u{300} noir\n"),
        "-le caf⟦e\u{301}⟧ noir\n+le caf⟦e\u{300}⟧ noir\n"
    );
}

// ---- reflowed code --------------------------------------------------------

/// Arguments wrapped onto their own lines, one renamed: only the rename
/// (and the trailing comma that came with the wrapping) stands out.
#[test]
fn wrapped_arguments_show_only_the_rename() {
    let old = "    let diff = compute(path, old_text, new_text, options);\n";
    let new = "    let diff = compute(\n        path,\n        old_text,\n        updated_text,\n        options,\n    );\n";
    assert_eq!(
        show("a.rs", old, new),
        "-    let diff = compute(path, old_text, ⟦new⟧_text, options);\n\
         +    let diff = compute(\n\
         +        path,\n\
         +        old_text,\n\
         +        ⟦updated⟧_text,\n\
         +        options⟦,⟧\n\
         +    );\n"
    );
}

/// A chain split across lines, one call renamed: only the rename stands
/// out, not the lines the layout change touched.
#[test]
fn a_split_chain_shows_only_the_rename() {
    let old = "let names = files.iter().map(|f| f.name()).collect();\n";
    let new = "let names = files\n    .iter()\n    .map(|f| f.title())\n    .collect();\n";
    assert_eq!(
        show("a.rs", old, new),
        "-let names = files.iter().map(|f| f.⟦name⟧()).collect();\n\
         +let names = files\n\
         +    .iter()\n\
         +    .map(|f| f.⟦title⟧())\n\
         +    .collect();\n"
    );
}

/// Two statements joined onto one line: nothing changed but the break.
#[test]
fn joined_statements_show_no_emphasis() {
    assert!(!show("a.rs", "a += 1;\nb += 1;\n", "a += 1; b += 1;\n").contains('⟦'));
}

/// An ordinary edit to one line looks the same as line-by-line diffing.
#[test]
fn a_one_line_edit_is_unchanged() {
    assert_eq!(
        show(
            "a.rs",
            "x\nlet total = compute(a, b);\ny\n",
            "x\nlet total = compute(a, c);\ny\n"
        ),
        "-let total = compute(a, ⟦b⟧);\n+let total = compute(a, ⟦c⟧);\n"
    );
}

// ---- naming where a change is ---------------------------------------------

/// The scopes the first added line is in.
fn scope_of_change(path: &str, old: &str, new: &str) -> Vec<String> {
    let diff = FileDiff::compute(path, Some(old.as_bytes()), Some(new.as_bytes()));
    let Content::Text(text) = &diff.content else {
        panic!("not a text diff");
    };
    let line = text
        .lines(Whitespace::Exact)
        .iter()
        .find(|l| l.kind == LineKind::Added)
        .and_then(|l| l.new)
        .unwrap();
    text.scope(line).into_iter().map(str::to_owned).collect()
}

#[test]
fn a_change_in_a_method_names_its_impl_and_fn() {
    let old = "struct Doc;\n\nimpl Doc {\n    fn offset(&self) -> u32 {\n        1\n    }\n}\n";
    let new = old.replace("        1\n", "        2\n");
    assert_eq!(
        scope_of_change("a.rs", old, &new),
        ["impl Doc", "fn offset"]
    );
}

#[test]
fn a_change_at_file_level_names_nothing() {
    let old = "use a;\n\nfn f() {}\n";
    let new = "use b;\n\nfn f() {}\n";
    assert!(scope_of_change("a.rs", old, new).is_empty());
}

#[test]
fn a_change_in_a_nested_function_names_the_whole_path() {
    let old = "def outer():\n    def inner():\n        return 1\n    return inner\n";
    let new = old.replace("return 1", "return 2");
    assert_eq!(
        scope_of_change("a.py", old, &new),
        ["def outer", "def inner"]
    );
}

/// A syntax error elsewhere in the file doesn't stop the change being named.
#[test]
fn a_syntax_error_elsewhere_degrades_gracefully() {
    let old = "fn fine() {\n    a();\n}\n\nfn broken( {\n";
    let new = old.replace("a();", "b();");
    assert_eq!(scope_of_change("a.rs", old, &new), ["fn fine"]);
}
