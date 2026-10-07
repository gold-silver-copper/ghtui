//! Full-screen snapshots through ratatui's `TestBackend`, in light and dark
//! schemes (and 256 colors). Snapshots include cell styles, so color changes
//! show up in review. Rendering also exercises the theme's debug assertion
//! that every fg/bg pair used is declared (and therefore contrast-tested).

use ghtui_api::browse::{RepoSort, SearchKind, SearchResults};
use ghtui_api::model::{
    ChecksState, Inbox, Label, Mergeable, PrDetail, PrRef, PrState, PrSummary, RepoId,
    ReviewDecision,
};
use ghtui_api::rate_limit::{Bucket, RateLimits};
use ghtui_theme::{ColorDepth, DEFAULT_SEED, Mode, Theme};
use ghtui_ui::Icons;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use ghtui_ui::pages::{PrTab, ProfileTab};

use crate::browse::{Data, DataKey, Need, needs};
use crate::fixtures::{self, fetched, press};
use crate::keymap::Keymap;
use crate::route::{OPEN, Route};
use crate::state::{Msg, Overlay, State, update};
use crate::view::view;

/// 2026-10-03T12:00:00Z
const NOW: u64 = 1_791_028_800;

fn summary(
    pr: &str,
    title: &str,
    state: PrState,
    review: Option<ReviewDecision>,
    checks: Option<ChecksState>,
) -> PrSummary {
    PrSummary {
        pr: PrRef::parse(pr).unwrap(),
        title: title.into(),
        author: "octocat".into(),
        state,
        updated_at: "2026-10-03T09:00:00Z".into(),
        additions: 120,
        deletions: 34,
        comments: 3,
        review,
        checks,
    }
}

fn inbox() -> Inbox {
    Inbox {
        review_requested: vec![
            summary(
                "ratatui/ratatui#1820",
                "Add a virtualized list widget for very long collections",
                PrState::Open,
                Some(ReviewDecision::ReviewRequired),
                Some(ChecksState::Passing),
            ),
            summary(
                "tokio-rs/tokio#7001",
                "Fix waker leak in the multi-thread scheduler when tasks are cancelled during shutdown",
                PrState::Draft,
                None,
                Some(ChecksState::Pending),
            ),
        ],
        authored: vec![
            summary(
                "gold-silver-copper/ghtui#12",
                "Theme: generate syntax palette from seed",
                PrState::Open,
                Some(ReviewDecision::Approved),
                Some(ChecksState::Failing),
            ),
            summary(
                "gold-silver-copper/ghtui#9",
                "Cache GraphQL responses in redb",
                PrState::Open,
                Some(ReviewDecision::ChangesRequested),
                None,
            ),
        ],
        review_requested_total: 2,
        authored_total: 31,
    }
}

pub(crate) fn pr_detail() -> PrDetail {
    PrDetail {
        summary: summary(
            "gold-silver-copper/ghtui#12",
            "Theme: generate syntax palette from seed",
            PrState::Open,
            Some(ReviewDecision::Approved),
            Some(ChecksState::Failing),
        ),
        body: "Derives keyword, string and type colors from the seed's HCT palettes.\n\n\
               Every syntax role is now checked against every diff background, in both \
               schemes and at both color depths."
            .into(),
        created_at: "2026-09-30T12:00:00Z".into(),
        base_ref: "main".into(),
        head_ref: "syntax-palette".into(),
        base_oid: "0123456789abcdef0123456789abcdef01234567".into(),
        head_oid: "fedcba9876543210fedcba9876543210fedcba98".into(),
        head_repo: Some("octocat/ghtui".into()),
        changed_files: 7,
        mergeable: Mergeable::Conflicting,
        milestone: Some(ghtui_api::model::MilestoneRef {
            number: 3,
            title: "Links".into(),
        }),
        labels: vec![
            Label {
                name: "enhancement".into(),
                color: "a2eeef".into(),
            },
            Label {
                name: "theme".into(),
                color: "7057ff".into(),
            },
        ],
    }
}

fn state(mode: Mode, depth: ColorDepth) -> State {
    let mut state = State::new(
        Theme::new(DEFAULT_SEED, mode, depth),
        Icons::default(),
        Keymap::default(),
        (100, 30),
    );
    state.viewer = Some("octocat".into());
    state.clock = || NOW;
    state.rate_limits = RateLimits {
        graphql: Some(Bucket {
            remaining: 4987,
            limit: 5000,
            reset_at: NOW + 3600,
        }),
        rest: None,
    };
    state
}

fn with_inbox(mode: Mode, depth: ColorDepth) -> State {
    let mut state = state(mode, depth);
    update(&mut state, Msg::Inbox(Ok(inbox())));
    fetched(
        &mut state,
        DataKey::ViewerRepos,
        Data::Repos(vec![
            fixtures::repo_summary("gold-silver-copper/ghtui", 1234),
            fixtures::repo_summary("gold-silver-copper/fux", 87),
        ]),
    );
    state
}

fn with_pr(mode: Mode) -> State {
    let mut state = with_inbox(mode, ColorDepth::TrueColor);
    let pr = PrRef::parse("gold-silver-copper/ghtui#12").unwrap();
    let _ = state.push(Route::pr(pr.clone()));
    update(&mut state, Msg::Pr(pr.clone(), Box::new(Ok(pr_detail()))));
    fetched(
        &mut state,
        DataKey::PrActivity(pr),
        Data::PrActivity(Box::new(fixtures::activity())),
    );
    state
}

/// Pushes `route` and delivers `data` for its page's own fetch (its last).
fn open(state: &mut State, route: Route, data: Data) {
    let Some(Need::Data(key)) = needs(&route).pop() else {
        panic!("{route:?} doesn't end in a data fetch");
    };
    let _ = state.push(route);
    fetched(state, key, data);
}

fn ghtui() -> RepoId {
    RepoId::new("gold-silver-copper", "ghtui")
}

/// A repository's Code tab with everything it asked for delivered.
fn with_repo(mode: Mode, depth: ColorDepth) -> State {
    let mut state = state(mode, depth);
    let _ = state.push(Route::Repo(ghtui()));
    fetched(
        &mut state,
        DataKey::Repo(ghtui()),
        Data::Repo(Box::new(fixtures::overview())),
    );
    fetched(
        &mut state,
        DataKey::LastCommits(ghtui(), "HEAD".into(), String::new()),
        Data::LastCommits(std::sync::Arc::new(fixtures::last_commits())),
    );
    state
}

/// A file in the repository.
fn with_file(mode: Mode) -> State {
    let mut state = with_repo(mode, ColorDepth::TrueColor);
    let route = Route::blob(ghtui(), "main".into(), "src/main.rs".into());
    open(&mut state, route, Data::Blob(Box::new(fixtures::blob())));
    state
}

/// A file's blame.
fn with_blame(mode: Mode) -> State {
    let mut state = with_repo(mode, ColorDepth::TrueColor);
    let (repo, rev, path) = (ghtui(), "main".to_owned(), "src/main.rs".to_owned());
    let blob = DataKey::Blob(repo.clone(), rev.clone(), path.clone());
    let route = Route::Blame {
        repo,
        rev,
        path,
        lines: Some((5, 5)),
    };
    let _ = state.push(route.clone());
    fetched(&mut state, blob, Data::Blob(Box::new(fixtures::blob())));
    let Some(Need::Data(key)) = needs(&route).pop() else {
        panic!("a blame fetches its blame");
    };
    fetched(&mut state, key, Data::Blame(Box::new(fixtures::blame())));
    state
}
/// A gist.
fn with_gist(mode: Mode) -> State {
    let mut state = state(mode, ColorDepth::TrueColor);
    let route = Route::Gist {
        owner: Some("octocat".into()),
        id: "6cad326836d38bd3a7ae".into(),
    };
    open(&mut state, route, Data::Gist(Box::new(fixtures::gist())));
    state
}

/// Someone's gists.
fn with_gists(mode: Mode) -> State {
    let mut state = state(mode, ColorDepth::TrueColor);
    let route = Route::Gists("octocat".into());
    open(&mut state, route, Data::Gists(Box::new(fixtures::gists())));
    state
}
/// An organization's teams.
fn with_teams(mode: Mode) -> State {
    let mut state = state(mode, ColorDepth::TrueColor);
    open(
        &mut state,
        Route::Teams("github".into()),
        Data::Teams(Box::new(fixtures::teams())),
    );
    state
}

/// A team.
fn with_team(mode: Mode) -> State {
    let mut state = state(mode, ColorDepth::TrueColor);
    let route = Route::Team {
        org: "github".into(),
        slug: "core".into(),
    };
    open(&mut state, route, Data::Team(Box::new(fixtures::team())));
    state
}
/// A repository's security advisories.
fn with_advisories(mode: Mode) -> State {
    let mut state = with_repo(mode, ColorDepth::TrueColor);
    let route = Route::Advisories(Some(ghtui()));
    open(&mut state, route, Data::Advisories(fixtures::advisories()));
    state
}

/// A security advisory from GitHub's database.
fn with_advisory(mode: Mode) -> State {
    let mut state = state(mode, ColorDepth::TrueColor);
    let route = Route::Advisory {
        repo: None,
        ghsa: "GHSA-6r3q-mjv7-xr8m".into(),
    };
    open(
        &mut state,
        route,
        Data::Advisory(Box::new(fixtures::advisory())),
    );
    state
}
/// A wiki's Home.
fn with_wiki(mode: Mode) -> State {
    let mut state = with_repo(mode, ColorDepth::TrueColor);
    let route = Route::Wiki {
        repo: ghtui(),
        page: None,
    };
    open(&mut state, route, Data::Wiki(Box::new(fixtures::wiki())));
    state
}
/// The repository's open issues.
fn with_issues(mode: Mode) -> State {
    let mut state = with_repo(mode, ColorDepth::TrueColor);
    let route = Route::Issues {
        repo: ghtui(),
        query: OPEN.into(),
    };
    let results = fixtures::issue_results(Some("c1"));
    open(&mut state, route, Data::Search(Box::new(results)));
    state
}

/// A commit, its repository's header loaded first.
fn with_commit(mode: Mode) -> State {
    let mut state = with_repo(mode, ColorDepth::TrueColor);
    let commit = fixtures::commit();
    let route = Route::Commit {
        repo: ghtui(),
        oid: commit.oid.clone(),
    };
    open(&mut state, route, Data::Commit(Box::new(commit)));
    state
}

/// A pull request's Checks tab.
fn with_pr_checks(mode: Mode) -> State {
    let mut state = with_pr(mode);
    let pr = PrRef::parse("gold-silver-copper/ghtui#12").unwrap();
    let _ = state.push(Route::Pr {
        pr: pr.clone(),
        tab: PrTab::Checks,
    });
    // The switch refreshes the pull request; it comes back unchanged.
    update(&mut state, Msg::Pr(pr.clone(), Box::new(Ok(pr_detail()))));
    let activity = Data::PrActivity(Box::new(fixtures::activity()));
    fetched(&mut state, DataKey::PrActivity(pr.clone()), activity);
    let checks = Data::Checks(Box::new(fixtures::checks()));
    fetched(&mut state, DataKey::PrChecks(pr), checks);
    state
}

/// A repository's Actions tab.
fn with_actions(mode: Mode) -> State {
    let mut state = with_repo(mode, ColorDepth::TrueColor);
    let route = Route::Actions(ghtui());
    open(
        &mut state,
        route,
        Data::Checks(Box::new(fixtures::checks())),
    );
    state
}

/// `state` in a `w`×`h` terminal.
fn sized(mut state: State, w: u16, h: u16) -> State {
    update(&mut state, Msg::Resize(w, h));
    state
}

/// The repository page with `n` more tabs open after it: an issue, a
/// pull request, a profile, a file, a commit…, then back on the second.
fn with_tabs(mode: Mode, n: usize) -> State {
    let mut state = with_repo(mode, ColorDepth::TrueColor);
    let more = [
        Route::Issue {
            repo: ghtui(),
            number: 14,
        },
        Route::pr(PrRef::parse("gold-silver-copper/ghtui#12").unwrap()),
        Route::user("octocat"),
        Route::blob(ghtui(), "main".into(), "crates/app/src/main.rs".into()),
        Route::Commit {
            repo: ghtui(),
            oid: fixtures::commit().oid,
        },
        Route::Releases(ghtui()),
        Route::Stargazers(ghtui()),
    ];
    for route in more.into_iter().cycle().take(n) {
        let _ = state.open_tab(crate::route::Target::Page(route));
    }
    let _ = state.switch_to_tab(1);
    state.notice = None;
    state
}

/// An organization's profile.
fn with_org(mode: Mode) -> State {
    let mut state = state(mode, ColorDepth::TrueColor);
    let route = Route::user("ratatui");
    open(
        &mut state,
        route,
        Data::Profile(Box::new(fixtures::org_profile())),
    );
    state
}

/// A repository's discussions.
fn with_discussions(mode: Mode) -> State {
    let mut state = with_repo(mode, ColorDepth::TrueColor);
    let of = ghtui_api::browse::DiscussionsOf::Repo(ghtui());
    let route = Route::Discussions { of, category: None };
    open(
        &mut state,
        route,
        Data::Discussions(Box::new(fixtures::discussions())),
    );
    state
}

/// A discussion.
fn with_discussion(mode: Mode) -> State {
    let mut state = with_repo(mode, ColorDepth::TrueColor);
    let of = ghtui_api::browse::DiscussionsOf::Repo(ghtui());
    let route = Route::Discussion { of, number: 14603 };
    open(
        &mut state,
        route,
        Data::Discussion(Box::new(fixtures::discussion())),
    );
    state
}
/// A workflow run.
fn with_run(mode: Mode) -> State {
    let mut state = with_repo(mode, ColorDepth::TrueColor);
    let route = Route::WorkflowRun {
        repo: ghtui(),
        run: 7,
        attempt: None,
    };
    open(
        &mut state,
        route,
        Data::Run(Box::new(fixtures::workflow_run())),
    );
    state
}

/// A job, its failed step's log open; `step` is the step a link pointed at.
fn with_job(mode: Mode, step: Option<(u32, u32)>) -> State {
    let mut state = with_repo(mode, ColorDepth::TrueColor);
    let route = Route::Job {
        repo: ghtui(),
        run: Some(7),
        job: 2,
        step,
        query: String::new(),
    };
    let _ = state.push(route);
    let (job, log) = fixtures::job();
    fetched(
        &mut state,
        DataKey::Job(ghtui(), 2),
        Data::Job(Box::new(job)),
    );
    let log = Data::Log(std::sync::Arc::new(log));
    fetched(&mut state, DataKey::JobLog(ghtui(), 2), log);
    state
}

/// A workflow and its runs.
fn with_workflow(mode: Mode) -> State {
    let mut state = with_repo(mode, ColorDepth::TrueColor);
    let route = Route::Workflow {
        repo: ghtui(),
        file: "ci.yml".into(),
    };
    let _ = state.push(route);
    let (workflow, runs) = fixtures::workflow_runs();
    let key = DataKey::Workflow(ghtui(), "ci.yml".into());
    fetched(&mut state, key, Data::Workflow(Box::new(workflow)));
    let key = DataKey::WorkflowRuns(ghtui(), "ci.yml".into());
    fetched(&mut state, key, Data::Runs(Box::new(runs)));
    state
}

/// A repository's stargazers.
fn with_stargazers(mode: Mode) -> State {
    let mut state = with_repo(mode, ColorDepth::TrueColor);
    let route = Route::Stargazers(ghtui());
    open(&mut state, route, Data::Users(Box::new(fixtures::people())));
    state
}

/// A repository's forks.
fn with_forks(mode: Mode) -> State {
    let mut state = with_repo(mode, ColorDepth::TrueColor);
    let route = Route::Forks(ghtui());
    open(
        &mut state,
        route,
        Data::RepoPage(Box::new(fixtures::forks())),
    );
    state
}

/// A repository's releases.
fn with_releases(mode: Mode) -> State {
    let mut state = with_repo(mode, ColorDepth::TrueColor);
    let route = Route::Releases(ghtui());
    open(
        &mut state,
        route,
        Data::Releases(Box::new(fixtures::releases())),
    );
    state
}

/// A release's page.
fn with_release(mode: Mode) -> State {
    let mut state = with_repo(mode, ColorDepth::TrueColor);
    let route = Route::Release {
        repo: ghtui(),
        tag: "v0.2.0".into(),
    };
    open(
        &mut state,
        route,
        Data::Release(Box::new(fixtures::release(true))),
    );
    state
}

/// A repository's tags.
fn with_tags(mode: Mode) -> State {
    let mut state = with_repo(mode, ColorDepth::TrueColor);
    open(
        &mut state,
        Route::Tags(ghtui()),
        Data::Tags(Box::new(fixtures::tags())),
    );
    state
}

/// A repository's branches.
fn with_branches(mode: Mode) -> State {
    let mut state = with_repo(mode, ColorDepth::TrueColor);
    open(
        &mut state,
        Route::Branches(ghtui()),
        Data::Branches(Box::new(fixtures::branches())),
    );
    state
}
/// A repository's milestones.
fn with_milestones(mode: Mode) -> State {
    let mut state = with_repo(mode, ColorDepth::TrueColor);
    let route = Route::Milestones {
        repo: ghtui(),
        closed: false,
    };
    open(
        &mut state,
        route,
        Data::Milestones(Box::new(fixtures::milestones())),
    );
    state
}

/// A milestone.
fn with_milestone(mode: Mode) -> State {
    let mut state = with_repo(mode, ColorDepth::TrueColor);
    let route = Route::Milestone {
        repo: ghtui(),
        number: 3,
    };
    open(
        &mut state,
        route,
        Data::Milestone(Box::new(fixtures::milestone())),
    );
    state
}
/// A repository's deployments.
fn with_deployments(mode: Mode) -> State {
    let mut state = with_repo(mode, ColorDepth::TrueColor);
    let route = Route::Deployments {
        repo: ghtui(),
        environment: None,
    };
    open(
        &mut state,
        route,
        Data::Deployments(Box::new(fixtures::deployments())),
    );
    state
}
/// Two tags compared.
fn with_compare(mode: Mode) -> State {
    let mut state = with_repo(mode, ColorDepth::TrueColor);
    let route = Route::Compare {
        repo: ghtui(),
        spec: "v0.1.0...v0.2.0".into(),
    };
    open(
        &mut state,
        route,
        Data::Compare(Box::new(fixtures::comparison())),
    );
    state
}
/// A branch's commits.
fn with_history(mode: Mode) -> State {
    let mut state = with_repo(mode, ColorDepth::TrueColor);
    let route = Route::Commits {
        repo: ghtui(),
        rev: "main".into(),
        path: String::new(),
    };
    open(
        &mut state,
        route,
        Data::History(Box::new(fixtures::history(Some("h1")))),
    );
    state
}

/// An issue, its repository's header loaded first.
fn with_issue(mode: Mode) -> State {
    let mut state = state(mode, ColorDepth::TrueColor);
    fetched(
        &mut state,
        DataKey::Repo(ghtui()),
        Data::Repo(Box::new(fixtures::overview())),
    );
    let route = Route::Issue {
        repo: ghtui(),
        number: 14,
    };
    let issue = Data::Issue(Some(Box::new(fixtures::issue())));
    open(&mut state, route, issue);
    state
}

fn with_profile(mode: Mode, tab: ProfileTab) -> State {
    let mut state = state(mode, ColorDepth::TrueColor);
    let route = Route::User {
        login: "octocat".into(),
        tab,
    };
    let _ = state.push(route.clone());
    let profile = Data::Profile(Box::new(fixtures::profile()));
    fetched(&mut state, DataKey::Profile("octocat".into()), profile);
    // The tab's own list.
    if let Some(key) = crate::browse::paged(&route) {
        let list = match key {
            DataKey::Users(_) => Data::Users(Box::new(fixtures::people())),
            _ => Data::RepoPage(Box::new(fixtures::profile_repos())),
        };
        fetched(&mut state, key, list);
    }
    state
}

fn with_search(mode: Mode, kind: SearchKind, query: &str, results: SearchResults) -> State {
    let mut state = state(mode, ColorDepth::TrueColor);
    let query = query.to_owned();
    open(
        &mut state,
        Route::Search { kind, query },
        Data::Search(Box::new(results)),
    );
    state
}

/// Repositories matching "terminal file manager".
fn with_repo_search(mode: Mode) -> State {
    let results = fixtures::repo_results();
    with_search(mode, SearchKind::Repos, "terminal file manager", results)
}

fn render(state: &State) -> String {
    let (w, h) = state.size;
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|frame| view(state, frame, NOW)).unwrap();
    format!("{:?}", terminal.backend().buffer())
}

#[test]
fn home_dark() {
    insta::assert_snapshot!(render(&with_inbox(Mode::Dark, ColorDepth::TrueColor)));
}

#[test]
fn home_light() {
    insta::assert_snapshot!(render(&with_inbox(Mode::Light, ColorDepth::TrueColor)));
}

#[test]
fn home_dark_256() {
    insta::assert_snapshot!(render(&with_inbox(Mode::Dark, ColorDepth::Ansi256)));
}

#[test]
fn home_third_row_selected_light() {
    let mut state = with_inbox(Mode::Light, ColorDepth::TrueColor);
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
    update(
        &mut state,
        Msg::Inbox(Err(ghtui_api::ApiError::Network(
            "connection refused".into(),
        ))),
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
fn actions_light() {
    insta::assert_snapshot!(render(&with_actions(Mode::Light)));
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
    state.notice = None;
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
    let blob = ghtui_api::browse::Blob {
        path: "src/main.rs".into(),
        size: text.len() as u64,
        text: Some(text),
        truncated: false,
    };
    let url = "https://github.com/gold-silver-copper/ghtui/blob/main/src/main.rs#L60-L62";
    let crate::route::Target::Page(route) = crate::route::Target::from_url(url) else {
        panic!("{url}");
    };
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
    press(&mut state, "2");
    insta::assert_snapshot!(render(&state));
}

#[test]
fn pr_refresh_error_dark() {
    let mut state = with_pr(Mode::Dark);
    let pr = PrRef::parse("gold-silver-copper/ghtui#12").unwrap();
    update(
        &mut state,
        Msg::Pr(pr, Box::new(Err(ghtui_api::ApiError::RateLimited(42)))),
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
    press(&mut state, "<Esc>/");
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
    press(&mut state, "2/");
    insta::assert_snapshot!(render(&state));
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
    let mut state = with_inbox(Mode::Dark, ColorDepth::TrueColor);
    crate::nav::open_menu(&mut state);
    insta::assert_snapshot!(render(&state));
}

#[test]
fn palette_light() {
    let mut state = with_inbox(Mode::Light, ColorDepth::TrueColor);
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
        small(with_inbox(Mode::Dark, ColorDepth::TrueColor))
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

pub(crate) mod diff {
    use std::collections::HashSet;
    use std::sync::Arc;

    use ghtui_api::model::PrRef;
    use ghtui_diff::FileDiff;
    use ghtui_git::Oid;
    use ghtui_git::files::{ChangedFile, FileStatus, ZERO_OID};
    use ghtui_git::repo::PrRefs;
    use ghtui_theme::{ColorDepth, Mode};
    use ghtui_ui::diff_doc::{Doc, Pos};

    use super::{NOW, Terminal, TestBackend, press, render, state, view};
    use crate::diff_screen::{DiffOf, DiffScreen, DiffState, Pane};
    use crate::fixtures::diff_msg;
    use crate::state::{DiffMsg, Msg, Screen, State, update};

    fn pr() -> PrRef {
        PrRef::parse("o/r#7").unwrap()
    }

    fn file(
        status: FileStatus,
        old: Option<&str>,
        new: Option<&str>,
        modes: (u32, u32),
    ) -> ChangedFile {
        ChangedFile {
            status,
            old_path: old.map(Into::into),
            new_path: new.map(Into::into),
            old_mode: modes.0,
            new_mode: modes.1,
            old_oid: Oid::new("1".repeat(40)),
            new_oid: if new.is_some() {
                Oid::new("2".repeat(40))
            } else {
                Oid::new(ZERO_OID)
            },
            similarity: (status == FileStatus::Renamed).then_some(94),
        }
    }

    const OLD_RS: &str = "use std::fmt;\n\n/// A point.\npub struct Point {\n    x: i32,\n    y: i32,\n}\n\nimpl Point {\n    pub fn new(x: i32, y: i32) -> Self {\n        Point { x, y }\n    }\n}\n";
    const NEW_RS: &str = "use std::fmt;\n\n/// A point in 2D.\npub struct Point {\n    x: i32,\n    y: i32,\n}\n\nimpl Point {\n    pub fn new(x: i32, y: i32) -> Self {\n        Point { x, y }\n    }\n\n    pub fn origin() -> Self {\n        Self::new(0, \"0\".len() as i32 - 1)\n    }\n}\n";

    fn loaded(doc: Doc) -> DiffState {
        let mut diff = DiffState::loading();
        let refs = PrRefs {
            head: Oid::new("h"),
            base: Oid::new("b"),
            merge_base: Oid::new("m"),
        };
        diff.set_files(refs, doc);
        diff
    }

    /// Opens a diff screen on `diff`, as wide as the terminal.
    fn open_diff(s: &mut State, diff: DiffState, cursor: Pos, focus: Pane) {
        s.diffs.insert(DiffOf::Pr(pr()), diff);
        let mut screen = DiffScreen::new(DiffOf::Pr(pr()), s.size.0);
        screen.cursor = cursor;
        screen.focus = focus;
        s.screens.push(Screen::Diff(Box::new(screen)));
        let _ = s.settle_diff();
    }

    /// The fixture's changed files.
    fn fixture_files() -> Vec<ChangedFile> {
        let regular = (0o100644, 0o100644);
        vec![
            file(
                FileStatus::Modified,
                Some("src/point.rs"),
                Some("src/point.rs"),
                regular,
            ),
            file(
                FileStatus::Renamed,
                Some("docs/old.md"),
                Some("docs/new.md"),
                regular,
            ),
            file(FileStatus::Added, None, Some("notes.txt"), (0, 0o100644)),
            file(FileStatus::Deleted, Some("gone.py"), None, (0o100644, 0)),
            file(
                FileStatus::Modified,
                Some("logo.png"),
                Some("logo.png"),
                regular,
            ),
            file(
                FileStatus::Modified,
                Some("run.sh"),
                Some("run.sh"),
                (0o100644, 0o100755),
            ),
            file(
                FileStatus::Modified,
                Some("Cargo.lock"),
                Some("Cargo.lock"),
                regular,
            ),
            file(
                FileStatus::Modified,
                Some("zz/pending.rs"),
                Some("zz/pending.rs"),
                regular,
            ),
        ]
    }

    pub(crate) fn diff_state() -> DiffState {
        let files = fixture_files();
        let mut doc = Doc::new(files, &HashSet::new());
        let diffs = [
            FileDiff::compute(
                "src/point.rs",
                Some(OLD_RS.as_bytes()),
                Some(NEW_RS.as_bytes()),
            ),
            FileDiff::compute("docs/new.md", Some(b"# Title\n"), Some(b"# Title\n")),
            FileDiff::compute("notes.txt", None, Some(b"first\nsecond, no newline")),
            FileDiff::compute("gone.py", Some(b"def f():\n    return 1\n"), None),
            FileDiff::compute("logo.png", Some(b"\x89PNG\0a"), Some(b"\x89PNG\0bc")),
            FileDiff::compute("run.sh", Some(b"echo hi\n"), Some(b"echo hi\n")),
            FileDiff::compute("Cargo.lock", Some(b"a\n"), Some(b"b\n")),
        ];
        for (i, d) in diffs.into_iter().enumerate() {
            doc.set_diff(i, Arc::new(d));
        }
        loaded(doc)
    }

    pub(super) fn screen_state(mode: Mode, depth: ColorDepth, cursor: Pos, focus: Pane) -> State {
        let mut s = state(mode, depth);
        s.size = (110, 34);
        open_diff(&mut s, diff_state(), cursor, focus);
        s
    }

    /// The fixture's diff in true color, the cursor on `row` of `file`.
    fn diff_at(mode: Mode, file: usize, row: usize) -> State {
        screen_state(mode, ColorDepth::TrueColor, Pos { file, row }, Pane::Diff)
    }

    pub(super) fn commit_of() -> DiffOf {
        DiffOf::Commit(super::ghtui(), crate::fixtures::commit().oid)
    }

    /// A commit's files: its tabs instead of a pull request's.
    #[test]
    fn commit_files_dark() {
        let mut s = state(Mode::Dark, ColorDepth::TrueColor);
        s.size = (110, 34);
        let of = commit_of();
        s.diffs.insert(of.clone(), diff_state());
        s.screens
            .push(Screen::Diff(Box::new(DiffScreen::new(of, s.size.0))));
        let _ = s.settle_diff();
        insta::assert_snapshot!(render(&s));
    }

    /// A link to a file in a diff (`#diff-<hash>`, maybe with a line) goes
    /// to that file once the files are listed.
    #[test]
    fn diff_links_go_to_their_file() {
        let mut s = state(Mode::Dark, ColorDepth::TrueColor);
        let hash = crate::diff_screen::path_hash("notes.txt");
        let url = format!("https://github.com/o/r/pull/7/files#diff-{hash}R1");
        let _ = s.follow(&ghtui_ui::page::Link::Url(url));
        s.diffs.insert(DiffOf::Pr(pr()), diff_state());
        let _ = s.settle_diff();
        let Screen::Diff(d) = s.screen() else {
            panic!("the link didn't open a diff");
        };
        let doc = &s.diffs[&d.of].doc;
        assert_eq!(doc.files()[d.cursor.file].meta.path(), "notes.txt");
        assert_eq!(d.anchor, None);
    }

    /// A link to a review comment (`#discussion_r<id>`) opens the diff at
    /// its thread once the threads are in.
    #[test]
    fn review_comment_links_go_to_their_thread() {
        let mut s = state(Mode::Dark, ColorDepth::TrueColor);
        let url = "https://github.com/o/r/pull/7#discussion_r42".to_owned();
        let _ = s.follow(&ghtui_ui::page::Link::Url(url));
        s.diffs.insert(DiffOf::Pr(pr()), diff_state());
        let _ = s.settle_diff();
        let Screen::Diff(d) = s.screen() else {
            panic!("the link didn't open a diff");
        };
        assert!(d.anchor.is_some(), "waits for the threads");
        let mut thread = crate::fixtures::thread("t", Some(3), false, false);
        thread.comments[0].url = "https://github.com/o/r/pull/7#discussion_r42".into();
        if let Some(diff) = s.diffs.get_mut(&DiffOf::Pr(pr())) {
            diff.set_threads(vec![
                crate::fixtures::thread("u", Some(1), false, false),
                thread,
            ]);
        }
        let _ = s.settle_diff();
        let Screen::Diff(d) = s.screen() else {
            panic!("not a diff");
        };
        let doc = &s.diffs[&d.of].doc;
        assert_eq!(d.anchor, None);
        assert_eq!(doc.annotation_at(d.cursor), Some(1), "the second thread");
    }

    /// A commit's diff has no review threads: a review comment's anchor
    /// there is let go rather than waited on.
    #[test]
    fn review_comment_anchors_on_a_commit_are_let_go() {
        let mut s = state(Mode::Dark, ColorDepth::TrueColor);
        let of = commit_of();
        s.diffs.insert(of.clone(), diff_state());
        let mut screen = DiffScreen::new(of, s.size.0);
        screen.anchor = Some("discussion_r42".into());
        s.screens.push(Screen::Diff(Box::new(screen)));
        let _ = s.settle_diff();
        assert!(matches!(s.screen(), Screen::Diff(d) if d.anchor.is_none()));
    }

    /// Threads that come before their file's diff don't lose the link's
    /// place: it waits for the diff, then goes to the thread.
    #[test]
    fn review_comment_links_wait_for_their_file() {
        let mut s = state(Mode::Dark, ColorDepth::TrueColor);
        let url = "https://github.com/o/r/pull/7#discussion_r42".to_owned();
        let _ = s.follow(&ghtui_ui::page::Link::Url(url));
        let mut diff = loaded(Doc::new(fixture_files(), &HashSet::new()));
        let mut thread = crate::fixtures::thread("t", Some(3), false, false);
        thread.comments[0].url = "https://github.com/o/r/pull/7#discussion_r42".into();
        diff.set_threads(vec![thread]);
        s.diffs.insert(DiffOf::Pr(pr()), diff);
        let _ = s.settle_diff();
        let waiting = matches!(s.screen(), Screen::Diff(d) if d.anchor.is_some());
        assert!(waiting, "waits for src/point.rs's diff");
        if let Some(diff) = s.diffs.get_mut(&DiffOf::Pr(pr())) {
            let d = FileDiff::compute(
                "src/point.rs",
                Some(OLD_RS.as_bytes()),
                Some(NEW_RS.as_bytes()),
            );
            diff.doc.set_diff(0, Arc::new(d));
            diff.refresh_annotations();
        }
        let _ = s.settle_diff();
        let Screen::Diff(d) = s.screen() else {
            panic!("not a diff");
        };
        assert_eq!(d.anchor, None);
        assert_eq!(s.diffs[&d.of].doc.annotation_at(d.cursor), Some(0));
    }

    #[test]
    fn diff_dark() {
        insta::assert_snapshot!(render(&diff_at(Mode::Dark, 0, 4)));
    }

    #[test]
    fn diff_light() {
        insta::assert_snapshot!(render(&diff_at(Mode::Light, 0, 4)));
    }

    #[test]
    fn diff_dark_256_tree_focused() {
        insta::assert_snapshot!(render(&screen_state(
            Mode::Dark,
            ColorDepth::Ansi256,
            Pos { file: 2, row: 0 },
            Pane::Tree,
        )));
    }

    #[test]
    fn diff_scrolled_shows_sticky_header_light() {
        insta::assert_snapshot!(render(&diff_at(Mode::Light, 7, 1)));
    }

    #[test]
    fn diff_split_wide_light() {
        let mut s = diff_at(Mode::Light, 0, 6);
        s.size = (200, 30);
        let _ = s.settle_diff();
        insta::assert_snapshot!(render(&s));
    }

    #[test]
    fn diff_viewed_and_reviewed_dark() {
        let mut s = diff_at(Mode::Dark, 0, 3);
        let diff = s.diffs.get_mut(&DiffOf::Pr(pr())).unwrap();
        let hash = diff.doc.files()[0].blocks()[0].hash.clone();
        diff.set_review(ghtui_store::ReviewState {
            reviewed_hunks: vec![hash],
            ..Default::default()
        });
        diff.doc.set_viewed(1, ghtui_ui::diff_doc::Viewed::Viewed);
        let _ = s.settle_diff();
        insta::assert_snapshot!(render(&s));
    }

    fn with_threads(mode: Mode) -> State {
        use ghtui_api::model::{NodeId, ReviewComment, ReviewThread, Side};
        let mut s = diff_at(mode, 0, 0);
        let comment = |id: &str, author: &str, body: &str, pending: bool| ReviewComment {
            id: NodeId::new(id),
            author: author.into(),
            body: body.into(),
            created_at: "2026-10-03T08:00:00Z".into(),
            url: String::new(),
            original_commit: Some("abc".into()),
            pending,
        };
        let thread = |id: &str, line: Option<u32>, resolved: bool, file_level: bool, comments| {
            ReviewThread {
                id: NodeId::new(id),
                path: "src/point.rs".into(),
                side: Side::Right,
                start_side: None,
                line,
                start_line: None,
                original_line: line,
                original_start_line: None,
                outdated: false,
                resolved,
                file_level,
                can_reply: true,
                can_resolve: true,
                can_unresolve: true,
                comments,
            }
        };
        let diff = s.diffs.get_mut(&DiffOf::Pr(pr())).unwrap();
        diff.set_threads(vec![
            thread(
                "t1",
                Some(3),
                false,
                false,
                vec![
                    comment("c1", "alice", "Should this say \"2D\" or \"two-dimensional\"? The rest of the docs spell it out.", false),
                    comment("c2", "octocat", "Good point, I'll spell it out.", true),
                ],
            ),
            thread("t2", Some(10), true, false, vec![comment("c3", "bob", "Looks fine now.", false)]),
            thread("t3", None, false, true, vec![comment("c4", "carol", "Please add tests for this file.", false)]),
        ]);
        diff.set_review(ghtui_store::ReviewState {
            pending: vec![ghtui_store::DraftComment {
                id: 1,
                path: "src/point.rs".into(),
                body: "Prefer `Self::default()` here.".into(),
                side: ghtui_store::DraftSide::Right,
                line: Some(15),
                start_line: None,
                start_side: None,
                commit: "h".into(),
                error: Some("pull_request_review_thread.line must be part of the diff".into()),
            }],
            ..Default::default()
        });
        let _ = s.settle_diff();
        s
    }

    #[test]
    fn threads_dark() {
        insta::assert_snapshot!(render(&with_threads(Mode::Dark)));
    }

    #[test]
    fn threads_light() {
        insta::assert_snapshot!(render(&with_threads(Mode::Light)));
    }

    #[test]
    fn compose_suggestion_dark() {
        use crate::review::{Compose, ComposeTarget, Preview};
        use ghtui_diff::anchor::{LinePos, Side};
        let mut s = with_threads(Mode::Dark);
        let target = ComposeTarget::Line {
            path: "src/point.rs".into(),
            start: LinePos {
                side: Side::Right,
                line: 14,
            },
            end: LinePos {
                side: Side::Right,
                line: 14,
            },
        };
        let mut compose = Compose::new(
            &s.theme,
            target,
            "```suggestion\n    pub fn zero() -> Self {\n```",
        );
        compose.preview = Some(Preview {
            start_line: 14,
            original: vec!["    pub fn origin() -> Self {".into()],
            suggested: vec!["    pub fn zero() -> Self {".into()],
        });
        s.overlay = Some(crate::state::Overlay::Compose(Box::new(compose)));
        insta::assert_snapshot!(render(&s));
    }

    #[test]
    fn submit_dialog_light() {
        let mut s = with_threads(Mode::Light);
        let mut dialog = crate::review::SubmitDialog::new(&s.theme);
        dialog.cycle(true);
        dialog.input.insert_str("Nice work overall.");
        s.overlay = Some(crate::state::Overlay::Submit(Box::new(dialog)));
        insta::assert_snapshot!(render(&s));
    }

    /// Intra-line emphasis, code moved between files, and a
    /// formatting-only change folded away.
    fn better_than_github(mode: Mode) -> State {
        let regular = (0o100644, 0o100644);
        let files = vec![
            file(
                FileStatus::Modified,
                Some("src/math.rs"),
                Some("src/math.rs"),
                regular,
            ),
            file(
                FileStatus::Modified,
                Some("src/util.rs"),
                Some("src/util.rs"),
                regular,
            ),
        ];
        let helper = "pub fn clamp(value: i32, low: i32, high: i32) -> i32 {\n    value.max(low).min(high)\n}\n";
        let math_old = format!(
            "pub fn area(width: u32, height: u32) -> u32 {{\n    width * height\n}}\n\n{helper}\nlet total = compute(alpha, beta);\n// a\n// b\n// c\n// d\ncall(\n    one,\n    two\n);\n"
        );
        let math_new = "pub fn area(width: u32, height: u32) -> u32 {\n    width * height\n}\n\nlet total = compute(alpha, gamma);\n// a\n// b\n// c\n// d\ncall(one, two);\n";
        let util_old = "// helpers\n";
        let util_new = format!("// helpers\n{helper}");
        let mut doc = Doc::new(files, &HashSet::new());
        doc.set_diff(
            0,
            Arc::new(FileDiff::compute(
                "src/math.rs",
                Some(math_old.as_bytes()),
                Some(math_new.as_bytes()),
            )),
        );
        doc.set_diff(
            1,
            Arc::new(FileDiff::compute(
                "src/util.rs",
                Some(util_old.as_bytes()),
                Some(util_new.as_bytes()),
            )),
        );
        let texts: Vec<_> = doc
            .files()
            .iter()
            .map(|f| f.text().unwrap().clone())
            .collect();
        let moves = ghtui_diff::moves::detect_moves(&[
            (0, &texts[0], texts[0].lines(ghtui_diff::Whitespace::Exact)),
            (1, &texts[1], texts[1].lines(ghtui_diff::Whitespace::Exact)),
        ]);
        assert_eq!(moves.len(), 1);
        doc.set_moves(moves);
        let mut s = state(mode, ColorDepth::TrueColor);
        s.size = (110, 30);
        open_diff(&mut s, loaded(doc), Pos::default(), Pane::Diff);
        s
    }

    #[test]
    fn better_than_github_dark() {
        insta::assert_snapshot!(render(&better_than_github(Mode::Dark)));
    }

    #[test]
    fn better_than_github_light() {
        insta::assert_snapshot!(render(&better_than_github(Mode::Light)));
    }

    #[test]
    fn diff_loading_dark() {
        let mut s = state(Mode::Dark, ColorDepth::TrueColor);
        let mut diff = DiffState::loading();
        diff.progress = Some("Receiving objects:  42% (420/1000), 1.2 MiB | 3.4 MiB/s".into());
        open_diff(&mut s, diff, Pos::default(), Pane::Diff);
        insta::assert_snapshot!(render(&s));
    }

    #[test]
    fn small_diff_80x24() {
        let mut s = diff_at(Mode::Dark, 0, 4);
        s.size = (80, 24);
        let _ = s.settle_diff();
        insta::assert_snapshot!(render(&s));
    }

    /// A long git error wraps instead of being cut off.
    #[test]
    fn diff_long_error_dark() {
        let mut s = state(Mode::Dark, ColorDepth::TrueColor);
        s.size = (80, 24);
        let mut diff = DiffState::loading();
        diff.progress = None;
        diff.error = Some(
            "git fetch failed: fatal: unable to access 'https://github.com/o/r.git/': \
             Could not resolve host: github.com (is the network up?)"
                .into(),
        );
        open_diff(&mut s, diff, Pos::default(), Pane::Diff);
        let screen = render(&s);
        assert!(screen.contains("network up?)"), "{screen}");
        insta::assert_snapshot!(screen);
    }

    /// Every screen and overlay, at every tiny size and a few extreme
    /// ones: rendering never panics.
    #[test]
    fn every_screen_survives_every_size() {
        use super::{
            DataKey, Overlay, ProfileTab, fetched, ghtui, with_file, with_inbox, with_issue,
            with_issues, with_pr, with_profile, with_repo, with_repo_search,
        };
        use crate::browse::Data;
        type Build = Box<dyn Fn() -> State>;
        let tc = ColorDepth::TrueColor;
        let pressed = |mut s: State, keys: &str| {
            press(&mut s, keys);
            s
        };
        let builders: Vec<(&str, Build)> = vec![
            ("home", Box::new(move || with_inbox(Mode::Dark, tc))),
            ("home loading", Box::new(move || state(Mode::Dark, tc))),
            ("repo", Box::new(move || with_repo(Mode::Light, tc))),
            ("file", Box::new(|| with_file(Mode::Light))),
            ("issues", Box::new(|| with_issues(Mode::Dark))),
            ("issue", Box::new(|| with_issue(Mode::Light))),
            ("pr", Box::new(|| with_pr(Mode::Dark))),
            (
                "pr commits",
                Box::new(move || pressed(with_pr(Mode::Dark), "2")),
            ),
            (
                "profile",
                Box::new(|| with_profile(Mode::Light, ProfileTab::Overview)),
            ),
            ("search", Box::new(|| with_repo_search(Mode::Dark))),
            (
                "hints",
                Box::new(move || pressed(with_repo(Mode::Dark, tc), "i")),
            ),
            (
                "search box",
                Box::new(move || pressed(with_repo(Mode::Dark, tc), "/oct")),
            ),
            (
                "menu",
                Box::new(move || pressed(with_repo(Mode::Dark, tc), "<Space>")),
            ),
            (
                "which key",
                Box::new(move || pressed(with_repo(Mode::Dark, tc), "g")),
            ),
            (
                "list filter",
                Box::new(move || pressed(with_repo(Mode::Light, tc), "2/")),
            ),
            (
                "palette",
                Box::new(move || {
                    let mut s = with_inbox(Mode::Light, tc);
                    let _ = s.open_picker(crate::picker::Kind::Commands);
                    s
                }),
            ),
            (
                "go to file",
                Box::new(move || {
                    let mut s = pressed(with_repo(Mode::Light, tc), "f");
                    let files = vec!["src/main.rs".into(), "crates/ui/src/page.rs".into()];
                    fetched(
                        &mut s,
                        DataKey::Files(ghtui(), "main".into()),
                        Data::Files(Arc::new(files), false),
                    );
                    s
                }),
            ),
            ("diff", Box::new(|| diff_at(Mode::Dark, 0, 4))),
            (
                "diff tree",
                Box::new(move || screen_state(Mode::Dark, tc, Pos { file: 2, row: 0 }, Pane::Tree)),
            ),
            (
                "diff search",
                Box::new(move || {
                    let s = diff_at(Mode::Dark, 0, 4);
                    pressed(s, "/point")
                }),
            ),
            ("threads", Box::new(|| with_threads(Mode::Dark))),
            (
                "submit",
                Box::new(|| {
                    let mut s = with_threads(Mode::Light);
                    let dialog = crate::review::SubmitDialog::new(&s.theme);
                    s.overlay = Some(Overlay::Submit(Box::new(dialog)));
                    s
                }),
            ),
            ("moves", Box::new(|| better_than_github(Mode::Dark))),
            (
                "diff loading",
                Box::new(move || {
                    let mut s = state(Mode::Dark, tc);
                    open_diff(&mut s, DiffState::loading(), Pos::default(), Pane::Diff);
                    s
                }),
            ),
        ];
        let tiny = (0..=12).flat_map(|w| (0..=12).map(move |h| (w, h)));
        let sizes: Vec<(u16, u16)> = tiny.chain([(1, 200), (200, 1), (80, 24)]).collect();
        for (name, build) in &builders {
            for &(w, h) in &sizes {
                let mut s = build();
                update(&mut s, Msg::Resize(w, h));
                let _ = s.settle_diff();
                let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
                    terminal.draw(|frame| view(&s, frame, NOW)).unwrap();
                }));
                assert!(caught.is_ok(), "{name} panicked at {w}x{h}");
            }
        }
    }

    /// Scrolling stays cheap on a big PR: rendering only touches visible rows.
    /// Run with `cargo test --release -p ghtui -- --ignored scroll_timing`.
    #[test]
    #[ignore = "timing; run in release"]
    #[expect(clippy::print_stderr, reason = "reports the timings")]
    fn scroll_timing_on_500_files_20k_lines() {
        let regular = (0o100644, 0o100644);
        let files: Vec<ChangedFile> = (0..500)
            .map(|i| {
                let path = format!("src/m{i}.rs");
                file(FileStatus::Modified, Some(&path), Some(&path), regular)
            })
            .collect();
        let mut doc = Doc::new(files, &HashSet::new());
        let old: String = (0..60)
            .map(|i| format!("    let v{i} = compute({i}, \"x\");\n"))
            .collect();
        let new: String = old.replace("compute(", "compute_fast(");
        let diff = Arc::new(FileDiff::compute(
            "m.rs",
            Some(old.as_bytes()),
            Some(new.as_bytes()),
        ));
        for i in 0..500 {
            doc.set_diff(i, diff.clone());
        }
        let changed: u32 = doc
            .files()
            .iter()
            .map(|f| f.diff.as_ref().unwrap().additions)
            .sum();
        assert!(changed >= 20_000, "{changed}");

        let mut s = state(Mode::Dark, ColorDepth::TrueColor);
        s.size = (160, 50);
        open_diff(&mut s, loaded(doc), Pos::default(), Pane::Diff);

        let mut terminal = Terminal::new(TestBackend::new(160, 50)).unwrap();
        let frames = 2000;
        let start = std::time::Instant::now();
        for i in 0..frames {
            press(&mut s, if i % 100 < 95 { "j" } else { "<C-d>" });
            terminal.draw(|frame| view(&s, frame, NOW)).unwrap();
        }
        let per_frame = start.elapsed() / frames;
        eprintln!("update + render per frame: {per_frame:?}");
        assert!(
            per_frame < std::time::Duration::from_millis(8),
            "{per_frame:?} per frame"
        );
    }

    /// A single 50k-line file: diffing and highlighting run on the diff job's
    /// workers (never the UI task); once its result arrives, the first screen
    /// must be ready within 100ms. This measures the worker's compute plus the
    /// UI's update and first render.
    /// Run with `cargo test --release -p ghtui -- --ignored big_file_timing`.
    #[test]
    #[ignore = "timing; run in release"]
    #[expect(clippy::print_stderr, reason = "reports the timings")]
    fn big_file_timing_50k_lines() {
        use std::time::{Duration, Instant};

        let old: String = (0..50_000)
            .map(|i| format!("    let value_{i} = compute({i}, \"label\");\n"))
            .collect();
        let new = old
            .replace("compute(100,", "compute_fast(100,")
            .replace("compute(25000,", "compute_fast(25000,")
            .replace("compute(49999,", "compute_fast(49999,");

        let start = Instant::now();
        let diff = Arc::new(FileDiff::compute(
            "big.rs",
            Some(old.as_bytes()),
            Some(new.as_bytes()),
        ));
        let compute = start.elapsed();

        let regular = (0o100644, 0o100644);
        let big = file(
            FileStatus::Modified,
            Some("big.rs"),
            Some("big.rs"),
            regular,
        );
        let mut s = state(Mode::Dark, ColorDepth::TrueColor);
        s.size = (160, 50);
        let doc = Doc::new(vec![big], &HashSet::new());
        open_diff(&mut s, loaded(doc), Pos::default(), Pane::Diff);
        let mut terminal = Terminal::new(TestBackend::new(160, 50)).unwrap();

        let start = Instant::now();
        let job = s.diffs[&DiffOf::Pr(pr())].job;
        diff_msg(
            &mut s,
            &pr(),
            DiffMsg::Job(job, crate::diff_job::JobMsg::File(0, diff)),
        );
        terminal.draw(|frame| view(&s, frame, NOW)).unwrap();
        let first_screen = start.elapsed();

        // Full-file mode on 50k lines must stay interactive too.
        let start = Instant::now();
        press(&mut s, "F");
        terminal.draw(|frame| view(&s, frame, NOW)).unwrap();
        let full_file = start.elapsed();

        eprintln!("compute {compute:?}, first screen {first_screen:?}, full file {full_file:?}");
        assert!(
            compute + first_screen < Duration::from_millis(100),
            "{:?}",
            compute + first_screen
        );
        assert!(full_file < Duration::from_millis(100), "{full_file:?}");
    }
}

// ---- links that stay in ghtui ----------------------------------------------

mod links {
    use ghtui_theme::{ColorDepth, Mode};
    use ghtui_ui::pages::ProfileTab;

    use super::{
        press, with_actions, with_advisories, with_advisory, with_blame, with_branches,
        with_commit, with_compare, with_deployments, with_discussion, with_discussions, with_file,
        with_forks, with_gist, with_gists, with_history, with_inbox, with_issue, with_issues,
        with_job, with_milestone, with_milestones, with_pr, with_pr_checks, with_profile,
        with_release, with_releases, with_repo, with_repo_search, with_run, with_search,
        with_stargazers, with_tags, with_team, with_teams, with_wiki, with_workflow,
    };
    use crate::route::Target;
    use crate::state::{Screen, State};

    /// A link's shape: on github.com its first four path segments, with
    /// the owner and repository, numbers and commit IDs as wildcards; on
    /// GitHub's other hosts (raw files, gists, avatars) the host.
    fn shape(url: &str) -> Vec<String> {
        let Ok(parsed) = url::Url::parse(url) else {
            return vec![url.to_owned()];
        };
        let mut shape = vec![parsed.host_str().unwrap_or_default().to_owned()];
        if parsed.host_str() != Some("github.com") {
            return shape;
        }
        let segments = parsed.path_segments().into_iter().flatten();
        for (i, s) in segments.take(4).enumerate() {
            let wild = i < 2 || s.bytes().any(|b| b.is_ascii_digit());
            shape.push(if wild { "*".into() } else { s.to_owned() });
        }
        shape
    }

    /// Whether the corpus says links shaped like `url` open the browser
    /// (or will open a page once it's built).
    fn external_in_corpus(url: &str) -> bool {
        let wanted = shape(url);
        crate::route::tests::corpus()
            .iter()
            .any(|(entry, expect, _)| {
                matches!(*expect, "external" | "todo") && shape(entry) == wanted
            })
    }

    /// Every page ghtui has a fixture for.
    fn pages() -> Vec<(&'static str, State)> {
        let tc = ColorDepth::TrueColor;
        let mut commits = with_pr(Mode::Dark);
        press(&mut commits, "2");
        vec![
            ("home", with_inbox(Mode::Dark, tc)),
            ("repo", with_repo(Mode::Dark, tc)),
            ("file", with_file(Mode::Dark)),
            ("issues", with_issues(Mode::Dark)),
            ("issue", with_issue(Mode::Dark)),
            ("pr", with_pr(Mode::Dark)),
            ("pr commits", commits),
            ("profile", with_profile(Mode::Dark, ProfileTab::Overview)),
            (
                "repositories",
                with_profile(
                    Mode::Dark,
                    ProfileTab::Repositories(ghtui_api::browse::RepoSort::Updated),
                ),
            ),
            ("stars", with_profile(Mode::Dark, ProfileTab::Stars)),
            ("search", with_repo_search(Mode::Dark)),
            ("commit", with_commit(Mode::Dark)),
            ("commits", with_history(Mode::Dark)),
            ("pr checks", with_pr_checks(Mode::Dark)),
            ("actions", with_actions(Mode::Dark)),
            ("stargazers", with_stargazers(Mode::Dark)),
            ("forks", with_forks(Mode::Dark)),
            ("releases", with_releases(Mode::Dark)),
            ("release", with_release(Mode::Dark)),
            ("tags", with_tags(Mode::Dark)),
            ("branches", with_branches(Mode::Dark)),
            (
                "discussion search",
                with_search(
                    Mode::Dark,
                    ghtui_api::browse::SearchKind::Discussions,
                    "q",
                    crate::fixtures::discussion_results(),
                ),
            ),
            (
                "commit search",
                with_search(
                    Mode::Dark,
                    ghtui_api::browse::SearchKind::Commits,
                    "q",
                    crate::fixtures::commit_results(),
                ),
            ),
            (
                "code search",
                with_search(
                    Mode::Dark,
                    ghtui_api::browse::SearchKind::Code,
                    "q",
                    crate::fixtures::code_results(),
                ),
            ),
            ("wiki", with_wiki(Mode::Dark)),
            ("advisories", with_advisories(Mode::Dark)),
            ("advisory", with_advisory(Mode::Dark)),
            ("teams", with_teams(Mode::Dark)),
            ("team", with_team(Mode::Dark)),
            ("gist", with_gist(Mode::Dark)),
            ("gists", with_gists(Mode::Dark)),
            ("blame", with_blame(Mode::Dark)),
            ("compare", with_compare(Mode::Dark)),
            ("deployments", with_deployments(Mode::Dark)),
            ("milestones", with_milestones(Mode::Dark)),
            ("milestone", with_milestone(Mode::Dark)),
            ("run", with_run(Mode::Dark)),
            ("discussions", with_discussions(Mode::Dark)),
            ("discussion", with_discussion(Mode::Dark)),
            ("job", with_job(Mode::Dark, None)),
            ("workflow", with_workflow(Mode::Dark)),
        ]
    }

    /// ghtui's own pages lead to the new ones as GitHub's do: an issue's
    /// and a pull request's sidebar to their milestone, a repository's
    /// toolbar to its branches and tags.
    #[test]
    fn pages_link_to_milestones_branches_and_tags() {
        use crate::route::Route;
        let routes = |s: &State| -> Vec<Route> {
            let Screen::Page(p) = s.screen() else {
                return Vec::new();
            };
            p.page
                .links
                .iter()
                .filter_map(|l| l.url())
                .filter_map(|u| match Target::from_url(u) {
                    Target::Page(r) => Some(r),
                    _ => None,
                })
                .collect()
        };
        let milestone = |repo: &str| Route::Milestone {
            repo: ghtui_api::model::RepoId::parse(repo).unwrap(),
            number: 3,
        };
        let issue = super::sized(with_issue(Mode::Dark), 160, 40);
        assert!(routes(&issue).contains(&milestone("gold-silver-copper/ghtui")));
        let pr = super::sized(with_pr(Mode::Dark), 160, 40);
        let Screen::Page(p) = pr.screen() else {
            panic!("not a page");
        };
        let Route::Pr { pr: pr_ref, .. } = &p.route else {
            panic!("not a pull request");
        };
        assert!(routes(&pr).contains(&milestone(&pr_ref.repo.to_string())));
        let repo = with_repo(Mode::Dark, ColorDepth::TrueColor);
        let repo_routes = routes(&repo);
        let id = super::ghtui();
        assert!(repo_routes.contains(&Route::Branches(id.clone())));
        assert!(repo_routes.contains(&Route::Tags(id)));
    }

    /// Every github.com link on ghtui's pages (and their tabs) opens a
    /// ghtui page, unless the corpus (`tests/github_urls.txt`) says links of
    /// its shape open the browser, and why.
    #[test]
    fn github_links_stay_in_ghtui() {
        let mut leaving = Vec::new();
        for (name, s) in pages() {
            let Screen::Page(p) = s.screen() else {
                panic!("{name} isn't a page");
            };
            let tabs = s.chrome().tabs.into_iter().map(|(_, t)| t);
            let links = p.page.links.iter().filter_map(|l| l.url());
            let targets = links.map(Target::from_url).chain(tabs);
            for target in targets {
                // A page of a kind the corpus has an example of.
                let kind = match &target {
                    Target::Page(route) => Some(crate::route::tests::variant(route)),
                    Target::Files(_) => Some("files".to_owned()),
                    Target::External(_) => None,
                };
                if let Some(kind) = kind
                    && !crate::route::tests::corpus()
                        .iter()
                        .any(|(_, e, _)| *e == kind)
                {
                    leaving.push(format!("{name}: no {kind} in the corpus"));
                }
                if let Target::External(url) = target
                    && url.contains("github")
                    && !external_in_corpus(&url)
                {
                    leaving.push(format!("{name}: {url}"));
                }
            }
        }
        leaving.sort();
        leaving.dedup();
        assert!(
            leaving.is_empty(),
            "links that leave ghtui:\n{}",
            leaving.join("\n")
        );
    }
}

// ---- keys on every screen ---------------------------------------------------

mod keys {
    use ghtui_theme::{ColorDepth, Mode};
    use ghtui_ui::diff_doc::Pos;

    use super::diff::screen_state;
    use super::{
        press, render, with_advisories, with_advisory, with_blame, with_branches, with_commit,
        with_compare, with_deployments, with_discussion, with_discussions, with_gist, with_gists,
        with_issues, with_job, with_milestone, with_milestones, with_pr, with_run, with_search,
        with_team, with_teams, with_wiki, with_workflow,
    };
    use crate::diff_screen::Pane;
    use crate::keymap::Action;
    use crate::state::{Overlay, Screen, State, apply};

    type Make = fn() -> State;

    /// A list, a pull request, the diff with each pane focused (the
    /// selection away from the edges so moving can move), and every page
    /// links lead to.
    fn screens() -> [(&'static str, Make); 25] {
        fn moved(mut s: State) -> State {
            press(&mut s, "jj");
            s
        }
        /// A page in a short terminal, scrolled into its middle, so every
        /// way of moving can move.
        fn mid(s: State) -> State {
            let mut s = super::sized(s, 100, 12);
            press(&mut s, "j");
            s
        }
        [
            ("issue list", || moved(with_issues(Mode::Dark))),
            ("pull request", || moved(with_pr(Mode::Dark))),
            ("diff", || {
                let at = Pos { file: 0, row: 4 };
                screen_state(Mode::Dark, ColorDepth::TrueColor, at, Pane::Diff)
            }),
            ("file tree", || {
                let at = Pos { file: 2, row: 0 };
                moved(screen_state(
                    Mode::Dark,
                    ColorDepth::TrueColor,
                    at,
                    Pane::Tree,
                ))
            }),
            ("commit", || with_commit(Mode::Dark)),
            ("commit files", || {
                let mut s = with_commit(Mode::Dark);
                press(&mut s, "<Right><Right>");
                if let Some((screen, _)) = s.diff_parts() {
                    screen.cursor = Pos { file: 0, row: 4 };
                }
                s.diffs
                    .insert(super::diff::commit_of(), super::diff::diff_state());
                s
            }),
            ("workflow run", || mid(with_run(Mode::Dark))),
            ("job", || {
                let mut s = mid(with_job(Mode::Dark, None));
                press(&mut s, "j");
                s
            }),
            ("workflow", || mid(with_workflow(Mode::Dark))),
            ("discussions", || mid(with_discussions(Mode::Dark))),
            ("discussion", || mid(with_discussion(Mode::Dark))),
            ("branches", || mid(with_branches(Mode::Dark))),
            ("milestones", || mid(with_milestones(Mode::Dark))),
            ("milestone", || mid(with_milestone(Mode::Dark))),
            ("deployments", || mid(with_deployments(Mode::Dark))),
            ("comparison", || mid(with_compare(Mode::Dark))),
            ("blame", || mid(with_blame(Mode::Dark))),
            ("gist", || mid(with_gist(Mode::Dark))),
            ("gists", || mid(with_gists(Mode::Dark))),
            ("teams", || mid(with_teams(Mode::Dark))),
            ("team", || mid(with_team(Mode::Dark))),
            ("advisories", || mid(with_advisories(Mode::Dark))),
            ("advisory", || mid(with_advisory(Mode::Dark))),
            ("wiki", || mid(with_wiki(Mode::Dark))),
            ("code search", || {
                let results = crate::fixtures::code_results();
                mid(with_search(
                    Mode::Dark,
                    ghtui_api::browse::SearchKind::Code,
                    "q",
                    results,
                ))
            }),
        ]
    }

    /// Every action does something on every screen, or says why not.
    #[test]
    fn no_action_is_silent() {
        let mut silent = Vec::new();
        for (name, make) in screens() {
            for &action in Action::ALL {
                let mut s = make();
                let before = render(&s);
                let cmds = apply(&mut s, action);
                let _ = s.settle();
                if cmds.is_empty() && s.notice.is_none() && !s.quit && render(&s) == before {
                    silent.push(format!("{name}: {}", action.name()));
                }
            }
        }
        assert!(silent.is_empty(), "silent:\n{}", silent.join("\n"));
    }

    /// `→` walks a pull request's tabs into Files changed and `←` walks
    /// back; so do `l` `h`, the numbers, and `←` `→` wrapping around.
    #[test]
    fn tabs_go_into_the_files_and_back() {
        let pairs = [
            ("<Right><Right><Right>", "<Left><Left><Left>"),
            ("lll", "hhh"),
            ("4", "1"),
            ("<Left>", "<Right>"),
        ];
        for (there, back) in pairs {
            let mut s = with_pr(Mode::Dark);
            let start = s.chrome().active;
            press(&mut s, there);
            assert!(matches!(s.screen(), Screen::Diff(_)), "{there}");
            press(&mut s, back);
            assert!(matches!(s.screen(), Screen::Page(_)), "{back}");
            assert_eq!(s.chrome().active, start, "{back}");
        }
    }

    /// A commit's tabs work the same: into its files and back.
    #[test]
    fn commit_tabs_go_into_the_files_and_back() {
        let mut s = with_commit(Mode::Dark);
        press(&mut s, "<Right>");
        assert!(matches!(
            s.route(),
            Some(crate::route::Route::CommitChecks { .. })
        ));
        press(&mut s, "<Right>");
        assert!(
            matches!(s.screen(), Screen::Diff(d) if d.of == super::diff::commit_of()),
            "the commit's diff, by its full ID"
        );
        press(&mut s, "<Left><Left>");
        assert!(matches!(
            s.route(),
            Some(crate::route::Route::Commit { .. })
        ));
    }

    /// In the menu, `h` and `l` are `←` and `→`: close it, run the row.
    #[test]
    fn vim_keys_work_in_the_menu() {
        let mut s = with_pr(Mode::Dark);
        press(&mut s, "<Space>h");
        assert!(s.overlay.is_none());
        press(&mut s, "<Space>l");
        assert!(!matches!(s.overlay, Some(Overlay::Menu(_))));
    }
}

// ---- screenshots for design review ----------------------------------------------

/// Writes the main screens as HTML (exact cells and colors) to
/// `$GHTUI_SHOTS`, for looking at the design in a browser.
#[test]
#[ignore = "design review: GHTUI_SHOTS=dir cargo test -p ghtui screenshots -- --ignored"]
fn screenshots() {
    let Some(dir) = std::env::var_os("GHTUI_SHOTS") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let shot = |name: &str, state: &State| {
        let (w, h) = state.size;
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|frame| view(state, frame, NOW)).unwrap();
        std::fs::write(
            dir.join(format!("{name}.html")),
            to_html(terminal.backend().buffer()),
        )
        .unwrap();
    };
    let sized = |mut s: State, w: u16, h: u16| {
        update(&mut s, Msg::Resize(w, h));
        s
    };
    for (mode, tag) in [(Mode::Dark, "dark"), (Mode::Light, "light")] {
        shot(
            &format!("home_{tag}"),
            &sized(with_inbox(mode, ColorDepth::TrueColor), 120, 36),
        );
        let mut repo = sized(with_repo(mode, ColorDepth::TrueColor), 150, 40);
        let commit = |headline: &str, date: &str| ghtui_api::browse::CommitInfo {
            oid: "abc1234".into(),
            headline: headline.into(),
            author: "octocat".into(),
            date: date.into(),
        };
        fetched(
            &mut repo,
            DataKey::LastCommits(ghtui(), "HEAD".into(), String::new()),
            Data::LastCommits(std::sync::Arc::new(
                [
                    (
                        ".github".to_owned(),
                        commit("ci: run clippy on stable", "2026-09-20T10:00:00Z"),
                    ),
                    (
                        "crates".to_owned(),
                        commit("Browse GitHub like the website", "2026-10-03T10:00:00Z"),
                    ),
                    (
                        "Cargo.toml".to_owned(),
                        commit("Update dependencies", "2026-09-01T10:00:00Z"),
                    ),
                    (
                        "README.md".to_owned(),
                        commit("Document single-key navigation", "2026-10-02T10:00:00Z"),
                    ),
                ]
                .into_iter()
                .collect(),
            )),
        );
        shot(&format!("repo_{tag}"), &repo);
        shot(
            &format!("repo_narrow_{tag}"),
            &sized(with_repo(mode, ColorDepth::TrueColor), 90, 36),
        );
        shot(&format!("issues_{tag}"), &sized(with_issues(mode), 130, 36));
        shot(&format!("issue_{tag}"), &sized(with_issue(mode), 130, 40));
        shot(&format!("pr_{tag}"), &sized(with_pr(mode), 130, 44));
        let mut commits = sized(with_pr(mode), 120, 30);
        press(&mut commits, "2");
        shot(&format!("pr_commits_{tag}"), &commits);
        shot(&format!("file_{tag}"), &sized(with_file(mode), 120, 24));
        let profile = with_profile(mode, ProfileTab::Overview);
        shot(&format!("profile_{tag}"), &sized(profile, 120, 36));
        shot(
            &format!("search_{tag}"),
            &sized(with_repo_search(mode), 120, 30),
        );
        let mut s = sized(with_repo(mode, ColorDepth::TrueColor), 120, 36);
        s.visits = vec![crate::nav::Visit {
            url: "https://github.com/octocat".into(),
            title: "@octocat".into(),
            count: 3,
            last: NOW,
        }];
        press(&mut s, "i");
        shot(&format!("hints_{tag}"), &s);
        press(&mut s, "<Esc>/oct");
        shot(&format!("search_box_{tag}"), &s);
        press(&mut s, "<Esc><Space>");
        shot(&format!("menu_{tag}"), &s);
        press(&mut s, "<Esc>?");
        shot(&format!("help_{tag}"), &s);
    }
}

fn to_html(buf: &ratatui::buffer::Buffer) -> String {
    use ratatui::style::{Color, Modifier};
    let css = |c: Color, fallback: &str| match c {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        _ => fallback.to_owned(),
    };
    let mut out = String::from(
        "<!doctype html><meta charset=utf-8><style>body{margin:0;background:#000}pre{margin:0;font:14px 'JetBrains Mono','SF Mono',Menlo,monospace;letter-spacing:0}div{height:19px;line-height:19px;white-space:pre}span{display:inline-block;height:19px;vertical-align:top}</style><pre>",
    );
    let area = buf.area;
    for y in area.top()..area.bottom() {
        out.push_str("<div>");
        let mut skip = 0;
        for x in area.left()..area.right() {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let cell = &buf[(x, y)];
            let sym = cell.symbol();
            let w = ghtui_ui::text::width(sym).max(1);
            skip = w - 1;
            let mut style = format!(
                "color:{};background:{}",
                css(cell.fg, "#ccc"),
                css(cell.bg, "#000")
            );
            if cell.modifier.contains(Modifier::BOLD) {
                style.push_str(";font-weight:700");
            }
            if cell.modifier.contains(Modifier::ITALIC) {
                style.push_str(";font-style:italic");
            }
            if cell.modifier.contains(Modifier::UNDERLINED) {
                style.push_str(";text-decoration:underline");
            }
            let text = sym
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            let width = if w > 1 {
                format!(";display:inline-block;width:{w}ch")
            } else {
                String::new()
            };
            out.push_str(&format!("<span style=\"{style}{width}\">{text}</span>"));
        }
        out.push_str("</div>");
    }
    out.push_str("</pre>");
    out
}
