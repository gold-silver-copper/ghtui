//! Fetching, refreshing and writing.

use super::*;
use crate::route::issues;

#[test]
fn duplicate_fetches_are_suppressed() {
    let mut state = state();
    let sections: Vec<Cmd> = (state.home.iter())
        .map(|s| fetch(s.search.clone().unwrap().key()))
        .collect();
    assert_eq!(state.load_visible(false), sections);
    assert_eq!(state.load_visible(true), []);
    // Who you are is asked again until it's known.
    state.viewer = None;
    assert_eq!(state.load_visible(true), vec![Cmd::Api(Api::FetchViewer)]);
}

#[test]
fn cached_data_shows_while_fetching() {
    let mut state = state();
    let _ = state.push(Route::user("octocat"));
    update(
        &mut state,
        Msg::Fetched {
            key: DataKey::Profile("octocat".into()),
            result: Ok(Data::Profile(Box::new(crate::fixtures::profile()))),
            cached_at: Some(0),
            source: None,
        },
    );
    assert!(page(&state).page().lines[0].text().contains("The Octocat"));
    let busy = state.busy().unwrap_or_default();
    assert!(busy.starts_with("Refreshing · cached "), "{busy}");
}

#[test]
fn failed_refresh_keeps_cached_data() {
    let mut state = with_prs(2);
    let _ = state.load_visible(true);
    mine_failed(&mut state, ApiError::RateLimited(30));
    assert!(selected_text(&state).contains("PR 1"));
    let first = page(&state).page().lines[0].text();
    assert!(
        first.contains("Couldn't refresh") && first.contains("rate limited"),
        "{first}"
    );
}

/// One outage that fails each of the page's needs is said once.
#[test]
fn a_refresh_error_shared_by_needs_is_said_once() {
    let mut state = with_prs(2);
    let repos = state.home[2].search.clone().unwrap().key();
    let fetched = |result| Msg::Fetched {
        key: repos.clone(),
        result,
        cached_at: None,
        source: None,
    };
    let none = SearchResults::Repos(crate::api::browse::Results::default());
    update(&mut state, fetched(Ok(Data::Search(Box::new(none)))));
    let _ = state.load_visible(true);
    mine_failed(&mut state, ApiError::RateLimited(30));
    update(&mut state, fetched(Err(ApiError::RateLimited(30))));
    let first = page(&state).page().lines[0].text();
    assert_eq!(first.matches("rate limited").count(), 1, "{first}");
}

/// A review GitHub turned down whole changed nothing there, so what's
/// held stays fresh; one that went through makes it stale.
#[test]
fn only_a_review_that_went_through_counts_as_a_write() {
    let mut state = with_repo();
    let key = DataKey::Issue(ghtui(), 14);
    let _ = state.push(issue(14));
    let issue = Data::Issue(Some(Box::new(crate::fixtures::issue())));
    fetched(&mut state, key.clone(), issue);
    let of = DiffOf::Pr(PrRef::parse("o/r#1").unwrap());
    let mut outcome = crate::review::SubmitOutcome {
        error: Some("Couldn't reach the PR".into()),
        ..Default::default()
    };
    let submitted = |o| Msg::Diff(of.clone(), DiffMsg::ReviewSubmitted(o));
    update(&mut state, submitted(outcome.clone()));
    assert_eq!(state.load_visible(false), Vec::new());
    outcome.accepted.push(1);
    update(&mut state, submitted(outcome));
    let refetch = Cmd::Api(Api::Fetch { key, cached: false });
    assert!(state.load_visible(false).contains(&refetch));
}

#[test]
fn issue_comments_post_and_refresh() {
    let mut state = state();
    let _ = state.push(issue(14));
    let key = DataKey::Issue(ghtui(), 14);
    press(&mut state, "c");
    assert!(state.overlay.is_none(), "nothing to comment on yet");
    fetched(
        &mut state,
        key.clone(),
        Data::Issue(Some(Box::new(crate::fixtures::issue()))),
    );
    press(&mut state, "c");
    let compose = overlay!(state, Compose);
    assert_eq!(
        compose.target.title(&state),
        "Comment on gold-silver-copper/ghtui#14"
    );
    press(&mut state, "Thanks!");
    let cmds = update(
        &mut state,
        Msg::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
    );
    let comment = Change::Comment {
        subject: NodeId::new("I_14"),
        body: "Thanks!".into(),
    };
    let sent = vec![fixtures::change(comment, act::By::Compose)];
    assert_eq!(cmds, sent);
    let cmds = answer(&mut state, &sent, Ok(()));
    assert!(state.overlay.is_none());
    let refetch = Cmd::Api(Api::Fetch {
        key: key.clone(),
        cached: false,
    });
    assert!(cmds.contains(&refetch), "{cmds:?}");
    // Another comment, posted while that fetch is on its way: the fetch
    // may miss it, so the page is fetched again once it lands.
    let cmds = answer(&mut state, &sent, Ok(()));
    assert!(!cmds.contains(&refetch));
    let refetch = vec![refetch];
    let issue = || Data::Issue(Some(Box::new(crate::fixtures::issue())));
    assert_eq!(fetched(&mut state, key.clone(), issue()), refetch);
    assert_eq!(fetched(&mut state, key, issue()), Vec::new(), "only once");
}

/// The fresh copy of a cached list, with a new issue on top, keeps the
/// selection on the issue you picked.
#[test]
fn a_fresh_copy_keeps_the_selected_issue() {
    let mut state = state();
    let _ = state.push(issues("is:open"));
    let (kind, query) = route(&state).search().unwrap();
    let key = DataKey::Search(kind, query);
    let list = |numbers: &[u64]| {
        Data::Search(Box::new(SearchResults::Issues(crate::fixtures::page(
            numbers.len() as u64,
            numbers
                .iter()
                .map(|&n| crate::fixtures::issue_summary(n, Status::Issue(IssueState::Open)))
                .collect(),
            None,
        ))))
    };
    update(
        &mut state,
        Msg::Fetched {
            key: key.clone(),
            result: Ok(list(&[14, 11, 8])),
            cached_at: Some(0),
            source: None,
        },
    );
    while !selected_text(&state).contains("Issue 11") {
        press(&mut state, "<Down>");
    }
    fetched(&mut state, key, list(&[20, 14, 11, 8]));
    let text = selected_text(&state);
    assert!(text.contains("Issue 11"), "{text}");
}

/// Editing the comment you selected keeps it selected, and the view on
/// it, when the fresh copy arrives.
#[test]
fn an_edited_comment_stays_selected() {
    let mut s = with_repo();
    s.size = (100, 12);
    let _ = s.push(Route::Issues(issues::Issue {
        repo: ghtui(),
        number: 14,
    }));
    let key = DataKey::Issue(ghtui(), 14);
    let mut issue = crate::fixtures::issue();
    update(
        &mut s,
        Msg::Fetched {
            key: key.clone(),
            result: Ok(Data::Issue(Some(Box::new(issue.clone())))),
            cached_at: Some(0),
            source: None,
        },
    );
    let Screen::Page(p) = s.screen_mut() else {
        panic!("not on a page")
    };
    let page_ = p.page();
    let at = |i: &crate::ui::page::Item| page_.lines[i.start].text().contains("hubot");
    let n = page_.items.iter().position(at).unwrap();
    (p.selected, p.scroll) = (Some(n), page_.items[n].start);
    let top = |s: &State| page(s).page().lines[page(s).scroll].text();
    let before = (selected_text(&s), top(&s));
    issue.body.push_str("\n\nEdited.");
    issue.first_comment().unwrap().body = "Edited.".into();
    fetched(&mut s, key, Data::Issue(Some(Box::new(issue))));
    assert_eq!((selected_text(&s), top(&s)), before);
}

/// A failed refresh puts its banner above the cached copy without
/// moving what you scrolled to.
#[test]
fn a_failed_refresh_keeps_the_view() {
    let mut state = with_prs(40);
    for _ in 0..25 {
        press(&mut state, "<Down>");
    }
    let top = |state: &State| {
        let p = page(state);
        p.page().lines[p.scroll].text()
    };
    let before = top(&state);
    assert!(page(&state).scroll > 0, "scrolled");
    let _ = state.load_visible(true);
    mine_failed(&mut state, ApiError::RateLimited(30));
    assert!(
        page(&state).page().lines[0]
            .text()
            .contains("Couldn't refresh")
    );
    assert_eq!(top(&state), before);
}
