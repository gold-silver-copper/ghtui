//! A page is built again only when what it shows changes: its own data,
//! what else it shows, or a time it shows relative to now.

use super::*;
use crate::source::{Clone, From};
use crate::ui::page::Page;
use std::sync::atomic::{AtomicU64, Ordering};

/// [`with_pr`] on `clock`, its conversation octocat's comments: each
/// one's ID, text and when it was written.
fn talking(clock: fn() -> u64, comments: Vec<(u64, String, u64)>) -> State {
    use crate::api::browse::Comment;
    let mut state = with_pr();
    state.clock = clock;
    let total = comments.len() as u64;
    let items = (comments.into_iter())
        .map(|(id, body, at)| {
            crate::api::timeline::Item::Comment(Comment {
                id: Some(id),
                author: "octocat".into(),
                body,
                created_at: crate::ui::time::iso8601(at),
                ..Default::default()
            })
        })
        .collect();
    let mut activity = fixtures::activity();
    activity.timeline = crate::api::model::Capped::new(items, total);
    fetched(&mut state, DataKey::PrActivity(pr()), Data::from(activity));
    state
}

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

    let overview = Data::from(fixtures::overview());
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
    let _ = act(&mut state, Action::OpenInTab);
    let _ = state.push(Route::code(ghtui()));
    issue_shows(&mut state, |d| d.title = "Renamed meanwhile".into());
    assert!(!page_text(&state).contains("Renamed meanwhile"));
    press(&mut state, "<C-PageUp>");
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

/// Times that turn over at different seconds move on together, at the
/// minute timer: a page is built at most once a minute for them, and
/// other timers (a watched pull request's) never build it for them.
#[test]
fn recent_times_build_a_page_at_most_once_a_minute() {
    static CLOCK: AtomicU64 = AtomicU64::new(NOW);
    let comments = [130, 347, 615, 1801].map(|ago| (ago, "Looks right".into(), NOW - ago));
    let mut state = talking(|| CLOCK.load(Ordering::Relaxed), comments.into());
    let mut builds = 0;
    for second in 1..=300 {
        CLOCK.store(NOW + second, Ordering::Relaxed);
        let before = built(&state);
        update(&mut state, Msg::Timer(Timer::Watch(0)));
        assert!(std::ptr::eq(built(&state), before), "at {second}s");
        if second % 60 == 0 {
            update(&mut state, Msg::Timer(Timer::Minute));
            builds += usize::from(!std::ptr::eq(built(&state), before));
        }
    }
    // Each minute, one of them reads differently.
    assert_eq!(builds, 5);
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
            result: Ok(Data::from(fixtures::blob())),
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
    static CLOCK: AtomicU64 = AtomicU64::new(NOW);
    // 300 comments with code, one every 10 minutes until two hours ago.
    let body = "Looks right, but:\n\n```rust\nfn main() {\n    let x = compute(1);\n    println!(\"{x}\");\n}\n```\n";
    let comments = (0..300u64).map(|i| (i, body.repeat(3), NOW - 7200 - (299 - i) * 600));
    let mut state = talking(|| CLOCK.load(Ordering::Relaxed), comments.collect());
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
