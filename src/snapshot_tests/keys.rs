use crate::api::model::PrRef;
use crate::route::Route;
use crate::theme::{ColorDepth, Mode};
use crate::ui::diff_doc::Pos;

use super::diff::screen_state;
use super::{
    press, render, with_advisories, with_advisory, with_blame, with_branches, with_commit,
    with_compare, with_deployments, with_discussion, with_discussions, with_gist, with_gists,
    with_issues, with_job, with_milestone, with_milestones, with_notifications, with_pr,
    with_pr_review, with_run, with_search, with_team, with_teams, with_wiki, with_workflow,
};
use crate::diff_screen::Pane;
use crate::keymap::Action;
use crate::state::{Overlay, Screen, State, apply};

type Make = fn() -> State;

/// A list, a pull request, the diff with each pane focused (the
/// selection away from the edges so moving can move), and every page
/// links lead to.
fn screens() -> [(&'static str, Make); 27] {
    fn moved(mut s: State) -> State {
        press(&mut s, "jj");
        s
    }
    /// A page in a short terminal, scrolled into its middle, so every
    /// way of moving can move.
    fn mid(s: State) -> State {
        let mut s = super::sized(s, 100, 12);
        press(&mut s, "j");
        s
    }
    [
        ("issue list", || moved(with_issues(Mode::Dark))),
        ("pull request", || moved(with_pr(Mode::Dark))),
        ("review", || moved(with_pr_review(Mode::Dark))),
        ("diff", || {
            let at = Pos { file: 0, row: 4 };
            screen_state(Mode::Dark, ColorDepth::TrueColor, at, Pane::Diff)
        }),
        ("file tree", || {
            let at = Pos { file: 2, row: 0 };
            moved(screen_state(
                Mode::Dark,
                ColorDepth::TrueColor,
                at,
                Pane::Tree,
            ))
        }),
        ("commit", || with_commit(Mode::Dark)),
        ("commit files", || {
            let mut s = with_commit(Mode::Dark);
            press(&mut s, "gtgt");
            if let Some((screen, _)) = s.diff_parts() {
                screen.cursor = Pos { file: 0, row: 4 };
            }
            s.diffs
                .insert(super::diff::commit_of(), super::diff::diff_state());
            s
        }),
        ("workflow run", || mid(with_run(Mode::Dark))),
        // From the top: a failed job opens at its first error.
        ("job", || {
            let mut s = super::sized(with_job(Mode::Dark, None), 100, 12);
            press(&mut s, "ggjj");
            s
        }),
        ("workflow", || mid(with_workflow(Mode::Dark))),
        ("discussions", || mid(with_discussions(Mode::Dark))),
        ("discussion", || mid(with_discussion(Mode::Dark))),
        ("branches", || mid(with_branches(Mode::Dark))),
        ("milestones", || mid(with_milestones(Mode::Dark))),
        ("milestone", || mid(with_milestone(Mode::Dark))),
        ("deployments", || mid(with_deployments(Mode::Dark))),
        ("comparison", || mid(with_compare(Mode::Dark))),
        // Scrolled off its line, which it otherwise follows to the
        // bottom when resized.
        ("blame", || {
            let mut s = super::sized(with_blame(Mode::Dark), 100, 12);
            press(&mut s, "ggj");
            s
        }),
        ("gist", || mid(with_gist(Mode::Dark))),
        ("gists", || mid(with_gists(Mode::Dark))),
        ("teams", || mid(with_teams(Mode::Dark))),
        ("team", || mid(with_team(Mode::Dark))),
        ("advisories", || mid(with_advisories(Mode::Dark))),
        ("notifications", || mid(with_notifications(Mode::Dark))),
        ("advisory", || mid(with_advisory(Mode::Dark))),
        ("wiki", || mid(with_wiki(Mode::Dark))),
        ("code search", || {
            let results = crate::fixtures::code_results();
            mid(with_search(
                Mode::Dark,
                crate::api::browse::SearchKind::Code,
                "q",
                results,
            ))
        }),
    ]
}

/// Every action does something on every screen, or says why not.
#[test]
fn no_action_is_silent() {
    let mut silent = Vec::new();
    for (name, make) in screens() {
        for &action in Action::ALL {
            let mut s = make();
            let before = render(&s);
            let cmds = apply(&mut s, action);
            let _ = s.settle();
            if cmds.is_empty() && s.notices.shown().is_none() && !s.quit && render(&s) == before {
                silent.push(format!("{name}: {}", action.name()));
            }
        }
    }
    assert!(silent.is_empty(), "silent:\n{}", silent.join("\n"));
}

/// `gt` walks a pull request's tabs into Files changed and `gT` walks
/// back; so does a count before `gt`. They don't wrap around.
#[test]
fn tabs_go_into_the_files_and_back() {
    let pairs = [("gtgtgt", "gTgTgT"), ("4gt", "1gt")];
    for (there, back) in pairs {
        let mut s = with_pr(Mode::Dark);
        let start = s.chrome().active;
        press(&mut s, there);
        assert!(matches!(s.screen(), Screen::Diff(_)), "{there}");
        press(&mut s, back);
        assert!(matches!(s.screen(), Screen::Page(_)), "{back}");
        assert_eq!(s.chrome().active, start, "{back}");
        press(&mut s, "gT");
        assert_eq!(s.chrome().active, start, "no wrapping");
    }
}

/// A commit's tabs work the same: into its files and back.
#[test]
fn commit_tabs_go_into_the_files_and_back() {
    let mut s = with_commit(Mode::Dark);
    press(&mut s, "gt");
    assert!(matches!(
        s.route(),
        Some(crate::route::Route::CommitChecks { .. })
    ));
    press(&mut s, "gt");
    assert!(
        matches!(s.screen(), Screen::Diff(d) if d.of == super::diff::commit_of()),
        "the commit's diff, by its full ID"
    );
    press(&mut s, "gTgT");
    assert!(matches!(
        s.route(),
        Some(crate::route::Route::Commit { .. })
    ));
}

/// `esc` cancels and never leaves the page; `ctrl-o` goes back.
#[test]
fn esc_never_leaves_the_page() {
    let mut s = with_pr(Mode::Dark);
    let depth = s.screens.len();
    press(&mut s, "<Esc>g<Esc>3<Esc><Esc>");
    assert_eq!(s.screens.len(), depth);
    assert!(s.typed.is_empty() && s.count.is_none());
    press(&mut s, "<C-o>");
    assert_eq!(s.screens.len(), depth - 1);
}

/// A page's tabs aren't history, and each keeps its place: back in
/// Files changed, the cursor is where it was left.
#[test]
fn page_tabs_keep_their_place() {
    let mut s = with_pr(Mode::Dark);
    let depth = s.screens.len();
    press(&mut s, "4gt");
    let at = Pos { file: 2, row: 3 };
    if let Screen::Diff(d) = s.screen_mut() {
        d.cursor = at;
    }
    press(&mut s, "1gt");
    assert!(matches!(s.screen(), Screen::Page(_)));
    press(&mut s, "4gt");
    assert!(matches!(s.screen(), Screen::Diff(d) if d.cursor == at));
    assert_eq!(s.screens.len(), depth, "no history");
}

/// After a key that starts sequences, a panel lists what can follow;
/// a count goes before.
#[test]
fn sequences_say_what_can_follow() {
    let mut s = with_pr(Mode::Dark);
    press(&mut s, "2g");
    let (typed, rows) = s.which_key().unwrap();
    assert_eq!(typed, "2g…");
    assert!(rows.iter().any(|r| r.key == "t"), "{rows:?}");
    // Only what does something here: `gb` is a file's.
    assert!(!rows.iter().any(|r| r.key == "b"), "{rows:?}");
    press(&mut s, "t");
    assert_eq!(s.chrome().active, Some(1), "2gt: the second tab");
    let depth = s.screens.len();
    press(&mut s, "z");
    press(&mut s, "q");
    assert_eq!(
        s.screens.len(),
        depth,
        "zq is no binding; q in it doesn't close"
    );
    // A key of another screen says so, not what it would do there.
    press(&mut s, "w");
    let said = format!("{:?}", s.notices.shown());
    assert!(said.contains("w does nothing here"), "{said}");
}

/// A file's lines are rows: a count goes to one, `y` copies its link,
/// `V` a range's, and `gb` blames from there.
#[test]
fn file_lines_copy_and_blame() {
    let mut s = super::with_file(Mode::Dark);
    press(&mut s, "3gg");
    let copied = |s: &mut State, keys: &str| match press(s, keys).as_slice() {
        [crate::state::Cmd::Copy(url)] => url.clone(),
        cmds => panic!("{cmds:?}"),
    };
    assert!(copied(&mut s, "y").ends_with("src/main.rs#L3"));
    assert!(copied(&mut s, "Vjjy").ends_with("src/main.rs#L3-L5"));
    press(&mut s, "<Esc>gb");
    assert!(matches!(
        s.route(),
        Some(Route::Blame {
            lines: Some((5, 5)),
            ..
        })
    ));
}

/// The selected line, and the word under the cursor.
fn word_here(s: &State) -> (Option<u32>, Option<String>) {
    let Screen::Page(p) = s.screen() else {
        panic!("not on a page")
    };
    let word = p
        .word_cursor()
        .map(|(_, code, w)| code.get(w.bytes).unwrap().to_owned());
    (p.selected.and_then(|i| p.line_of(i)), word)
}

/// In a file, `w` and `b` move a word cursor along the line; `*` goes
/// to the word's next line below, the cursor on it, and `#` back up.
#[test]
fn words_in_a_file() {
    let mut s = super::with_file(Mode::Dark);
    press(&mut s, "1ggwww");
    assert_eq!(word_here(&s), (Some(1), Some("::".into())));
    press(&mut s, "wwwb");
    assert_eq!(word_here(&s), (Some(1), Some("io".into())));
    press(&mut s, "*");
    assert_eq!(word_here(&s), (Some(4), Some("io".into())));
    assert!(format!("{:?}", s.notices.shown()).contains("Match 2 of 2"));
    press(&mut s, "#");
    assert_eq!(word_here(&s), (Some(1), Some("io".into())));
    // `n` after `#` goes on up: there's nothing above.
    press(&mut s, "n");
    assert!(format!("{:?}", s.notices.shown()).contains("No earlier match"));
}

/// `gx` opens the web address under the word cursor; elsewhere on
/// the line, its link.
#[test]
fn gx_opens_the_address_under_the_cursor() {
    let mut s = super::with_repo(Mode::Dark, ColorDepth::TrueColor);
    let route = Route::blob(super::ghtui(), "main".into(), "a.rs".into());
    let blob = crate::api::browse::Blob {
        path: "a.rs".into(),
        text: Some("// See https://example.com/docs.\n".into()),
        size: 32,
        truncated: false,
    };
    super::open(&mut s, route, crate::browse::Data::Blob(Box::new(blob)));
    let opened = |s: &mut State, keys: &str| match press(s, keys).as_slice() {
        [crate::state::Cmd::OpenUrl(url)] => url.clone(),
        cmds => panic!("{cmds:?}"),
    };
    assert!(opened(&mut s, "1gggx").ends_with("/a.rs#L1"));
    assert_eq!(opened(&mut s, "wwwwgx"), "https://example.com/docs");
}

/// In Files changed, `w` moves along the cursor's line and `*` goes
/// to the word's next line, `#` back.
#[test]
fn words_in_the_diff() {
    let at = Pos { file: 0, row: 0 };
    let mut s = screen_state(Mode::Dark, ColorDepth::TrueColor, at, Pane::Diff);
    let code = |s: &State| {
        let Screen::Diff(screen) = s.screen() else {
            panic!("not the diff")
        };
        let doc = &s.diff().unwrap().doc;
        doc.code_at(screen.cursor, screen.half)
            .unwrap_or_default()
            .to_owned()
    };
    for _ in 0..40 {
        if code(&s).contains("pub fn new") {
            break;
        }
        press(&mut s, "j");
    }
    press(&mut s, "wwwwwww");
    let Screen::Diff(screen) = s.screen() else {
        panic!()
    };
    assert_eq!(screen.word, Some(18), "the first i32");
    press(&mut s, "*");
    assert!(code(&s).contains("as i32"), "{}", code(&s));
    press(&mut s, "#");
    assert!(code(&s).contains("pub fn new"), "{}", code(&s));
}

/// A job's log lines are rows: one a link points at is selected, `j`
/// moves line by line, `y` copies a line's link, which opens at it.
#[test]
fn log_lines_are_rows() {
    let mut s = with_job(Mode::Dark, Some((3, 2)));
    let copied = |s: &mut State| match press(s, "y").as_slice() {
        [crate::state::Cmd::Copy(url)] => url.clone(),
        cmds => panic!("{cmds:?}"),
    };
    assert!(copied(&mut s).ends_with("/actions/runs/7/job/2#step:3:2"));
    press(&mut s, "j");
    let url = copied(&mut s);
    assert!(url.ends_with("#step:3:3"), "{url}");
    press(&mut s, "<Enter>");
    assert!(format!("{:?}", s.notices.shown()).contains("Line 3 of step 3"));
    let _ = s.go(crate::route::Dest::from_url(&url));
    let (job, log) = crate::fixtures::job();
    let ghtui = super::ghtui();
    super::fetched(
        &mut s,
        crate::browse::DataKey::Job(ghtui.clone(), 2),
        crate::browse::Data::Job(Box::new(job)),
    );
    super::fetched(
        &mut s,
        crate::browse::DataKey::JobLog(ghtui, 2),
        crate::browse::Data::Log(std::sync::Arc::new(log)),
    );
    assert_eq!(copied(&mut s), url);
}

/// A blame's runs of lines are rows that open their commit.
#[test]
fn blame_runs_open_their_commit() {
    let mut s = with_blame(Mode::Dark);
    let selected = |s: &State| match s.screen() {
        Screen::Page(p) => p.selected_link().and_then(|l| l.url().map(str::to_owned)),
        Screen::Diff(_) => None,
    };
    // Opened at line 5: its run is selected.
    let link = selected(&s);
    assert!(
        link.as_ref().is_some_and(|l| l.contains("/commit/bbbbbbb")),
        "{link:?}"
    );
    press(&mut s, "ggj");
    assert!(selected(&s).is_some_and(|l| l.contains("/commit/")));
    press(&mut s, "<Enter>");
    assert!(matches!(s.route(), Some(Route::Commit { .. })));
}

/// A pull request's commit goes up to it, and on to its next commit.
#[test]
fn pr_commits_step_and_go_up() {
    let mut s = with_pr(Mode::Dark);
    let pr = PrRef::parse("gold-silver-copper/ghtui#12").unwrap();
    let commit = |oid: &str| Route::Commit {
        repo: pr.repo.clone(),
        oid: oid.into(),
        pr: Some(12),
    };
    let first = "0a1b2c3d4e5f60718293a4b5c6d7e8f901234567";
    let _ = s.push(commit(first));
    press(&mut s, "]C");
    assert_eq!(
        s.route(),
        Some(&commit("1b2c3d4e5f60718293a4b5c6d7e8f9012345678a"))
    );
    press(&mut s, "u");
    assert_eq!(s.route(), Some(&Route::pr(pr)));
}

/// `>` replies quoting the selected comment, in an editor right under
/// it; Enter on a comment doesn't start writing.
#[test]
fn a_quote_reply_opens_under_the_comment() {
    use crate::ui::page::Link;
    let mut s = super::sized(super::with_issue(Mode::Dark), 100, 40);
    let on_hubot = |s: &State| match s.screen() {
        Screen::Page(p) => {
            matches!(p.selected_link(), Some(Link::Quote { author, .. }) if author == "hubot")
        }
        Screen::Diff(_) => false,
    };
    for _ in 0..10 {
        if on_hubot(&s) {
            break;
        }
        press(&mut s, "}");
    }
    assert!(on_hubot(&s), "hubot's comment is selected");
    press(&mut s, "<Enter>");
    assert!(s.overlay.is_none(), "Enter doesn't start writing");
    press(&mut s, ">");
    let Some(Overlay::Compose(c)) = &s.overlay else {
        panic!("> opens the editor")
    };
    assert!(c.text().starts_with("> @hubot wrote:"), "{}", c.text());
    let screen = render(&s);
    let row = |text: &str| screen.lines().position(|l| l.contains(text));
    let (comment, editor) = (row("Agreed. 120 columns"), row("Comment on"));
    assert!(
        comment.zip(editor).is_some_and(|(c, e)| c < e && e - c < 6),
        "the editor is just under the comment: {comment:?} {editor:?}\n{screen}"
    );
}

/// In the menu, `h` and `l` are `←` and `→`: close it, run the row.
#[test]
fn vim_keys_work_in_the_menu() {
    let mut s = with_pr(Mode::Dark);
    press(&mut s, "<Space>h");
    assert!(s.overlay.is_none());
    press(&mut s, "<Space>l");
    assert!(!matches!(s.overlay, Some(Overlay::Menu(_))));
}
