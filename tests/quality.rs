//! Diff quality on small, realistic cases: what a reviewer sees. Each case
//! renders the changed lines with emphasis marked `⟦like this⟧`.

#![allow(
    clippy::panic,
    clippy::unwrap_used,
    reason = "the rendering helper isn't a #[test] function, but a panic in it is still a test failure"
)]

use ghtui::diff::{Content, DiffLine, FileDiff, Whitespace};

/// The changed lines of `old` → `new` (as `path`), `-`/`+` prefixed, with
/// emphasised ranges in `⟦⟧`, read by lines.
fn show(path: &str, old: &str, new: &str) -> String {
    render(path, old, new, false)
}

/// [`show`], read by syntax; a line none of whose code is new or gone is
/// marked `·`, and a block folded as formatting-only has its lines marked
/// `~` instead.
fn by_syntax(path: &str, old: &str, new: &str) -> String {
    render(path, old, new, true)
}

fn render(path: &str, old: &str, new: &str, structural: bool) -> String {
    let diff = FileDiff::compute(path, Some(old.as_bytes()), Some(new.as_bytes()));
    let Content::Text(text) = &diff.content else { panic!("not a text diff") };
    if structural && let Some(why) = text.fallback() {
        panic!("compared by lines: {}", why.reason());
    }
    let a = text.view(Whitespace::Exact, structural);
    let mut out = String::new();
    for (i, line) in (0u32..).zip(&a.lines) {
        let sign = match line {
            DiffLine::Context { .. } => continue,
            DiffLine::Removed(_) => '-',
            DiffLine::Added(_) => '+',
        };
        let folded = a
            .blocks
            .iter()
            .any(|b| b.formatting_only && b.entries.contains(&i));
        let content = text.line(line.shown());
        let mut shown = String::new();
        let mut at = 0;
        for &(a, b) in a.intraline.get(&i).into_iter().flatten() {
            let (a, b) = (a as usize, b as usize);
            shown.push_str(content.get(at..a).unwrap());
            shown.push('⟦');
            shown.push_str(content.get(a..b).unwrap());
            shown.push('⟧');
            at = b;
        }
        shown.push_str(content.get(at..).unwrap());
        let fold = match (folded, a.kept.contains(&i)) {
            (true, _) => "~",
            (false, true) => "·",
            (false, false) => "",
        };
        out.push_str(&format!("{fold}{sign}{shown}\n"));
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
    let Content::Text(text) = &diff.content else { panic!("not a text diff") };
    let line = text
        .alignment(Whitespace::Exact)
        .lines
        .iter()
        .find_map(|l| match l {
            DiffLine::Added(n) => Some(*n),
            _ => None,
        })
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

// ---- by syntax -------------------------------------------------------------

/// Arguments wrapped onto their own lines, one renamed: only the rename
/// stands out, not the trailing comma the wrapping brought.
#[test]
fn by_syntax_wrapped_arguments_show_only_the_rename() {
    let old = "    let diff = compute(path, old_text, new_text, options);\n";
    let new = "    let diff = compute(\n        path,\n        old_text,\n        updated_text,\n        options,\n    );\n";
    assert_eq!(
        by_syntax("a.rs", old, new),
        "-    let diff = compute(path, old_text, ⟦new⟧_text, options);\n\
         ·+    let diff = compute(\n\
         ·+        path,\n\
         ·+        old_text,\n\
         +        ⟦updated⟧_text,\n\
         ·+        options,\n\
         ·+    );\n"
    );
}

/// Code wrapped in a new `if`: the `if` and its closing brace are what
/// changed, not the re-indented lines inside.
#[test]
fn by_syntax_code_wrapped_in_an_if() {
    let old = "fn f() {\n    prepare();\n    run(a, b);\n}\n";
    let new = "fn f() {\n    if ready {\n        prepare();\n        run(a, b);\n    }\n}\n";
    assert_eq!(
        by_syntax("a.rs", old, new),
        "·-    prepare();\n\
         ·-    run(a, b);\n\
         +    if ready {\n\
         ·+        prepare();\n\
         ·+        run(a, b);\n\
         +    }\n"
    );
}

/// Swapped parameters change the signature: one of them is shown moving.
#[test]
fn by_syntax_reordered_parameters() {
    let old = "fn area(width: u32, height: u32) -> u32 {\n    width * height\n}\n";
    let new = "fn area(height: u32, width: u32) -> u32 {\n    width * height\n}\n";
    assert_eq!(
        by_syntax("a.rs", old, new),
        "-fn area(⟦width: u32,⟧ height: u32) -> u32 {\n\
         +fn area(height: u32⟦, width: u32⟧) -> u32 {\n"
    );
}

#[test]
fn by_syntax_reflowed_struct_literal() {
    let old = "let p = Point { x: 1, y: 2, z: 3 };\n";
    let new = "let p = Point {\n    x: 1,\n    y: 2,\n    z: 4,\n};\n";
    assert_eq!(
        by_syntax("a.rs", old, new),
        "-let p = Point { x: 1, y: 2, z: ⟦3⟧ };\n\
         ·+let p = Point {\n\
         ·+    x: 1,\n\
         ·+    y: 2,\n\
         +    z: ⟦4⟧,\n\
         ·+};\n"
    );
}

/// An ordinary edit to one line looks as it does by lines.
#[test]
fn by_syntax_a_one_line_edit() {
    let (old, new) = (
        "x();\nlet total = compute(a, b);\ny();\n",
        "x();\nlet total = compute(a, c);\ny();\n",
    );
    assert_eq!(by_syntax("a.rs", old, new), show("a.rs", old, new));
}

/// Comments and strings are compared by their words.
#[test]
fn by_syntax_comment_and_string_words() {
    let old = "// Returns the total of all rows.\nsay(\"Hello there\");\n";
    let new = "// Returns the sum of all rows.\nsay(\"Hello over there\");\n";
    assert_eq!(
        by_syntax("a.rs", old, new),
        "-// Returns the ⟦total⟧ of all rows.\n\
         ·-say(\"Hello there\");\n\
         +// Returns the ⟦sum⟧ of all rows.\n\
         +say(\"Hello ⟦over⟧ there\");\n"
    );
}

/// Re-wrapping a call is formatting only, trailing comma and all, so it
/// folds away.
#[test]
fn by_syntax_reformatting_is_formatting_only() {
    let old = "result = call(alpha,\n              beta)\n";
    let new = "result = call(\n    alpha,\n    beta,\n)\n";
    assert!(
        by_syntax("a.py", old, new)
            .lines()
            .all(|l| l.starts_with('~')),
        "{}",
        by_syntax("a.py", old, new)
    );
    // A Python one-element tuple's comma is what makes it a tuple.
    assert!(by_syntax("a.py", "x = (a)\n", "x = (a,)\n").contains("⟦,⟧"));
}
