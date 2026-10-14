//! A page is built again only when what it shows changes: its own data,
//! what else it shows, or a time it shows relative to now.

use super::*;
use crate::route::code;
use crate::source::{Clone, From};
use crate::ui::page::Page;

/// The page on screen, as built: the same one until it's built again.
fn built(state: &State) -> *const Page {
    page(state).page()
}

/// Messages without the page's data (rate limits after every request,
/// notices, another page's data, GitHub asked again whether it's back)
/// leave the page as built; its own data, and who's signed in, build it
/// again.
#[test]
fn only_what_a_page_shows_rebuilds_it() {
    let mut state = with_repo();
    let before = built(&state);
    update(&mut state, Msg::RateLimits(RateLimits::default()));
    update(&mut state, Msg::Notice(Notice::Info("hi".into())));
    update(&mut state, Msg::Problem(Problem::Drafts, None));
    let other = DataKey::Issue(ghtui(), 9);
    fetched(
        &mut state,
        other,
        Data::Issue(Some(Box::new(fixtures::issue()))),
    );
    update(&mut state, Msg::Offline(crate::offline::Msg::Probed(false)));
    update(&mut state, Msg::Timer(Timer::Minute));
    assert!(std::ptr::eq(built(&state), before));

    let overview = Data::Repo(Box::new(fixtures::overview()));
    fetched(&mut state, DataKey::Repo(ghtui()), overview);
    assert!(!std::ptr::eq(built(&state), before));
    let before = built(&state);
    update(&mut state, Msg::Viewer(Ok("me".into())));
    assert!(!std::ptr::eq(built(&state), before));
}

/// A page parked in another tab, whose data came in meanwhile, is built
/// with it when its tab is back.
#[test]
fn a_page_away_is_built_with_what_came_meanwhile() {
    let mut state = with_issue();
    let key = state.needs(&route(&state)).pop().unwrap();
    let _ = act(&mut state, Action::OpenInTab);
    let _ = state.push(Route::Code(code::Repo(ghtui())));
    let mut issue = fixtures::issue();
    issue.title = "Renamed meanwhile".into();
    fetched(&mut state, key, Data::Issue(Some(Box::new(issue))));
    assert!(!page_text(&state).contains("Renamed meanwhile"));
    press(&mut state, "[b");
    assert!(page_text(&state).contains("Renamed meanwhile"));
}

/// The minute timer builds a page again only once a time it shows reads
/// differently.
#[test]
fn a_page_is_built_again_once_its_times_move_on() {
    let mut state = with_issue();
    let before = built(&state);
    update(&mut state, Msg::Timer(Timer::Minute));
    assert!(std::ptr::eq(built(&state), before));
    // A year on, every "ago" reads differently.
    state.clock = || NOW + 400 * 86_400;
    update(&mut state, Msg::Timer(Timer::Minute));
    assert!(!std::ptr::eq(built(&state), before));
    let before = built(&state);
    update(&mut state, Msg::Timer(Timer::Minute));
    assert!(std::ptr::eq(built(&state), before));
}

/// Where git read a file comes with the file, and the page says it.
#[test]
fn a_file_says_where_git_read_it() {
    let mut state = with_repo();
    let route = blob("main", "src/main.rs");
    let _ = state.push(route.clone());
    let key = state.needs(&route).pop().unwrap();
    let source = From::Git {
        clone: Clone::Cache,
        at: None,
        listed: None,
    };
    update(
        &mut state,
        Msg::Fetched {
            key,
            result: Ok(Data::Blob(Box::new(fixtures::blob()))),
            cached_at: None,
            source: Some(source),
        },
    );
    assert!(page_text(&state).contains("Read by git from ghtui's clone"));
}

/// How often a long conversation is built while nothing happens on it,
/// and what a build costs: an hour of minute timers and the rate limits
/// each background request sends. Run with
/// `cargo test --release -p ghtui -- --ignored idle_hour_timing`.
#[test]
#[ignore = "timing; run in release"]
#[expect(clippy::print_stderr, reason = "reports the timings")]
fn idle_hour_timing_on_a_long_conversation() {
    use std::sync::atomic::{AtomicU64, Ordering};

    use crate::api::browse::Comment;
    use crate::api::model::Capped;
    use crate::api::timeline::Item;

    static CLOCK: AtomicU64 = AtomicU64::new(NOW);
    let mut state = with_pr();
    state.clock = || CLOCK.load(Ordering::Relaxed);
    let pr = PrRef::parse("gold-silver-copper/ghtui#12").unwrap();
    // 300 comments with code, one every 10 minutes until two hours ago.
    let body = "Looks right, but:\n\n```rust\nfn main() {\n    let x = compute(1);\n    println!(\"{x}\");\n}\n```\n";
    let items: Vec<Item> = (0..300u64)
        .map(|i| {
            Item::Comment(Comment {
                id: Some(i),
                author: "octocat".into(),
                body: body.repeat(3),
                created_at: crate::ui::time::iso8601(NOW - 7200 - (299 - i) * 600),
                ..Default::default()
            })
        })
        .collect();
    let mut activity = fixtures::activity();
    activity.timeline = Capped::new(items, 300);
    fetched(
        &mut state,
        DataKey::PrActivity(pr),
        Data::PrActivity(Box::new(activity)),
    );
    let (mut builds, mut spent) = (0u32, std::time::Duration::ZERO);
    let mut step = |state: &mut State, msg: Msg| {
        let before = built(state);
        let start = std::time::Instant::now();
        update(state, msg);
        if !std::ptr::eq(built(state), before) {
            builds += 1;
            spent += start.elapsed();
        }
    };
    for minute in 1..=60 {
        CLOCK.store(NOW + minute * 60, Ordering::Relaxed);
        for _ in 0..6 {
            step(&mut state, Msg::RateLimits(RateLimits::default()));
        }
        step(&mut state, Msg::Timer(Timer::Minute));
    }
    eprintln!(
        "builds in an idle hour: {builds}, {:?} each",
        spent.checked_div(builds).unwrap_or_default()
    );
}
