//! Full-screen snapshots through ratatui's `TestBackend`, in light and dark
//! schemes (and 256 colors). Snapshots include cell styles, so color changes
//! show up in review. Rendering also exercises the theme's debug assertion
//! that every fg/bg pair used is declared (and therefore contrast-tested).

use crate::api::browse::{IssueKind, PrState, RepoSort, SearchKind, SearchResults};
use crate::api::model::{
    Capped, ChecksState, Label, Mergeable, PrDetail, PrRef, RepoId, ReviewDecision,
};
use crate::api::rate_limit::{Bucket, RateLimits};
use crate::theme::{ColorDepth, DEFAULT_SEED, Mode, Theme};
use crate::ui::Icons;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use crate::ui::pages::{PrTab, ProfileTab};

use crate::browse::{Data, DataKey, Need, needs};
use crate::fixtures::{self, fetched, press};
use crate::keymap::Keymap;
use crate::route::{OPEN, Route};
use crate::state::{Msg, Overlay, State, update};
use crate::view::view;

/// `insta::assert_snapshot!` for the tests in this module's own files:
/// their snapshots are in `src/snapshots` too, where insta would otherwise
/// look next to the file.
macro_rules! assert_snapshot {
    ($($snapshot:tt)*) => {
        insta::with_settings!(
            { snapshot_path => concat!(env!("CARGO_MANIFEST_DIR"), "/src/snapshots") },
            { insta::assert_snapshot!($($snapshot)*) }
        )
    };
}

mod failures;
mod screens;
mod screenshots;

pub(crate) use failures::*;
pub(crate) use screens::*;

/// 2026-10-03T12:00:00Z
const NOW: u64 = 1_791_028_800;

/// A conversation longer than what's fetched (its newest comments,
/// reviews and commits) says how much is left out, and its tabs count it
/// all.
#[test]
fn long_conversations_say_what_is_left_out() {
    let text = |state: &State| -> String {
        let crate::state::Screen::Page(p) = state.screen() else {
            panic!("not a page");
        };
        p.page()
            .lines
            .iter()
            .map(crate::ui::page::PageLine::text)
            .collect::<Vec<_>>()
            .join("\n")
    };
    let mut issue = fixtures::issue();
    issue.comments = Capped::new(issue.comments.items, 120);
    let mut state = with_issue(Mode::Dark);
    let route = Route::Issue {
        repo: ghtui(),
        number: 14,
    };
    open(&mut state, route, Data::Issue(Some(Box::new(issue))));
    assert!(text(&state).contains("… 119 earlier comments on GitHub"));
    assert!(text(&state).contains("120 comments"));

    let pr = PrRef::parse("gold-silver-copper/ghtui#12").unwrap();
    let mut activity = fixtures::activity();
    activity.comments = Capped::new(activity.comments.items, 101);
    activity.reviews = Capped::new(activity.reviews.items, 3);
    activity.commits = Capped::new(activity.commits.items, 250);
    let mut state = with_pr(Mode::Dark);
    fetched(
        &mut state,
        DataKey::PrActivity(pr),
        Data::PrActivity(Box::new(activity)),
    );
    let page = text(&state);
    assert!(page.contains("… 100 earlier comments on GitHub"), "{page}");
    assert!(page.contains("… 1 earlier review on GitHub"), "{page}");
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
    assert!(text(&state).contains("… 248 earlier commits"));
}

/// A dismissed review, with the state the API gives it (pinned by
/// ghtui-api's `a_dismissed_reviews_wire_spelling`), reads as dismissed in
/// the conversation, not as a plain review.
#[test]
fn a_dismissed_review_reads_as_dismissed() {
    let pr = PrRef::parse("gold-silver-copper/ghtui#12").unwrap();
    let mut activity = fixtures::activity();
    activity.reviews = crate::api::model::Capped::new(
        vec![crate::api::browse::ReviewSummary {
            id: None,
            author: "hubot".into(),
            state: crate::api::model::ReviewState::Dismissed,
            body: String::new(),
            submitted_at: "2026-10-01T10:00:00Z".into(),
        }],
        1,
    );
    let mut state = with_pr(Mode::Dark);
    fetched(
        &mut state,
        DataKey::PrActivity(pr),
        Data::PrActivity(Box::new(activity)),
    );
    let crate::state::Screen::Page(p) = state.screen() else {
        panic!("not a page");
    };
    let page = p
        .page()
        .lines
        .iter()
        .map(crate::ui::page::PageLine::text)
        .collect::<Vec<_>>()
        .join("\n");
    let hubot: Vec<&str> = page.lines().filter(|l| l.contains("hubot")).collect();
    assert!(
        page.contains("had a review dismissed"),
        "hubot's lines: {hubot:?}"
    );
}

#[test]
fn home_dark() {
    insta::assert_snapshot!(render(&with_home(Mode::Dark, ColorDepth::TrueColor)));
}

#[test]
fn home_light() {
    insta::assert_snapshot!(render(&with_home(Mode::Light, ColorDepth::TrueColor)));
}

#[test]
fn home_dark_256() {
    insta::assert_snapshot!(render(&with_home(Mode::Dark, ColorDepth::Ansi256)));
}

#[test]
fn home_third_row_selected_light() {
    let mut state = with_home(Mode::Light, ColorDepth::TrueColor);
    press(&mut state, "jj");
    insta::assert_snapshot!(render(&state));
}

#[test]
fn home_loading_dark() {
    let mut state = state(Mode::Dark, ColorDepth::TrueColor);
    let _ = state.load_visible(false);
    insta::assert_snapshot!(render(&state));
}

#[test]
fn home_error_light() {
    let mut state = state(Mode::Light, ColorDepth::TrueColor);
    let _ = state.load_visible(false);
    // One section fails; the others show.
    fill_home(&mut state);
    let key = state.home[1].search.clone().unwrap().key();
    state.data.remove(&key);
    let _ = state.load_visible(false);
    update(
        &mut state,
        Msg::Fetched {
            key,
            result: Err(crate::api::ApiError::Network("connection refused".into())),
            cached_at: None,
        },
    );
    insta::assert_snapshot!(render(&state));
}

#[test]
fn repo_dark() {
    insta::assert_snapshot!(render(&with_repo(Mode::Dark, ColorDepth::TrueColor)));
}

#[test]
fn repo_light() {
    insta::assert_snapshot!(render(&with_repo(Mode::Light, ColorDepth::TrueColor)));
}

#[test]
fn repo_readme_dark_256() {
    let mut state = with_repo(Mode::Dark, ColorDepth::Ansi256);
    press(&mut state, "G");
    insta::assert_snapshot!(render(&state));
}

#[test]
fn repo_wide_terminal_centers_light() {
    let mut state = with_repo(Mode::Light, ColorDepth::TrueColor);
    update(&mut state, Msg::Resize(160, 30));
    insta::assert_snapshot!(render(&state));
}

#[test]
fn pr_checks_dark() {
    insta::assert_snapshot!(render(&with_pr_checks(Mode::Dark)));
}

#[test]
fn pr_review_dark() {
    insta::assert_snapshot!(render(&sized(with_pr_review(Mode::Dark), 100, 44)));
}

#[test]
fn actions_light() {
    insta::assert_snapshot!(render(&with_actions(Mode::Light)));
}

#[test]
fn notifications_dark() {
    insta::assert_snapshot!(render(&with_notifications(Mode::Dark)));
}

#[test]
fn profile_repositories_light() {
    let tab = ProfileTab::Repositories(RepoSort::Name);
    insta::assert_snapshot!(render(&with_profile(Mode::Light, tab)));
}

#[test]
fn profile_followers_dark() {
    insta::assert_snapshot!(render(&with_profile(Mode::Dark, ProfileTab::Followers)));
}

#[test]
fn tabs_light() {
    insta::assert_snapshot!(render(&with_tabs(Mode::Light, 3)));
}

#[test]
fn tabs_dark() {
    insta::assert_snapshot!(render(&with_tabs(Mode::Dark, 3)));
}

/// More tabs than fit: the one on screen shows, with how many are hidden
/// either side.
#[test]
fn tabs_overflow_80_dark() {
    let mut state = with_tabs(Mode::Dark, 12);
    let _ = state.switch_to_tab(6);
    state.notices.dismiss();
    insta::assert_snapshot!(render(&sized(state, 80, 24)));
}

#[test]
fn org_profile_dark() {
    insta::assert_snapshot!(render(&sized(with_org(Mode::Dark), 100, 50)));
}

/// The whole overview: README, pinned, the contribution graph, activity.
#[test]
fn profile_overview_tall_light() {
    let state = with_profile(Mode::Light, ProfileTab::Overview);
    insta::assert_snapshot!(render(&sized(state, 100, 80)));
}

/// In 256 colors the graph's shades stay apart.
#[test]
fn profile_overview_256_dark() {
    let mut state = with_profile(Mode::Dark, ProfileTab::Overview);
    state.theme = Theme::new(DEFAULT_SEED, Mode::Dark, ColorDepth::Ansi256);
    insta::assert_snapshot!(render(&sized(state, 100, 80)));
}

#[test]
fn discussions_dark() {
    insta::assert_snapshot!(render(&with_discussions(Mode::Dark)));
}

#[test]
fn discussion_light() {
    insta::assert_snapshot!(render(&sized(with_discussion(Mode::Light), 100, 50)));
}

#[test]
fn workflow_run_dark() {
    insta::assert_snapshot!(render(&with_run(Mode::Dark)));
}

#[test]
fn job_light() {
    insta::assert_snapshot!(render(&sized(with_job(Mode::Light, None), 100, 40)));
}

/// A link to a step's line opens that step's log there.
#[test]
fn job_step_dark() {
    insta::assert_snapshot!(render(&sized(with_job(Mode::Dark, Some((2, 2))), 100, 40)));
}

#[test]
fn workflow_light() {
    insta::assert_snapshot!(render(&with_workflow(Mode::Light)));
}

#[test]
fn stargazers_dark() {
    insta::assert_snapshot!(render(&with_stargazers(Mode::Dark)));
}

#[test]
fn forks_light() {
    insta::assert_snapshot!(render(&with_forks(Mode::Light)));
}

#[test]
fn releases_dark() {
    insta::assert_snapshot!(render(&with_releases(Mode::Dark)));
}

#[test]
fn release_light() {
    insta::assert_snapshot!(render(&with_release(Mode::Light)));
}

#[test]
fn tags_dark() {
    insta::assert_snapshot!(render(&with_tags(Mode::Dark)));
}

#[test]
fn milestones_dark() {
    insta::assert_snapshot!(render(&with_milestones(Mode::Dark)));
}

#[test]
fn milestone_light() {
    insta::assert_snapshot!(render(&with_milestone(Mode::Light)));
}

#[test]
fn deployments_dark() {
    insta::assert_snapshot!(render(&with_deployments(Mode::Dark)));
}

#[test]
fn compare_light() {
    insta::assert_snapshot!(render(&with_compare(Mode::Light)));
}

#[test]
fn blame_dark() {
    insta::assert_snapshot!(render(&with_blame(Mode::Dark)));
}

#[test]
fn gist_light() {
    insta::assert_snapshot!(render(&with_gist(Mode::Light)));
}

#[test]
fn gists_dark() {
    insta::assert_snapshot!(render(&with_gists(Mode::Dark)));
}

#[test]
fn teams_light() {
    insta::assert_snapshot!(render(&with_teams(Mode::Light)));
}

#[test]
fn team_dark() {
    insta::assert_snapshot!(render(&with_team(Mode::Dark)));
}

#[test]
fn advisories_dark() {
    insta::assert_snapshot!(render(&with_advisories(Mode::Dark)));
}

#[test]
fn advisory_light() {
    insta::assert_snapshot!(render(&sized(with_advisory(Mode::Light), 100, 40)));
}

#[test]
fn wiki_light() {
    insta::assert_snapshot!(render(&with_wiki(Mode::Light)));
}

#[test]
fn branches_light() {
    insta::assert_snapshot!(render(&with_branches(Mode::Light)));
}

#[test]
fn commits_light() {
    insta::assert_snapshot!(render(&with_history(Mode::Light)));
}

#[test]
fn commit_light() {
    insta::assert_snapshot!(render(&with_commit(Mode::Light)));
}

#[test]
fn commit_dark() {
    insta::assert_snapshot!(render(&with_commit(Mode::Dark)));
}

#[test]
fn file_light() {
    insta::assert_snapshot!(render(&with_file(Mode::Light)));
}

/// A link to lines opens scrolled to them, marked.
#[test]
fn file_lines_dark() {
    let mut state = with_repo(Mode::Dark, ColorDepth::TrueColor);
    let text: String = (1..=80).map(|n| format!("let x{n} = {n};\n")).collect();
    let blob = crate::api::browse::Blob {
        path: "src/main.rs".into(),
        size: text.len() as u64,
        text: Some(text),
        truncated: false,
    };
    let url = "https://github.com/gold-silver-copper/ghtui/blob/main/src/main.rs#L60-L62";
    let crate::route::Target::Page(route) = crate::route::Dest::from_url(url).target else {
        panic!("{url}");
    };
    let route = route.split(None).unwrap_or(route);
    open(&mut state, route, Data::Blob(Box::new(blob)));
    insta::assert_snapshot!(render(&state));
}

#[test]
fn issues_dark() {
    insta::assert_snapshot!(render(&with_issues(Mode::Dark)));
}

#[test]
fn issue_light() {
    insta::assert_snapshot!(render(&with_issue(Mode::Light)));
}

#[test]
fn pr_dark() {
    insta::assert_snapshot!(render(&with_pr(Mode::Dark)));
}

#[test]
fn pr_light() {
    insta::assert_snapshot!(render(&with_pr(Mode::Light)));
}

#[test]
fn pr_commits_dark() {
    let mut state = with_pr(Mode::Dark);
    press(&mut state, "2gt");
    insta::assert_snapshot!(render(&state));
}

#[test]
fn pr_refresh_error_dark() {
    let mut state = with_pr(Mode::Dark);
    let pr = PrRef::parse("gold-silver-copper/ghtui#12").unwrap();
    update(
        &mut state,
        Msg::Pr(pr, Box::new(Err(crate::api::ApiError::RateLimited(42)))),
    );
    insta::assert_snapshot!(render(&state));
}

#[test]
fn profile_light() {
    insta::assert_snapshot!(render(&with_profile(Mode::Light, ProfileTab::Overview)));
}

#[test]
fn search_dark() {
    insta::assert_snapshot!(render(&with_repo_search(Mode::Dark)));
}

#[test]
fn search_discussions_dark() {
    let results = fixtures::discussion_results();
    let state = with_search(Mode::Dark, SearchKind::Discussions, "offline", results);
    insta::assert_snapshot!(render(&state));
}

#[test]
fn search_commits_light() {
    let results = fixtures::commit_results();
    let state = with_search(Mode::Light, SearchKind::Commits, "tui", results);
    insta::assert_snapshot!(render(&state));
}

#[test]
fn search_code_dark() {
    let results = fixtures::code_results();
    let state = with_search(Mode::Dark, SearchKind::Code, "Terminal", results);
    insta::assert_snapshot!(render(&state));
}

#[test]
fn search_users_light() {
    let results = fixtures::user_results();
    let state = with_search(Mode::Light, SearchKind::Users, "octocat", results);
    insta::assert_snapshot!(render(&state));
}

#[test]
fn quick_ways_around_dark() {
    let mut state = with_repo(Mode::Dark, ColorDepth::TrueColor);
    state.visits = vec![crate::nav::Visit {
        url: "https://github.com/octocat".into(),
        title: "@octocat".into(),
        count: 3,
        last: NOW - 600,
    }];
    press(&mut state, "i");
    insta::assert_snapshot!("hints_dark", render(&state));
    press(&mut state, "<Esc>g/");
    insta::assert_snapshot!("search_empty_dark", render(&state));
    press(&mut state, "oct");
    insta::assert_snapshot!("search_typed_dark", render(&state));
    press(&mut state, "<Esc><Space>");
    insta::assert_snapshot!("menu_dark", render(&state));
    press(&mut state, "<Esc>g");
    insta::assert_snapshot!("which_key_dark", render(&state));
}

#[test]
fn go_to_file_light() {
    let mut state = with_repo(Mode::Light, ColorDepth::TrueColor);
    press(&mut state, "f");
    fetched(
        &mut state,
        DataKey::Files(ghtui(), "main".into()),
        Data::Files(
            std::sync::Arc::new(vec![
                "src/main.rs".into(),
                "crates/app/src/main.rs".into(),
                "crates/ui/src/page.rs".into(),
            ]),
            false,
        ),
    );
    press(&mut state, "main");
    insta::assert_snapshot!(render(&state));
}

#[test]
fn list_filter_light() {
    let mut state = with_repo(Mode::Light, ColorDepth::TrueColor);
    press(&mut state, "2gt/");
    insta::assert_snapshot!(render(&state));
}

/// A filter is typed into the list's own field, not the header's, and
/// Enter applies it.
#[test]
fn a_filter_is_edited_in_place() {
    let mut state = with_repo(Mode::Light, ColorDepth::TrueColor);
    press(&mut state, "2gt/label:bug");
    let Some(Overlay::Search(sb)) = &state.overlay else {
        panic!("not filtering");
    };
    let (field, in_place) = state.search_field(sb);
    assert!(in_place);
    let (w, h) = state.size;
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|frame| view(&state, frame, NOW)).unwrap();
    let buf = terminal.backend().buffer();
    let row = |r: ratatui::layout::Rect| -> String {
        (r.left()..r.right())
            .map(|x| buf[(x, r.y)].symbol())
            .collect()
    };
    assert!(row(field).contains("⌕ is:open label:bug"), "{}", row(field));
    assert!(!row(state.header_search()).contains("label:bug"));
    press(&mut state, "<Enter>");
    let query = state.route().and_then(Route::list_query);
    assert_eq!(query, Some("is:open label:bug"));
}

#[test]
fn profile_stars_dark() {
    insta::assert_snapshot!(render(&with_profile(Mode::Dark, ProfileTab::Stars)));
}

#[test]
fn repo_wide_with_about_dark() {
    let mut state = with_repo(Mode::Dark, ColorDepth::TrueColor);
    update(&mut state, Msg::Resize(150, 36));
    insta::assert_snapshot!(render(&state));
}

#[test]
fn menu_overlay_dark() {
    let mut state = with_home(Mode::Dark, ColorDepth::TrueColor);
    crate::nav::open_menu(&mut state);
    insta::assert_snapshot!(render(&state));
}

#[test]
fn palette_light() {
    let mut state = with_home(Mode::Light, ColorDepth::TrueColor);
    let _ = state.open_picker(crate::picker::Kind::Commands);
    if let Some(Overlay::Picker(p)) = &mut state.overlay {
        p.input.insert_str("ratatui");
    }
    insta::assert_snapshot!(render(&state));
}

/// The common small terminal: lists go compact (one row per item).
#[test]
fn small_terminal_80x24() {
    let small = |mut state: State| {
        update(&mut state, Msg::Resize(80, 24));
        render(&state)
    };
    insta::assert_snapshot!(
        "small_home",
        small(with_home(Mode::Dark, ColorDepth::TrueColor))
    );
    insta::assert_snapshot!("small_issues", small(with_issues(Mode::Dark)));
    insta::assert_snapshot!("small_pr", small(with_pr(Mode::Dark)));
    let mut menu = with_repo(Mode::Dark, ColorDepth::TrueColor);
    crate::nav::open_menu(&mut menu);
    insta::assert_snapshot!("small_menu", small(menu));
}

#[test]
fn narrow_terminal_does_not_panic() {
    for (w, h) in [(1, 1), (9, 2), (20, 5), (40, 10)] {
        for mode in [Mode::Light, Mode::Dark] {
            let mut state = with_pr(mode);
            update(&mut state, Msg::Resize(w, h));
            render(&state);
            state.screens.truncate(1);
            update(&mut state, Msg::Resize(w, h));
            render(&state);
            crate::nav::open_menu(&mut state);
            render(&state);
            let mut state = with_repo(mode, ColorDepth::TrueColor);
            update(&mut state, Msg::Resize(w, h));
            render(&state);
        }
    }
}

// ---- diff screen -----------------------------------------------------------

pub(crate) use diff::diff_state as diff_fixture;

pub(crate) mod diff;

// ---- links that stay in ghtui ----------------------------------------------

mod links;

// ---- keys on every screen ---------------------------------------------------

mod keys;

/// Changing GitHub: each action asks first where it should, says why
/// when it can't, sends one change, and ends up showing what GitHub says.
mod changes;

/// Merging when ready, and through a merge queue.
mod merging;
