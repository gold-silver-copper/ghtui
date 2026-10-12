//! Keys on pages, the actions menu and the status bar.

use super::*;

/// On a list that can't scroll, a page down goes to its last row and
/// a page up to its first.
#[test]
fn paging_a_list_that_fits_goes_to_its_edge() {
    let mut state = with_home(3);
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
        press(&mut state, "j");
    }
    assert!(selected_text(&state).contains("README.md"));
    // ...then the README scrolls, and the last row leaves the screen.
    for _ in 0..12 {
        press(&mut state, "j");
    }
    let p = page(&state);
    assert!(p.scroll > 0);
    assert_eq!(p.selected, None, "nothing selected while reading");
    press(&mut state, "G");
    let p = page(&state);
    assert_eq!(p.scroll, p.page().height() - state.page_height());
    // Back up: rows get selected again once they're near.
    for _ in 0..20 {
        press(&mut state, "k");
    }
    assert!(page(&state).selected.is_some());
    press(&mut state, "gg");
    assert_eq!(page(&state).scroll, 0);
    assert_eq!(page(&state).selected, Some(0));
}

#[test]
fn h_l_and_u_move_like_a_file_manager() {
    let mut state = with_repo();
    press(&mut state, "j<Enter>");
    let crates = Route::Tree {
        repo: repo(),
        rev: "main".into(),
        path: "crates".into(),
    };
    assert_eq!(route(&state), crates, "l opens");
    press(&mut state, "u");
    assert_eq!(route(&state), Route::Repo(repo()), "u: the folder above");
    press(&mut state, "<C-o>");
    assert_eq!(route(&state), crates, "ctrl-o: back");
    press(&mut state, "uu");
    assert_eq!(
        route(&state),
        Route::user("gold-silver-copper"),
        "up from a repository: its owner"
    );
    press(&mut state, "<C-o><C-o>");
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
        DataKey::Issue(repo(), 14),
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
    for _ in 0..40 {
        press(&mut state, "<Down>");
    }
    assert_eq!(page(&state).selected, Some(2), "the comment box at the end");
    press(&mut state, "<Up>");
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

/// Messages that don't carry page data (rate limits after every
/// request, notices, diff progress) don't rebuild the page.
#[test]
fn only_page_data_rebuilds_pages() {
    let mut state = with_repo();
    let generation = state.data_gen;
    update(&mut state, Msg::RateLimits(RateLimits::default()));
    update(&mut state, Msg::Notice(Notice::Info("hi".into())));
    update(&mut state, Msg::Problem(Problem::Drafts, None));
    assert_eq!(state.data_gen, generation);
    update(&mut state, Msg::Viewer(Ok("me".into())));
    assert_eq!(state.data_gen, generation + 1);
}

/// A URL is only ever a URL: even one spelled like an old internal
/// action can't star a repository.
#[test]
fn urls_never_run_page_actions() {
    let mut state = with_repo();
    let cmds = state.follow(&crate::ui::page::Link::from("ghtui:star"));
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, Cmd::Api(Api::Change(Change::Star { .. }, _)))),
        "{cmds:?}"
    );
    let cmds = state.follow(&crate::ui::page::Link::Star);
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Cmd::Api(Api::Change(Change::Star { .. }, _))))
    );
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
    press(&mut state, "2gt");
    let hints: Vec<String> = state.key_hints().into_iter().map(|(_, w)| w).collect();
    assert!(
        hints.contains(&"filter".to_owned()) && hints.contains(&"tabs".to_owned()),
        "{hints:?}"
    );
}

#[test]
fn key_scopes_follow_the_screen() {
    let mut state = with_repo();
    press(&mut state, "zw");
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
