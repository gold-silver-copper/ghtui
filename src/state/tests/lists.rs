//! Lists and their next pages.

use super::*;
use crate::route::{issues, people, runs, security};

/// The repository's `src` history on main, the code toolbar's link
/// followed, and its key.
fn with_history() -> (State, DataKey) {
    let mut state = with_repo();
    let url = format!("https://github.com/{}/commits/main/src", ghtui());
    let Target::Page(route) = Dest::from_url(&url).target else { panic!("{url}") };
    let key = DataKey::History(ghtui(), "main".into(), "src".into());
    let cmds = state.push(route);
    assert!(cmds.contains(&fetch(key.clone())), "{cmds:?}");
    (state, key)
}

/// A branch's commits page: the code toolbar's link opens it, and it
/// loads more like any list.
#[test]
fn commit_history_loads_more() {
    let (mut state, key) = with_history();
    fetched(
        &mut state,
        key.clone(),
        Data::from(fixtures::history(Some("h1"))),
    );
    press(&mut state, "<End>");
    assert!(selected_text(&state).starts_with("Load more"));
    let more_at = page(&state).selected;
    let cmds = press(&mut state, "<Enter>");
    assert_eq!(cmds, [more(&key, "h1")]);
    // The next page: other commits.
    let mut more = fixtures::history(None);
    for c in &mut more.items {
        c.oid = c.oid.replace('a', "b");
    }
    update(
        &mut state,
        Msg::FetchedMore(key.clone(), "h1".into(), Ok(Data::from(more)), None),
    );
    let Some(Data::History(h)) = state.get(&key) else { panic!() };
    assert_eq!((h.items.len(), h.next.as_deref()), (4, None));
    // The first commit loaded takes the place of "Load more".
    let p = page(&state);
    assert_eq!(p.selected, more_at);
    assert!(matches!(
        p.selected_link(),
        Some(crate::ui::page::Link::Url(_))
    ));
}

/// A list extended with Load more is fetched again only when asked: a
/// pending run or a write elsewhere would cut it back to its first page.
#[test]
fn an_extended_list_is_kept_until_asked() {
    let mut state = with_repo();
    let file = "ci.yml".to_owned();
    let key = DataKey::Runs(ghtui(), format!("workflow:{file}"));
    let _ = state.push(Route::Runs(runs::Workflow {
        repo: ghtui(),
        file,
    }));
    let page = |id: u64, next: Option<&str>| {
        let mut runs = fixtures::workflow_runs().1;
        for (i, run) in runs.items.iter_mut().enumerate() {
            (run.id, run.outcome) = (id + i as u64, crate::api::browse::CheckOutcome::Pending);
        }
        runs.next = next.map(str::to_owned);
        Data::from(runs)
    };
    let refetch = refetch(key.clone());
    fetched(&mut state, key.clone(), page(10, Some("2")));
    assert!(state.load_visible(false).contains(&refetch), "pending runs");
    fetched(&mut state, key.clone(), page(10, Some("2")));
    press(&mut state, "<End>");
    press(&mut state, "<Enter>");
    update(
        &mut state,
        Msg::FetchedMore(key.clone(), "2".into(), Ok(page(20, None)), None),
    );
    let star = Change::Star {
        repo: ghtui(),
        id: NodeId::new("R_1"),
        starred: true,
    };
    fixtures::changed(&mut state, star, act::By::Star, Ok(()));
    assert!(!state.load_visible(false).contains(&refetch));
    let Some(Data::Runs(runs)) = state.get(&key) else { panic!() };
    assert_eq!(runs.items.len(), 4);
    assert!(
        state.load_visible(true).contains(&refetch),
        "r refetches it"
    );
}

/// A next page asked for before a refresh belongs to the old list: it
/// lands after the refreshed list's own Load more, and must neither be
/// appended to the new list nor make that newer page be dropped.
#[test]
fn a_next_page_from_before_a_refresh_is_not_appended() {
    let (mut state, key) = with_history();
    // Commits named `c0`, `c1`, ...
    let page = |c: char, next: Option<&str>| {
        let mut page = fixtures::history(next);
        for (i, commit) in page.items.iter_mut().enumerate() {
            commit.oid = format!("{c}{i}");
        }
        Data::from(page)
    };
    fetched(&mut state, key.clone(), page('a', Some("h1")));
    press(&mut state, "<End>");
    let old = press(&mut state, "<Enter>");
    assert_eq!(old, [more(&key, "h1")]);
    // r, and the refreshed list arrives before the old next page.
    let _ = press(&mut state, "r");
    fetched(&mut state, key.clone(), page('c', Some("h2")));
    press(&mut state, "<End>");
    let new = press(&mut state, "<Enter>");
    assert_eq!(new, [more(&key, "h2")]);
    // The page after the old list's h1, then the one after h2.
    update(
        &mut state,
        Msg::FetchedMore(key.clone(), "h1".into(), Ok(page('b', None)), None),
    );
    update(
        &mut state,
        Msg::FetchedMore(key.clone(), "h2".into(), Ok(page('d', None)), None),
    );
    let Some(Data::History(h)) = state.get(&key) else { panic!() };
    let oids: Vec<&str> = h.items.iter().map(|c| c.oid.as_str()).collect();
    assert_eq!(
        oids,
        ["c0", "c1", "d0", "d1"],
        "the refreshed list then its own next page"
    );
}

/// Load more pressed while the list refreshes isn't dropped: it's
/// asked of the refreshed list once that's in.
#[test]
fn load_more_during_a_refresh_waits_for_it() {
    let (mut state, key) = with_history();
    let page = |next| Data::from(fixtures::history(next));
    fetched(&mut state, key.clone(), page(Some("h1")));
    let _ = press(&mut state, "r");
    press(&mut state, "<End>");
    assert_eq!(press(&mut state, "<Enter>"), Vec::new());
    let cmds = fetched(&mut state, key.clone(), page(Some("h2")));
    assert_eq!(cmds, [more(&key, "h2")]);
}

/// A refresh that fails leaves the list as it was, so the next page
/// asked for before it still belongs there.
#[test]
fn a_next_page_survives_a_failed_refresh() {
    let (mut state, key) = with_history();
    let page = |next: Option<&str>| Data::from(fixtures::history(next));
    fetched(&mut state, key.clone(), page(Some("h1")));
    press(&mut state, "<End>");
    press(&mut state, "<Enter>");
    let _ = press(&mut state, "r");
    let result = Err(ApiError::Network("offline".into()));
    let cached_at = None;
    update(&mut state, Msg::fetched_at(key.clone(), result, cached_at));
    update(
        &mut state,
        Msg::FetchedMore(key.clone(), "h1".into(), Ok(page(None)), None),
    );
    let Some(Data::History(h)) = state.get(&key) else { panic!() };
    // The same commits, so only the cursor moves.
    assert_eq!(h.next, None, "the next page was dropped");
}

/// Security advisories come a page at a time; past the first page,
/// the list offers the rest like any other list.
#[test]
fn advisories_load_more() {
    let mut state = with_repo();
    let route = Route::Security(security::Advisories(Some(ghtui())));
    let key = DataKey::Advisories(Some(ghtui()));
    let cmds = state.push(route);
    assert!(cmds.contains(&fetch(key.clone())), "{cmds:?}");
    let list = fixtures::page(3, fixtures::advisories(), Some("c1"));
    fetched(&mut state, key.clone(), Data::from(list));
    press(&mut state, "<End>");
    assert!(
        selected_text(&state).starts_with("Load more"),
        "{:?}",
        selected_text(&state)
    );
    let cmds = press(&mut state, "<Enter>");
    assert_eq!(cmds, [more(&key, "c1")]);
}

/// An organization's People box shows a few members; it says how many
/// it left out, as every other cut-short list does.
#[test]
fn org_people_say_how_many_are_left_out() {
    // 3 of 14 members.
    let state = with_org();
    let lines: Vec<String> = page(&state)
        .page()
        .lines
        .iter()
        .map(crate::ui::page::PageLine::text)
        .collect();
    let people = lines
        .iter()
        .position(|l| l.contains("People"))
        .unwrap_or_else(|| panic!("{lines:#?}"));
    assert!(
        lines
            .iter()
            .skip(people)
            .take_while(|l| !l.contains("Top languages"))
            .any(|l| l.contains("11 more")),
        "{lines:#?}"
    );
}

/// A list that stops short of GitHub's count, as a milestone's past
/// search's 1000 does, keeps the count and says what's left out.
#[test]
fn a_list_cut_short_keeps_its_count() {
    let mut state = with_repo();
    let (repo, number) = (ghtui(), 3);
    let _ = state.push(Route::Issues(issues::Milestone {
        repo: repo.clone(),
        number,
    }));
    let mut milestone = fixtures::milestone();
    milestone.items.total = 1500;
    milestone.items.next = None;
    fetched(
        &mut state,
        DataKey::Milestone(repo, number),
        Data::from(milestone),
    );
    let text: String = page(&state)
        .page()
        .lines
        .iter()
        .map(|l| l.text() + "\n")
        .collect();
    assert!(text.contains("2 of 1500"), "{text}");
    assert!(text.contains("1498 more on GitHub"), "{text}");
}

/// A profile's repositories sort by name and stars as well as last
/// update, and `/` there searches that owner's repositories.
#[test]
fn profile_repositories_sort_and_filter() {
    use crate::api::browse::RepoSort;
    use crate::ui::pages::ProfileTab;
    let mut state = state();
    let tab = |sort| {
        Route::People(people::User {
            login: "octocat".into(),
            tab: ProfileTab::Repositories(sort),
        })
    };
    let _ = state.push(tab(RepoSort::Updated));
    for sort in [RepoSort::Name, RepoSort::Stars, RepoSort::Updated] {
        let cmds = act(&mut state, Action::Sort);
        assert_eq!(route(&state), tab(sort));
        // Last updated is still loading from when the tab opened.
        let key = DataKey::OwnerRepos("octocat".into(), sort);
        assert!(
            sort == RepoSort::Updated || cmds.contains(&fetch(key)),
            "{sort:?}"
        );
    }
    press(&mut state, "/");
    let sb = overlay!(state, Search);
    assert_eq!(sb.input.lines().join(""), "user:octocat ");
}

/// People and repository lists load more like any list.
#[test]
fn every_list_appends_its_next_page() {
    use fixtures::{forks, people};
    let mut users = Data::from(people());
    assert_eq!(users.next_cursor(), Some("u1"));
    let mut more = people();
    for u in &mut more.items {
        u.login.push_str("-2");
    }
    users.append(Data::from(more));
    assert!(matches!(&users, Data::Users(r) if r.items.len() == 6));
    let mut repos = Data::from(forks());
    assert_eq!(repos.next_cursor(), Some("f1"));
    let mut more = forks();
    for r in &mut more.items {
        r.repo = RepoId::new(r.repo.owner.clone(), format!("{}-2", r.repo.name));
    }
    repos.append(Data::from(more));
    assert!(matches!(&repos, Data::RepoPage(r) if r.items.len() == 4));
}

/// A filtered list's Open and Closed count what the filter finds, as
/// GitHub's do, and show nothing until counted rather than the
/// repository's totals. A list filtered only by state costs nothing
/// more: its counts are the repository's.
#[test]
fn a_filtered_list_counts_what_it_finds() {
    let counted = |cmds: &[Cmd]| {
        cmds.iter().find_map(|c| match c {
            Cmd::Api(Api::Fetch {
                key: DataKey::StateCounts(q),
                ..
            }) => Some(q.clone()),
            _ => None,
        })
    };
    let mut s = state();
    let cmds = s.push(issues("is:closed author:octocat sort:created-asc"));
    let query = counted(&cmds).unwrap();
    assert_eq!(
        query,
        "repo:gold-silver-copper/ghtui is:issue author:octocat"
    );
    let overview = Data::from(fixtures::overview());
    let _ = fetched(&mut s, DataKey::Repo(ghtui()), overview);
    let header = |s: &State| {
        let lines = page(s).page().lines.iter();
        let lines = lines.map(crate::ui::page::PageLine::text);
        lines.filter(|l| l.contains("Closed")).collect::<String>()
    };
    assert!(!header(&s).contains("41"), "{}", header(&s));
    let counts = crate::api::browse::StateCounts {
        open: 15,
        closed: 34,
    };
    let _ = fetched(
        &mut s,
        DataKey::StateCounts(query),
        Data::StateCounts(counts),
    );
    let text = header(&s);
    assert!(
        text.contains("15 Open") && text.contains("34 Closed"),
        "{text}"
    );
    assert_eq!(counted(&s.push(issues("is:closed sort:created-asc"))), None);
}
