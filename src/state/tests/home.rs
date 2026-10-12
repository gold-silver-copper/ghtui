//! Home.

use super::*;

#[test]
fn home_selects_the_first_row_and_opens_it() {
    let mut state = with_home(3);
    assert_eq!(route(&state), Route::Home);
    assert!(
        selected_text(&state).contains("PR 1"),
        "a fresh page selects its first row"
    );
    press(&mut state, "j");
    assert!(selected_text(&state).contains("PR 2"));
    press(&mut state, "k");
    let cmds = fetches(press(&mut state, "<Enter>"));
    let pr = PrRef::parse("o/r#1").unwrap();
    assert_eq!(
        cmds,
        vec![
            Cmd::Api(Api::FetchPr(pr.clone())),
            fetch(DataKey::PrActivity(pr.clone()))
        ]
    );
    assert_eq!(route(&state), Route::pr(pr.clone()));
    assert_eq!(state.busy().as_deref(), Some("Loading"));

    update(
        &mut state,
        Msg::Pr(pr.clone(), Box::new(Err(ApiError::Network("down".into())))),
    );
    let text: Vec<String> = page(&state)
        .page()
        .lines
        .iter()
        .map(crate::ui::page::PageLine::text)
        .collect();
    assert!(
        text.iter().any(|l| l.contains("network error: down")),
        "{text:?}"
    );
    assert_eq!(
        press(&mut state, "r"),
        vec![
            Cmd::Api(Api::FetchViewer),
            Cmd::Api(Api::FetchPr(pr.clone()))
        ],
        "the activity is still loading"
    );

    press(&mut state, "<C-o>");
    assert_eq!(route(&state), Route::Home);
    assert!(
        selected_text(&state).contains("PR 1"),
        "the selection survives"
    );
    act(&mut state, Action::Forward);
    assert_eq!(route(&state), Route::pr(pr), "forward again");
    press(&mut state, "gh");
    assert!(!state.quit);
    // q closes a view; ZZ, or :q in the palette, quits.
    press(&mut state, "q");
    assert!(!state.quit);
    press(&mut state, ":q<Enter>");
    assert!(state.quit, ":q quits");
    state.quit = false;
    press(&mut state, "ZZ");
    assert!(state.quit);
}

/// A section shows its search's first rows; the rest are a row that
/// opens the search's own list in ghtui: the same fetch, so the same
/// order and count.
#[test]
fn a_section_opens_the_rest_of_its_list() {
    let mut state = state();
    crate::fixtures::section(&mut state, 1, mine(30, 31));
    state.home[1].search.as_mut().unwrap().rows = 5;
    state.data_gen += 1;
    state.sync_page();
    let text = page(&state)
        .page()
        .lines
        .iter()
        .map(crate::ui::page::PageLine::text)
        .collect::<Vec<_>>();
    assert!(
        text.iter()
            .any(|l| l.contains("Your pull requests  5 of 31")),
        "{text:?}"
    );
    press(&mut state, "G");
    assert_eq!(selected_text(&state).trim(), "… 26 more");
    let key = mine_key(&state);
    press(&mut state, "<Enter>");
    let DataKey::Search(kind, query) = key.clone() else {
        panic!("{key:?}")
    };
    assert_eq!(route(&state), Route::Search { kind, query });
    assert!(selected_text(&state).contains("PR 1"), "the same list");
    press(&mut state, "<C-o>");
    let cmds = press(&mut state, "o");
    let [Cmd::OpenUrl(url)] = cmds.as_slice() else {
        panic!("{cmds:?}")
    };
    assert!(url.contains("author:@me"), "{url}");
}

/// Sections are changed from Home: moved, renamed, removed and
/// brought back, each written to config.toml; Home then shows what the
/// file says.
#[test]
fn home_sections_are_changed_in_the_config() {
    use crate::home::Edit;
    let mut state = with_home(2);
    press(&mut state, "j");
    assert!(matches!(state.section_here(), Some((1, _))));
    let cmds = press(&mut state, "<A-Up>");
    let [Cmd::EditHome { edit, shown, .. }] = cmds.as_slice() else {
        panic!("{cmds:?}")
    };
    assert_eq!(*edit, Edit::Move { at: 1, up: true });
    assert_eq!(*shown, crate::home::defaults());
    assert!(
        press(&mut state, "<A-Up>").is_empty(),
        "one change at a time"
    );
    let mut moved = crate::home::defaults();
    moved.swap(0, 1);
    update(
        &mut state,
        Msg::HomeEdited {
            edit: edit.clone(),
            result: Ok((moved.clone(), None)),
            what: "Moved".into(),
        },
    );
    assert_eq!(state.home, moved);
    assert!(
        matches!(state.section_here(), Some((0, _))),
        "the selection moved with it"
    );
    let cmds = press(&mut state, "<Delete>");
    let [Cmd::EditHome { edit, .. }] = cmds.as_slice() else {
        panic!("{cmds:?}")
    };
    assert_eq!(*edit, Edit::Remove { at: 0 });
    let (removed, table) = (moved[1..].to_vec(), "[[home]]\ntitle = \"x\"".to_owned());
    update(
        &mut state,
        Msg::HomeEdited {
            edit: edit.clone(),
            result: Ok((removed, Some(table.clone()))),
            what: "Removed".into(),
        },
    );
    let restore = Edit::Restore { at: 0, table };
    let cmds = press(&mut state, "<C-z>");
    assert!(
        matches!(cmds.as_slice(), [Cmd::EditHome { edit, .. }] if *edit == restore),
        "{cmds:?}"
    );
    // A change the file refused says why, and can be tried again.
    update(
        &mut state,
        Msg::HomeEdited {
            edit: restore.clone(),
            result: Err("config.toml changed since ghtui read it".into()),
            what: String::new(),
        },
    );
    warns(&state, "changed since");
    let cmds = press(&mut state, "<C-z>");
    assert!(
        matches!(cmds.as_slice(), [Cmd::EditHome { edit, .. }] if *edit == restore),
        "{cmds:?}"
    );
}

/// Any list of issues, pull requests or repositories can be saved to
/// Home, under a title you give it.
#[test]
fn a_list_is_saved_to_home() {
    let mut state = state();
    let _ = state.push(Route::Search {
        kind: crate::api::browse::SearchKind::Issues,
        query: "repo:o/r is:pr label:bug".into(),
    });
    let _ = act(&mut state, Action::SaveSection);
    let cmds = press(&mut state, "<C-u>Bugs<Enter>");
    assert!(state.overlay.is_none(), "the prompt closed");
    assert_eq!(
        cmds,
        vec![Cmd::EditHome {
            path: "config.toml".into(),
            shown: crate::home::defaults(),
            edit: crate::home::Edit::Add {
                title: "Bugs".into(),
                kind: crate::api::browse::SearchKind::Pulls,
                query: "repo:o/r is:pr label:bug".into(),
            },
            what: "Added “Bugs” to Home".into(),
        }]
    );
    let _ = state.push(Route::user("octocat"));
    let _ = act(&mut state, Action::SaveSection);
    assert!(state.overlay.is_none());
}

/// Home on screen fetches its sections again once they're old, one
/// fetch at a time; a Home in another tab or history doesn't.
#[test]
fn home_stays_fresh_only_on_screen() {
    static NOW: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1000);
    let mut state = state();
    state.clock = || NOW.load(std::sync::atomic::Ordering::Relaxed);
    let _ = state.load_visible(false);
    let cmds = timers(&mut state);
    let home = Cmd::Timer(Timer::Home, crate::home::FRESH_SECS * 1000);
    assert!(cmds.contains(&home), "{cmds:?}");
    assert!(!timers(&mut state).contains(&home), "one timer");
    // Still loading: nothing more is asked.
    NOW.store(
        1000 + crate::home::FRESH_SECS,
        std::sync::atomic::Ordering::Relaxed,
    );
    assert!(update(&mut state, Msg::Timer(Timer::Home)).is_empty());
    crate::fixtures::section(&mut state, 1, mine(1, 1));
    NOW.store(
        1000 + 2 * crate::home::FRESH_SECS,
        std::sync::atomic::Ordering::Relaxed,
    );
    let asked = |cmds: &[Cmd], key: &DataKey| {
        cmds.iter()
            .any(|c| matches!(c, Cmd::Api(Api::Fetch { key: k, .. }) if k == key))
    };
    let cmds = update(&mut state, Msg::Timer(Timer::Home));
    assert_eq!(cmds.len(), 1, "{cmds:?}");
    assert!(asked(&cmds, &mine_key(&state)), "{cmds:?}");
    // Away from Home, its timer does nothing and isn't set again.
    let _ = state.push(Route::user("octocat"));
    crate::fixtures::section(&mut state, 1, mine(1, 1));
    NOW.store(
        1000 + 4 * crate::home::FRESH_SECS,
        std::sync::atomic::Ordering::Relaxed,
    );
    assert!(update(&mut state, Msg::Timer(Timer::Home)).is_empty());
    assert!(!timers(&mut state).contains(&home));
    // Back home, what's old is fetched again.
    let cmds = state.back();
    assert!(asked(&cmds, &mine_key(&state)), "{cmds:?}");
}

/// Home's sections from the disk cache are from an earlier session:
/// going back to Home fetches them.
#[test]
fn going_back_home_fetches_cached_sections() {
    let mut state = state();
    let key = mine_key(&state);
    let (value, fetched_at) = (Data::Search(Box::new(mine(1, 1))), 0);
    let cached = Remote::cached(Some(crate::store::Cached { value, fetched_at }));
    state.data.insert(key.clone(), cached);
    let _ = state.start_at(Target::Page(Route::Repo(repo())).into());
    let cmds = state.back();
    let asked = |c: &Cmd| matches!(c, Cmd::Api(Api::Fetch { key: k, .. }) if *k == key);
    assert!(cmds.iter().any(asked), "{cmds:?}");
}
