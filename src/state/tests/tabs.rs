use super::*;
use crate::route::{code, issues, releases};

/// Two routes to tell tabs apart by.
fn issue(n: u64) -> Route {
    Route::Issues(issues::Issue {
        repo: ghtui(),
        number: n,
    })
}

#[test]
fn open_switch_and_close() {
    let mut s = with_repo();
    assert_eq!(s.tab_count(), 1);
    // Nothing selected on a reading page: T opens it again.
    let _ = s.push(issue(1));
    let _ = act(&mut s, Action::OpenInTab);
    assert_eq!((s.tab_count(), s.active_tab()), (2, 1));
    assert_eq!(route(&s), issue(1));
    let _ = s.push(issue(2));
    press(&mut s, "<C-PageUp>");
    assert_eq!((s.active_tab(), route(&s)), (0, issue(1)));
    press(&mut s, "<C-PageDown>");
    assert_eq!((s.active_tab(), route(&s)), (1, issue(2)));
    // Wrapping around, and straight to a number.
    press(&mut s, "<C-PageDown>");
    assert_eq!(s.active_tab(), 0);
    press(&mut s, "<A-2>");
    assert_eq!(s.active_tab(), 1);
    press(&mut s, "<A-5>");
    says(&s, "No tab 5");
    // Closing shows the next tab, or the previous at the end.
    press(&mut s, "<C-w>");
    assert_eq!((s.tab_count(), route(&s)), (1, issue(1)));
    press(&mut s, "<C-w>");
    assert_eq!(s.tab_count(), 1);
    says(&s, "last tab");
}

/// Back and forward stay within a tab.
#[test]
fn each_tab_has_its_own_history() {
    let mut s = with_repo();
    let _ = s.push(issue(1));
    let _ = act(&mut s, Action::OpenInTab);
    let _ = s.push(issue(2));
    let _ = s.push(issue(3));
    act(&mut s, Action::Back);
    assert_eq!(route(&s), issue(2));
    press(&mut s, "<C-PageUp>");
    assert_eq!(route(&s), issue(1));
    act(&mut s, Action::Forward);
    assert_eq!(route(&s), issue(1), "nothing to go forward to here");
    press(&mut s, "<C-PageDown>");
    act(&mut s, Action::Forward);
    assert_eq!(route(&s), issue(3));
}

/// Two tabs on one pull request share its diff (and so its review).
#[test]
fn tabs_share_a_pull_requests_diff() {
    let (mut s, pr) = crate::state::tests::diff::diff_state(120);
    if let Screen::Diff(d) = s.screen_mut() {
        d.focus = crate::diff_screen::Pane::Tree;
    }
    let _ = act(&mut s, Action::OpenInTab);
    assert_eq!(s.tab_count(), 2);
    assert!(matches!(s.screen(), Screen::Diff(d) if d.of == DiffOf::Pr(pr.clone())));
    assert_eq!(s.diffs.len(), 1, "one diff for both tabs");
    // From the diff, T opens the pull request's page.
    let _ = act(&mut s, Action::OpenInTab);
    assert_eq!(route(&s), Route::pr(pr));
}

/// The tabs open at quit come back, in order, the first on screen.
#[test]
fn tabs_reopen() {
    let mut s = with_repo();
    let _ = s.push(issue(1));
    let _ = act(&mut s, Action::OpenInTab);
    let _ = s.push(issue(2));
    let urls = s.tab_urls();
    let mut again = state();
    again.restore_tabs(&urls);
    assert_eq!(again.tab_titles(), ["ghtui#1", "ghtui#2"]);
    assert_eq!((again.active_tab(), route(&again)), (0, issue(1)));
}

/// A tab opened from a link to a commit's or a range's diff
/// reopens on what it showed: a diff by full ids stays a diff, and
/// a short SHA or a ref (which open the commit page) stay pages.
#[test]
fn a_tab_from_a_diff_link_reopens_on_it() {
    let (a, b) = ("a".repeat(40), "b".repeat(40));
    for (link, diff) in [
        (
            "https://github.com/o/r/commit/abc1234.diff".to_owned(),
            false,
        ),
        ("https://github.com/o/r/commit/main.diff".to_owned(), false),
        (format!("https://github.com/o/r/commit/{a}.diff"), true),
        (format!("https://github.com/o/r/commit/{a}#files"), true),
        (
            format!("https://github.com/o/r/compare/{a}..{b}#files"),
            true,
        ),
    ] {
        let mut s = state();
        s.restore_tabs(std::slice::from_ref(&link));
        let shown = (s.tab_titles(), matches!(s.screen(), Screen::Diff(_)));
        assert_eq!(shown.1, diff, "{link}");
        let urls = s.tab_urls();
        let mut again = state();
        again.restore_tabs(&urls);
        let reopened = (
            again.tab_titles(),
            matches!(again.screen(), Screen::Diff(_)),
        );
        assert_eq!(reopened, shown, "{link} saved as {urls:?}");
    }
}

/// A tab on a branch with a slash in its name (`feature/x`)
/// reopens on that branch, not on `feature` with `x` as a path.
#[test]
fn a_tab_on_a_slashed_branch_reopens_on_it() {
    let tree = Route::Code(code::Tree {
        repo: RepoId::new("o", "r"),
        rev: "feature/x".into(),
        path: "src".into(),
    });
    let mut s = state();
    let _ = s.push(tree.clone());
    let urls = s.tab_urls();
    let mut again = state();
    again.restore_tabs(&urls);
    assert_eq!(route(&again), tree, "{urls:?}");
}

/// A page asked for on the command line loads, also when it's the
/// last of the reopened tabs.
#[test]
fn the_page_asked_for_loads_after_reopened_tabs() {
    let releases = Route::Releases(releases::Releases(RepoId::new("o", "r")));
    let mut s = state();
    s.restore_tabs(&[issue(1).url(), releases.url()]);
    let cmds = s.start_at(Target::Page(releases).into());
    assert_eq!(s.tab_count(), 3);
    let fetches = |key: &DataKey| fixtures::asks(&cmds, key);
    assert!(
        fetches(&DataKey::Releases(RepoId::new("o", "r"))),
        "{cmds:?}"
    );
}

/// A comment's link keeps its anchor however it's opened: in a new
/// tab, or from the command line after reopened tabs.
#[test]
fn anchors_survive_every_way_in() {
    let anchor = |s: &State| match s.screen() {
        Screen::Page(p) => p.anchor.clone(),
        Screen::Diff(_) => None,
    };
    let url = "https://github.com/o/r/issues/1#issuecomment-5";
    let mut s = state();
    let _ = s.open_tab(Dest::from_url(url));
    assert_eq!(s.tab_count(), 2);
    assert_eq!(anchor(&s).as_deref(), Some("issuecomment-5"));
    let mut s = state();
    s.restore_tabs(&[issue(2).url(), issue(3).url()]);
    let _ = s.start_at(Dest::from_url(url));
    assert_eq!(anchor(&s).as_deref(), Some("issuecomment-5"));
}

/// A plain link to a file on a slashed branch waits for GitHub to
/// say where the branch ends, then is that file, in its place.
/// Links at the default branch or at the revision on screen don't
/// ask.
#[test]
fn plain_links_open_the_branch_github_names() {
    let mut s = with_repo();
    let url = "https://github.com/gold-silver-copper/ghtui/blob/feature/x/src/lib.rs#L2";
    let key = DataKey::RefIn(ghtui(), "feature/x/src/lib.rs".into());
    assert_eq!(fetches(s.go(Dest::from_url(url))), vec![fetch(key.clone())]);
    let depth = s.screens.len();
    let cmds = fetched(&mut s, key, Data::RefIn(Some("feature/x".into())));
    let file = Route::Code(code::Blob {
        repo: ghtui(),
        rev: "feature/x".into(),
        path: "src/lib.rs".into(),
        lines: Some((2, 2)),
    });
    assert_eq!(route(&s), file);
    assert_eq!(s.screens.len(), depth, "in its place");
    let text = DataKey::Blob(ghtui(), "feature/x".into(), "src/lib.rs".into());
    assert_eq!(fetches(cmds), vec![fetch(text)]);

    let base = "https://github.com/gold-silver-copper/ghtui";
    let _ = s.go(Dest::from_url(&format!("{base}/tree/feature/x/src")));
    assert_eq!(
        route(&s),
        Route::Code(code::Tree {
            repo: ghtui(),
            rev: "feature/x".into(),
            path: "src".into()
        })
    );
    let _ = s.go(Dest::from_url(&format!("{base}/blob/main/README.md")));
    assert_eq!(route(&s), blob("main", "README.md"));
}

/// A comment's link on an issue number that's a pull request keeps
/// its anchor through GitHub's redirect to the pull request.
#[test]
fn an_issue_link_redirected_to_its_pr_keeps_the_anchor() {
    let url = "https://github.com/o/r/issues/7#issuecomment-5";
    let mut s = state();
    let _ = s.open_tab(Dest::from_url(url));
    let anchor = |s: &State| match s.screen() {
        Screen::Page(p) => p.anchor.clone(),
        Screen::Diff(_) => None,
    };
    assert_eq!(anchor(&s).as_deref(), Some("issuecomment-5"));
    let _ = fetched(
        &mut s,
        DataKey::Issue(RepoId::new("o", "r"), 7),
        Data::Issue(None),
    );
    assert_eq!(
        s.route(),
        Some(&Route::pr(PrRef::parse("o/r#7").unwrap())),
        "redirected"
    );
    assert_eq!(anchor(&s).as_deref(), Some("issuecomment-5"));
}

/// A capital hint letter follows the link in a new tab.
#[test]
fn capital_hints_open_a_tab() {
    let mut s = with_repo();
    press(&mut s, "i");
    let label = overlay!(s, Hints).labels[0].label.to_ascii_uppercase();
    press(&mut s, &label);
    assert_eq!(s.tab_count(), 2);
}
