//! The palette, the search box and pickers.

use super::*;

#[test]
fn palette_goes_places_and_runs_actions() {
    let mut state = with_prs(1);
    press(&mut state, ":");
    assert!(matches!(state.overlay, Some(Overlay::Picker(_))));
    let cmds = fetches(press(&mut state, "a/b#9<Enter>"));
    let key = DataKey::Issue(RepoId::new("a", "b"), 9);
    assert_eq!(
        cmds,
        vec![
            fetch(DataKey::Repo(RepoId::new("a", "b"))),
            fetch(DataKey::InProjects(RepoId::new("a", "b"), 9)),
            fetch(key.clone())
        ]
    );
    assert!(state.overlay.is_none());

    // The number is a pull request: GitHub's redirect.
    let cmds = fetched(&mut state, key, Data::Issue(None));
    let pr = PrRef::parse("a/b#9").unwrap();
    assert_eq!(route(&state), Route::pr(pr.clone()));
    assert_eq!(cmds[0], crate::fixtures::fetch_pr(&pr));
    assert_eq!(state.screens.len(), 2, "replaced, not pushed");

    // A bare number resolves against the repo on screen.
    press(&mut state, ":12<Enter>");
    assert_eq!(
        route(&state),
        Route::Issue {
            repo: RepoId::new("a", "b"),
            number: 12
        }
    );

    press(&mut state, ":@octocat<Enter>");
    assert_eq!(route(&state), Route::user("octocat"));

    press(&mut state, ":actions menu<Enter>");
    assert!(matches!(state.overlay, Some(Overlay::Menu(_))));
    press(&mut state, "<Esc>");
    assert!(state.overlay.is_none());
    press(&mut state, "<C-k>");
    assert!(
        matches!(state.overlay, Some(Overlay::Picker(_))),
        "GitHub's palette key"
    );
}

/// Live suggestions answer in any order: an answer for an older input
/// doesn't replace the one for what's typed.
#[test]
fn an_older_suggestion_reply_does_not_replace_a_newer_one() {
    let mut state = with_repo();
    press(&mut state, "g/");
    press(&mut state, "ra");
    let cmds = update(&mut state, Msg::SuggestDue("ra".into()));
    assert_eq!(cmds, vec![Cmd::Api(Api::Suggest("ra".into()))]);
    press(&mut state, "t");
    let cmds = update(&mut state, Msg::SuggestDue("rat".into()));
    assert_eq!(cmds, vec![Cmd::Api(Api::Suggest("rat".into()))]);
    let repos = |name: &str| Ok(vec![crate::fixtures::repo_summary(name, 5)]);
    update(&mut state, Msg::Suggested("rat".into(), repos("o/ratatui")));
    update(&mut state, Msg::Suggested("ra".into(), repos("o/rails")));
    let sb = overlay!(state, Search);
    let picks: Vec<nav::Pick> = state
        .suggestions(sb)
        .into_iter()
        .filter_map(|(_, p)| p)
        .collect();
    assert_eq!(
        picks.last(),
        Some(&nav::Pick::Go(
            Target::Page(Route::Repo(RepoId::new("o", "ratatui"))).into()
        )),
        "the suggestions for `rat` vanished"
    );
}

#[test]
fn palette_searches_github() {
    let state = state();
    let choices: Vec<Choice> = state
        .commands("ratatui widgets")
        .into_iter()
        .filter_map(|(_, c)| c)
        .collect();
    assert!(choices.contains(&Choice::Search("ratatui widgets".into())));
    assert_eq!(
        state.commands("ratatui/ratatui")[0].1,
        Some(Choice::Go(
            Target::Page(Route::Repo(RepoId::new("ratatui", "ratatui"))).into()
        ))
    );
}

/// A comment's link given to the palette opens at the comment.
#[test]
fn palette_links_keep_their_anchor() {
    let mut s = state();
    let _ = s.open_picker(picker::Kind::Commands);
    press(
        &mut s,
        "https://github.com/o/r/issues/1#issuecomment-5<Enter>",
    );
    assert_eq!(
        route(&s),
        Route::Issue {
            repo: RepoId::new("o", "r"),
            number: 1
        }
    );
    assert_eq!(page(&s).anchor.as_deref(), Some("issuecomment-5"));
}

#[test]
fn palette_fuzzy_matches() {
    let state = state();
    let labels: Vec<String> = state
        .commands("open on")
        .into_iter()
        .map(|(item, _)| item.label)
        .collect();
    assert_eq!(labels.first().map(String::as_str), Some("Open on GitHub"));
    assert!(fuzzy_score("xyz", "Refresh").is_none());
    assert!(fuzzy_score("rfr", "Refresh").is_some());
}

#[test]
fn palette_list_stays_in_bounds() {
    let mut state = state();
    press(&mut state, ":");
    for _ in 0..(state.commands("").len() + PALETTE_ROWS) {
        press(&mut state, "<Down>");
    }
    let palette = overlay!(state, Picker);
    assert_eq!(palette.selected, state.commands("").len() - 1);
}

/// A list the picker couldn't fetch says why, rather than loading for
/// good, and opening the picker again asks again.
#[test]
fn a_picker_list_that_failed_says_why() {
    let mut state = with_repo();
    for (keys, key, why) in [
        (
            "f",
            DataKey::Files(ghtui(), "main".into()),
            "Couldn't load files",
        ),
        ("B", DataKey::Refs(ghtui()), "Couldn't load branches"),
    ] {
        assert_eq!(press(&mut state, keys), vec![fetch(key.clone())]);
        let result = Err(ApiError::Network("timed out".into()));
        let cached_at = None;
        let _ = update(
            &mut state,
            Msg::Fetched {
                key: key.clone(),
                result,
                cached_at,
            },
        );
        let f = overlay!(state, Picker);
        let rows: Vec<String> = state
            .picker_rows(f)
            .into_iter()
            .map(|(i, _)| i.label)
            .collect();
        assert_eq!(rows, [format!("{why}: network error: timed out")]);
        press(&mut state, "<Esc>");
        assert_eq!(press(&mut state, keys), vec![fetch(key)]);
        press(&mut state, "<Esc>");
    }
}

#[test]
fn go_to_file_and_switch_branches() {
    let mut state = with_repo();
    let cmds = press(&mut state, "f");
    let files = DataKey::Files(ghtui(), "main".into());
    assert_eq!(cmds, vec![fetch(files.clone())]);
    fetched(
        &mut state,
        files,
        Data::Files(
            Arc::new(vec![
                "src/main.rs".into(),
                "crates/ui/src/main_view.rs".into(),
                "README.md".into(),
            ]),
            false,
        ),
    );
    press(&mut state, "main");
    let f = overlay!(state, Picker);
    let rows: Vec<String> = state
        .picker_rows(f)
        .into_iter()
        .map(|(i, _)| i.label)
        .collect();
    assert_eq!(rows, ["src/main.rs", "crates/ui/src/main_view.rs"]);
    press(&mut state, "<Enter>");
    assert_eq!(route(&state), blob("main", "src/main.rs"));
    let cmds = press(&mut state, "B");
    assert_eq!(cmds, vec![fetch(DataKey::Refs(ghtui()))]);
    fetched(
        &mut state,
        DataKey::Refs(ghtui()),
        Data::Refs(Box::new(crate::api::browse::Refs {
            branches: crate::api::model::Capped::new(vec!["main".into(), "next".into()], 300),
            tags: vec!["v1.0".into()].into(),
        })),
    );
    // Branches it didn't fetch can't be picked, and it says so.
    let f = overlay!(state, Picker);
    let rows: Vec<String> = state
        .picker_rows(f)
        .into_iter()
        .map(|(i, _)| i.label)
        .collect();
    assert_eq!(
        rows.last().map(String::as_str),
        Some("298 more branches on GitHub")
    );
    press(&mut state, "next<Enter>");
    assert_eq!(
        route(&state),
        blob("next", "src/main.rs"),
        "the same file on the other branch"
    );
}

#[test]
fn search_box_jumps_searches_and_loads_more() {
    let mut state = with_repo();
    press(&mut state, "g/");
    let sb = overlay!(state, Search);
    assert!(!sb.filter);
    press(&mut state, "bug");
    let sb = overlay!(state, Search);
    let rows: Vec<Option<nav::Pick>> = state.suggestions(sb).into_iter().map(|(_, p)| p).collect();
    assert_eq!(
        rows.iter().flatten().next(),
        Some(&nav::Pick::Search(
            crate::api::browse::SearchKind::Issues,
            "repo:gold-silver-copper/ghtui bug".into()
        )),
        "on a repository, its own issues first"
    );
    // Live suggestions arrive after a pause, below what's there.
    let cmds = update(&mut state, Msg::SuggestDue("bug".into()));
    assert_eq!(cmds, vec![Cmd::Api(Api::Suggest("bug".into()))]);
    update(
        &mut state,
        Msg::Suggested(
            "bug".into(),
            Ok(vec![crate::fixtures::repo_summary("o/bugs", 5)]),
        ),
    );
    let sb = overlay!(state, Search);
    let picks: Vec<nav::Pick> = state
        .suggestions(sb)
        .into_iter()
        .filter_map(|(_, p)| p)
        .collect();
    assert_eq!(
        picks.last(),
        Some(&nav::Pick::Go(
            Target::Page(Route::Repo(RepoId::new("o", "bugs"))).into()
        ))
    );
    press(&mut state, "<Enter>");
    let route = route(&state);
    let (kind, query) = route.search().unwrap();
    let key = DataKey::Search(kind, query);
    fetched(
        &mut state,
        key.clone(),
        Data::Search(Box::new(crate::fixtures::issue_results(Some("c1")))),
    );
    press(&mut state, "G");
    assert!(selected_text(&state).starts_with("Load more"));
    let cmds = press(&mut state, "<Enter>");
    assert_eq!(
        cmds,
        vec![Cmd::Api(Api::FetchMore {
            key: key.clone(),
            after: "c1".into()
        })]
    );
    // The next page: other issues.
    let mut more = crate::fixtures::issue_results(None);
    if let SearchResults::Issues(r) = &mut more {
        for i in &mut r.items {
            i.number += 100;
        }
    }
    update(
        &mut state,
        Msg::FetchedMore(key.clone(), "c1".into(), Ok(Data::Search(Box::new(more)))),
    );
    let Some(Data::Search(results)) = state.get(&key) else {
        panic!()
    };
    assert_eq!(browse::next_cursor(results), None);
    let SearchResults::Issues(r) = &**results else {
        panic!()
    };
    assert_eq!(r.items.len(), 6);

    // Loading more doesn't block a refresh, and a next page that
    // arrives after the refresh (for the old list) is dropped.
    press(&mut state, "G");
    fetched(
        &mut state,
        key.clone(),
        Data::Search(Box::new(crate::fixtures::issue_results(Some("c2")))),
    );
    press(&mut state, "G<Enter>");
    assert_eq!(state.busy().as_deref(), Some("Loading more"));
    let cmds = press(&mut state, "r");
    assert!(!crate::fixtures::asked(&cmds).is_empty(), "{cmds:?}");
    fetched(
        &mut state,
        key.clone(),
        Data::Search(Box::new(crate::fixtures::issue_results(Some("c2")))),
    );
    update(
        &mut state,
        Msg::FetchedMore(
            key.clone(),
            "c2".into(),
            Ok(Data::Search(Box::new(crate::fixtures::issue_results(None)))),
        ),
    );
    let Some(Data::Search(results)) = state.get(&key) else {
        panic!()
    };
    let SearchResults::Issues(r) = &**results else {
        panic!()
    };
    assert_eq!(r.items.len(), 3, "the stale page wasn't appended");
}

#[test]
fn visited_pages_come_back_as_suggestions() {
    let mut state = with_repo();
    let _ = state.push(Route::user("octocat"));
    let _ = state.push(Route::Home);
    press(&mut state, "g/");
    let sb = overlay!(state, Search);
    let labels: Vec<String> = state
        .suggestions(sb)
        .into_iter()
        .filter_map(|(row, _)| match row {
            crate::ui::chrome::SuggestRow::Item { label, .. } => Some(label),
            crate::ui::chrome::SuggestRow::Heading(_) => None,
        })
        .collect();
    assert!(labels.contains(&"@octocat".to_owned()), "{labels:?}");
    assert!(
        labels.contains(&"gold-silver-copper/ghtui".to_owned()),
        "{labels:?}"
    );
    press(&mut state, "octo<Enter>");
    assert_eq!(route(&state), Route::user("octocat"));
    // Its letters scattered in a visited page's title aren't that page:
    // ↵ searches for what's typed.
    let _ = state.push(Route::Home);
    press(&mut state, "g/occat<Enter>");
    assert!(route(&state).search().is_some(), "{:?}", route(&state));
}

/// A URL or GitHub path typed after a search's `repo:o/r`, or after a
/// list's filter, is noticed: `↵` goes there instead of searching for it.
#[test]
fn a_link_typed_after_a_scope_or_a_filter_opens() {
    let pr = Route::pr(PrRef::parse("ratatui/ratatui#5").unwrap());
    let mut state = with_repo();
    press(
        &mut state,
        "g/repo:gold-silver-copper/ghtui https://github.com/ratatui/ratatui/pull/5<Enter>",
    );
    assert_eq!(route(&state), pr);

    let search = Route::Search {
        kind: crate::api::browse::SearchKind::Issues,
        query: "repo:gold-silver-copper/ghtui bug".into(),
    };
    let _ = state.push(search.clone());
    press(&mut state, "/");
    assert!(overlay!(state, Search).filter);
    press(&mut state, "ratatui/ratatui/pull/5<Enter>");
    assert_eq!(route(&state), pr);

    // Words stay a filter.
    let _ = state.push(search);
    press(&mut state, "/label:bug<Enter>");
    let Route::Search { query, .. } = route(&state) else {
        panic!("not a search");
    };
    assert_eq!(query, "repo:gold-silver-copper/ghtui bug label:bug");
}

/// People aren't open or closed: `t` says so on a search for them,
/// rather than adding `is:open` to it.
#[test]
fn a_people_search_has_no_open_or_closed() {
    let mut state = with_repo();
    let route = Route::Search {
        kind: crate::api::browse::SearchKind::Users,
        query: "octo".into(),
    };
    let _ = state.push(route.clone());
    assert!(press(&mut state, "t").is_empty());
    assert_eq!(super::route(&state), route);
    says(&state, "People aren't open or closed");
}
