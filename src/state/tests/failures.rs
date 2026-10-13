//! What pages say when their data fails or is still on its way.

use super::*;

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
    let _ = state.push(Route::Repo(ghtui()));
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
    let route = Route::Blame {
        repo,
        rev,
        path,
        lines: None,
    };
    let _ = state.push(route.clone());
    fetched(&mut state, blob, Data::Blob(Box::new(fixtures::blob())));
    let Some(Need::Data(key)) = needs(&route).pop() else {
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
    update(&mut state, Msg::Pr(pr.clone(), Box::new(Ok(pr_detail()))));
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
    update(&mut state, Msg::Pr(pr, Box::new(Ok(pr_detail()))));
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
            for need in all.iter().cloned() {
                let fail = &need == failing;
                let msg = match need {
                    Need::Pr(pr) => {
                        Msg::Pr(pr, Box::new(if fail { err() } else { Ok(pr_detail()) }))
                    }
                    Need::Data(key) => Msg::Fetched {
                        result: if fail { err() } else { Ok(sample(&key)) },
                        key,
                        cached_at: None,
                    },
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
fn decoration(route: &Route, need: &Need) -> bool {
    use DataKey as K;
    crate::browse::is_header(route, need)
        || matches!(
            (route, need),
            (_, Need::Data(K::LastCommits(..)))
                | (Route::CommitChecks { .. }, Need::Data(K::Commit(..)))
                | (
                    Route::Pr {
                        tab: PrTab::Checks,
                        ..
                    },
                    Need::Data(K::PrActivity(_))
                )
        )
}

/// What GitHub might answer for `key`, for the needs that don't fail.
fn sample(key: &DataKey) -> Data {
    use DataKey as K;
    let search = |kind: &SearchKind| match kind {
        SearchKind::Repos => fixtures::repo_results(),
        SearchKind::Issues | SearchKind::Pulls => fixtures::issue_results(None),
        SearchKind::Users => fixtures::user_results(),
        SearchKind::Discussions => fixtures::discussion_results(),
        SearchKind::Commits => fixtures::commit_results(),
        SearchKind::Code => fixtures::code_results(),
    };
    match key {
        K::Repo(_) => Data::Repo(Box::new(fixtures::overview())),
        K::Readme(_) => Data::Readme(Some(Box::new(fixtures::readme()))),
        K::Tree(..) => Data::Tree(fixtures::tree()),
        K::Blob(..) => Data::Blob(Box::new(fixtures::blob())),
        K::Search(kind, _) => Data::Search(Box::new(search(kind))),
        K::StateCounts(_) => {
            Data::StateCounts(crate::api::browse::StateCounts { open: 2, closed: 5 })
        }
        K::Issue(..) => Data::Issue(Some(Box::new(fixtures::issue()))),
        K::PrActivity(_) => Data::PrActivity(Box::new(fixtures::activity())),
        K::Reviews(_) => Data::Reviews(Box::new(fixtures::reviews())),
        K::CommitComments(..) => Data::CommitComments(Box::default()),
        K::Lines(..) => Data::Lines(Arc::default()),
        K::InProjects(..) => Data::InProjects(Box::default()),
        K::Profile(_) => Data::Profile(Box::new(fixtures::profile())),
        K::Files(..) => Data::Files(Arc::new(vec!["README.md".into()]), false),
        K::Refs(_) => Data::Refs(Box::default()),
        K::NewForm(_) => Data::NewForm(Box::new(fixtures::new_form())),
        K::RefIn(..) => Data::RefIn(None),
        K::LastCommits(..) => Data::LastCommits(Arc::new(fixtures::last_commits())),
        K::Commit(..) => Data::Commit(Box::new(fixtures::commit())),
        K::History(..) => Data::History(Box::new(fixtures::history(None))),
        K::PrChecks(_) | K::CommitChecks(..) => Data::Checks(Box::new(fixtures::checks())),
        K::Users(_) => Data::Users(Box::new(fixtures::people())),
        K::Forks(_) | K::OwnerRepos(..) | K::Stars(_) => {
            Data::RepoPage(Box::new(fixtures::profile_repos()))
        }
        K::Releases(_) => Data::Releases(Box::new(fixtures::releases())),
        K::Release(..) => Data::Release(Box::new(fixtures::release(true))),
        K::Tags(_) => Data::Tags(Box::new(fixtures::tags())),
        K::Branches(_) => Data::Branches(Box::new(fixtures::branches())),
        K::Wiki(..) => Data::Wiki(Box::new(fixtures::wiki())),
        K::Advisories(_) => {
            let items = fixtures::advisories();
            let total = items.len() as u64;
            Data::Advisories(Box::new(fixtures::page(total, items, None)))
        }
        K::Advisory(..) => Data::Advisory(Box::new(fixtures::advisory())),
        K::Sidebar(..) => Data::Sidebar(Box::new(fixtures::sidebar())),
        K::Teams(_) => Data::Teams(Box::new(fixtures::teams())),
        K::Team(..) => Data::Team(Box::new(fixtures::team())),
        K::Gist(_) => Data::Gist(Box::new(fixtures::gist())),
        K::Gists(_) => Data::Gists(Box::new(fixtures::gists())),
        K::Blame(..) => Data::Blame(Box::new(fixtures::blame())),
        K::Compare(..) => Data::Compare(Box::new(fixtures::comparison())),
        K::Deployments(..) => Data::Deployments(Box::new(fixtures::deployments())),
        K::Milestones(..) => Data::Milestones(Box::new(fixtures::milestones())),
        K::Milestone(..) => Data::Milestone(Box::new(fixtures::milestone())),
        K::Discussions(..) => Data::Discussions(Box::new(fixtures::discussions())),
        K::Discussion(..) => Data::Discussion(Box::new(fixtures::discussion())),
        K::Run(..) => Data::Run(Box::new(fixtures::workflow_run())),
        K::Job(..) => Data::Job(Box::new(fixtures::job().0)),
        K::JobLog(..) => Data::Log(Arc::new(fixtures::job().1)),
        K::Workflow(..) => Data::Workflow(Box::new(fixtures::workflow_runs().0)),
        K::Runs(..) => Data::Runs(Box::new(fixtures::workflow_runs().1)),
        K::Notifications(_) => Data::Notifications(Box::new(fixtures::notifications())),
        K::Projects(_) => Data::Projects(Box::new(fixtures::projects())),
        K::Project(..) => Data::Project(Box::new(fixtures::project())),
        K::Alerts(_, kind, _) => Data::Alerts(Box::new(crate::security::fixtures::alerts(*kind))),
        K::Alert(a) => Data::Alert(Box::new(crate::security::fixtures::alert(a.kind, a.number))),
        K::Subscription(_) => Data::Subscription(Box::new(fixtures::subscription())),
        K::Site(key) => crate::site::fixtures::sample(key),
        K::Housekeeping(k) => {
            Data::Housekeeping(Box::new(crate::housekeeping::fixtures::sample(k)))
        }
        K::Settings(_, page) => Data::Settings(Box::new(crate::settings::fixtures::page(*page))),
    }
}
