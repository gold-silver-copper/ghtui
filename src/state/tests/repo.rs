//! A repository's pages.

use super::*;

/// An empty repository says what it's about, in its sidebar on a
/// wide screen and above its note on a narrow one.
#[test]
fn an_empty_repository_shows_its_about() {
    for width in [100, 140] {
        let mut s = state();
        s.size = (width, 30);
        let _ = s.push(Route::Repo(repo()));
        let mut o = crate::fixtures::overview();
        (o.entries, o.default_branch) = (Vec::new(), None);
        fetched(&mut s, DataKey::Repo(repo()), Data::Repo(Box::new(o)));
        let page = page(&s).page();
        let aside = page.aside.iter().flat_map(|a| &a.lines);
        let lines = page.lines.iter().chain(aside);
        let text: String = lines.map(crate::ui::page::PageLine::text).collect();
        assert!(text.contains("keyboard-driven"), "{width}: {text}");
    }
}

#[test]
fn a_repositorys_pages_are_under_their_tabs() {
    let mut state = with_repo();
    let active = |state: &State| {
        let c = state.chrome();
        c.active
            .and_then(|i| c.tabs.get(i))
            .map(|(t, _)| t.label.clone())
    };
    let pages = [
        (
            Route::Milestone {
                repo: repo(),
                number: 3,
            },
            "Issues",
        ),
        (
            Route::Discussions {
                of: crate::api::browse::DiscussionsOf::Repo(repo()),
                category: None,
            },
            "Discussions",
        ),
        (Route::Advisories(Some(repo())), "Security"),
        (
            Route::Wiki {
                repo: repo(),
                page: None,
            },
            "Wiki",
        ),
        (Route::Branches(repo()), "Code"),
        (
            Route::Job {
                repo: repo(),
                run: Some(1),
                job: 2,
                step: None,
                query: String::new(),
            },
            "Actions",
        ),
    ];
    for (route, tab) in pages {
        let _ = state.push(route.clone());
        assert_eq!(active(&state).as_deref(), Some(tab), "{route:?}");
    }
    // Every tab has a number key: `5` is Actions, `8` Security.
    let _ = state.push(Route::Repo(repo()));
    press(&mut state, "5gt");
    assert_eq!(route(&state), Route::actions(repo()));
    press(&mut state, "8gt");
    assert_eq!(route(&state), Route::Advisories(Some(repo())));
}

#[test]
fn repo_tabs_filters_state_sort_and_star() {
    let mut state = with_repo();
    let tabs: Vec<String> = state
        .chrome()
        .tabs
        .iter()
        .map(|(t, _)| t.label.clone())
        .collect();
    assert_eq!(
        tabs,
        [
            "Code",
            "Issues",
            "Pull requests",
            "Discussions",
            "Actions",
            "Projects",
            "Wiki",
            "Security"
        ]
    );
    let cmds = fetches(press(&mut state, "2gt"));
    let open = issues(OPEN);
    assert_eq!(route(&state), open);
    let (kind, query) = open.search().unwrap();
    assert_eq!(
        cmds,
        vec![fetch(DataKey::Search(kind, query))],
        "the header isn't fetched again"
    );
    assert_eq!(state.chrome().active, Some(1));
    // r refreshes the list's counts too, which the header holds.
    let cmds = fetches(press(&mut state, "r"));
    let header = Cmd::Api(Api::Fetch {
        key: DataKey::Repo(repo()),
        cached: false,
    });
    assert!(cmds.contains(&header), "{cmds:?}");

    // `/` on a list edits its filter.
    press(&mut state, "/");
    let sb = overlay!(state, Search);
    assert!(sb.filter);
    assert_eq!(sb.input.lines().join(""), "is:open ");
    press(&mut state, "label:bug<Enter>");
    assert_eq!(route(&state), issues("is:open label:bug"));
    act(&mut state, Action::ToggleState);
    assert_eq!(route(&state), issues("is:closed label:bug"));
    act(&mut state, Action::Sort);
    assert_eq!(
        route(&state),
        issues("is:closed label:bug sort:created-asc")
    );

    press(&mut state, "gt");
    assert!(
        matches!(route(&state), Route::Pulls { .. }),
        "→: the next tab"
    );
    press(&mut state, "1gt");
    assert_eq!(route(&state), Route::Repo(repo()));

    let cmds = press(&mut state, ",*");
    let star = Change::Star {
        repo: repo(),
        id: NodeId::new("R_ghtui"),
        starred: true,
    };
    assert_eq!(cmds, vec![Cmd::Api(Api::Change(star, act::By::Star))]);
    assert!(state.overview(&repo()).unwrap().starred);
    assert_eq!(state.overview(&repo()).unwrap().summary.stars, 1235);
    answer(&mut state, &cmds, Err(ApiError::Network("down".into())));
    assert!(!state.overview(&repo()).unwrap().starred, "rolled back");
    assert_eq!(state.overview(&repo()).unwrap().summary.stars, 1234);
}

#[test]
fn directories_and_files_open_from_the_repo() {
    let mut state = with_repo();
    while !selected_text(&state).contains("crates") {
        press(&mut state, "j");
    }
    let cmds = fetches(press(&mut state, "<Enter>"));
    let tree = DataKey::Tree(repo(), "main".into(), "crates".into());
    assert_eq!(
        cmds,
        vec![
            fetch(tree.clone()),
            fetch(DataKey::LastCommits(repo(), "main".into(), "crates".into()))
        ],
        "the listing, and each entry's latest commit"
    );
    fetched(&mut state, tree, Data::Tree(crate::fixtures::tree()));
    while !selected_text(&state).contains("README.md") {
        press(&mut state, "j");
    }
    press(&mut state, "<Enter>");
    assert_eq!(route(&state), blob("main", "crates/README.md"));
}
