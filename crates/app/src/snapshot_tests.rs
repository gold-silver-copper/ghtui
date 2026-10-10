//! Full-screen snapshots through ratatui's `TestBackend`, in light and dark
//! schemes (and 256 colors). Snapshots include cell styles, so color changes
//! show up in review. Rendering also exercises the theme's debug assertion
//! that every fg/bg pair used is declared (and therefore contrast-tested).

use ghtui_api::browse::{IssueState, RepoSort, SearchKind, SearchResults};
use ghtui_api::model::{
    Capped, ChecksState, Label, Mergeable, PrDetail, PrRef, PrSummary, RepoId, ReviewDecision,
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
    state: IssueState,
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

/// A pull request as Home's sections list it.
fn row(
    pr: &str,
    title: &str,
    state: IssueState,
    review: Option<ReviewDecision>,
    checks: Option<ChecksState>,
) -> ghtui_api::browse::IssueSummary {
    ghtui_api::browse::IssueSummary {
        review,
        checks,
        ..fixtures::found_pr(pr, title, state)
    }
}

/// Home's default sections: review requests, your pull requests (2 of
/// 31), your repositories.
fn fill_home(state: &mut State) {
    let requests = vec![
        row(
            "ratatui/ratatui#1820",
            "Add a virtualized list widget for very long collections",
            IssueState::Open,
            Some(ReviewDecision::ReviewRequired),
            Some(ChecksState::Passing),
        ),
        row(
            "tokio-rs/tokio#7001",
            "Fix waker leak in the multi-thread scheduler when tasks are cancelled during shutdown",
            IssueState::Draft,
            None,
            Some(ChecksState::Pending),
        ),
    ];
    fixtures::section(state, 0, fixtures::found_prs(requests, 2));
    let mine = vec![
        row(
            "gold-silver-copper/ghtui#12",
            "Theme: generate syntax palette from seed",
            IssueState::Open,
            Some(ReviewDecision::Approved),
            Some(ChecksState::Failing),
        ),
        row(
            "gold-silver-copper/ghtui#9",
            "Cache GraphQL responses in redb",
            IssueState::Open,
            Some(ReviewDecision::ChangesRequested),
            None,
        ),
    ];
    fixtures::section(state, 1, fixtures::found_prs(mine, 31));
    let repos = vec![
        fixtures::repo_summary("gold-silver-copper/ghtui", 1234),
        fixtures::repo_summary("gold-silver-copper/fux", 87),
    ];
    let repos = SearchResults::Repos(ghtui_api::browse::Results {
        total: 2,
        items: repos,
        next: None,
    });
    fixtures::section(state, 2, repos);
}

pub(crate) fn pr_detail() -> PrDetail {
    PrDetail {
        id: ghtui_api::model::NodeId::new("PR_12"),
        merge_state: ghtui_api::model::MergeState::Dirty,
        behind_by: Some(2),
        may: ghtui_api::model::PrPermits {
            merge: true,
            merge_as_admin: false,
            close: true,
            reopen: true,
            edit: true,
            push_head: true,
        },
        merge_methods: vec![
            ghtui_api::change::MergeMethod::Squash,
            ghtui_api::change::MergeMethod::Merge,
        ],
        summary: summary(
            "gold-silver-copper/ghtui#12",
            "Theme: generate syntax palette from seed",
            IssueState::Open,
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
        commit_count: 3,
        check_count: 5,
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
        ]
        .into(),
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
    // Never written: changes to Home's sections are commands, not run here.
    state.config_path = Some("config.toml".into());
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

fn with_home(mode: Mode, depth: ColorDepth) -> State {
    let mut state = state(mode, depth);
    fill_home(&mut state);
    state
}

fn with_pr(mode: Mode) -> State {
    let mut state = with_home(mode, ColorDepth::TrueColor);
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

/// A pull request's tabs are counted from the pull request until their
/// own data is in, so opening it at its files counts them all; and the
/// Checks tab's icon is the state of what it lists, not a fixed mark.
#[test]
fn pr_tabs_count_and_say_how_the_checks_are() {
    let tab = |state: &State, label: &str| {
        let tabs = state.chrome().tabs;
        let (t, _) = tabs.into_iter().find(|(t, _)| t.label == label).unwrap();
        (t.icon, t.count)
    };
    let pr = PrRef::parse("gold-silver-copper/ghtui#12").unwrap();
    let mut state = with_home(Mode::Dark, ColorDepth::TrueColor);
    let _ = state.open_diff(crate::diff_screen::DiffOf::Pr(pr.clone()), None);
    let mut passing = pr_detail();
    passing.summary.checks = Some(ChecksState::Passing);
    update(
        &mut state,
        Msg::Pr(pr.clone(), Box::new(Ok(passing.clone()))),
    );
    assert_eq!(
        tab(&state, "Conversation").1,
        Some(passing.summary.comments)
    );
    assert_eq!(tab(&state, "Commits").1, Some(3));
    assert_eq!(tab(&state, "Checks"), ("✓", Some(5)));
    // The checks it lists are still going, whatever the rollup said.
    let _ = state.push(Route::Pr {
        pr: pr.clone(),
        tab: PrTab::Checks,
    });
    let mut checks = fixtures::checks();
    let mut running = checks.items[0].clone();
    running.outcome = ghtui_api::browse::CheckOutcome::Pending;
    checks.items = vec![running, checks.items[0].clone()].into();
    checks
        .items
        .iter_mut()
        .skip(1)
        .for_each(|c| c.outcome = ghtui_api::browse::CheckOutcome::Success);
    fetched(
        &mut state,
        DataKey::PrChecks(pr),
        Data::Checks(Box::new(checks)),
    );
    assert_eq!(tab(&state, "Checks"), ("◔", Some(2)));
}

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
            .map(ghtui_ui::page::PageLine::text)
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
    assert!(text(&state).contains("… 119 earlier comments on GitHub (o)"));
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
    assert!(
        page.contains("… 100 earlier comments on GitHub (o)"),
        "{page}"
    );
    assert!(page.contains("… 1 earlier review on GitHub (o)"), "{page}");
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
    press(&mut state, "2");
    assert!(text(&state).contains("… 248 earlier commits"));
}

/// A dismissed review, with the state the API gives it (pinned by
/// ghtui-api's `a_dismissed_reviews_wire_spelling`), reads as dismissed in
/// the conversation, not as a plain review.
#[test]
fn a_dismissed_review_reads_as_dismissed() {
    let pr = PrRef::parse("gold-silver-copper/ghtui#12").unwrap();
    let mut activity = fixtures::activity();
    activity.reviews = ghtui_api::model::Capped::new(
        vec![ghtui_api::browse::ReviewSummary {
            author: "hubot".into(),
            state: ghtui_api::model::ReviewState::Dismissed,
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
        .map(ghtui_ui::page::PageLine::text)
        .collect::<Vec<_>>()
        .join("\n");
    let hubot: Vec<&str> = page.lines().filter(|l| l.contains("hubot")).collect();
    assert!(
        page.contains("had a review dismissed"),
        "hubot's lines: {hubot:?}"
    );
}

/// Pushes `route` and delivers `data` for its page's own fetch (its last).
/// Opens `route` with `data` for its own fetch (its last but the counts),
/// and its tabs' counts, if it counts any, as the samples have them.
fn open(state: &mut State, route: Route, data: Data) {
    let mut keys = needs(&route).into_iter().filter_map(|n| match n {
        Need::Data(key) => Some(key),
        Need::Pr(_) => None,
    });
    let (counts, own): (Vec<DataKey>, Vec<DataKey>) =
        keys.by_ref().partition(|k| matches!(k, DataKey::Counts(_)));
    let Some(key) = own.last().cloned() else {
        panic!("{route:?} doesn't end in a data fetch");
    };
    let _ = state.push(route);
    fetched(state, key, data);
    for key in counts {
        let data = sample(&key);
        fetched(state, key, data);
    }
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
        DataKey::Readme(ghtui()),
        Data::Readme(Some(Box::new(fixtures::readme()))),
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
    let items = fixtures::advisories();
    let list = ghtui_api::browse::Results {
        total: items.len() as u64,
        items,
        next: None,
    };
    open(&mut state, route, Data::Advisories(Box::new(list)));
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
        let _ = state.open_tab(crate::route::Target::Page(route).into());
    }
    let _ = state.switch_to_tab(1);
    state.notices.dismiss();
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
            result: Err(ghtui_api::ApiError::Network("connection refused".into())),
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
    let blob = ghtui_api::browse::Blob {
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
    let mut state = with_home(Mode::Dark, ColorDepth::TrueColor);
    let _ = crate::nav::open_menu(&mut state);
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
    let _ = crate::nav::open_menu(&mut menu);
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
            let _ = crate::nav::open_menu(&mut state);
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
    use crate::diff_screen::{DiffOf, DiffPrefs, DiffScreen, DiffState, Pane};
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
            old_oid: Oid::parse(&"1".repeat(40)).unwrap(),
            new_oid: if new.is_some() {
                Oid::parse(&"2".repeat(40)).unwrap()
            } else {
                Oid::parse(ZERO_OID).unwrap()
            },
            similarity: (status == FileStatus::Renamed).then_some(94),
        }
    }

    const OLD_RS: &str = "use std::fmt;\n\n/// A point.\npub struct Point {\n    x: i32,\n    y: i32,\n}\n\nimpl Point {\n    pub fn new(x: i32, y: i32) -> Self {\n        Point { x, y }\n    }\n}\n";
    const NEW_RS: &str = "use std::fmt;\n\n/// A point in 2D.\npub struct Point {\n    x: i32,\n    y: i32,\n}\n\nimpl Point {\n    pub fn new(x: i32, y: i32) -> Self {\n        Point { x, y }\n    }\n\n    pub fn origin() -> Self {\n        Self::new(0, \"0\".len() as i32 - 1)\n    }\n}\n";

    fn loaded(doc: &Doc) -> DiffState {
        let refs = PrRefs {
            head: Oid::parse(&"e".repeat(40)).unwrap(),
            base: Oid::parse(&"b".repeat(40)).unwrap(),
            merge_base: Oid::parse(&"c".repeat(40)).unwrap(),
        };
        DiffState::fresh().with_doc(refs, doc)
    }

    /// Opens a diff screen on `diff`, as wide as the terminal.
    fn open_diff(s: &mut State, diff: DiffState, cursor: Pos, focus: Pane) {
        s.diffs.insert(DiffOf::Pr(pr()), diff);
        let mut screen = DiffScreen::new(DiffOf::Pr(pr()), DiffPrefs::fit(s.size.0));
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
        let mut doc = Doc::new(files, &HashSet::new(), Default::default());
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
        loaded(&doc)
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
        DiffOf::Commit(
            super::ghtui(),
            Oid::parse(&crate::fixtures::commit().oid).unwrap(),
        )
    }

    /// A commit's files: its tabs instead of a pull request's.
    #[test]
    fn commit_files_dark() {
        let mut s = state(Mode::Dark, ColorDepth::TrueColor);
        s.size = (110, 34);
        let of = commit_of();
        s.diffs.insert(of.clone(), diff_state());
        s.screens.push(Screen::Diff(Box::new(DiffScreen::new(
            of,
            DiffPrefs::fit(s.size.0),
        ))));
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
            diff.edit(|i| {
                i.threads = vec![crate::fixtures::thread("u", Some(1), false, false), thread];
            });
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
        let mut screen = DiffScreen::new(of, DiffPrefs::fit(s.size.0));
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
        let mut diff = loaded(&Doc::new(
            fixture_files(),
            &HashSet::new(),
            Default::default(),
        ));
        let mut thread = crate::fixtures::thread("t", Some(3), false, false);
        thread.comments[0].url = "https://github.com/o/r/pull/7#discussion_r42".into();
        diff.edit(|i| i.threads = vec![thread]);
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
        diff.edit(|i| {
            i.review = ghtui_store::ReviewState {
                reviewed_hunks: vec![hash],
                ..Default::default()
            }
        });
        let path = diff.doc.files()[1].meta.path().to_owned();
        diff.edit(|i| {
            i.viewed = Some(ghtui_api::model::ViewedFiles {
                pull_request_id: ghtui_api::model::NodeId::new("PR_1"),
                states: [(path, ghtui_ui::diff_doc::Viewed::Viewed)].into(),
            });
        });
        let _ = s.settle_diff();
        insta::assert_snapshot!(render(&s));
    }

    /// The pull request whose diff is on screen.
    fn diff_pr(s: &State) -> ghtui_api::model::PrRef {
        match s.screen() {
            Screen::Diff(d) => d.of.pr().cloned(),
            Screen::Page(_) => None,
        }
        .expect("a pull request's diff")
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
        diff.edit(|i| i.threads = vec![
            thread(
                "t1",
                Some(3),
                false,
                false,
                vec![
                    comment("c1", "alice", "Should this say \"2D\" or \"two-dimensional\"? The rest of the docs spell it out.", false),
                    comment("c2", "octocat", "Good point, I'll spell it out.", true),
                ].into(),
            ),
            thread("t2", Some(10), true, false, vec![comment("c3", "bob", "Looks fine now.", false)].into()),
            thread("t3", None, false, true, vec![comment("c4", "carol", "Please add tests for this file.", false)].into()),
        ]);
        diff.edit(|i| {
            i.review = ghtui_store::ReviewState {
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
            }
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
        let mut dialog = crate::review::SubmitDialog::new(&s.theme, diff_pr(&s));
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
        let mut doc = Doc::new(files, &HashSet::new(), Default::default());
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
        let moves = ghtui_diff::moves::detect_moves(&[(0, &texts[0]), (1, &texts[1])]);
        assert_eq!(moves.len(), 1);
        doc.set_moves(moves);
        let mut s = state(mode, ColorDepth::TrueColor);
        s.size = (110, 30);
        open_diff(&mut s, loaded(&doc), Pos::default(), Pane::Diff);
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
        let mut diff = DiffState::fresh();
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
        let mut diff = DiffState::fresh();
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
            DataKey, Overlay, ProfileTab, fetched, ghtui, with_file, with_home, with_issue,
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
            ("home", Box::new(move || with_home(Mode::Dark, tc))),
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
                    let mut s = with_home(Mode::Light, tc);
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
                    let dialog = crate::review::SubmitDialog::new(&s.theme, diff_pr(&s));
                    s.overlay = Some(Overlay::Submit(Box::new(dialog)));
                    s
                }),
            ),
            ("moves", Box::new(|| better_than_github(Mode::Dark))),
            (
                "diff loading",
                Box::new(move || {
                    let mut s = state(Mode::Dark, tc);
                    open_diff(&mut s, DiffState::fresh(), Pos::default(), Pane::Diff);
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
        let mut doc = Doc::new(files, &HashSet::new(), Default::default());
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
        open_diff(&mut s, loaded(&doc), Pos::default(), Pane::Diff);

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
        let doc = Doc::new(vec![big], &HashSet::new(), Default::default());
        open_diff(&mut s, loaded(&doc), Pos::default(), Pane::Diff);
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
        with_forks, with_gist, with_gists, with_history, with_home, with_issue, with_issues,
        with_job, with_milestone, with_milestones, with_pr, with_pr_checks, with_profile,
        with_release, with_releases, with_repo, with_repo_search, with_run, with_search,
        with_stargazers, with_tags, with_team, with_teams, with_wiki, with_workflow,
    };
    use crate::route::{Dest, Target};
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
            ("home", with_home(Mode::Dark, tc)),
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
            p.page()
                .links
                .iter()
                .filter_map(|l| l.url())
                .filter_map(|u| match Dest::from_url(u).target {
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
            let links = p.page().links.iter().filter_map(|l| l.url());
            let targets = links.map(|u| Dest::from_url(u).target).chain(tabs);
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
            // From the top: a failed job opens at its first error.
            ("job", || {
                let mut s = super::sized(with_job(Mode::Dark, None), 100, 12);
                press(&mut s, "gjj");
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
            // Scrolled off its line, which it otherwise follows to the
            // bottom when resized.
            ("blame", || {
                let mut s = super::sized(with_blame(Mode::Dark), 100, 12);
                press(&mut s, "gj");
                s
            }),
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
                if cmds.is_empty() && s.notices.shown().is_none() && !s.quit && render(&s) == before
                {
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
            &sized(with_home(mode, ColorDepth::TrueColor), 120, 36),
        );
        let mut repo = sized(with_repo(mode, ColorDepth::TrueColor), 150, 40);
        let commit = |headline: &str, date: &str| ghtui_api::browse::CommitInfo {
            oid: "abc1234".into(),
            headline: headline.into(),
            author: ghtui_api::browse::Person::User(ghtui_api::browse::Login::unchecked("octocat")),
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

/// Text at its worst: long, wide (CJK), with emoji, ZWJ sequences, flags,
/// stacked combining marks and a right-to-left override.
fn awful(n: usize) -> String {
    "漢字かな 👩‍👩‍👧‍👦 🇯🇵 e\u{301}\u{302}\u{303} \u{202e}rtl\u{202c} Ｆｕｌｌ ".repeat(n)
}

/// Stretches what's on the screens' data to its worst.
fn stretch(state: &mut State) {
    for remote in state.data.values_mut() {
        match &mut remote.data {
            Some(Data::Issue(Some(d))) => {
                d.title = awful(40);
                d.body = format!("{}\n\n```\n{}\n```", awful(30), "x".repeat(5000));
                for c in d.comments.iter_mut() {
                    c.author = "a".repeat(39);
                    c.body = awful(20);
                }
            }
            Some(Data::Branches(r)) => {
                for b in &mut r.items {
                    b.name = format!("feature/{}", awful(10));
                    b.headline = Some(awful(20));
                }
            }
            Some(Data::Blob(b)) => {
                b.text = Some(format!("{}\n{}\n", "y".repeat(10_000), awful(50)));
            }
            Some(Data::Repo(o)) => {
                o.summary.description = Some(awful(30));
                o.topics = vec![awful(2); 30].into();
            }
            _ => {}
        }
    }
    state.data_gen += 1;
}

/// Every screen, with ordinary and with awful data, renders at any size
/// (one cell, a phone, a classic terminal) without panicking or drawing
/// outside the screen.
#[test]
fn every_screen_renders_at_odd_sizes() {
    type Build = fn() -> State;
    let builders: Vec<(&str, Build)> = vec![
        ("home", || with_home(Mode::Dark, ColorDepth::TrueColor)),
        ("pr", || with_pr(Mode::Dark)),
        ("repo", || with_repo(Mode::Dark, ColorDepth::TrueColor)),
        ("file", || with_file(Mode::Dark)),
        ("blame", || with_blame(Mode::Dark)),
        ("gist", || with_gist(Mode::Dark)),
        ("teams", || with_team(Mode::Dark)),
        ("advisory", || with_advisory(Mode::Dark)),
        ("wiki", || with_wiki(Mode::Dark)),
        ("issues", || with_issues(Mode::Dark)),
        ("commit", || with_commit(Mode::Dark)),
        ("checks", || with_pr_checks(Mode::Dark)),
        ("actions", || with_actions(Mode::Dark)),
        ("tabs", || with_tabs(Mode::Dark, 9)),
        ("org", || with_org(Mode::Dark)),
        ("discussion", || with_discussion(Mode::Dark)),
        ("run", || with_run(Mode::Dark)),
        ("job", || with_job(Mode::Dark, Some((3, 2)))),
        ("releases", || with_releases(Mode::Dark)),
        ("branches", || with_branches(Mode::Dark)),
        ("milestone", || with_milestone(Mode::Dark)),
        ("deployments", || with_deployments(Mode::Dark)),
        ("compare", || with_compare(Mode::Dark)),
        ("history", || with_history(Mode::Dark)),
        ("issue", || with_issue(Mode::Dark)),
        ("profile", || with_profile(Mode::Dark, ProfileTab::Overview)),
        ("search", || with_repo_search(Mode::Dark)),
    ];
    for (name, build) in builders {
        for awful_data in [false, true] {
            for (w, h) in [(1, 1), (20, 5), (80, 24), (3, 40), (200, 2)] {
                let mut state = build();
                if awful_data {
                    stretch(&mut state);
                }
                update(&mut state, Msg::Resize(w, h));
                let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
                terminal
                    .draw(|frame| view(&state, frame, NOW))
                    .unwrap_or_else(|e| panic!("{name} at {w}x{h}: {e}"));
                assert_eq!(terminal.backend().buffer().area.width, w, "{name}");
            }
        }
    }
}

/// The page's lines, as text.
fn page_text(state: &State) -> String {
    let crate::state::Screen::Page(p) = state.screen() else {
        panic!("not a page");
    };
    p.page()
        .lines
        .iter()
        .map(ghtui_ui::page::PageLine::text)
        .collect::<Vec<_>>()
        .join("\n")
}

/// A filtered list counts its filter open and closed, as GitHub does, not
/// the whole repository's; and every sort and state of it shares the counts.
#[test]
fn a_filtered_list_counts_its_filter() {
    let mut state = with_issues(Mode::Dark);
    let route = Route::Issues {
        repo: ghtui(),
        query: "is:open label:I-ICE sort:comments-desc".into(),
    };
    let counted = route.counts().unwrap();
    assert_eq!(
        counted,
        ["open", "closed"].map(|s| (
            SearchKind::Issues,
            format!("repo:gold-silver-copper/ghtui is:issue label:I-ICE is:{s}")
        ))
    );
    let closed = Route::Issues {
        repo: ghtui(),
        query: "label:I-ICE is:closed".into(),
    };
    assert_eq!(closed.counts().unwrap(), counted);
    open(
        &mut state,
        route,
        Data::Search(Box::new(fixtures::issue_results(None))),
    );
    fetched(
        &mut state,
        DataKey::Counts(counted),
        Data::Counts(vec![Some(812), Some(8646)]),
    );
    let text = page_text(&state);
    assert!(text.contains("◉ 812 Open   ✓ 8.6k Closed"), "{text}");
}

/// Delivers a failed fetch of `key`.
fn failed(state: &mut State, key: DataKey) {
    update(
        state,
        Msg::Fetched {
            key,
            result: Err(ghtui_api::ApiError::Network("timed out".into())),
            cached_at: None,
        },
    );
}

/// A README that fails to load says so where it goes, under the files;
/// the overview (every repository page's header) doesn't depend on it.
#[test]
fn a_failed_readme_says_why_under_the_files() {
    let mut state = state(Mode::Dark, ColorDepth::TrueColor);
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
    let mut state = with_repo(Mode::Dark, ColorDepth::TrueColor);
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
    let mut state = with_pr(Mode::Dark);
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
    let mut state = with_home(Mode::Dark, ColorDepth::TrueColor);
    let pr = PrRef::parse("gold-silver-copper/ghtui#12").unwrap();
    let route = Route::pr(pr.clone());
    let _ = state.push(route.clone());
    update(&mut state, Msg::Pr(pr, Box::new(Ok(pr_detail()))));
    let lines: Vec<String> = (state.build_page(&route, 100, NOW).lines.iter())
        .map(ghtui_ui::page::PageLine::text)
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
    let mut state = with_repo(Mode::Dark, ColorDepth::TrueColor);
    let route = Route::Search {
        kind: SearchKind::Repos,
        query: "ghtui".into(),
    };
    let _ = state.push(route.clone());
    let page = state.build_page(&route, 100, NOW);
    let text: Vec<String> = page
        .lines
        .iter()
        .map(ghtui_ui::page::PageLine::text)
        .collect();
    assert!(text.iter().any(|l| l.contains("Searching…")), "{text:#?}");
}

/// A workflow whose runs fail to load says so under the workflow.
#[test]
fn failed_workflow_runs_say_why_instead_of_loading() {
    let mut state = with_repo(Mode::Dark, ColorDepth::TrueColor);
    let _ = state.push(Route::Workflow {
        repo: ghtui(),
        file: "ci.yml".into(),
    });
    let (workflow, _) = fixtures::workflow_runs();
    let key = DataKey::Workflow(ghtui(), "ci.yml".into());
    fetched(&mut state, key, Data::Workflow(Box::new(workflow)));
    failed(&mut state, DataKey::WorkflowRuns(ghtui(), "ci.yml".into()));
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
    fn err<T>() -> Result<T, ghtui_api::ApiError> {
        Err(ghtui_api::ApiError::Network("timed out".into()))
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
        "https://github.com/o/r/compare/a...b",
        "https://github.com/o/r/wiki",
        "https://github.com/o/r/security/advisories",
        "https://github.com/o/r/security/advisories/GHSA-2222-3333-4444",
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
    ];
    for url in urls {
        let crate::route::Target::Page(route) = crate::route::Dest::from_url(url).target else {
            panic!("{url} isn't a page");
        };
        // The page a plain URL turns out to be.
        let route = route.split(None).unwrap_or(route);
        let all = state(Mode::Dark, ColorDepth::TrueColor).needs(&route);
        for failing in &all {
            let mut state = state(Mode::Dark, ColorDepth::TrueColor);
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
            (_, Need::Data(K::LastCommits(..) | K::Counts(_)))
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
    use ghtui_api::browse::Results;
    use std::sync::Arc;
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
        // An issue list's open and closed, or a search's kinds.
        K::Counts(searches) if searches.len() == 2 => Data::Counts(vec![Some(7), Some(41)]),
        K::Counts(searches) => {
            Data::Counts((0..searches.len() as u64).map(|n| Some(n * 12)).collect())
        }
        K::Issue(..) => Data::Issue(Some(Box::new(fixtures::issue()))),
        K::PrActivity(_) => Data::PrActivity(Box::new(fixtures::activity())),
        K::Profile(_) => Data::Profile(Box::new(fixtures::profile())),
        K::Files(..) => Data::Files(Arc::new(vec!["README.md".into()]), false),
        K::Refs(_) => Data::Refs(Box::default()),
        K::RefIn(..) => Data::RefIn(None),
        K::LastCommits(..) => Data::LastCommits(Arc::new(fixtures::last_commits())),
        K::Commit(..) => Data::Commit(Box::new(fixtures::commit())),
        K::History(..) => Data::History(Box::new(fixtures::history(None))),
        K::PrChecks(_) | K::BranchChecks(_) | K::CommitChecks(..) => {
            Data::Checks(Box::new(fixtures::checks()))
        }
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
            Data::Advisories(Box::new(Results {
                total,
                items,
                next: None,
            }))
        }
        K::Advisory(..) => Data::Advisory(Box::new(fixtures::advisory())),
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
        K::WorkflowRuns(..) => Data::Runs(Box::new(fixtures::workflow_runs().1)),
    }
}

/// Changing GitHub: each action asks first where it should, says why
/// when it can't, sends one change, and ends up showing what GitHub says.
mod changes {
    use ghtui_api::ApiError;
    use ghtui_api::browse::{CheckOutcome, IssueState};
    use ghtui_api::change::{Change, MergeMethod};
    use ghtui_api::model::{MergeState, NodeId, PrRef, ReviewEvent};
    use ghtui_store::{DraftComment, DraftSide, ReviewState};
    use ghtui_theme::Mode;

    use super::{ghtui, pr_detail, render, with_job, with_pr, with_repo, with_run};
    use crate::act::{By, Subject};
    use crate::browse::{Data, DataKey};
    use crate::fixtures::{answer, fetched, press, says, warns};
    use crate::keymap::Action;
    use crate::review::SubmitOutcome;
    use crate::route::Route;
    use crate::state::{Api, Cmd, DiffMsg, Msg, Overlay, State, Timer, timers, update};

    fn pr() -> PrRef {
        PrRef::parse("gold-silver-copper/ghtui#12").unwrap()
    }

    fn confirm(s: &State) -> &crate::act::Confirm {
        match &s.overlay {
            Some(Overlay::Confirm(c)) => c,
            _ => panic!("no confirmation"),
        }
    }

    /// A pull request whose state is `state`.
    fn pr_in(state: IssueState) -> State {
        pr_with(|d| d.summary.state = state)
    }

    /// The pull request, changed by `f`.
    fn pr_with(f: impl FnOnce(&mut ghtui_api::model::PrDetail)) -> State {
        let mut s = with_pr(Mode::Dark);
        update(&mut s, Msg::Pr(pr(), Box::new(Ok(mergeable(f)))));
        s
    }

    /// The pull request without its conflicts (GitHub would merge it),
    /// changed by `f`.
    fn mergeable(f: impl FnOnce(&mut ghtui_api::model::PrDetail)) -> ghtui_api::model::PrDetail {
        let mut detail = pr_detail();
        detail.mergeable = ghtui_api::model::Mergeable::Yes;
        detail.merge_state = MergeState::Unstable;
        f(&mut detail);
        detail
    }

    #[test]
    fn merge_confirm_dark() {
        let mut s = pr_with(|_| {});
        press(&mut s, "M");
        insta::assert_snapshot!(render(&s));
    }

    /// Merging asks first, saying what stands in the way; a refusal stays
    /// in the dialog, with the permission it needs; once GitHub says it's
    /// merged, the pull request is fetched again.
    #[test]
    fn merging_asks_first_and_ends_up_as_github_has_it() {
        let mut s = pr_with(|_| {});
        assert!(press(&mut s, "M").is_empty());
        let c = confirm(&s);
        let problems: Vec<&str> = (c.facts.iter())
            .filter(|(_, p)| *p)
            .map(|(f, _)| f.as_str())
            .collect();
        assert_eq!(problems, ["Some checks failed"], "{:?}", c.facts);
        assert!(
            c.facts
                .iter()
                .any(|(f, _)| f == "Its branch is 2 commits behind main (B updates it)"),
            "{:?}",
            c.facts
        );
        let methods: Vec<&str> = c.choices.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(methods, ["Squash and merge", "Create a merge commit"]);
        press(&mut s, "<Tab>");
        let merge = Change::Merge {
            pr: NodeId::new("PR_12"),
            method: MergeMethod::Merge,
            head: pr_detail().head_oid,
        };
        let sent = press(&mut s, "<Enter>");
        assert_eq!(
            sent,
            vec![Cmd::Api(Api::Change(merge, By::Confirm(Subject::Pr(pr()))))]
        );
        assert!(confirm(&s).sending);
        // Keys wait while it's sent.
        assert!(press(&mut s, "<Enter>").is_empty());

        let refused = ApiError::GraphQl(vec!["Must have push access to repository".into()]);
        answer(&mut s, &sent, Err(refused));
        let error = confirm(&s).error.clone().unwrap_or_default();
        assert!(error.contains("Must have push access"), "{error}");
        assert!(error.contains("Contents: write"), "{error}");
        assert!(!confirm(&s).sending);

        press(&mut s, "<Enter>");
        let cmds = answer(&mut s, &sent, Ok(()));
        assert!(s.overlay.is_none());
        says(&s, "Merged");
        assert!(
            cmds.contains(&Cmd::Api(Api::FetchPr(pr()))),
            "the pull request is fetched again: {cmds:?}"
        );
        // Until GitHub shows it merged, nothing else is offered.
        press(&mut s, "X");
        says(&s, "hasn't shown the last change");
        let merged = mergeable(|d| d.summary.state = IssueState::Merged);
        update(&mut s, Msg::Pr(pr(), Box::new(Ok(merged))));
        assert!(s.awaiting.is_none());
    }

    /// On Home, an action works on the selected row: the pull request
    /// loads, then the dialog opens about it, and stays about it when the
    /// list changes under it. Once GitHub shows the merge, Home is fetched
    /// again.
    #[test]
    fn acting_on_a_homes_row_is_about_that_row() {
        let mut s = super::with_home(Mode::Dark, ghtui_theme::ColorDepth::TrueColor);
        s.prs.clear();
        press(&mut s, "jj");
        let cmds = press(&mut s, "M");
        assert_eq!(cmds, vec![Cmd::Api(Api::FetchPr(pr()))]);
        says(&s, "Loading gold-silver-copper/ghtui#12");
        update(&mut s, Msg::Pr(pr(), Box::new(Ok(mergeable(|_| {})))));
        assert_eq!(confirm(&s).about, Subject::Pr(pr()));
        assert!(
            confirm(&s)
                .title
                .starts_with("Merge gold-silver-copper/ghtui#12")
        );
        // The list moves; the dialog is still about #12.
        press(&mut s, "<Esc>");
        press(&mut s, "M");
        let mine = crate::fixtures::found_prs(Vec::new(), 0);
        crate::fixtures::section(&mut s, 1, mine);
        assert_eq!(confirm(&s).about, Subject::Pr(pr()));
        let sent = press(&mut s, "<Enter>");
        answer(&mut s, &sent, Ok(()));
        let w = s.awaiting.clone().unwrap();
        assert_eq!(
            (w.route, w.about),
            (Route::Home, crate::act::Subject::Pr(pr()))
        );
        // Home asked again at once; GitHub's search may not show it yet.
        for i in 0..2 {
            let none = crate::fixtures::found_prs(Vec::new(), 0);
            crate::fixtures::section(&mut s, i, none);
        }
        let merged = mergeable(|d| d.summary.state = IssueState::Merged);
        let cmds = update(&mut s, Msg::Pr(pr(), Box::new(Ok(merged))));
        assert!(s.awaiting.is_none());
        let home = s.home[1].search.clone().unwrap().key();
        let refetched = |c: &Cmd| matches!(c, Cmd::Api(Api::Fetch { key, .. }) if *key == home);
        assert!(cmds.iter().any(refetched), "{cmds:?}");
        // An issue row offers closing, not merging.
        crate::fixtures::section(
            &mut s,
            1,
            crate::fixtures::found_prs(
                vec![crate::fixtures::issue_summary(3, false, IssueState::Open)],
                1,
            ),
        );
        press(&mut s, "gjj");
        press(&mut s, "M");
        says(&s, "That works on a pull request");
    }

    /// A pull request closed from a list that shows open ones stays where
    /// it was, closed, when the list is fetched again without it: the key
    /// pressed next is still about it, not the row that took its place.
    #[test]
    fn a_row_closed_from_a_list_stays_under_the_cursor() {
        let mut s = super::with_home(Mode::Dark, ghtui_theme::ColorDepth::TrueColor);
        s.prs.clear();
        press(&mut s, "jj");
        press(&mut s, "X");
        update(&mut s, Msg::Pr(pr(), Box::new(Ok(mergeable(|_| {})))));
        assert!(
            confirm(&s)
                .title
                .starts_with("Close gold-silver-copper/ghtui#12")
        );
        let sent = press(&mut s, "<Enter>");
        answer(&mut s, &sent, Ok(()));
        let rows = |s: &State| match s
            .picked::<ghtui_api::browse::SearchResults>(&s.home[1].search.clone().unwrap().key())
        {
            Some(ghtui_api::browse::SearchResults::Issues(r)) => r
                .items
                .iter()
                .map(|i| (i.number, i.state))
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        // GitHub's search no longer has it, and the PR is closed.
        let closed = mergeable(|d| d.summary.state = IssueState::Closed);
        update(&mut s, Msg::Pr(pr(), Box::new(Ok(closed))));
        let others = rows(&s).into_iter().filter(|(n, _)| *n != 12);
        let others: Vec<_> = others
            .map(|(n, _)| crate::fixtures::issue_summary(n, true, IssueState::Open))
            .collect();
        crate::fixtures::section(&mut s, 1, crate::fixtures::found_prs(others, 30));
        assert!(
            rows(&s).contains(&(12, IssueState::Closed)),
            "{:?}",
            rows(&s)
        );
        press(&mut s, "X");
        assert!(
            confirm(&s)
                .title
                .starts_with("Reopen gold-silver-copper/ghtui#12"),
            "{}",
            confirm(&s).title
        );
        // Refreshing shows GitHub's list as it is.
        press(&mut s, "<Esc>r");
        crate::fixtures::section(&mut s, 1, crate::fixtures::found_prs(Vec::new(), 0));
        assert!(rows(&s).is_empty());
    }

    /// The menu on a row that hasn't loaded loads it, saying it's checking
    /// what you may do until GitHub says, then why you can't, if you can't.
    #[test]
    fn the_menu_checks_a_row_before_offering_its_changes() {
        let mut s = super::with_home(Mode::Dark, ghtui_theme::ColorDepth::TrueColor);
        s.prs.clear();
        press(&mut s, "jj");
        let cmds = press(&mut s, "<Space>");
        assert!(cmds.contains(&Cmd::Api(Api::FetchPr(pr()))), "{cmds:?}");
        let close = |s: &State| {
            let Some(Overlay::Menu(menu)) = &s.overlay else {
                panic!("no menu")
            };
            let row = menu
                .rows
                .iter()
                .find(|d| d.action == Action::Close)
                .unwrap();
            (row.label.clone(), row.unavailable.clone())
        };
        assert_eq!(
            close(&s),
            (
                "Close or reopen (checking gold-silver-copper/ghtui#12 first)".to_owned(),
                None
            )
        );
        let theirs = mergeable(|d| d.may = ghtui_api::model::PrPermits::default());
        update(&mut s, Msg::Pr(pr(), Box::new(Ok(theirs))));
        let (label, why) = close(&s);
        assert_eq!(label, "Close or reopen");
        assert!(why.is_some_and(|w| w.contains("doesn't let you close it")));
    }

    /// Updating the branch, the page follows GitHub until it shows the new
    /// head (GitHub moves it after it answers), and merging then names that
    /// head: merging the old one is refused as "Head branch was modified".
    #[test]
    fn after_updating_the_branch_merging_waits_for_the_new_head() {
        // Behind, though the repository doesn't require it up to date.
        let mut s = pr_with(|_| {});
        press(&mut s, "B");
        let update_branch = Change::UpdateBranch {
            pr: NodeId::new("PR_12"),
            rebase: false,
            head: pr_detail().head_oid,
        };
        assert_eq!(confirm(&s).choices[0].1, update_branch);
        assert_eq!(
            confirm(&s).facts[0].0,
            "Its branch, syntax-palette, is 2 commits behind main; updating pushes to it"
        );
        let sent = press(&mut s, "<Enter>");
        answer(&mut s, &sent, Ok(()));
        // GitHub still shows the old head.
        update(&mut s, Msg::Pr(pr(), Box::new(Ok(mergeable(|_| {})))));
        press(&mut s, "M");
        says(&s, "hasn't shown the last change");
        let cmds = timers(&mut s);
        assert!(cmds.contains(&Cmd::Timer(Timer::Live, 5_000)), "{cmds:?}");
        let cmds = update(&mut s, Msg::Timer(Timer::Live));
        assert!(cmds.contains(&Cmd::Api(Api::FetchPr(pr()))), "{cmds:?}");
        let moved = mergeable(|d| {
            d.head_oid = "1".repeat(40);
            d.behind_by = Some(0);
        });
        update(&mut s, Msg::Pr(pr(), Box::new(Ok(moved))));
        press(&mut s, "M");
        let head = match &confirm(&s).choices[0].1 {
            Change::Merge { head, .. } => head.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(head, "1".repeat(40));
        press(&mut s, "<Esc>");
        press(&mut s, "B");
        says(&s, "It's up to date with main");
    }

    /// What GitHub says you may not do is refused, saying why, and what
    /// GitHub says stops a merge is refused before asking.
    #[test]
    fn what_github_forbids_is_refused() {
        let mut s = pr_with(|d| d.may = ghtui_api::model::PrPermits::default());
        press(&mut s, "M");
        says(&s, "Merging takes write access to gold-silver-copper/ghtui");
        press(&mut s, "X");
        says(&s, "GitHub doesn't let you close it");
        press(&mut s, "B");
        says(&s, "which you can't push to");

        let mut s = with_pr(Mode::Dark);
        press(&mut s, "M");
        says(&s, "It conflicts with main: resolve that on GitHub");
        let mut s = pr_with(|d| d.merge_state = MergeState::Blocked);
        press(&mut s, "M");
        says(&s, "blocked");
        let mut s = pr_with(|d| d.merge_state = MergeState::Behind);
        press(&mut s, "M");
        says(&s, "B updates it");

        let mut s = with_run(Mode::Dark);
        let mut repo = crate::fixtures::overview();
        repo.can_write = false;
        fetched(&mut s, DataKey::Repo(ghtui()), Data::Repo(Box::new(repo)));
        press(&mut s, "<C-r>");
        says(
            &s,
            "Re-running and cancelling take write access to gold-silver-copper/ghtui",
        );
    }

    /// An action that can't apply says why, on the key and in the menu
    /// alike, and sends nothing.
    #[test]
    fn what_cant_be_done_says_why() {
        let mut s = pr_in(IssueState::Merged);
        assert!(press(&mut s, "M").is_empty());
        assert!(s.overlay.is_none());
        says(&s, "It's merged");
        let merge = (s.doables().into_iter())
            .find(|d| d.action == Action::Merge)
            .unwrap();
        assert_eq!(merge.unavailable.as_deref(), Some("It's merged"));
        press(&mut s, "X");
        says(&s, "can't be reopened");

        let mut s = with_pr(Mode::Dark);
        press(&mut s, "W");
        says(&s, "It isn't a draft");
        s.viewer = Some(pr_detail().summary.author);
        press(&mut s, "A");
        says(&s, "You can't approve your own pull request");

        let mut s = with_repo(Mode::Dark, ghtui_theme::ColorDepth::TrueColor);
        press(&mut s, "M");
        says(&s, "That works on a pull request (or its row in a list)");
        press(&mut s, "<C-r>");
        says(&s, "Re-running works on a workflow run or a job");
    }

    /// Marking a draft ready asks nothing: it's sent at once. GitHub's
    /// answer goes back to what sent it: a dialog or a comment opened while
    /// it's on its way is left as it is.
    #[test]
    fn a_draft_is_marked_ready_at_once_and_hears_back_alone() {
        let mut s = pr_in(IssueState::Draft);
        let ready = Change::ReadyForReview {
            pr: NodeId::new("PR_12"),
        };
        let sent = press(&mut s, "W");
        assert_eq!(
            sent,
            vec![Cmd::Api(Api::Change(ready, By::Act(Subject::Pr(pr()))))]
        );
        press(&mut s, "X");
        let refused = ApiError::GraphQl(vec!["Not now".into()]);
        answer(&mut s, &sent, Err(refused));
        assert_eq!(confirm(&s).error, None);
        warns(&s, "Not now");
        press(&mut s, "<Esc>WcLGTM");
        answer(&mut s, &sent, Ok(()));
        says(&s, "Ready for review");
        assert!(
            matches!(&s.overlay, Some(Overlay::Compose(c)) if c.text() == "LGTM"),
            "the comment is still being written"
        );
    }

    /// A star answered while a re-run from a list's row is on its way, or
    /// once GitHub accepted it, leaves the page following that run.
    #[test]
    fn a_star_leaves_the_wait_for_another_change_alone() {
        let mut s = super::with_workflow(Mode::Dark);
        let run = Data::Run(Box::new(crate::fixtures::workflow_run()));
        fetched(&mut s, DataKey::Run(ghtui(), 7, None), run);
        let rerun = press(&mut s, "<C-r><Enter>");
        press(&mut s, "<Esc>");
        let star = press(&mut s, "s");
        answer(&mut s, &star, Ok(()));
        answer(&mut s, &rerun, Ok(()));
        // The page follows the run, not the list.
        let run = vec![crate::browse::Need::Data(DataKey::Run(ghtui(), 7, None))];
        assert_eq!(crate::act::poll(&mut s), run);
        answer(&mut s, &star, Ok(()));
        assert_eq!(crate::act::poll(&mut s), run);
    }

    /// Approving from the pull request's page reads your saved drafts
    /// first, submits them with the approval, and saves what's left.
    #[test]
    fn approving_from_the_page_takes_your_saved_drafts() {
        let mut s = with_pr(Mode::Dark);
        s.viewer = Some("me".into());
        assert_eq!(
            press(&mut s, "A"),
            vec![
                Cmd::Api(Api::FetchPendingReview(pr())),
                Cmd::LoadReview(pr())
            ]
        );
        let Some(Overlay::Submit(dialog)) = &s.overlay else {
            panic!("no dialog");
        };
        assert_eq!((dialog.event, dialog.quick), (ReviewEvent::Approve, true));
        // Enter before the drafts are read waits for them.
        assert!(press(&mut s, "<Enter>").is_empty());

        let draft = DraftComment {
            id: 3,
            path: "src/lib.rs".into(),
            body: "nit".into(),
            side: DraftSide::Right,
            line: Some(4),
            start_line: None,
            start_side: None,
            commit: pr_detail().head_oid,
            error: None,
        };
        let saved = ReviewState {
            pending: vec![draft.clone()],
            ..ReviewState::default()
        };
        update(
            &mut s,
            Msg::Diff(pr().into(), DiffMsg::ReviewLoaded(Ok(saved))),
        );
        press(&mut s, "LGTM");
        let cmds = press(&mut s, "<Enter>");
        assert_eq!(
            cmds,
            vec![Cmd::Api(Api::SubmitReview {
                pr: pr(),
                head: ghtui_git::Oid::parse(&pr_detail().head_oid).unwrap(),
                drafts: vec![draft],
                event: ReviewEvent::Approve,
                body: "LGTM".into(),
                seen: 0,
            })]
        );
        let outcome = SubmitOutcome {
            accepted: vec![3],
            submitted: true,
            ..SubmitOutcome::default()
        };
        let cmds = update(
            &mut s,
            Msg::Diff(pr().into(), DiffMsg::ReviewSubmitted(outcome)),
        );
        assert!(s.overlay.is_none());
        says(&s, "Approved gold-silver-copper/ghtui#12");
        let left = ReviewState {
            last_reviewed_head: Some(pr_detail().head_oid),
            ..ReviewState::default()
        };
        assert!(cmds.contains(&Cmd::SaveReview(pr(), left)), "{cmds:?}");
    }

    /// A finished run offers its failed jobs or all of them; a job adds
    /// itself; a finished run has nothing to cancel.
    #[test]
    fn runs_rerun_and_cancel() {
        let mut s = with_run(Mode::Dark);
        press(&mut s, "<C-r>");
        let choices: Vec<&str> = confirm(&s)
            .choices
            .iter()
            .map(|(l, _)| l.as_str())
            .collect();
        assert_eq!(choices, ["Re-run failed jobs", "Re-run all jobs"]);
        assert_eq!(
            press(&mut s, "<Enter>"),
            vec![Cmd::Api(Api::Change(
                Change::Rerun {
                    repo: ghtui(),
                    run: 7,
                    failed_only: true
                },
                By::Confirm(Subject::Run(ghtui(), 7, None))
            ))]
        );
        let mut s = with_run(Mode::Dark);
        press(&mut s, "X");
        says(&s, "The run has finished: there's nothing to cancel");

        let mut s = with_job(Mode::Dark, None);
        press(&mut s, "<C-r>");
        assert_eq!(confirm(&s).title, "Re-run CI #412?");
        let first = confirm(&s).choices.first().map(|(l, _)| l.clone());
        assert_eq!(first.as_deref(), Some("Re-run this job"));
    }

    /// A job that ended in a run still going is the run's to cancel, and
    /// not to re-run yet.
    #[test]
    fn a_finished_job_in_a_running_run_cancels_the_run() {
        let mut s = with_job(Mode::Dark, None);
        let (mut job, _) = crate::fixtures::job();
        job.run.outcome = CheckOutcome::Pending;
        fetched(
            &mut s,
            DataKey::Job(ghtui(), 2),
            Data::Job(Box::new(job.clone())),
        );
        press(&mut s, "<C-r>");
        says(&s, "The run is still going: X cancels it");
        press(&mut s, "X");
        assert_eq!(confirm(&s).title, "Cancel CI #412?");
        let cancel = Change::CancelRun {
            repo: ghtui(),
            run: 7,
        };
        let sent = press(&mut s, "<Enter>");
        assert_eq!(
            sent,
            vec![Cmd::Api(Api::Change(
                cancel,
                By::Confirm(Subject::Job(ghtui(), Some(7), 2))
            ))]
        );
        answer(&mut s, &sent, Ok(()));
        // GitHub takes a while to stop it: it's asked for once.
        press(&mut s, "X");
        says(&s, "hasn't shown the last change");
        assert!(s.busy().is_some_and(|b| b.starts_with("Cancel requested")));
        job.run.outcome = CheckOutcome::Cancelled;
        fetched(&mut s, DataKey::Job(ghtui(), 2), Data::Job(Box::new(job)));
        assert!(s.awaiting.is_none());
    }

    /// Re-running a job, the page moves to the job GitHub re-runs it as,
    /// once GitHub has it, and follows that one.
    #[test]
    fn a_rerun_job_is_followed_to_its_new_attempt() {
        let mut s = with_job(Mode::Dark, None);
        let rerun = Change::RerunJob {
            repo: ghtui(),
            job: 2,
        };
        let by = By::Confirm(Subject::Job(ghtui(), Some(7), 2));
        update(&mut s, Msg::Changed(rerun, by, Ok(())));
        let (mut job, _) = crate::fixtures::job();
        job.run.attempt = 2;
        job.run.outcome = CheckOutcome::Pending;
        fetched(
            &mut s,
            DataKey::Job(ghtui(), 2),
            Data::Job(Box::new(job.clone())),
        );
        assert!(
            matches!(s.route(), Some(Route::Job { job: 2, .. })),
            "not there yet"
        );
        job.run.rerun = Some(9);
        let cmds = update(
            &mut s,
            Msg::Fetched {
                key: DataKey::Job(ghtui(), 2),
                result: Ok(Data::Job(Box::new(job))),
                cached_at: None,
            },
        );
        assert!(
            matches!(s.route(), Some(Route::Job { job: 9, .. })),
            "{:?}",
            s.route()
        );
        let fetch = Cmd::Api(Api::Fetch {
            key: DataKey::Job(ghtui(), 9),
            cached: true,
        });
        assert!(cmds.contains(&fetch), "{cmds:?}");
    }

    /// While a job on screen is running, it's fetched again on a timer
    /// (never two at once); once it's done, the timer stops.
    #[test]
    fn a_running_job_is_followed_until_it_ends() {
        let mut s = with_job(Mode::Dark, None);
        let live = Cmd::Timer(Timer::Live, 5_000);
        assert!(
            !timers(&mut s).contains(&live),
            "a finished job isn't followed"
        );
        let (mut job, mut log) = crate::fixtures::job();
        job.outcome = CheckOutcome::Pending;
        log.running = true;
        fetched(
            &mut s,
            DataKey::Job(ghtui(), 2),
            Data::Job(Box::new(job.clone())),
        );
        fetched(
            &mut s,
            DataKey::JobLog(ghtui(), 2),
            Data::Log(std::sync::Arc::new(log)),
        );
        let cmds = timers(&mut s);
        assert!(cmds.contains(&live), "{cmds:?}");
        assert!(!timers(&mut s).contains(&live), "one timer at a time");
        let cmds = update(&mut s, Msg::Timer(Timer::Live));
        let fetch = |key| Cmd::Api(Api::Fetch { key, cached: false });
        assert!(cmds.contains(&fetch(DataKey::Job(ghtui(), 2))), "{cmds:?}");
        assert!(
            cmds.contains(&fetch(DataKey::JobLog(ghtui(), 2))),
            "{cmds:?}"
        );
        // It ends: no more fetching.
        job.outcome = CheckOutcome::Failure;
        let (_, log) = crate::fixtures::job();
        fetched(&mut s, DataKey::Job(ghtui(), 2), Data::Job(Box::new(job)));
        fetched(
            &mut s,
            DataKey::JobLog(ghtui(), 2),
            Data::Log(std::sync::Arc::new(log)),
        );
        assert!(s.running_here().is_empty());
    }
}
