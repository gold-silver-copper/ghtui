use crate::theme::{ColorDepth, Mode};
use crate::ui::pages::ProfileTab;

use super::{
    press, with_actions, with_advisories, with_advisory, with_blame, with_branches, with_commit,
    with_compare, with_deployments, with_discussion, with_discussions, with_file, with_forks,
    with_gist, with_gists, with_history, with_home, with_issue, with_issues, with_job,
    with_milestone, with_milestones, with_notifications, with_pr, with_pr_checks, with_profile,
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
        .any(|(entry, expect, _)| matches!(*expect, "external" | "todo") && shape(entry) == wanted)
}

/// Every page ghtui has a fixture for.
fn pages() -> Vec<(&'static str, State)> {
    let tc = ColorDepth::TrueColor;
    let mut commits = with_pr(Mode::Dark);
    press(&mut commits, "2gt");
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
                ProfileTab::Repositories(crate::api::browse::RepoSort::Updated),
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
                crate::api::browse::SearchKind::Discussions,
                "q",
                crate::fixtures::discussion_results(),
            ),
        ),
        (
            "commit search",
            with_search(
                Mode::Dark,
                crate::api::browse::SearchKind::Commits,
                "q",
                crate::fixtures::commit_results(),
            ),
        ),
        (
            "code search",
            with_search(
                Mode::Dark,
                crate::api::browse::SearchKind::Code,
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
        ("notifications", with_notifications(Mode::Dark)),
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
        repo: crate::api::model::RepoId::parse(repo).unwrap(),
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
