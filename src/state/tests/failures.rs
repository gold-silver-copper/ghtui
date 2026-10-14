//! What pages say when their data fails or is still on its way.

use super::*;
use crate::route::code;

/// Delivers a failed fetch of `key`.
fn failed(state: &mut State, key: DataKey) {
    update(
        state,
        Msg::Fetched {
            key,
            result: Err(crate::api::ApiError::Network("timed out".into())),
            cached_at: None,
        },
    );
}

/// A README that fails to load says so where it goes, under the files;
/// the overview (every repository page's header) doesn't depend on it.
#[test]
fn a_failed_readme_says_why_under_the_files() {
    let mut state = state();
    let _ = state.push(Route::Code(code::Repo(ghtui())));
    let overview = Data::Repo(Box::new(fixtures::overview()));
    fetched(&mut state, DataKey::Repo(ghtui()), overview);
    failed(&mut state, DataKey::Readme(ghtui()));
    let text = page_text(&state);
    assert!(text.contains("Cargo.toml"), "{text}");
    let why = "Couldn't load the README: network error: timed out. r tries again.";
    assert!(text.contains(why), "{text}");
    assert!(state.overview(&ghtui()).is_some());
}

/// A blame whose query fails says so, under the file that loaded.
#[test]
fn a_failed_blame_says_why_instead_of_loading() {
    let mut state = with_repo();
    let (repo, rev, path) = (ghtui(), "main".to_owned(), "src/main.rs".to_owned());
    let blob = DataKey::Blob(repo.clone(), rev.clone(), path.clone());
    let route = Route::Code(code::Blame {
        repo,
        rev,
        path,
        lines: None,
    });
    let _ = state.push(route.clone());
    fetched(&mut state, blob, Data::Blob(Box::new(fixtures::blob())));
    let Some(key) = state.needs(&route).pop() else {
        panic!("a blame fetches its blame");
    };
    failed(&mut state, key);
    let text = page_text(&state);
    assert!(!text.contains("Loading the blame…"), "{text}");
    assert!(text.contains("timed out"), "{text}");
}

/// A pull request whose checks fail to load says so on its Checks tab.
#[test]
fn failed_pr_checks_say_why_instead_of_loading() {
    let mut state = with_pr();
    let pr = PrRef::parse("gold-silver-copper/ghtui#12").unwrap();
    let _ = state.push(Route::Pr {
        pr: pr.clone(),
        tab: PrTab::Checks,
    });
    crate::fixtures::pr_fetched(&mut state, &pr, Ok(pr_detail()));
    let activity = Data::PrActivity(Box::new(fixtures::activity()));
    fetched(&mut state, DataKey::PrActivity(pr.clone()), activity);
    failed(&mut state, DataKey::PrChecks(pr));
    let text = page_text(&state);
    assert!(!text.contains("Loading checks…"), "{text}");
    assert!(text.contains("timed out"), "{text}");
}

/// While a pull request's conversation loads, the timeline under its
/// opening comment goes on to say so.
#[test]
fn a_loading_conversation_stays_on_the_timeline() {
    let mut state = with_home();
    let pr = PrRef::parse("gold-silver-copper/ghtui#12").unwrap();
    let route = Route::pr(pr.clone());
    let _ = state.push(route.clone());
    crate::fixtures::pr_fetched(&mut state, &pr, Ok(pr_detail()));
    let lines: Vec<String> = (state.build_page(&route, 100, NOW).lines.iter())
        .map(crate::ui::page::PageLine::text)
        .collect();
    let at = lines
        .iter()
        .position(|l| l.contains("Loading the conversation…"));
    let above = at.and_then(|at| lines.get(at.checked_sub(1)?));
    assert_eq!(above.map(|l| l.trim()), Some("│"), "{lines:#?}");
}

/// A search that hasn't answered yet says it's searching.
#[test]
fn a_search_in_flight_says_searching() {
    let mut state = with_repo();
    let route = Route::Search {
        kind: SearchKind::Repos,
        query: "ghtui".into(),
    };
    let _ = state.push(route.clone());
    let page = state.build_page(&route, 100, NOW);
    let text: Vec<String> = page
        .lines
        .iter()
        .map(crate::ui::page::PageLine::text)
        .collect();
    assert!(text.iter().any(|l| l.contains("Searching…")), "{text:#?}");
}

/// A workflow whose runs fail to load says so under the workflow.
#[test]
fn failed_workflow_runs_say_why_instead_of_loading() {
    let mut state = with_repo();
    let _ = state.push(Route::Workflow {
        repo: ghtui(),
        file: "ci.yml".into(),
    });
    let (workflow, _) = fixtures::workflow_runs();
    let key = DataKey::Workflow(ghtui(), "ci.yml".into());
    fetched(&mut state, key, Data::Workflow(Box::new(workflow)));
    failed(&mut state, DataKey::Runs(ghtui(), "workflow:ci.yml".into()));
    let text = page_text(&state);
    assert!(!text.contains("Loading runs…"), "{text}");
    assert!(text.contains("timed out"), "{text}");
}

/// On every page, each need that fails while the others load says why
/// where it goes, and nothing on the page looks like it's still loading.
/// One at a time, so a need shown only once another has loaded (a
/// profile's tab under the profile) fails too.
#[test]
fn every_route_says_why_when_its_needs_fail() {
    fn err<T>() -> Result<T, crate::api::ApiError> {
        Err(crate::api::ApiError::Network("timed out".into()))
    }
    let urls = [
        "https://github.com/",
        "https://github.com/o/r",
        "https://github.com/o/r/tree/main/src",
        "https://github.com/o/r/blob/main/src/main.rs",
        "https://github.com/o/r/blame/main/src/main.rs",
        "https://github.com/o/r/issues",
        "https://github.com/o/r/pulls",
        "https://github.com/search?q=tui&type=repositories",
        "https://github.com/o/r/issues/1",
        "https://github.com/o/r/pull/2",
        "https://github.com/o/r/pull/2/commits",
        "https://github.com/o/r/pull/2/checks",
        "https://github.com/o/r/stargazers",
        "https://github.com/o/r/watchers",
        "https://github.com/o/r/forks",
        "https://github.com/o/r/releases",
        "https://github.com/o/r/releases/tag/v1",
        "https://github.com/o/r/tags",
        "https://github.com/o/r/branches",
        "https://github.com/o/r/milestones",
        "https://github.com/o/r/milestone/1",
        "https://github.com/o/r/labels",
        "https://github.com/o/r/fork",
        "https://github.com/new",
        "https://github.com/o/r/milestones/new",
        "https://github.com/o/r/milestones/1/edit",
        "https://github.com/o/r/edit/main/README.md",
        "https://github.com/o/r/compare/a...b",
        "https://github.com/o/r/wiki",
        "https://github.com/o/r/security/advisories",
        "https://github.com/o/r/security/advisories/GHSA-2222-3333-4444",
        "https://github.com/o/r/settings",
        "https://github.com/o/r/settings/branches",
        "https://github.com/o/r/settings/access",
        "https://github.com/o/r/settings/hooks",
        "https://github.com/o/r/settings/hooks/11",
        "https://github.com/o/r/settings/keys",
        "https://github.com/orgs/o/teams",
        "https://github.com/orgs/o/teams/t",
        "https://gist.github.com/o/0123456789abcdef0123",
        "https://gist.github.com/o",
        "https://github.com/o/r/deployments",
        "https://github.com/o/r/discussions",
        "https://github.com/o/r/discussions/3",
        "https://github.com/o/r/actions",
        "https://github.com/o/r/actions/runs/4",
        "https://github.com/o/r/actions/runs/4/job/5",
        "https://github.com/o/r/actions/workflows/ci.yml",
        "https://github.com/o/r/commit/0123456789abcdef0123456789abcdef01234567/checks",
        "https://github.com/o/r/commits/main",
        "https://github.com/o/r/commit/0123456789abcdef0123456789abcdef01234567",
        "https://github.com/o",
        "https://github.com/o?tab=followers",
        "https://github.com/o?tab=repositories",
        "https://github.com/orgs/o/projects",
        "https://github.com/o/r/projects",
        "https://github.com/orgs/o/projects/1",
        "https://github.com/orgs/o/projects/1/views/2?pane=issue&itemId=1003",
    ];
    for url in urls {
        let crate::route::Target::Page(route) = crate::route::Dest::from_url(url).target else {
            panic!("{url} isn't a page");
        };
        // The page a plain URL turns out to be.
        let route = route.split(None).unwrap_or(route);
        let all = state().needs(&route);
        for failing in &all {
            let mut state = state();
            let _ = state.push(route.clone());
            for key in all.iter().cloned() {
                let fail = &key == failing;
                let msg = Msg::Fetched {
                    result: if fail {
                        err()
                    } else {
                        Ok(crate::fixtures::sample(&key))
                    },
                    key,
                    cached_at: None,
                };
                update(&mut state, msg);
            }
            let text = page_text(&state);
            let at = format!("{url}, {failing:?} failing:\n{text}");
            assert!(!text.contains("Loading"), "{at}");
            assert_eq!(
                text.contains("timed out"),
                !decoration(&route, failing),
                "{at}"
            );
        }
    }
}

/// A need whose page leaves it out until it loads, saying nothing when it
/// fails: what the header or the chrome shows (counts, tabs, a branch, a
/// commit's title), or the latest commit beside each file.
fn decoration(route: &Route, need: &DataKey) -> bool {
    use DataKey as K;
    crate::browse::is_header(route, need)
        || matches!(
            (route, need),
            (_, K::LastCommits(..))
                | (Route::Code(code::CommitChecks { .. }), K::Commit(..))
                | (
                    Route::Pr {
                        tab: PrTab::Checks,
                        ..
                    },
                    K::PrActivity(_)
                )
        )
}
