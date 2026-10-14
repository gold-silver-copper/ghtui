//! Following links: by keys, letters, comment links and the mouse.

use super::*;
use crate::route::{code, issues, pulls, search};

#[test]
fn browser_and_copy_use_the_selection_or_the_page() {
    let mut state = with_prs(1);
    assert_eq!(
        press(&mut state, "o"),
        vec![Cmd::OpenUrl("https://github.com/o/r/pull/1".into())]
    );
    assert_eq!(
        press(&mut state, "y"),
        vec![Cmd::Copy("https://github.com/o/r/pull/1".into())]
    );
    let _ = state.push(issues("is:closed"));
    assert_eq!(
        press(&mut state, "o"),
        vec![Cmd::OpenUrl(
            "https://github.com/gold-silver-copper/ghtui/issues?q=is:closed".into()
        )]
    );
}

/// A link to a comment opens its conversation scrolled to it.
#[test]
fn comment_links_scroll_to_the_comment() {
    let mut s = state();
    s.size = (100, 12);
    let comment = "https://github.com/cli/cli/discussions/14603#discussioncomment-18765100";
    let _ = s.follow(&crate::ui::page::Link::Url(comment.into()));
    let of = crate::api::browse::DiscussionsOf::Repo(RepoId::new("cli", "cli"));
    let detail = Data::Discussion(Box::new(crate::fixtures::discussion()));
    fetched(&mut s, DataKey::Discussion(of, 14603), detail);
    let Screen::Page(p) = s.screen() else {
        panic!()
    };
    let line = p.page().anchors["discussioncomment-18765100"];
    assert!(
        p.scroll > 0 && p.scroll <= line && line < p.scroll + 10,
        "{} vs {line}",
        p.scroll
    );

    // Issue comments too.
    let mut s = with_repo();
    s.size = (100, 12);
    let url = format!(
        "https://github.com/{}/issues/14#issuecomment-1000001",
        ghtui()
    );
    let _ = s.follow(&crate::ui::page::Link::Url(url));
    let issue = Data::Issue(Some(Box::new(crate::fixtures::issue())));
    fetched(&mut s, DataKey::Issue(ghtui(), 14), issue);
    let Screen::Page(p) = s.screen() else {
        panic!()
    };
    assert!(p.page().anchors.contains_key("issuecomment-1000001"));
    assert!(p.scroll > 0);
}

/// A comment's link opened on a cached issue whose body has grown
/// since: the fresh copy moves the comment, and the page follows it,
/// unless you've scrolled away by then.
#[test]
fn a_comment_link_follows_its_comment_when_the_issue_refreshes() {
    let open = |moved: &dyn Fn(&mut State)| {
        let mut s = with_repo();
        s.size = (100, 12);
        let url = format!(
            "https://github.com/{}/issues/14#issuecomment-1000001",
            ghtui()
        );
        let _ = s.follow(&crate::ui::page::Link::Url(url));
        let key = DataKey::Issue(ghtui(), 14);
        let cached = Data::Issue(Some(Box::new(crate::fixtures::issue())));
        let _ = update(
            &mut s,
            Msg::Fetched {
                key: key.clone(),
                result: Ok(cached),
                cached_at: Some(0),
            },
        );
        moved(&mut s);
        let shown = |s: &State| {
            let p = page(s);
            let lines = p.page().lines.iter().skip(p.scroll).take(10);
            (
                p.scroll,
                lines
                    .map(crate::ui::page::PageLine::text)
                    .collect::<Vec<_>>(),
            )
        };
        let at = shown(&s);
        let mut issue = crate::fixtures::issue();
        issue.body.push_str(&"\n\nMore.".repeat(20));
        fetched(&mut s, key, Data::Issue(Some(Box::new(issue))));
        (
            at,
            shown(&s),
            page(&s).page().anchors["issuecomment-1000001"],
        )
    };
    let ((before, _), (after, _), line) = open(&|_| {});
    assert!(after > before, "{before} -> {after}");
    assert!(after <= line && line < after + 10, "{after} vs {line}");
    let ((before, _), (after, _), _) = open(&|s| drop(press(s, "gg")));
    assert_eq!(after, before, "yanked back after scrolling");
    // Selecting an item on screen, as j can, is moving too.
    let select = |s: &mut State| {
        let Screen::Page(p) = s.screen_mut() else {
            panic!("not on a page")
        };
        let shown = p.page().items.iter().position(|i| i.end > p.scroll);
        assert!(shown.is_some_and(|i| p.page().items[i].start < p.scroll + 10));
        p.selected = shown;
    };
    let ((_, before), (_, after), _) = open(&select);
    assert_eq!(after, before, "yanked back after selecting");
}

#[test]
fn letter_hints_follow_links() {
    let mut state = with_repo();
    press(&mut state, "i");
    let hints = overlay!(state, Hints);
    let i = hints
        .links
        .iter()
        .position(|u| u.url() == Some("https://github.com/gold-silver-copper"))
        .expect("the owner is a link");
    let label = hints.labels[i].label.clone();
    press(&mut state, &label);
    assert!(state.overlay.is_none());
    assert_eq!(route(&state), Route::user("gold-silver-copper"));
    // A wrong letter cancels.
    press(&mut state, "<Esc>l");
    press(&mut state, "<Esc>");
    assert!(state.overlay.is_none());
}

#[test]
fn the_mouse_follows_links_switches_tabs_and_scrolls() {
    let mut state = with_repo();
    let (x, y) = find(&state, "crates");
    let _ = click(&mut state, x, y);
    assert!(
        matches!(route(&state), Route::Code(code::Tree { .. })),
        "a click follows a link"
    );
    let _ = state.back();
    // Tabs.
    let lay = state.layout();
    let tabs: Vec<_> = state.chrome().tabs.into_iter().map(|(t, _)| t).collect();
    let rects = crate::ui::chrome::tab_layout(lay.tabs.unwrap(), &tabs, state.chrome().active);
    let _ = click(&mut state, rects[2].x + 1, rects[2].y);
    assert!(matches!(route(&state), Route::Pulls(pulls::Pulls { .. })));
    // The header's search field.
    let field = crate::ui::chrome::header_layout(lay.header, &[], &[]).search;
    let chrome = state.chrome();
    let crumbs: Vec<_> = chrome.crumbs.iter().map(|(c, _)| c.clone()).collect();
    let right: Vec<_> = chrome.right.iter().map(|(t, _)| t.clone()).collect();
    let field2 = crate::ui::chrome::header_layout(lay.header, &crumbs, &right).search;
    assert_eq!(field.y, field2.y);
    let _ = click(&mut state, field2.x + 1, field2.y);
    assert!(matches!(state.overlay, Some(Overlay::Search(_))));
    let bottom = state.size.1 - 1;
    let _ = click(&mut state, 0, bottom);
    assert!(state.overlay.is_none(), "a click outside closes it");
    // The wheel.
    press(&mut state, "1gt");
    update(&mut state, Msg::Resize(100, 16));
    let before = page(&state).scroll;
    update(
        &mut state,
        Msg::Mouse(MouseEvent {
            kind: crossterm::event::MouseEventKind::ScrollDown,
            column: 10,
            row: 10,
            modifiers: KeyModifiers::NONE,
        }),
    );
    assert_eq!(page(&state).scroll, before + 3);
}

/// The header counts your unread notifications as GitHub does, polled
/// when GitHub asks (an older poll's timer lets go), and hidden when it
/// can't be asked; clicked or followed by its letters, it opens the
/// inbox.
#[test]
fn the_header_counts_the_unread_and_opens_the_inbox() {
    use crate::api::notifications::Unread;
    let mut state = with_repo();
    let inbox = Route::Search(search::Notifications {
        query: String::new(),
    });
    let header =
        |s: &State| -> Vec<String> { s.chrome().right.into_iter().map(|(t, _)| t).collect() };
    let polled = |count, more, poll_secs| {
        Msg::Unread(Ok(Unread {
            count,
            more,
            poll_secs,
        }))
    };
    let cmds = update(&mut state, polled(3, false, 60));
    assert_eq!(cmds, vec![Cmd::Timer(Timer::Unread(1), 60_000)]);
    assert!(header(&state).contains(&"● 3 unread".to_owned()));
    let poll = vec![Cmd::Api(Api::FetchUnread)];
    assert_eq!(update(&mut state, Msg::Timer(Timer::Unread(1))), poll);
    // Asked again after a change: the earlier timer lets go.
    let cmds = update(&mut state, polled(50, true, 120));
    assert_eq!(cmds, vec![Cmd::Timer(Timer::Unread(2), 120_000)]);
    assert!(update(&mut state, Msg::Timer(Timer::Unread(1))).is_empty());
    assert!(header(&state).contains(&"● 50+ unread".to_owned()));

    // Clicked.
    let lay = state.layout();
    let chrome = state.chrome();
    let crumbs: Vec<_> = chrome.crumbs.iter().map(|(c, _)| c.clone()).collect();
    let rects = crate::ui::chrome::header_layout(lay.header, &crumbs, &header(&state)).right;
    let _ = click(&mut state, rects[0].x + 1, rects[0].y);
    assert_eq!(route(&state), inbox);

    // Followed by its letters.
    let _ = state.back();
    press(&mut state, "i");
    let Some(Overlay::Hints(hints)) = &state.overlay else {
        panic!("no hints");
    };
    let label = (hints.labels.iter())
        .find(|h| (h.x, h.y) == (rects[0].x, rects[0].y))
        .map(|h| h.label.clone())
        .unwrap();
    press(&mut state, &label);
    assert_eq!(route(&state), inbox);

    // None unread, or a poll that failed: no count.
    update(&mut state, polled(0, false, 60));
    assert_eq!(header(&state), ["@octocat"]);
    update(&mut state, polled(2, false, 60));
    let failed = Msg::Unread(Err(ApiError::Network("down".into())));
    assert_eq!(
        update(&mut state, failed),
        vec![Cmd::Timer(Timer::Unread(5), 60_000)]
    );
    assert_eq!(header(&state), ["@octocat"]);
}

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
    let milestone = |repo: &str| {
        Route::Issues(issues::Milestone {
            repo: crate::api::model::RepoId::parse(repo).unwrap(),
            number: 3,
        })
    };
    let issue = super::sized(with_issue(), 160, 40);
    assert!(routes(&issue).contains(&milestone("gold-silver-copper/ghtui")));
    let pr = super::sized(with_pr(), 160, 40);
    let Screen::Page(p) = pr.screen() else {
        panic!("not a page");
    };
    let Route::Pulls(pulls::Pr { pr: pr_ref, .. }) = &p.route else {
        panic!("not a pull request");
    };
    assert!(routes(&pr).contains(&milestone(&pr_ref.repo.to_string())));
    let repo = with_repo();
    let repo_routes = routes(&repo);
    let id = super::ghtui();
    assert!(repo_routes.contains(&Route::Code(code::Branches(id.clone()))));
    assert!(repo_routes.contains(&Route::Code(code::Tags(id))));
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
