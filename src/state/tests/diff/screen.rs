use std::collections::HashSet;
use std::sync::Arc;

use crate::api::model::PrRef;
use crate::diff::FileDiff;
use crate::git::Oid;
use crate::git::files::{ChangedFile, FileStatus, ZERO_OID};
use crate::git::repo::PrRefs;
use crate::ui::diff_doc::{Doc, Pos};

use crate::diff_screen::{DiffOf, DiffPrefs, DiffScreen, DiffState, Pane};
use crate::fixtures::diff_msg;
use crate::state::tests::{NOW, press, screen_text, state};
use crate::state::{DiffMsg, Msg, Screen, State, update};

fn pr() -> PrRef {
    PrRef::parse("o/r#7").unwrap()
}

fn file(
    status: FileStatus,
    old: Option<&str>,
    new: Option<&str>,
    modes: (u32, u32),
) -> ChangedFile {
    ChangedFile {
        status,
        old_path: old.map(Into::into),
        new_path: new.map(Into::into),
        old_mode: modes.0,
        new_mode: modes.1,
        old_oid: Oid::parse(&"1".repeat(40)).unwrap(),
        new_oid: if new.is_some() {
            Oid::parse(&"2".repeat(40)).unwrap()
        } else {
            Oid::parse(ZERO_OID).unwrap()
        },
        similarity: (status == FileStatus::Renamed).then_some(94),
    }
}

const OLD_RS: &str = "use std::fmt;\n\n/// A point.\npub struct Point {\n    x: i32,\n    y: i32,\n}\n\nimpl Point {\n    pub fn new(x: i32, y: i32) -> Self {\n        Point { x, y }\n    }\n}\n";
const NEW_RS: &str = "use std::fmt;\n\n/// A point in 2D.\npub struct Point {\n    x: i32,\n    y: i32,\n}\n\nimpl Point {\n    pub fn new(x: i32, y: i32) -> Self {\n        Point { x, y }\n    }\n\n    pub fn origin() -> Self {\n        Self::new(0, \"0\".len() as i32 - 1)\n    }\n}\n";

fn loaded(doc: &Doc) -> DiffState {
    let refs = PrRefs {
        head: Oid::parse(&"e".repeat(40)).unwrap(),
        base: Oid::parse(&"b".repeat(40)).unwrap(),
        merge_base: Oid::parse(&"c".repeat(40)).unwrap(),
    };
    DiffState::fresh().with_doc(refs, doc)
}

/// Opens a diff screen on `diff`, as wide as the terminal.
pub(in crate::state) fn open_diff(s: &mut State, diff: DiffState, cursor: Pos, focus: Pane) {
    s.diffs.insert(DiffOf::Pr(pr()), diff);
    let mut screen = DiffScreen::new(DiffOf::Pr(pr()), DiffPrefs::fit(s.size.0));
    screen.cursor = cursor;
    screen.focus = focus;
    s.screens.push(Screen::Diff(Box::new(screen)));
    let _ = s.settle_diff();
}

/// The fixture's changed files.
fn fixture_files() -> Vec<ChangedFile> {
    let regular = (0o100644, 0o100644);
    vec![
        file(
            FileStatus::Modified,
            Some("src/point.rs"),
            Some("src/point.rs"),
            regular,
        ),
        file(
            FileStatus::Renamed,
            Some("docs/old.md"),
            Some("docs/new.md"),
            regular,
        ),
        file(FileStatus::Added, None, Some("notes.txt"), (0, 0o100644)),
        file(FileStatus::Deleted, Some("gone.py"), None, (0o100644, 0)),
        file(
            FileStatus::Modified,
            Some("logo.png"),
            Some("logo.png"),
            regular,
        ),
        file(
            FileStatus::Modified,
            Some("run.sh"),
            Some("run.sh"),
            (0o100644, 0o100755),
        ),
        file(
            FileStatus::Modified,
            Some("Cargo.lock"),
            Some("Cargo.lock"),
            regular,
        ),
        file(
            FileStatus::Modified,
            Some("zz/pending.rs"),
            Some("zz/pending.rs"),
            regular,
        ),
    ]
}

pub(crate) fn fixture_diff() -> DiffState {
    let files = fixture_files();
    let mut doc = Doc::new(files, &HashSet::new(), Default::default());
    let diffs = [
        FileDiff::compute(
            "src/point.rs",
            Some(OLD_RS.as_bytes()),
            Some(NEW_RS.as_bytes()),
        ),
        FileDiff::compute("docs/new.md", Some(b"# Title\n"), Some(b"# Title\n")),
        FileDiff::compute("notes.txt", None, Some(b"first\nsecond, no newline")),
        FileDiff::compute("gone.py", Some(b"def f():\n    return 1\n"), None),
        FileDiff::compute("logo.png", Some(b"\x89PNG\0a"), Some(b"\x89PNG\0bc")),
        FileDiff::compute("run.sh", Some(b"echo hi\n"), Some(b"echo hi\n")),
        FileDiff::compute("Cargo.lock", Some(b"a\n"), Some(b"b\n")),
    ];
    for (i, d) in diffs.into_iter().enumerate() {
        doc.set_diff(i, Arc::new(d));
    }
    loaded(&doc)
}

pub(in crate::state) fn screen_state(cursor: Pos, focus: Pane) -> State {
    let mut s = state();
    s.size = (110, 34);
    open_diff(&mut s, fixture_diff(), cursor, focus);
    s
}

/// The fixture's diff in true color, the cursor on `row` of `file`.
pub(in crate::state) fn diff_at(file: usize, row: usize) -> State {
    screen_state(Pos { file, row }, Pane::Diff)
}

pub(in crate::state) fn commit_of() -> DiffOf {
    DiffOf::Commit(
        crate::state::tests::ghtui(),
        Oid::parse(&crate::fixtures::commit().oid).unwrap(),
    )
}

/// A commit's files have its tabs instead of a pull request's.
#[test]
fn a_commits_files_have_its_tabs() {
    let mut s = state();
    s.size = (110, 34);
    let of = commit_of();
    s.diffs.insert(of.clone(), fixture_diff());
    s.screens.push(Screen::Diff(Box::new(DiffScreen::new(
        of,
        DiffPrefs::fit(s.size.0),
    ))));
    let _ = s.settle_diff();
    let screen = screen_text(&s);
    let tabs = screen.lines().nth(1).unwrap_or_default();
    assert!(
        tabs.contains("Commit") && tabs.contains("Files changed"),
        "{screen}"
    );
    assert!(!screen.contains("Conversation"), "{screen}");
}

/// A link to a file in a diff (`#diff-<hash>`, maybe with a line) goes
/// to that file once the files are listed.
#[test]
fn diff_links_go_to_their_file() {
    let mut s = state();
    let hash = crate::ui::pages::url::path_hash("notes.txt");
    let url = format!("https://github.com/o/r/pull/7/files#diff-{hash}R1");
    let _ = s.follow(&crate::ui::page::Link::Url(url));
    s.diffs.insert(DiffOf::Pr(pr()), fixture_diff());
    let _ = s.settle_diff();
    let Screen::Diff(d) = s.screen() else {
        panic!("the link didn't open a diff");
    };
    let doc = &s.diffs[&d.of].doc;
    assert_eq!(doc.files()[d.cursor.file].meta.path(), "notes.txt");
    assert_eq!(d.anchor, None);
}

/// The diff on screen and its file tree.
fn diff_parts(s: &mut State) -> (DiffScreen, &DiffState) {
    let (screen, diff) = s.diff_parts().expect("a diff");
    (screen.clone(), diff)
}

/// Every way the tree's selection moves shows its file in the diff.
#[test]
fn the_diff_follows_the_file_tree() {
    let mut s = screen_state(Pos::default(), Pane::Tree);
    let at = |s: &mut State| {
        let (screen, _) = diff_parts(s);
        (screen.tree_selected, screen.cursor.file)
    };
    let file = |s: &mut State, keys: &str| {
        press(s, keys);
        at(s)
    };
    // Into src/, then docs/, from folder to folder.
    assert_eq!(file(&mut s, "ggl"), (1, 0), "src/point.rs");
    assert_eq!(file(&mut s, "]]l"), (3, 1), "docs/new.md");
    assert_eq!(file(&mut s, "2j"), (5, 3), "gone.py");
    assert_eq!(file(&mut s, "G"), (10, 7), "zz/pending.rs");
    assert_eq!(file(&mut s, "5gg"), (4, 2), "notes.txt");
    // A click in the tree selects there.
    let content = s.layout().content;
    let (screen, diff) = diff_parts(&mut s);
    let tree = crate::diff_screen::layout(content, &screen.prefs, &diff.tree).tree;
    let tree = tree.expect("a tree");
    let _ = update(
        &mut s,
        Msg::Mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: tree.x + 2,
            row: tree.y + 3 + 6,
            modifiers: crossterm::event::KeyModifiers::NONE,
        }),
    );
    assert_eq!(at(&mut s), (6, 4), "logo.png");
}

/// `y` copies the line's link, or the selected lines', as GitHub
/// anchors them; the link opens on those lines, selected.
#[test]
fn line_links_copy_and_open_on_their_lines() {
    let mut s = diff_at(0, 4);
    let copied = |s: &mut State, keys: &str| match press(s, keys).as_slice() {
        [crate::state::Cmd::Copy(url)] => url.clone(),
        cmds => panic!("{cmds:?}"),
    };
    let hash = crate::ui::pages::url::path_hash("src/point.rs");
    let files = format!("https://github.com/o/r/pull/7/files#diff-{hash}");
    assert_eq!(
        copied(&mut s, "y"),
        format!("{files}L3"),
        "the removed line"
    );
    let url = copied(&mut s, "Vjjy");
    assert_eq!(url, format!("{files}L3-R4"));
    press(&mut s, "<Esc>");
    let _ = s.follow(&crate::ui::page::Link::Url(url));
    let _ = s.settle_diff();
    let (screen, _) = diff_parts(&mut s);
    assert_eq!(screen.anchor, None);
    assert_eq!(screen.selection, Some(Pos { file: 0, row: 4 }));
    assert_eq!(screen.cursor, Pos { file: 0, row: 6 });
    // A commit's lines are on its page.
    let mut s = diff_at(0, 4);
    s.diffs.insert(commit_of(), fixture_diff());
    if let Some((screen, _)) = s.diff_parts() {
        screen.of = commit_of();
    }
    let url = copied(&mut s, "y");
    assert!(url.contains("/commit/"), "{url}");
    assert!(url.ends_with(&format!("#diff-{hash}L3")), "{url}");
}

/// `zc`, `zo` and `za` fold the thread under the cursor, or the file;
/// `zM` and `zR` every file.
#[test]
fn folds_close_threads_and_files() {
    let mut s = with_threads();
    let rows = |s: &mut State, file: usize| diff_parts(s).1.doc.files()[file].rows().len();
    let open = |s: &mut State| {
        let (screen, diff) = diff_parts(s);
        let thread = diff.doc.annotation_under(screen.cursor, screen.half);
        thread.map(|t| diff.doc.thread_open(t))
    };
    let thread = diff_parts(&mut s)
        .1
        .doc
        .annotation_pos(0)
        .expect("a thread");
    if let Some((screen, _)) = s.diff_parts() {
        screen.cursor = thread;
    }
    assert_eq!(open(&mut s), Some(true));
    press(&mut s, "zc");
    assert_eq!(open(&mut s), Some(false), "the thread folds");
    press(&mut s, "za");
    assert_eq!(open(&mut s), Some(true));
    // Folding a folded thread folds its file.
    press(&mut s, "zczc");
    assert_eq!(rows(&mut s, 0), 2, "the header, and the space after it");
    assert_eq!(diff_parts(&mut s).0.cursor, Pos { file: 0, row: 0 });
    press(&mut s, "za");
    assert!(rows(&mut s, 0) > 2);
    press(&mut s, "zM");
    let doc = &diff_parts(&mut s).1.doc;
    assert!(
        doc.files()
            .iter()
            .all(crate::ui::diff_doc::DocFile::collapsed)
    );
    press(&mut s, "zR");
    let doc = &diff_parts(&mut s).1.doc;
    assert!(doc.files().iter().all(|f| !f.collapsed()), "generated too");
}

/// The file tree fits its names within a third of the screen, and `<`
/// and `>` narrow and widen it.
#[test]
fn the_file_tree_fits_its_names_and_resizes() {
    let long = "docs/a-name-long-enough-to-want-more-than-a-third-of-the-screen.md";
    let doc = Doc::new(
        vec![file(FileStatus::Added, None, Some(long), (0, 0o100644))],
        &HashSet::new(),
        Default::default(),
    );
    let mut s = state();
    s.size = (216, 40);
    open_diff(&mut s, loaded(&doc), Pos::default(), Pane::Tree);
    let width = |s: &mut State| {
        let content = s.layout().content;
        let (screen, diff) = diff_parts(s);
        let tree = crate::diff_screen::layout(content, &screen.prefs, &diff.tree).tree;
        tree.map(|t| t.width)
    };
    assert_eq!(width(&mut s), Some(72), "a third of 216");
    press(&mut s, "<lt><lt>");
    assert_eq!(width(&mut s), Some(64));
    press(&mut s, ">");
    assert_eq!(width(&mut s), Some(68));
    s.size = (110, 34);
    assert_eq!(width(&mut s), Some(55), "at most half");
}

/// A link to a review comment (`#discussion_r<id>`) opens the diff at
/// its thread once the threads are in.
#[test]
fn review_comment_links_go_to_their_thread() {
    let mut s = state();
    let url = "https://github.com/o/r/pull/7#discussion_r42".to_owned();
    let _ = s.follow(&crate::ui::page::Link::Url(url));
    s.diffs.insert(DiffOf::Pr(pr()), fixture_diff());
    let _ = s.settle_diff();
    let Screen::Diff(d) = s.screen() else {
        panic!("the link didn't open a diff");
    };
    assert!(d.anchor.is_some(), "waits for the threads");
    let mut thread = crate::fixtures::thread("t", Some(3), false, false);
    thread.comments[0].url = "https://github.com/o/r/pull/7#discussion_r42".into();
    if let Some(diff) = s.diffs.get_mut(&DiffOf::Pr(pr())) {
        diff.edit(|i| {
            i.threads.finish(Ok(vec![
                crate::fixtures::thread("u", Some(1), false, false),
                thread,
            ]));
        });
    }
    let _ = s.settle_diff();
    let Screen::Diff(d) = s.screen() else {
        panic!("not a diff");
    };
    let doc = &s.diffs[&d.of].doc;
    assert_eq!(d.anchor, None);
    assert_eq!(doc.annotation_at(d.cursor), Some(1), "the second thread");
}

/// A commit's diff has no review threads: a review comment's anchor
/// there is let go rather than waited on.
#[test]
fn review_comment_anchors_on_a_commit_are_let_go() {
    let mut s = state();
    let of = commit_of();
    s.diffs.insert(of.clone(), fixture_diff());
    let mut screen = DiffScreen::new(of, DiffPrefs::fit(s.size.0));
    screen.anchor = Some("discussion_r42".into());
    s.screens.push(Screen::Diff(Box::new(screen)));
    let _ = s.settle_diff();
    assert!(matches!(s.screen(), Screen::Diff(d) if d.anchor.is_none()));
}

/// Threads that come before their file's diff don't lose the link's
/// place: it waits for the diff, then goes to the thread.
#[test]
fn review_comment_links_wait_for_their_file() {
    let mut s = state();
    let url = "https://github.com/o/r/pull/7#discussion_r42".to_owned();
    let _ = s.follow(&crate::ui::page::Link::Url(url));
    let mut diff = loaded(&Doc::new(
        fixture_files(),
        &HashSet::new(),
        Default::default(),
    ));
    let mut thread = crate::fixtures::thread("t", Some(3), false, false);
    thread.comments[0].url = "https://github.com/o/r/pull/7#discussion_r42".into();
    diff.edit(|i| i.threads.finish(Ok(vec![thread])));
    s.diffs.insert(DiffOf::Pr(pr()), diff);
    let _ = s.settle_diff();
    let waiting = matches!(s.screen(), Screen::Diff(d) if d.anchor.is_some());
    assert!(waiting, "waits for src/point.rs's diff");
    if let Some(diff) = s.diffs.get_mut(&DiffOf::Pr(pr())) {
        let d = FileDiff::compute(
            "src/point.rs",
            Some(OLD_RS.as_bytes()),
            Some(NEW_RS.as_bytes()),
        );
        diff.doc.set_diff(0, Arc::new(d));
    }
    let _ = s.settle_diff();
    let Screen::Diff(d) = s.screen() else {
        panic!("not a diff");
    };
    assert_eq!(d.anchor, None);
    assert_eq!(s.diffs[&d.of].doc.annotation_at(d.cursor), Some(0));
}

/// The pull request whose diff is on screen.
pub(in crate::state) fn diff_pr(s: &State) -> crate::api::model::PrRef {
    match s.screen() {
        Screen::Diff(d) => d.of.pr().cloned(),
        Screen::Page(_) => None,
    }
    .expect("a pull request's diff")
}

pub(in crate::state) fn with_threads() -> State {
    use crate::api::model::{NodeId, ReviewComment, ReviewThread, Side};
    let mut s = diff_at(0, 0);
    let comment = |id: &str, author: &str, body: &str, pending: bool| ReviewComment {
        id: NodeId::new(id),
        author: author.into(),
        body: body.into(),
        created_at: "2026-10-03T08:00:00Z".into(),
        url: String::new(),
        original_commit: Some("abc".into()),
        pending,
        editable: crate::fixtures::editable(id),
        review: None,
    };
    let thread =
        |id: &str, line: Option<u32>, resolved: bool, file_level: bool, comments| ReviewThread {
            id: NodeId::new(id),
            path: "src/point.rs".into(),
            side: Side::Right,
            start_side: None,
            line,
            start_line: None,
            original_line: line,
            original_start_line: None,
            outdated: false,
            resolved,
            file_level,
            can_reply: true,
            can_resolve: true,
            can_unresolve: true,
            comments,
            diff_hunk: String::new(),
        };
    // Two reacted to the question, you among them.
    let mut asked = comment(
        "c1",
        "alice",
        "Should this say \"2D\" or \"two-dimensional\"? The rest of the docs spell it out.",
        false,
    );
    asked.editable.reactions = vec![crate::api::edit::Reaction {
        content: crate::api::edit::ReactionContent::Hooray,
        count: 2,
        mine: true,
    }];
    let diff = s.diffs.get_mut(&DiffOf::Pr(pr())).unwrap();
    diff.edit(|i| {
        i.threads.finish(Ok(vec![
            thread(
                "t1",
                Some(3),
                false,
                false,
                vec![
                    asked,
                    comment("c2", "octocat", "Good point, I'll spell it out.", true),
                ]
                .into(),
            ),
            thread(
                "t2",
                Some(10),
                true,
                false,
                vec![comment("c3", "bob", "Looks fine now.", false)].into(),
            ),
            thread(
                "t3",
                None,
                false,
                true,
                vec![comment(
                    "c4",
                    "carol",
                    "Please add tests for this file.",
                    false,
                )]
                .into(),
            ),
        ]));
    });
    diff.edit(|i| {
        i.review = crate::store::ReviewState {
            pending: vec![crate::store::DraftComment {
                id: 1,
                path: "src/point.rs".into(),
                body: "Prefer `Self::default()` here.".into(),
                side: crate::store::DraftSide::Right,
                line: Some(15),
                start_line: None,
                start_side: None,
                commit: "h".into(),
                error: Some("pull_request_review_thread.line must be part of the diff".into()),
            }],
            ..Default::default()
        }
    });
    let _ = s.settle_diff();
    s
}

/// A thread whose first comment is Markdown: a link to follow, inline
/// code, a list, a quote, and a suggested change, shown as a diff of the
/// line it replaces.
fn with_markdown_thread() -> State {
    let mut s = with_threads();
    let body = "See [the guide](https://example.com/guide) on `Point`:\n\n\
                - spell out *two-dimensional*\n- keep it short\n\n\
                > The rest of the docs spell it out.\n\n\
                ```suggestion\n/// A point in two dimensions.\n```";
    let diff = s.diffs.get_mut(&DiffOf::Pr(pr())).unwrap();
    diff.edit(|i| {
        let threads = i.threads.data.as_mut().unwrap();
        threads.truncate(1);
        threads[0].comments.items[0].body = body.into();
        i.review = crate::store::ReviewState::default();
    });
    let _ = s.settle_diff();
    s
}

/// `i` in Files changed letters the links in threads' comments, and the
/// letter follows one; so does Enter on its line.
#[test]
fn links_in_threads_are_followed() {
    use crate::state::{Cmd, Overlay};
    use crate::ui::page::Link;
    let guide = "https://example.com/guide";
    let mut s = with_markdown_thread();
    press(&mut s, "i");
    let Some(Overlay::Hints(hints)) = &s.overlay else {
        panic!("no hints: {:?}", s.notices.shown());
    };
    let at = hints.links.iter().position(|l| *l == Link::from(guide));
    let label = hints.labels[at.expect("the guide is lettered")]
        .label
        .clone();
    let cmds = press(&mut s, &label);
    assert!(cmds.contains(&Cmd::OpenUrl(guide.into())), "{cmds:?}");

    press(&mut s, "/the guide<Enter>");
    let cmds = press(&mut s, "<Enter>");
    assert!(cmds.contains(&Cmd::OpenUrl(guide.into())), "{cmds:?}");
}

/// In Files changed, `,e` edits the review comment the cursor is on, in
/// the thread; once GitHub takes it, the threads are fetched again to
/// show it. A draft is edited with the draft's own keys.
#[test]
fn a_review_comment_is_edited_in_its_thread() {
    use crate::act::{By, Subject};
    use crate::api::change::Change;
    use crate::api::edit::{Edit, Written};
    use crate::state::{Api, Cmd, Overlay};
    use crate::ui::annotations::{Annotation, ThreadRowKind};
    use crate::ui::diff_doc::Row;
    let mut s = with_threads();
    // The reply's author line.
    let reply = {
        let doc = &s.diff().unwrap().doc;
        let file = &doc.files()[0];
        (0..)
            .map(|row| Pos { file: 0, row })
            .take_while(|p| doc.row(*p).is_some())
            .find(|p| {
                matches!(doc.row(*p), Some(Row::Thread(t))
                    if file.thread_row(t).is_some_and(|r| matches!(r.kind, ThreadRowKind::Head { comment: 1, .. })))
            })
            .unwrap()
    };
    if let Screen::Diff(d) = s.screen_mut() {
        d.cursor = Pos {
            row: reply.row + 1,
            ..reply
        };
    }
    press(&mut s, ",e");
    let Some(Overlay::Compose(c)) = &s.overlay else {
        panic!(",e opens the composer")
    };
    assert_eq!(c.text(), "Good point, I'll spell it out.");
    let cmds = press(&mut s, "!<C-s>");
    let edit = Edit::Body {
        of: Written::ReviewComment,
        id: crate::api::model::NodeId::new("c2"),
        body: "Good point, I'll spell it out.!".into(),
    };
    let sent = crate::fixtures::change(Change::Edit(edit), By::Edit(Subject::Pr(pr())));
    assert_eq!(cmds, [sent]);
    let after = crate::fixtures::answer(&mut s, &cmds, Ok(()));
    assert!(
        after.contains(&Cmd::Api(Api::FetchThreads(pr()))),
        "{after:?}"
    );
    assert!(s.overlay.is_none());

    // On the draft, it edits the draft.
    let doc = &s.diff().unwrap().doc;
    let draft = doc.annotations().iter().position(Annotation::is_draft);
    let at = doc.annotation_pos(u32::try_from(draft.unwrap()).unwrap());
    let at = at.unwrap();
    if let Screen::Diff(d) = s.screen_mut() {
        d.cursor = Pos {
            row: at.row + 1,
            ..at
        };
    }
    press(&mut s, ",e");
    assert!(matches!(
        &s.overlay,
        Some(Overlay::Compose(c)) if matches!(c.target, crate::compose::Target::Review(crate::review::Comment::Draft { .. }))
    ));
}

/// Intra-line emphasis, code moved between files, and a
/// formatting-only change folded away.
pub(in crate::state) fn better_than_github() -> State {
    let regular = (0o100644, 0o100644);
    let files = vec![
        file(
            FileStatus::Modified,
            Some("src/math.rs"),
            Some("src/math.rs"),
            regular,
        ),
        file(
            FileStatus::Modified,
            Some("src/util.rs"),
            Some("src/util.rs"),
            regular,
        ),
    ];
    let helper =
        "pub fn clamp(value: i32, low: i32, high: i32) -> i32 {\n    value.max(low).min(high)\n}\n";
    let math_old = format!(
        "pub fn area(width: u32, height: u32) -> u32 {{\n    width * height\n}}\n\n{helper}\nlet total = compute(alpha, beta);\n// a\n// b\n// c\n// d\ncall(\n    one,\n    two\n);\n"
    );
    let math_new = "pub fn area(width: u32, height: u32) -> u32 {\n    width * height\n}\n\nlet total = compute(alpha, gamma);\n// a\n// b\n// c\n// d\ncall(one, two);\n";
    let util_old = "// helpers\n";
    let util_new = format!("// helpers\n{helper}");
    let mut doc = Doc::new(files, &HashSet::new(), Default::default());
    doc.set_diff(
        0,
        Arc::new(FileDiff::compute(
            "src/math.rs",
            Some(math_old.as_bytes()),
            Some(math_new.as_bytes()),
        )),
    );
    doc.set_diff(
        1,
        Arc::new(FileDiff::compute(
            "src/util.rs",
            Some(util_old.as_bytes()),
            Some(util_new.as_bytes()),
        )),
    );
    let texts: Vec<_> = doc
        .files()
        .iter()
        .map(|f| f.text().unwrap().clone())
        .collect();
    let moves = crate::diff::moves::detect_moves(&[(0, &texts[0]), (1, &texts[1])]);
    assert_eq!(moves.len(), 1);
    doc.set_moves(moves);
    let mut s = state();
    s.size = (110, 30);
    open_diff(&mut s, loaded(&doc), Pos::default(), Pane::Diff);
    s
}

/// While the diff is fetched, git's progress shows in its place.
#[test]
fn a_loading_diff_shows_gits_progress() {
    let mut s = state();
    let mut diff = DiffState::fresh();
    diff.progress = Some("Receiving objects:  42% (420/1000), 1.2 MiB | 3.4 MiB/s".into());
    open_diff(&mut s, diff, Pos::default(), Pane::Diff);
    let screen = screen_text(&s);
    assert!(
        screen.contains("Receiving objects:  42% (420/1000)"),
        "{screen}"
    );
}

/// A long git error wraps instead of being cut off.
#[test]
fn a_long_git_error_wraps() {
    let mut s = state();
    s.size = (80, 24);
    let mut diff = DiffState::fresh();
    diff.progress = None;
    diff.error = Some(
        "git fetch failed: fatal: unable to access 'https://github.com/o/r.git/': \
         Could not resolve host: github.com (is the network up?)"
            .into(),
    );
    open_diff(&mut s, diff, Pos::default(), Pane::Diff);
    let screen = screen_text(&s);
    assert!(screen.contains("network up?)"), "{screen}");
}

/// Scrolling stays cheap on a big PR: rendering only touches visible rows.
/// Run with `cargo test --release -p ghtui -- --ignored scroll_timing`.
#[test]
#[ignore = "timing; run in release"]
#[expect(clippy::print_stderr, reason = "reports the timings")]
fn scroll_timing_on_500_files_20k_lines() {
    let regular = (0o100644, 0o100644);
    let files: Vec<ChangedFile> = (0..500)
        .map(|i| {
            let path = format!("src/m{i}.rs");
            file(FileStatus::Modified, Some(&path), Some(&path), regular)
        })
        .collect();
    let mut doc = Doc::new(files, &HashSet::new(), Default::default());
    let old: String = (0..60)
        .map(|i| format!("    let v{i} = compute({i}, \"x\");\n"))
        .collect();
    let new: String = old.replace("compute(", "compute_fast(");
    let diff = Arc::new(FileDiff::compute(
        "m.rs",
        Some(old.as_bytes()),
        Some(new.as_bytes()),
    ));
    for i in 0..500 {
        doc.set_diff(i, diff.clone());
    }
    let changed: u32 = doc
        .files()
        .iter()
        .map(|f| f.diff.as_ref().unwrap().additions)
        .sum();
    assert!(changed >= 20_000, "{changed}");

    let mut s = state();
    s.size = (160, 50);
    open_diff(&mut s, loaded(&doc), Pos::default(), Pane::Diff);

    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(160, 50)).unwrap();
    let frames = 2000;
    let start = std::time::Instant::now();
    for i in 0..frames {
        press(&mut s, if i % 100 < 95 { "j" } else { "<C-d>" });
        terminal
            .draw(|frame| crate::view::view(&s, frame, NOW))
            .unwrap();
    }
    let per_frame = start.elapsed() / frames;
    eprintln!("update + render per frame: {per_frame:?}");
    assert!(
        per_frame < std::time::Duration::from_millis(8),
        "{per_frame:?} per frame"
    );
}

/// A single 50k-line file: diffing and highlighting run on the diff job's
/// workers (never the UI task); once its result arrives, the first screen
/// must be ready within 100ms. This measures the worker's compute plus the
/// UI's update and first render.
/// Run with `cargo test --release -p ghtui -- --ignored big_file_timing`.
#[test]
#[ignore = "timing; run in release"]
#[expect(clippy::print_stderr, reason = "reports the timings")]
fn big_file_timing_50k_lines() {
    use std::time::{Duration, Instant};

    let old: String = (0..50_000)
        .map(|i| format!("    let value_{i} = compute({i}, \"label\");\n"))
        .collect();
    let new = old
        .replace("compute(100,", "compute_fast(100,")
        .replace("compute(25000,", "compute_fast(25000,")
        .replace("compute(49999,", "compute_fast(49999,");

    let start = Instant::now();
    let diff = Arc::new(FileDiff::compute(
        "big.rs",
        Some(old.as_bytes()),
        Some(new.as_bytes()),
    ));
    let compute = start.elapsed();

    let regular = (0o100644, 0o100644);
    let big = file(
        FileStatus::Modified,
        Some("big.rs"),
        Some("big.rs"),
        regular,
    );
    let mut s = state();
    s.size = (160, 50);
    let doc = Doc::new(vec![big], &HashSet::new(), Default::default());
    open_diff(&mut s, loaded(&doc), Pos::default(), Pane::Diff);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(160, 50)).unwrap();

    let start = Instant::now();
    let job = s.diffs[&DiffOf::Pr(pr())].job;
    diff_msg(
        &mut s,
        &pr(),
        DiffMsg::Job(job, crate::diff_job::JobMsg::File(0, diff)),
    );
    terminal
        .draw(|frame| crate::view::view(&s, frame, NOW))
        .unwrap();
    let first_screen = start.elapsed();

    // Full-file mode on 50k lines must stay interactive too.
    let start = Instant::now();
    press(&mut s, "zf");
    terminal
        .draw(|frame| crate::view::view(&s, frame, NOW))
        .unwrap();
    let full_file = start.elapsed();

    eprintln!("compute {compute:?}, first screen {first_screen:?}, full file {full_file:?}");
    assert!(
        compute + first_screen < Duration::from_millis(100),
        "{:?}",
        compute + first_screen
    );
    assert!(full_file < Duration::from_millis(100), "{full_file:?}");
}

/// Saving a draft on a big PR's diff rebuilds what it changed: 500
/// files, a thread on every other one, and a draft added to one file at
/// a time. Run with `cargo test --release -p ghtui -- --ignored draft_save_timing`.
#[test]
#[ignore = "timing; run in release"]
#[expect(clippy::print_stderr, reason = "reports the timings")]
fn draft_save_timing_on_500_files() {
    let regular = (0o100644, 0o100644);
    let path = |i: usize| format!("src/m{i}.rs");
    let files: Vec<ChangedFile> = (0..500)
        .map(|i| {
            file(
                FileStatus::Modified,
                Some(&path(i)),
                Some(&path(i)),
                regular,
            )
        })
        .collect();
    let mut doc = Doc::new(files, &HashSet::new(), Default::default());
    let old: String = (0..60)
        .map(|i| format!("    let v{i} = compute({i}, \"x\");\n"))
        .collect();
    let new: String = old.replace("compute(", "compute_fast(");
    let diff = Arc::new(FileDiff::compute(
        "m.rs",
        Some(old.as_bytes()),
        Some(new.as_bytes()),
    ));
    for i in 0..500 {
        doc.set_diff(i, diff.clone());
    }
    let mut s = state();
    s.size = (160, 50);
    open_diff(&mut s, loaded(&doc), Pos::default(), Pane::Diff);
    let threads: Vec<_> = (0..250)
        .map(|i| {
            let mut t = crate::fixtures::thread(&format!("T{i}"), Some(10), false, false);
            t.path = path(2 * i);
            t
        })
        .collect();
    let diff = s.diffs.get_mut(&DiffOf::Pr(pr())).unwrap();
    diff.edit(|i| i.threads.data = Some(threads));
    let saves: u32 = 100;
    let start = std::time::Instant::now();
    for n in 0..saves {
        let mut draft = crate::fixtures::draft(u64::from(n) + 1, 20, "Why not compute?");
        draft.path = path(n as usize * 5 % 500);
        let _ = diff.edit_review(&pr(), |i| i.review.pending.push(draft));
    }
    eprintln!("draft save: {:?}", start.elapsed() / saves);
}
