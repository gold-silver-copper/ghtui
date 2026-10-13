//! The states tests start from: each screen built from fixtures, as if
//! the keys and replies that lead there had happened.

use super::*;

/// A pull request as Home's sections list it.
pub(crate) fn row(
    pr: &str,
    title: &str,
    state: PrState,
    review: Option<ReviewDecision>,
    checks: Option<ChecksState>,
) -> crate::api::browse::IssueSummary {
    let mut row = fixtures::found_pr(pr, title, state);
    if let IssueKind::Pr {
        review: r,
        checks: c,
        ..
    } = &mut row.kind
    {
        (*r, *c) = (review, checks);
    }
    row
}

/// Home's default sections: review requests, your pull requests (2 of
/// 31), your repositories.
pub(crate) fn fill_home(state: &mut State) {
    let requests = vec![
        row(
            "ratatui/ratatui#1820",
            "Add a virtualized list widget for very long collections",
            PrState::Open,
            Some(ReviewDecision::ReviewRequired),
            Some(ChecksState::Passing),
        ),
        row(
            "tokio-rs/tokio#7001",
            "Fix waker leak in the multi-thread scheduler when tasks are cancelled during shutdown",
            PrState::Draft,
            None,
            Some(ChecksState::Pending),
        ),
    ];
    fixtures::section(state, 0, fixtures::found_prs(requests, 2));
    let mine = vec![
        row(
            "gold-silver-copper/ghtui#12",
            "Theme: generate syntax palette from seed",
            PrState::Open,
            Some(ReviewDecision::Approved),
            Some(ChecksState::Failing),
        ),
        row(
            "gold-silver-copper/ghtui#9",
            "Cache GraphQL responses in redb",
            PrState::Open,
            Some(ReviewDecision::ChangesRequested),
            None,
        ),
    ];
    fixtures::section(state, 1, fixtures::found_prs(mine, 31));
    let repos = vec![
        fixtures::repo_summary("gold-silver-copper/ghtui", 1234),
        fixtures::repo_summary("gold-silver-copper/fux", 87),
    ];
    let repos = SearchResults::Repos(crate::api::browse::Results {
        total: 2,
        items: repos,
        next: None,
    });
    fixtures::section(state, 2, repos);
}

/// A fresh start, signed in as octocat, at [`NOW`].
pub(crate) fn state() -> State {
    let mut state = State::new(
        Theme::new(DEFAULT_SEED, Mode::Dark, ColorDepth::TrueColor),
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

pub(crate) fn with_home() -> State {
    let mut state = state();
    fill_home(&mut state);
    state
}

pub(crate) fn with_pr() -> State {
    let mut state = with_home();
    let pr = PrRef::parse("gold-silver-copper/ghtui#12").unwrap();
    let _ = state.push(Route::pr(pr.clone()));
    update(&mut state, Msg::Pr(pr.clone(), Box::new(Ok(pr_detail()))));
    fetched(
        &mut state,
        DataKey::InProjects(pr.repo.clone(), pr.number),
        Data::InProjects(Box::default()),
    );
    fetched(
        &mut state,
        DataKey::PrActivity(pr),
        Data::PrActivity(Box::new(fixtures::activity())),
    );
    state
}

/// Pushes `route` and delivers `data` for its page's own fetch (its last).
pub(crate) fn open(state: &mut State, route: Route, data: Data) {
    let Some(Need::Data(key)) = needs(&route).pop() else {
        panic!("{route:?} doesn't end in a data fetch");
    };
    let _ = state.push(route);
    fetched(state, key, data);
}

pub(crate) fn ghtui() -> RepoId {
    RepoId::new("gold-silver-copper", "ghtui")
}

/// A repository's Code tab with everything it asked for delivered.
pub(crate) fn with_repo() -> State {
    let mut state = state();
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
pub(crate) fn with_file() -> State {
    let mut state = with_repo();
    let route = Route::blob(ghtui(), "main".into(), "src/main.rs".into());
    open(&mut state, route, Data::Blob(Box::new(fixtures::blob())));
    state
}

/// A file's blame.
pub(crate) fn with_blame() -> State {
    let mut state = with_repo();
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
pub(crate) fn with_gist() -> State {
    let mut state = state();
    let route = Route::Gist {
        owner: Some("octocat".into()),
        id: "6cad326836d38bd3a7ae".into(),
    };
    open(&mut state, route, Data::Gist(Box::new(fixtures::gist())));
    state
}

/// Someone's gists.
pub(crate) fn with_gists() -> State {
    let mut state = state();
    let route = Route::Gists("octocat".into());
    open(&mut state, route, Data::Gists(Box::new(fixtures::gists())));
    state
}
/// An organization's teams.
pub(crate) fn with_teams() -> State {
    let mut state = state();
    open(
        &mut state,
        Route::Teams("github".into()),
        Data::Teams(Box::new(fixtures::teams())),
    );
    state
}

/// A team.
pub(crate) fn with_team() -> State {
    let mut state = state();
    let route = Route::Team {
        org: "github".into(),
        slug: "core".into(),
    };
    open(&mut state, route, Data::Team(Box::new(fixtures::team())));
    state
}
/// A repository's security advisories.
pub(crate) fn with_advisories() -> State {
    let mut state = with_repo();
    let route = Route::Advisories(Some(ghtui()));
    let items = fixtures::advisories();
    let list = crate::api::browse::Results {
        total: items.len() as u64,
        items,
        next: None,
    };
    open(&mut state, route, Data::Advisories(Box::new(list)));
    state
}

/// A security advisory from GitHub's database.
pub(crate) fn with_advisory() -> State {
    let mut state = state();
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
pub(crate) fn with_wiki() -> State {
    let mut state = with_repo();
    let route = Route::Wiki {
        repo: ghtui(),
        page: None,
    };
    open(&mut state, route, Data::Wiki(Box::new(fixtures::wiki())));
    state
}
/// The repository's open issues.
pub(crate) fn with_issues() -> State {
    let mut state = with_repo();
    let route = Route::Issues {
        repo: ghtui(),
        query: OPEN.into(),
    };
    let results = fixtures::issue_results(Some("c1"));
    open(&mut state, route, Data::Search(Box::new(results)));
    state
}

/// A commit, its repository's header loaded first.
pub(crate) fn with_commit() -> State {
    let mut state = with_repo();
    let commit = fixtures::commit();
    let route = Route::Commit {
        repo: ghtui(),
        oid: commit.oid.clone(),
        pr: None,
    };
    let comments = DataKey::CommitComments(ghtui(), commit.oid.clone());
    open(&mut state, route, Data::Commit(Box::new(commit)));
    let comment = crate::api::commit_comments::CommitComment {
        id: 7,
        node_id: "CC_7".into(),
        author: "hubot".into(),
        body: "This one broke the 256-color theme.".into(),
        created_at: "2026-10-02T12:00:00Z".into(),
        path: Some("src/theme.rs".into()),
        line: Some(12),
        url: String::new(),
    };
    fetched(
        &mut state,
        comments,
        Data::CommitComments(Box::new(vec![comment].into())),
    );
    state
}

/// A pull request's Checks tab.
pub(crate) fn with_pr_checks() -> State {
    let mut state = with_pr();
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

/// A pull request's Review tab: a draft, a thread still open (and one
/// resolved), the reviewers and what stops the merge.
pub(crate) fn with_pr_review() -> State {
    let mut state = with_pr();
    let pr = PrRef::parse("gold-silver-copper/ghtui#12").unwrap();
    press(&mut state, "gr");
    // The diff it starts reads the drafts and threads.
    let of = crate::diff_screen::DiffOf::Pr(pr.clone());
    state.diffs.insert(of.clone(), fixture_diff());
    let review = crate::store::ReviewState {
        pending: vec![fixtures::draft(1, 5, "Could this name say what it holds?")],
        ..Default::default()
    };
    let mut open = fixtures::thread("t", Some(14), false, false);
    open.comments[0].url = format!("{}#discussion_r42", pr.url());
    let threads = vec![open, fixtures::thread("done", Some(3), true, false)];
    for msg in [
        crate::state::DiffMsg::ReviewLoaded(Ok(review)),
        crate::state::DiffMsg::ThreadsLoaded(Ok(threads)),
    ] {
        update(&mut state, Msg::Diff(of.clone(), msg));
    }
    fetched(
        &mut state,
        DataKey::Reviews(pr),
        Data::Reviews(Box::new(fixtures::reviews())),
    );
    state
}

/// A repository's Actions tab.
pub(crate) fn with_actions() -> State {
    let mut state = with_repo();
    let route = Route::actions(ghtui());
    let runs = Data::Runs(Box::new(fixtures::workflow_runs().1));
    open(&mut state, route, runs);
    state
}

/// Your inbox, from Home.
pub(crate) fn with_notifications() -> State {
    let mut state = with_home();
    let route = Route::Notifications {
        query: String::new(),
    };
    let inbox = Data::Notifications(Box::new(fixtures::notifications()));
    open(&mut state, route, inbox);
    state
}

/// `state` in a `w`×`h` terminal.
pub(crate) fn sized(mut state: State, w: u16, h: u16) -> State {
    update(&mut state, Msg::Resize(w, h));
    state
}

/// The repository page with `n` more tabs open after it: an issue, a
/// pull request, a profile, a file, a commit…, then back on the second.
pub(crate) fn with_tabs(n: usize) -> State {
    let mut state = with_repo();
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
            pr: None,
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
pub(crate) fn with_org() -> State {
    let mut state = state();
    let route = Route::user("ratatui");
    open(
        &mut state,
        route,
        Data::Profile(Box::new(fixtures::org_profile())),
    );
    state
}

/// A repository's discussions.
pub(crate) fn with_discussions() -> State {
    let mut state = with_repo();
    let of = crate::api::browse::DiscussionsOf::Repo(ghtui());
    let route = Route::Discussions { of, category: None };
    open(
        &mut state,
        route,
        Data::Discussions(Box::new(fixtures::discussions())),
    );
    state
}

/// A discussion.
pub(crate) fn with_discussion() -> State {
    let mut state = with_repo();
    let of = crate::api::browse::DiscussionsOf::Repo(ghtui());
    let route = Route::Discussion { of, number: 14603 };
    open(
        &mut state,
        route,
        Data::Discussion(Box::new(fixtures::discussion())),
    );
    state
}
/// A workflow run.
pub(crate) fn with_run() -> State {
    let mut state = with_repo();
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
pub(crate) fn with_job(step: Option<(u32, u32)>) -> State {
    let mut state = with_repo();
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
pub(crate) fn with_workflow() -> State {
    let mut state = with_repo();
    let route = Route::Workflow {
        repo: ghtui(),
        file: "ci.yml".into(),
    };
    let _ = state.push(route);
    let (workflow, runs) = fixtures::workflow_runs();
    let key = DataKey::Workflow(ghtui(), "ci.yml".into());
    fetched(&mut state, key, Data::Workflow(Box::new(workflow)));
    let key = DataKey::Runs(ghtui(), "workflow:ci.yml".into());
    fetched(&mut state, key, Data::Runs(Box::new(runs)));
    state
}

/// A repository's stargazers.
pub(crate) fn with_stargazers() -> State {
    let mut state = with_repo();
    let route = Route::Stargazers(ghtui());
    open(&mut state, route, Data::Users(Box::new(fixtures::people())));
    state
}

/// A repository's forks.
pub(crate) fn with_forks() -> State {
    let mut state = with_repo();
    let route = Route::Forks(ghtui());
    open(
        &mut state,
        route,
        Data::RepoPage(Box::new(fixtures::forks())),
    );
    state
}

/// A repository's releases.
pub(crate) fn with_releases() -> State {
    let mut state = with_repo();
    let route = Route::Releases(ghtui());
    open(
        &mut state,
        route,
        Data::Releases(Box::new(fixtures::releases())),
    );
    state
}

/// A release's page.
pub(crate) fn with_release() -> State {
    let mut state = with_repo();
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
pub(crate) fn with_tags() -> State {
    let mut state = with_repo();
    open(
        &mut state,
        Route::Tags(ghtui()),
        Data::Tags(Box::new(fixtures::tags())),
    );
    state
}

/// A repository's branches.
pub(crate) fn with_branches() -> State {
    let mut state = with_repo();
    open(
        &mut state,
        Route::Branches(ghtui()),
        Data::Branches(Box::new(fixtures::branches())),
    );
    state
}
/// A repository's milestones.
pub(crate) fn with_milestones() -> State {
    let mut state = with_repo();
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
pub(crate) fn with_milestone() -> State {
    let mut state = with_repo();
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
pub(crate) fn with_deployments() -> State {
    let mut state = with_repo();
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
pub(crate) fn with_compare() -> State {
    let mut state = with_repo();
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
pub(crate) fn with_history() -> State {
    let mut state = with_repo();
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
pub(crate) fn with_issue() -> State {
    let mut state = state();
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
    fetched(
        &mut state,
        DataKey::InProjects(ghtui(), 14),
        Data::InProjects(Box::new(fixtures::in_projects())),
    );
    state
}

pub(crate) fn with_profile(tab: ProfileTab) -> State {
    let mut state = state();
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

pub(crate) fn with_search(kind: SearchKind, query: &str, results: SearchResults) -> State {
    let mut state = state();
    let query = query.to_owned();
    open(
        &mut state,
        Route::Search { kind, query },
        Data::Search(Box::new(results)),
    );
    state
}

/// Repositories matching "terminal file manager".
pub(crate) fn with_repo_search() -> State {
    let results = fixtures::repo_results();
    with_search(SearchKind::Repos, "terminal file manager", results)
}

/// Every page ghtui has a fixture for, with its data in.
pub(crate) fn pages() -> Vec<(&'static str, State)> {
    let mut commits = with_pr();
    press(&mut commits, "2gt");
    vec![
        ("home", with_home()),
        ("repo", with_repo()),
        ("file", with_file()),
        ("issues", with_issues()),
        ("issue", with_issue()),
        ("pr", with_pr()),
        ("pr commits", commits),
        ("profile", with_profile(ProfileTab::Overview)),
        (
            "repositories",
            with_profile(ProfileTab::Repositories(
                crate::api::browse::RepoSort::Updated,
            )),
        ),
        ("stars", with_profile(ProfileTab::Stars)),
        ("search", with_repo_search()),
        ("commit", with_commit()),
        ("commits", with_history()),
        ("pr checks", with_pr_checks()),
        ("actions", with_actions()),
        ("stargazers", with_stargazers()),
        ("forks", with_forks()),
        ("releases", with_releases()),
        ("release", with_release()),
        ("tags", with_tags()),
        ("branches", with_branches()),
        (
            "discussion search",
            with_search(
                crate::api::browse::SearchKind::Discussions,
                "q",
                crate::fixtures::discussion_results(),
            ),
        ),
        (
            "commit search",
            with_search(
                crate::api::browse::SearchKind::Commits,
                "q",
                crate::fixtures::commit_results(),
            ),
        ),
        (
            "code search",
            with_search(
                crate::api::browse::SearchKind::Code,
                "q",
                crate::fixtures::code_results(),
            ),
        ),
        ("wiki", with_wiki()),
        ("advisories", with_advisories()),
        ("advisory", with_advisory()),
        ("teams", with_teams()),
        ("team", with_team()),
        ("gist", with_gist()),
        ("gists", with_gists()),
        ("blame", with_blame()),
        ("compare", with_compare()),
        ("deployments", with_deployments()),
        ("milestones", with_milestones()),
        ("milestone", with_milestone()),
        ("run", with_run()),
        ("discussions", with_discussions()),
        ("discussion", with_discussion()),
        ("job", with_job(None)),
        ("workflow", with_workflow()),
        ("notifications", with_notifications()),
    ]
}
