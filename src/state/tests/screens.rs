//! The states tests start from: each screen built from fixtures, as if
//! the keys and replies that lead there had happened.

use super::*;
use crate::route::{
    code, discussions, gists, issues, people, pulls, releases, runs, search, security, wiki,
};

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
    let repos = SearchResults::Repos(fixtures::page(2, repos, None));
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

/// gold-silver-copper/ghtui#12, the fixture pull request.
pub(crate) fn pr() -> PrRef {
    PrRef::parse("gold-silver-copper/ghtui#12").unwrap()
}

/// [`pr_detail`] without its conflicts (GitHub would merge it), changed
/// by `f`.
pub(crate) fn mergeable(f: impl FnOnce(&mut PrDetail)) -> PrDetail {
    let mut d = pr_detail();
    d.mergeable = crate::api::model::Mergeable::Yes;
    d.merge_state = crate::api::model::MergeState::Unstable;
    f(&mut d);
    d
}

/// GitHub's answer for [`pr`]: [`mergeable`], changed by `f`.
pub(crate) fn pr_shows(s: &mut State, f: impl FnOnce(&mut PrDetail)) -> Cmds {
    fixtures::pr_fetched(s, &pr(), Ok(mergeable(f)))
}

/// [`with_pr`], then GitHub's answer changed by `f`.
pub(crate) fn pr_with(f: impl FnOnce(&mut PrDetail)) -> State {
    let mut s = with_pr();
    pr_shows(&mut s, f);
    s
}

/// GitHub's answer for the fixture issue, changed by `f`.
pub(crate) fn issue_shows(
    s: &mut State,
    f: impl FnOnce(&mut crate::api::browse::IssueDetail),
) -> Cmds {
    let mut d = fixtures::issue();
    f(&mut d);
    let key = DataKey::Issue(ghtui(), 14);
    fetched(s, key, Data::Issue(Some(Box::new(d))))
}

/// [`with_issue`] in a tall terminal, then GitHub's answer changed by `f`.
pub(crate) fn issue_with(f: impl FnOnce(&mut crate::api::browse::IssueDetail)) -> State {
    let mut s = sized(with_issue(), 100, 40);
    issue_shows(&mut s, f);
    s
}

/// Each `name: from => route;` is a state `from`, gone to `route` with
/// what it fetches answered ([`at`]).
macro_rules! screens {
    ($($(#[$doc:meta])* $name:ident: $from:expr => $route:expr;)*) => {$(
        $(#[$doc])*
        pub(crate) fn $name() -> State {
            at($from, $route)
        }
    )*};
}

screens! {
    with_pr: with_home() => Route::pr(pr());
    /// A repository's Code tab with everything it asked for delivered.
    with_repo: state() => Route::code(ghtui());
    /// A file in the repository.
    with_file: with_repo() => Route::blob(ghtui(), "main".into(), "src/main.rs".into());
    /// A file's blame.
    with_blame: with_repo() => Route::Code(code::Blame {
        repo: ghtui(), rev: "main".into(), path: "src/main.rs".into(), lines: Some((5, 5)),
    });
    /// A gist.
    with_gist: state() => Route::Gists(gists::Gist {
        owner: Some("octocat".into()), id: "6cad326836d38bd3a7ae".into(),
    });
    /// Someone's gists.
    with_gists: state() => Route::Gists(gists::Gists("octocat".into()));
    /// An organization's teams.
    with_teams: state() => Route::People(people::Teams("github".into()));
    /// A team.
    with_team: state() => Route::People(people::Team { org: "github".into(), slug: "core".into() });
    /// A repository's security advisories.
    with_advisories: with_repo() => Route::Security(security::Advisories(Some(ghtui())));
    /// A security advisory from GitHub's database.
    with_advisory: state() => Route::Security(security::Advisory {
        repo: None, ghsa: "GHSA-6r3q-mjv7-xr8m".into(),
    });
    /// A wiki's Home.
    with_wiki: with_repo() => Route::Wiki(wiki::Wiki { repo: ghtui(), page: None });
    /// The repository's open issues.
    with_issues: with_repo() => Route::Issues(issues::Issues { repo: ghtui(), query: OPEN.into() });
    /// A commit, its repository's header loaded first.
    with_commit: with_repo() => Route::Code(code::Commit {
        repo: ghtui(), oid: fixtures::commit().oid, pr: None,
    });
    /// A pull request's Checks tab.
    with_pr_checks: with_pr() => Route::Pulls(pulls::Pr { pr: pr(), tab: PrTab::Checks });
    /// Your inbox, from Home.
    with_notifications: with_home() => Route::Search(search::Notifications {
        query: String::new(),
    });
    /// An organization's profile.
    with_org: state() => Route::user("ratatui");
    /// A repository's discussions.
    with_discussions: with_repo() => Route::Discussions(discussions::Discussions {
        of: crate::api::browse::DiscussionsOf::Repo(ghtui()), category: None,
    });
    /// A discussion.
    with_discussion: with_repo() => Route::Discussions(discussions::Discussion {
        of: crate::api::browse::DiscussionsOf::Repo(ghtui()), number: 14603,
    });
    /// A workflow run.
    with_run: with_repo() => Route::Runs(runs::WorkflowRun {
        repo: ghtui(), run: 7, attempt: None,
    });
    /// A workflow and its runs.
    with_workflow: with_repo() => Route::Runs(runs::Workflow {
        repo: ghtui(), file: "ci.yml".into(),
    });
    /// A release's page.
    with_release: with_repo() => Route::Releases(releases::Release {
        repo: ghtui(), tag: "v0.2.0".into(),
    });
    /// A repository's branches.
    with_branches: with_repo() => Route::Code(code::Branches(ghtui()));
    /// A repository's milestones.
    with_milestones: with_repo() => Route::Issues(issues::Milestones {
        repo: ghtui(), closed: false,
    });
    /// A milestone.
    with_milestone: with_repo() => Route::Issues(issues::Milestone { repo: ghtui(), number: 3 });
    /// A repository's deployments.
    with_deployments: with_repo() => Route::Runs(runs::Deployments {
        repo: ghtui(), environment: None,
    });
    /// Two tags compared.
    with_compare: with_repo() => Route::Code(code::Compare {
        repo: ghtui(), spec: "v0.1.0...v0.2.0".into(),
    });
    /// An issue, its repository's header loaded first.
    with_issue: state() => Route::Issues(issues::Issue { repo: ghtui(), number: 14 });
}

/// Pushes `route` and delivers `data` for its page's own fetch (its last).
pub(crate) fn open(state: &mut State, route: Route, data: Data) {
    let Some(key) = state.needs(&route).pop() else {
        panic!("{route:?} doesn't end in a data fetch")
    };
    let _ = state.push(route);
    fetched(state, key, data);
}

/// `state` gone to `route`, what it fetches answered as by
/// [`fixtures::served`].
pub(crate) fn at(mut state: State, route: Route) -> State {
    let cmds = state.push(route);
    fixtures::served(&mut state, &cmds);
    state
}

pub(crate) fn ghtui() -> RepoId {
    RepoId::new("gold-silver-copper", "ghtui")
}

/// A pull request's Review tab: a draft, a thread still open (and one
/// resolved), the reviewers and what stops the merge.
pub(crate) fn with_pr_review() -> State {
    let mut state = with_pr();
    let pr = pr();
    act(&mut state, Action::ReviewGlance);
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
        Data::from(fixtures::reviews()),
    );
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
        Route::Issues(issues::Issue {
            repo: ghtui(),
            number: 14,
        }),
        Route::pr(pr()),
        Route::user("octocat"),
        Route::blob(ghtui(), "main".into(), "crates/app/src/main.rs".into()),
        Route::Code(code::Commit {
            repo: ghtui(),
            oid: fixtures::commit().oid,
            pr: None,
        }),
        Route::Releases(releases::Releases(ghtui())),
        Route::People(people::Stargazers(ghtui())),
    ];
    for route in more.into_iter().cycle().take(n) {
        let _ = state.open_tab(crate::route::Target::Page(route).into());
    }
    let _ = state.switch_to_tab(1);
    state.notices.dismiss();
    state
}

/// A job, its failed step's log open; `step` is the step a link pointed at.
pub(crate) fn with_job(step: Option<(u32, u32)>) -> State {
    let (repo, run, query) = (ghtui(), Some(7), String::new());
    at(
        with_repo(),
        Route::Runs(runs::Job {
            repo,
            run,
            job: 2,
            step,
            query,
        }),
    )
}

pub(crate) fn with_profile(tab: ProfileTab) -> State {
    let login = "octocat".into();
    at(state(), Route::People(people::User { login, tab }))
}

pub(crate) fn with_search(kind: SearchKind, query: &str) -> State {
    let query = query.to_owned();
    at(state(), Route::Search(search::Search { kind, query }))
}

/// Every page ghtui has a fixture for, with its data in.
pub(crate) fn pages() -> Vec<(&'static str, State)> {
    let mut commits = with_pr();
    press(&mut commits, "2");
    let repo = |route| at(with_repo(), route);
    let commits_of = code::Commits {
        repo: ghtui(),
        rev: "main".into(),
        path: String::new(),
    };
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
        (
            "search",
            with_search(SearchKind::Repos, "terminal file manager"),
        ),
        ("commit", with_commit()),
        ("commits", repo(Route::Code(commits_of))),
        ("pr checks", with_pr_checks()),
        ("actions", repo(Route::actions(ghtui()))),
        (
            "stargazers",
            repo(Route::People(people::Stargazers(ghtui()))),
        ),
        ("forks", repo(Route::People(people::Forks(ghtui())))),
        (
            "releases",
            repo(Route::Releases(releases::Releases(ghtui()))),
        ),
        ("release", with_release()),
        ("tags", repo(Route::Code(code::Tags(ghtui())))),
        ("branches", with_branches()),
        (
            "discussion search",
            with_search(SearchKind::Discussions, "q"),
        ),
        ("commit search", with_search(SearchKind::Commits, "q")),
        ("code search", with_search(SearchKind::Code, "q")),
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
