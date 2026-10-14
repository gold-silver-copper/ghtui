//! What pages show: what's left out, failed or found, and where a link
//! lands.

use super::*;
use crate::api::model::Capped;
use crate::route::{issues, people, pulls};

/// A conversation longer than what's fetched (its newest comments,
/// reviews and commits) says how much is left out, and its tabs count it
/// all.
#[test]
fn long_conversations_say_what_is_left_out() {
    let mut issue = fixtures::issue();
    let shown = issue.timeline.len() as u64;
    issue.timeline = Capped::new(issue.timeline.items, shown + 116);
    issue.comment_count = 120;
    let mut state = with_issue();
    let route = Route::Issues(issues::Issue {
        repo: ghtui(),
        number: 14,
    });
    open(&mut state, route, Data::Issue(Some(Box::new(issue))));
    assert!(page_text(&state).contains("… 116 earlier comments and events on GitHub"));
    assert!(page_text(&state).contains("120 comments"));

    let pr = PrRef::parse("gold-silver-copper/ghtui#12").unwrap();
    let mut activity = fixtures::activity();
    let shown = activity.timeline.len() as u64;
    activity.timeline = Capped::new(activity.timeline.items, shown + 1);
    activity.comment_count = 101;
    activity.commits = Capped::new(activity.commits.items, 250);
    let mut state = with_pr();
    fetched(
        &mut state,
        DataKey::PrActivity(pr),
        Data::PrActivity(Box::new(activity)),
    );
    let page = page_text(&state);
    assert!(
        page.contains("… 1 earlier comment or event on GitHub"),
        "{page}"
    );
    let tabs = state.chrome().tabs;
    let count = |label: &str| {
        tabs.iter()
            .find(|(t, _)| t.label == label)
            .and_then(|(t, _)| t.count)
    };
    assert_eq!(
        (count("Conversation"), count("Commits")),
        (Some(101), Some(250))
    );
    press(&mut state, "2gt");
    assert!(page_text(&state).contains("… 248 earlier commits"));
}

/// A dismissed review, with the state the API gives it (pinned by
/// ghtui-api's `a_dismissed_reviews_wire_spelling`), reads as dismissed in
/// the conversation, not as a plain review.
#[test]
fn a_dismissed_review_reads_as_dismissed() {
    let pr = PrRef::parse("gold-silver-copper/ghtui#12").unwrap();
    let mut activity = fixtures::activity();
    let dismissed = fixtures::review(
        9,
        "hubot",
        crate::api::model::ReviewState::Dismissed,
        "",
        "2026-10-01T10:00:00Z",
    );
    activity.timeline = vec![crate::api::timeline::Item::Review(dismissed)].into();
    let mut state = with_pr();
    fetched(
        &mut state,
        DataKey::PrActivity(pr),
        Data::PrActivity(Box::new(activity)),
    );
    let page = page_text(&state);
    let hubot: Vec<&str> = page.lines().filter(|l| l.contains("hubot")).collect();
    assert!(
        page.contains("hubot reviewed (dismissed)"),
        "hubot's lines: {hubot:?}"
    );
}

/// One of Home's sections failing says why in its place; the others
/// show.
#[test]
fn a_failed_home_section_says_why_and_the_others_show() {
    let mut state = with_home();
    let key = state.home[1].search.clone().unwrap().key();
    state.data.remove(&key);
    let _ = state.load_visible(false);
    let err = ApiError::Network("connection refused".into());
    update(
        &mut state,
        Msg::Fetched {
            key,
            result: Err(err),
            cached_at: None,
        },
    );
    let page = page_text(&state);
    let why = "Couldn't load your pull requests: network error: connection refused";
    assert!(page.contains(why), "{page}");
    assert!(page.contains("Add a virtualized list widget"), "{page}");
    assert!(page.contains("gold-silver-copper/fux"), "{page}");
}

/// A pull request that fails to refresh says why, and keeps showing
/// what was loaded before.
#[test]
fn a_failed_refresh_keeps_what_was_loaded() {
    let mut state = with_pr();
    let pr = PrRef::parse("gold-silver-copper/ghtui#12").unwrap();
    crate::fixtures::pr_fetched(&mut state, &pr, Err(ApiError::RateLimited(42)));
    let page = page_text(&state);
    let why = "Couldn't refresh: rate limited by GitHub; retry in 42s.";
    assert!(page.contains(why), "{page}");
    assert!(
        page.contains("Derives keyword, string and type colors"),
        "{page}"
    );
}

/// More tabs than fit: the one on screen shows, with how many are
/// hidden either side.
#[test]
fn tabs_that_dont_fit_say_how_many_are_hidden() {
    let mut state = with_tabs(12);
    let _ = state.switch_to_tab(6);
    state.notices.dismiss();
    let state = sized(state, 80, 24);
    let screen = screen_text(&state);
    let header = screen.lines().next().unwrap_or_default();
    assert!(header.contains("‹ 5") && header.contains("6 ›"), "{screen}");
}

/// A link to lines of a file opens scrolled to them.
#[test]
fn a_link_to_lines_opens_on_them() {
    let mut state = with_repo();
    let text: String = (1..=80).map(|n| format!("let x{n} = {n};\n")).collect();
    let blob = crate::api::browse::Blob {
        path: "src/main.rs".into(),
        size: text.len() as u64,
        text: Some(text),
        truncated: false,
    };
    let url = "https://github.com/gold-silver-copper/ghtui/blob/main/src/main.rs#L60-L62";
    let crate::route::Target::Page(route) = Dest::from_url(url).target else {
        panic!("{url}");
    };
    let route = route.split(None).unwrap_or(route);
    open(&mut state, route, Data::Blob(Box::new(blob)));
    let screen = screen_text(&state);
    assert!(screen.contains("let x62 = 62;"), "{screen}");
    assert!(!screen.contains("let x1 = 1;"), "{screen}");
}

/// A link to a step's line opens that step's log, there.
#[test]
fn a_link_to_a_steps_line_opens_its_log() {
    let state = sized(with_job(Some((2, 2))), 100, 40);
    let screen = screen_text(&state);
    assert!(
        screen.contains("Syncing repository: gold-silver-copper/ghtui"),
        "{screen}"
    );
}

/// A filter is typed into the list's own field, not the header's, and
/// Enter applies it.
#[test]
fn a_filter_is_edited_in_place() {
    let mut state = with_repo();
    press(&mut state, "2gt/label:bug");
    let Some(Overlay::Search(sb)) = &state.overlay else {
        panic!("not filtering");
    };
    let (field, in_place) = state.search_field(sb);
    assert!(in_place);
    let buf = draw(&state);
    let row = |r: ratatui::layout::Rect| -> String {
        (r.left()..r.right())
            .map(|x| buf[(x, r.y)].symbol())
            .collect()
    };
    assert!(row(field).contains("⌕ is:open label:bug"), "{}", row(field));
    assert!(!row(state.header_search()).contains("label:bug"));
    press(&mut state, "<Enter>");
    let query = state.route().and_then(|r| r.family().query());
    assert_eq!(query, Some("is:open label:bug"));
}

/// `/` on a page selects the row it finds, so `↵` opens what was found,
/// as vim's cursor goes to the match.
#[test]
fn a_search_selects_the_row_it_finds() {
    let mut s = with_pr();
    press(&mut s, "/Fixes #12<Enter><Enter>");
    assert!(
        matches!(
            s.route(),
            Some(Route::Issues(issues::Issue { number: 12, .. }))
        ) || matches!(s.route(), Some(Route::Pulls(pulls::Pr { pr, .. })) if pr.number == 12),
        "{:?}",
        s.route()
    );
}

/// GitHub lists none of a big repository's stargazers: the page says so
/// rather than that nobody starred it.
#[test]
fn unlisted_stargazers_say_why() {
    let mut state = with_repo();
    let mut none = fixtures::people();
    (none.items, none.next) = (Vec::new(), None);
    open(
        &mut state,
        Route::People(people::Stargazers(ghtui())),
        Data::Users(Box::new(none)),
    );
    let page = page_text(&state);
    assert!(
        page.contains("GitHub doesn't list this repository's"),
        "{page}"
    );
}
