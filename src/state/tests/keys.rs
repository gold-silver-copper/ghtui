//! Keys on pages, the actions menu and the status bar.

use super::*;
use crate::route::code;
use crate::ui::diff_doc::Pos;

/// On a list that can't scroll, a page down goes to its last row and
/// a page up to its first.
#[test]
fn paging_a_list_that_fits_goes_to_its_edge() {
    let mut state = with_prs(3);
    press(&mut state, "<PageDown>");
    assert!(selected_text(&state).contains("PR 3"));
    press(&mut state, "<PageUp>");
    assert!(selected_text(&state).contains("PR 1"));
}

#[test]
fn j_moves_between_rows_and_scrolls_through_text() {
    let mut state = with_repo();
    state.size = (100, 20);
    update(&mut state, Msg::Resize(100, 20));
    let first = selected_text(&state);
    assert!(first.contains(".github"), "{first}");
    // Through the files...
    for _ in 0..3 {
        press(&mut state, "<Down>");
    }
    assert!(selected_text(&state).contains("README.md"));
    // ...then the README scrolls, and the last row leaves the screen.
    for _ in 0..200 {
        press(&mut state, "<Down>");
    }
    let p = page(&state);
    assert!(p.scroll > 0);
    assert_eq!(p.selected, None, "nothing selected while reading");
    press(&mut state, "<End>");
    let p = page(&state);
    assert_eq!(p.scroll, p.page().height() - state.page_height());
    // Back up: rows get selected again once they're near.
    for _ in 0..20 {
        press(&mut state, "<Up>");
    }
    assert!(page(&state).selected.is_some());
    press(&mut state, "<Home>");
    assert_eq!(page(&state).scroll, 0);
    assert_eq!(page(&state).selected, Some(0));
}

#[test]
fn h_l_and_u_move_like_a_file_manager() {
    let mut state = with_repo();
    press(&mut state, "<Down><Enter>");
    let crates = Route::Code(code::Tree {
        repo: ghtui(),
        rev: "main".into(),
        path: "crates".into(),
    });
    assert_eq!(route(&state), crates, "l opens");
    press(&mut state, "u");
    assert_eq!(
        route(&state),
        Route::Code(code::Repo(ghtui())),
        "u: the folder above"
    );
    press(&mut state, "<BS>");
    assert_eq!(route(&state), crates, "ctrl-o: back");
    press(&mut state, "uu");
    assert_eq!(
        route(&state),
        Route::user("gold-silver-copper"),
        "up from a repository: its owner"
    );
    press(&mut state, "<BS><BS>");
    assert_eq!(route(&state), crates);
}

#[test]
fn arrows_move_comment_by_comment_and_enter_quotes() {
    let mut state = state();
    update(&mut state, Msg::Resize(100, 14));
    let _ = state.push(issue(14));
    let mut issue = crate::fixtures::issue();
    issue.body = (1..=30).map(|n| format!("Line {n}.\n\n")).collect();
    fetched(
        &mut state,
        DataKey::Issue(ghtui(), 14),
        Data::Issue(Some(Box::new(issue))),
    );
    assert_eq!(
        page(&state).selected,
        None,
        "reading pages start unselected"
    );
    press(&mut state, "<Down>");
    assert_eq!(page(&state).selected, Some(0), "the first comment");
    // The long first comment scrolls before the next one is selected.
    let scroll = page(&state).scroll;
    press(&mut state, "<Down>");
    assert_eq!(page(&state).selected, Some(0));
    assert!(page(&state).scroll > scroll);
    for _ in 0..200 {
        press(&mut state, "<Down>");
    }
    assert_eq!(page(&state).selected, Some(3), "the comment box at the end");
    // Past the event that a pull request will close it.
    press(&mut state, "<Up><Up>");
    assert_eq!(page(&state).selected, Some(1), "hubot's comment");
    press(&mut state, ">");
    let compose = overlay!(state, Compose);
    assert!(
        compose.text().starts_with("> @hubot wrote:\n> Agreed."),
        "{}",
        compose.text()
    );
}

/// `/` filters the menu; letters then narrow it instead of running
/// rows, and Esc stops filtering before it closes the menu.
#[test]
fn the_menu_filters_as_you_type() {
    let mut state = with_repo();
    press(&mut state, "<Space>/");
    press(&mut state, "go to f");
    let menu = overlay!(state, Menu);
    let shown: Vec<&str> = menu.shown().iter().map(|d| d.label.as_str()).collect();
    assert_eq!(shown, ["Go to file"]);
    press(&mut state, "<Esc>");
    let Some(Overlay::Menu(menu)) = &state.overlay else {
        panic!("Esc stopped filtering, not the menu")
    };
    assert!(menu.filter.is_none() && menu.shown().len() > 1);
    press(&mut state, "/gotofile");
    let cmds = press(&mut state, "<Enter>");
    assert!(
        state
            .overlay
            .as_ref()
            .is_none_or(|o| !matches!(o, Overlay::Menu(_)))
    );
    assert!(!cmds.is_empty() || matches!(state.overlay, Some(Overlay::Picker(_))));
}

/// A URL is only ever a URL: even one spelled like an old internal
/// action can't star a repository.
#[test]
fn urls_never_run_page_actions() {
    let mut state = with_repo();
    let cmds = state.follow(&crate::ui::page::Link::from("ghtui:star"));
    assert!(
        !(fixtures::changes(&cmds).iter()).any(|(c, _)| matches!(c, Change::Star { .. })),
        "{cmds:?}"
    );
    let cmds = state.follow(&crate::ui::page::Link::Star);
    assert!((fixtures::changes(&cmds).iter()).any(|(c, _)| matches!(c, Change::Star { .. })));
}

/// Saying the same thing again shows it for its whole time again; the
/// first one's timer doesn't clear it early.
#[test]
fn a_repeated_notice_gets_its_own_time() {
    let mut state = with_repo();
    press(&mut state, "<A-5>");
    let first = timers(&mut state);
    press(&mut state, "<A-5>");
    let again = timers(&mut state);
    let expires = |c: &Cmd| matches!(c, Cmd::Timer(Timer::ExpireNotice(_), _));
    assert!(again.iter().any(expires), "{again:?}");
    for cmd in first {
        if let Cmd::Timer(timer, _) = cmd {
            let _ = update(&mut state, Msg::Timer(timer));
        }
    }
    says(&state, "No tab 5");
}

/// Esc on an error dismisses it (and goes nowhere); the messages list
/// keeps it, and choosing it copies it.
#[test]
fn errors_are_dismissed_and_kept() {
    let mut state = with_repo();
    let depth = state.screens.len();
    state.error("boom");
    let _ = timers(&mut state);
    press(&mut state, "<Esc>");
    assert!(state.notices.shown().is_none());
    assert_eq!(state.screens.len(), depth, "Esc only dismissed");
    act(&mut state, Action::Messages);
    let p = overlay!(state, Picker);
    let rows = state.picker_rows(p);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0.label, "boom");
    let cmds = press(&mut state, "<Enter>");
    assert!(matches!(&cmds[..], [Cmd::Copy(t)] if t == "boom"));
}

/// A message arriving while the list is open doesn't move the choice
/// to another message.
#[test]
fn a_new_message_keeps_the_chosen_one() {
    let mut state = with_repo();
    for text in ["one", "two"] {
        state.error(text);
        let _ = timers(&mut state);
    }
    act(&mut state, Action::Messages);
    press(&mut state, "<Down>");
    state.error("three");
    let _ = timers(&mut state);
    let cmds = press(&mut state, "<Enter>");
    assert!(
        matches!(&cmds[..], [Cmd::Copy(t)] if t == "one"),
        "{cmds:?}"
    );
}

#[test]
fn the_menu_lists_and_runs_what_you_can_do() {
    let mut state = with_repo();
    press(&mut state, "<Space>");
    let menu = overlay!(state, Menu);
    let labels: Vec<&str> = menu.rows.iter().map(|d| d.label.as_str()).collect();
    assert!(labels.contains(&"Go to file"), "{labels:?}");
    assert!(labels.contains(&"Star"), "{labels:?}");
    let fwd = menu
        .rows
        .iter()
        .find(|d| d.action == Action::Forward)
        .unwrap();
    assert!(fwd.unavailable.is_some(), "nothing to go forward to");
    // A row's own key runs it.
    let cmds = press(&mut state, "f");
    assert!(matches!(state.overlay, Some(Overlay::Picker(_))));
    assert!(!cmds.is_empty());
}

#[test]
fn status_bar_hints_follow_the_context() {
    let mut state = with_repo();
    let hints: Vec<String> = state.key_hints().into_iter().map(|(_, w)| w).collect();
    assert_eq!(hints[..2], ["menu", "open"]);
    assert!(hints.contains(&"go to file".to_owned()));
    press(&mut state, "2");
    let hints: Vec<String> = state.key_hints().into_iter().map(|(_, w)| w).collect();
    assert!(
        hints.contains(&"filter".to_owned()) && hints.contains(&"tabs".to_owned()),
        "{hints:?}"
    );
}

#[test]
fn key_scopes_follow_the_screen() {
    let mut state = with_repo();
    act(&mut state, Action::IgnoreWhitespace);
    // A review key on a page says where it works.
    says(&state, "Files changed");
    press(&mut state, "i");
    assert!(
        matches!(state.overlay, Some(Overlay::Hints(_))),
        "l: link letters on a page"
    );
    let doables = state.doables();
    assert!(doables.iter().any(|d| d.action == Action::Hints));
    assert!(!doables.iter().any(|d| d.action == Action::FileComment));
}

pub(super) type Make = fn() -> State;

/// A list, a pull request, the diff with each pane focused (the
/// selection away from the edges so moving can move), and every page
/// links lead to.
pub(super) fn screens() -> [(&'static str, Make); 31] {
    fn moved(mut s: State) -> State {
        press(&mut s, "<Down><Down>");
        s
    }
    /// A page in a short terminal, scrolled into its middle, so every
    /// way of moving can move.
    fn mid(s: State) -> State {
        let mut s = super::sized(s, 100, 12);
        press(&mut s, "<Down>");
        s
    }
    [
        ("issue list", || moved(with_issues())),
        ("pull request", || moved(with_pr())),
        ("review", || moved(with_pr_review())),
        ("diff", || {
            let at = Pos { file: 0, row: 4 };
            screen_state(at, Pane::Diff)
        }),
        ("file tree", || {
            let at = Pos { file: 2, row: 0 };
            moved(screen_state(at, Pane::Tree))
        }),
        ("commit", || with_commit()),
        ("commit files", || {
            let mut s = with_commit();
            press(&mut s, "<Tab><Tab>");
            if let Some((screen, _)) = s.diff_parts() {
                screen.cursor = Pos { file: 0, row: 4 };
            }
            s.diffs.insert(commit_of(), fixture_diff());
            s
        }),
        ("workflow run", || mid(with_run())),
        // From the top: a failed job opens at its first error.
        ("job", || {
            let mut s = super::sized(with_job(None), 100, 12);
            // Past the annotation and two steps: a line of the log.
            press(&mut s, "<Home><Down><Down><Down>");
            s
        }),
        ("workflow", || mid(with_workflow())),
        ("discussions", || mid(with_discussions())),
        ("discussion", || mid(with_discussion())),
        ("branches", || mid(with_branches())),
        ("milestones", || mid(with_milestones())),
        ("milestone", || mid(with_milestone())),
        ("deployments", || mid(with_deployments())),
        ("comparison", || mid(with_compare())),
        // Scrolled off its line, which it otherwise follows to the
        // bottom when resized.
        ("blame", || {
            let mut s = super::sized(with_blame(), 100, 12);
            press(&mut s, "<Home><Down>");
            s
        }),
        ("gist", || mid(with_gist())),
        ("gists", || mid(with_gists())),
        ("teams", || mid(with_teams())),
        ("team", || mid(with_team())),
        ("advisories", || mid(with_advisories())),
        ("notifications", || mid(with_notifications())),
        ("advisory", || mid(with_advisory())),
        ("wiki", || mid(with_wiki())),
        ("new issue", || moved(super::create::with_new_issue())),
        ("new pull request", || moved(super::create::with_new_pr())),
        ("project board", || mid(super::projects::with_project(None))),
        ("project table", || {
            mid(super::projects::with_project(Some(2)))
        }),
        ("code search", || mid(with_search(SearchKind::Code, "q"))),
    ]
}

/// Every action does something on every screen, or says why not.
#[test]
fn no_action_is_silent() {
    let mut silent = Vec::new();
    for (name, make) in screens() {
        for &action in Action::ALL {
            let mut s = make();
            let before = draw(&s);
            let cmds = apply(&mut s, action);
            let _ = s.settle();
            if cmds.is_empty() && s.notices.shown().is_none() && !s.quit && draw(&s) == before {
                silent.push(format!("{name}: {}", action.name()));
            }
        }
    }
    assert!(silent.is_empty(), "silent:\n{}", silent.join("\n"));
}

/// `⇥` walks a pull request's tabs into Files changed and `⇧⇥` walks
/// back; so do their numbers. They don't wrap around.
#[test]
fn tabs_go_into_the_files_and_back() {
    let pairs = [("<Tab><Tab><Tab>", "<S-Tab><S-Tab><S-Tab>"), ("4", "1")];
    for (there, back) in pairs {
        let mut s = with_pr();
        let start = s.chrome().active;
        press(&mut s, there);
        assert!(matches!(s.screen(), Screen::Diff(_)), "{there}");
        press(&mut s, back);
        assert!(matches!(s.screen(), Screen::Page(_)), "{back}");
        assert_eq!(s.chrome().active, start, "{back}");
        press(&mut s, "<S-Tab>");
        assert_eq!(s.chrome().active, start, "no wrapping");
    }
}

/// A commit's tabs work the same: into its files and back.
#[test]
fn commit_tabs_go_into_the_files_and_back() {
    let mut s = with_commit();
    press(&mut s, "<Tab>");
    assert!(matches!(
        s.route(),
        Some(crate::route::Route::Code(code::CommitChecks { .. }))
    ));
    press(&mut s, "<Tab>");
    assert!(
        matches!(s.screen(), Screen::Diff(d) if d.of == commit_of()),
        "the commit's diff, by its full ID"
    );
    press(&mut s, "gTgT");
    assert!(matches!(
        s.route(),
        Some(crate::route::Route::Code(code::Commit { .. }))
    ));
}

/// `esc` cancels what there is to cancel, and with nothing, goes back,
/// as `⌫` does.
#[test]
fn esc_cancels_then_goes_back() {
    let mut s = with_pr();
    let depth = s.screens.len();
    press(&mut s, "/");
    press(&mut s, "<Esc>");
    assert_eq!(s.screens.len(), depth, "esc closed the search");
    press(&mut s, "<Esc>");
    assert_eq!(s.screens.len(), depth - 1);
    press(&mut s, "<A-Right>");
    assert_eq!(s.screens.len(), depth);
    press(&mut s, "<BS>");
    assert_eq!(s.screens.len(), depth - 1);
}

/// A page's tabs aren't history, and each keeps its place: back in
/// Files changed, the cursor is where it was left.
#[test]
fn page_tabs_keep_their_place() {
    let mut s = with_pr();
    let depth = s.screens.len();
    press(&mut s, "4");
    let at = Pos { file: 2, row: 3 };
    if let Screen::Diff(d) = s.screen_mut() {
        d.cursor = at;
    }
    press(&mut s, "1");
    assert!(matches!(s.screen(), Screen::Page(_)));
    press(&mut s, "4");
    assert!(matches!(s.screen(), Screen::Diff(d) if d.cursor == at));
    assert_eq!(s.screens.len(), depth, "no history");
}

/// A key of another screen says where it works, not what it would do.
#[test]
fn a_key_of_another_screen_says_where_it_works() {
    let mut s = with_pr();
    press(&mut s, "t");
    let said = format!("{:?}", s.notices.shown());
    assert!(said.contains("t does nothing here"), "{said}");
    assert!(said.contains("works on lists"), "{said}");
    press(&mut s, "2");
    assert_eq!(s.chrome().active, Some(1), "2: the second tab");
}

/// A file's lines are rows: `:3` goes to one, `l` copies its link, `s`
/// selects a range for it, and `b` blames from there.
#[test]
fn file_lines_copy_and_blame() {
    let mut s = super::with_file();
    press(&mut s, ":3<Enter>");
    let copied = |s: &mut State, keys: &str| match press(s, keys).as_slice() {
        [crate::state::Cmd::Copy(url)] => url.clone(),
        cmds => panic!("{cmds:?}"),
    };
    assert!(copied(&mut s, "l").ends_with("src/main.rs#L3"));
    assert!(copied(&mut s, "s<Down><Down>l").ends_with("src/main.rs#L3-L5"));
    press(&mut s, "<Esc>b");
    assert!(matches!(
        s.route(),
        Some(Route::Code(code::Blame {
            lines: Some((5, 5)),
            ..
        }))
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
    let mut s = super::with_file();
    press(&mut s, "<Home><C-Right><C-Right><C-Right>");
    assert_eq!(word_here(&s), (Some(1), Some("::".into())));
    press(&mut s, "<C-Right><C-Right><C-Right><C-Left>");
    assert_eq!(word_here(&s), (Some(1), Some("io".into())));
    act(&mut s, Action::SearchWord);
    assert_eq!(word_here(&s), (Some(4), Some("io".into())));
    assert!(format!("{:?}", s.notices.shown()).contains("Match 2 of 2"));
    press(&mut s, "N");
    assert_eq!(word_here(&s), (Some(1), Some("io".into())));
}

/// Opening the link under the cursor opens the web address under the
/// word cursor; elsewhere on the line, its link.
#[test]
fn gx_opens_the_address_under_the_cursor() {
    let mut s = super::with_repo();
    let route = Route::blob(super::ghtui(), "main".into(), "a.rs".into());
    let blob = crate::fixtures::file("a.rs", "// See https://example.com/docs.\n");
    super::open(&mut s, route, crate::data::Data::from(blob));
    let opened = |s: &mut State| match super::act(s, Action::OpenLink).as_slice() {
        [crate::state::Cmd::OpenUrl(url)] => url.clone(),
        cmds => panic!("{cmds:?}"),
    };
    press(&mut s, ":1<Enter>");
    assert!(opened(&mut s).ends_with("/a.rs#L1"));
    press(&mut s, "<C-Right><C-Right><C-Right><C-Right>");
    assert_eq!(opened(&mut s), "https://example.com/docs");
}

/// In Files changed, `w` moves along the cursor's line and `*` goes
/// to the word's next line, `#` back.
#[test]
fn words_in_the_diff() {
    let at = Pos { file: 0, row: 0 };
    let mut s = screen_state(at, Pane::Diff);
    let code = |s: &State| {
        let Screen::Diff(screen) = s.screen() else {
            panic!("not the diff")
        };
        let doc = &s.diff().unwrap().doc;
        doc.code_at(screen.cursor, screen.half)
            .unwrap_or_default()
            .to_owned()
    };
    for _ in 0..200 {
        if code(&s).contains("pub fn new") {
            break;
        }
        press(&mut s, "<Down>");
    }
    press(
        &mut s,
        "<C-Right><C-Right><C-Right><C-Right><C-Right><C-Right><C-Right>",
    );
    let Screen::Diff(screen) = s.screen() else {
        panic!()
    };
    assert_eq!(screen.word, Some(18), "the first i32");
    act(&mut s, Action::SearchWord);
    assert!(code(&s).contains("as i32"), "{}", code(&s));
    press(&mut s, "N");
    assert!(code(&s).contains("pub fn new"), "{}", code(&s));
}

/// A job's log lines are rows: one a link points at is selected, `j`
/// moves line by line, `y` copies a line's link, which opens at it.
#[test]
fn log_lines_are_rows() {
    let mut s = with_job(Some((3, 2)));
    let copied = |s: &mut State| match press(s, "l").as_slice() {
        [crate::state::Cmd::Copy(url)] => url.clone(),
        cmds => panic!("{cmds:?}"),
    };
    assert!(copied(&mut s).ends_with("/actions/runs/7/job/2#step:3:2"));
    press(&mut s, "<Down>");
    let url = copied(&mut s);
    assert!(url.ends_with("#step:3:3"), "{url}");
    press(&mut s, "<Enter>");
    assert!(format!("{:?}", s.notices.shown()).contains("Line 3 of step 3"));
    let _ = s.go(crate::route::Dest::from_url(&url));
    let (job, log) = crate::fixtures::job();
    let ghtui = super::ghtui();
    super::fetched(
        &mut s,
        crate::data::DataKey::Job(ghtui.clone(), 2),
        crate::data::Data::from(job),
    );
    super::fetched(
        &mut s,
        crate::data::DataKey::JobLog(ghtui, 2),
        crate::data::Data::Log(std::sync::Arc::new(log)),
    );
    assert_eq!(copied(&mut s), url);
}

/// A blame's runs of lines are rows that open their commit.
#[test]
fn blame_runs_open_their_commit() {
    let mut s = with_blame();
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
    press(&mut s, "<Home><Down>");
    assert!(selected(&s).is_some_and(|l| l.contains("/commit/")));
    press(&mut s, "<Enter>");
    assert!(matches!(s.route(), Some(Route::Code(code::Commit { .. }))));
}

/// A pull request's commit goes up to it, and on to its next commit.
#[test]
fn pr_commits_step_and_go_up() {
    let mut s = with_pr();
    let pr = PrRef::parse("gold-silver-copper/ghtui#12").unwrap();
    let commit = |oid: &str| {
        Route::Code(code::Commit {
            repo: pr.repo.clone(),
            oid: oid.into(),
            pr: Some(12),
        })
    };
    let first = "0a1b2c3d4e5f60718293a4b5c6d7e8f901234567";
    let _ = s.push(commit(first));
    act(&mut s, Action::NextCommit);
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
    let mut s = super::sized(super::with_issue(), 100, 40);
    let on_hubot = |s: &State| match s.screen() {
        Screen::Page(p) => {
            matches!(p.selected_link(), Some(Link::Quote { author, .. }) if author == "hubot")
        }
        Screen::Diff(_) => false,
    };
    for _ in 0..200 {
        if on_hubot(&s) {
            break;
        }
        press(&mut s, "<Down>");
    }
    assert!(on_hubot(&s), "hubot's comment is selected");
    press(&mut s, "<Enter>");
    assert!(s.overlay.is_none(), "Enter doesn't start writing");
    press(&mut s, ">");
    let Some(Overlay::Compose(c)) = &s.overlay else {
        panic!("> opens the editor")
    };
    assert!(c.text().starts_with("> @hubot wrote:"), "{}", c.text());
    let screen = screen_text(&s);
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
    let mut s = with_pr();
    press(&mut s, "<Space><Left>");
    assert!(s.overlay.is_none());
    press(&mut s, "<Space><Right>");
    assert!(!matches!(s.overlay, Some(Overlay::Menu(_))));
}
