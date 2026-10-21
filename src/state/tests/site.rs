use crate::api::explore::Since;
use crate::api::packages::PackagesOf;

use super::*;
use crate::site::{Page, fixtures};
use crate::ui::pages::InsightsTab;

pub(super) fn insights(tab: InsightsTab) -> State {
    at(
        with_repo(),
        Route::Site(Page::Insights { repo: ghtui(), tab }),
    )
}

pub(super) fn explore() -> State {
    let (language, since) = (Some("rust".into()), Since::Weekly);
    at(state(), Route::Site(Page::Explore { language, since }))
}

pub(super) fn package() -> State {
    let at_ = fixtures::package_ref();
    at(
        state(),
        Route::Site(Page::Package {
            at: at_,
            version: None,
        }),
    )
}

/// Each Insights tab says its numbers beside its bars: the year's
/// commits and their days, the traffic's views and clones, and each
/// manifest's dependencies (dependents link to github.com, which alone
/// has them). At 90 columns nothing is cut without saying so.
#[test]
fn insights_tabs_say_their_numbers() {
    let s = sized(insights(InsightsTab::Commits), 90, 60);
    let text = page_text(&s);
    for part in [
        "Commits in the last 52 weeks  105",
        "Tue      25",
        "Week by week",
    ] {
        assert!(text.contains(part), "{part} in {text}");
    }
    let s = insights(InsightsTab::Traffic);
    let text = page_text(&s);
    for part in [
        "Views  54 · 12 unique visitors",
        "Git clones  7 · 2 unique cloners",
        "news.ycombinator.com",
    ] {
        assert!(text.contains(part), "{part} in {text}");
    }
    let s = insights(InsightsTab::Dependencies);
    let text = page_text(&s);
    for part in [
        "Cargo.lock  2 of 212",
        "ratatui  = 0.30.0  rust",
        "… 210 more on GitHub",
        "see them on github.com ↗",
    ] {
        assert!(text.contains(part), "{part} in {text}");
    }
    // The tab bar has Insights, at the repository's other tabs.
    let tabs: Vec<String> = s
        .chrome()
        .tabs
        .iter()
        .map(|(t, _)| t.label.clone())
        .collect();
    assert_eq!(tabs.last().map(String::as_str), Some("Insights"));
}

/// A topic says what it is and relates others; a list of someone's
/// stars shows its repositories, and their Stars tab their lists.
#[test]
fn topics_and_starred_lists_read_as_on_github() {
    let s = at(state(), Route::Site(Page::Topic("rust".into())));
    let text = page_text(&s);
    for part in [
        "Created by Graydon Hoare · Released July 7, 2010 · ☆ 1.2k stars",
        "#cargo · #webassembly",
    ] {
        assert!(text.contains(part), "{part} in {text}");
    }

    let (login, slug) = ("octocat".into(), "terminal-tools".into());
    let s = at(state(), Route::Site(Page::StarList { login, slug }));
    let text = page_text(&s);
    assert!(text.contains("Terminal tools  2 repositories"), "{text}");
    assert!(text.contains("Repositories  2"), "{text}");

    let s = with_profile(crate::ui::pages::ProfileTab::Stars);
    let text = page_text(&s);
    assert!(text.contains("Lists  2"), "{text}");
    assert!(text.contains("To read  0 repositories  Private"), "{text}");
}

/// `,B` lists your lists, the repository's checked; choosing one puts
/// it in (as GitHub's one mutation sets every list it's in), and your
/// lists are read until GitHub shows it. A name no list has makes one.
#[test]
fn a_repository_is_put_in_a_list_and_followed() {
    let mut s = with_repo();
    let key = DataKey::MyLists(ghtui());
    let cmds = act(&mut s, Action::Lists);
    assert!(crate::fixtures::asks(&cmds, &key), "{cmds:?}");
    crate::fixtures::served(&mut s, &cmds);
    assert_eq!(picked_labels(&s), ["✓ Terminal tools", "  To read"]);
    let sent = press(&mut s, "read<Enter>");
    let work = crate::site::lists::SetLists {
        repo: ghtui(),
        item: NodeId::new("R_ghtui"),
        lists: vec![NodeId::new("UL_terminal-tools"), NodeId::new("UL_to-read")],
        create: None,
        list: "To read".into(),
        add: true,
    };
    assert_eq!(sent, [crate::fixtures::start(work.clone())]);
    let cmds = crate::fixtures::done(&mut s, work, Ok(()));
    says(&s, "Added gold-silver-copper/ghtui to “To read”");
    assert!(crate::fixtures::asks(&cmds, &key), "{cmds:?}");
    assert!(s.following.is_some());
    let mut mine = fixtures::my_lists();
    for l in &mut mine.lists.iter_mut() {
        l.has = Some(true);
    }
    fetched(&mut s, key, Data::from(mine));
    assert!(s.following.is_none());

    // A new name makes the list, and puts the repository in it too.
    act(&mut s, Action::Lists);
    let sent = press(&mut s, "Rust<Enter>");
    let w = crate::fixtures::sent::<crate::site::lists::SetLists>(&sent);
    assert_eq!(w.create.as_deref(), Some("Rust"));
}

/// `,D` on a version asks first, saying what goes with it; the package
/// follows GitHub until the version's gone.
#[test]
fn a_packages_version_is_deleted_after_asking() {
    let mut s = package();
    press(&mut s, "/sha256:3a7b<Enter>");
    press(&mut s, "D");
    let ask = overlay!(s, Ask);
    assert_eq!(ask.title, "Delete version sha256:3a7b55e1 of ghtui?");
    let sent = press(&mut s, "<Enter>");
    let delete = crate::site::packages::DeleteVersion {
        at: fixtures::package_ref(),
        id: 101,
        name: "sha256:3a7b55e1".into(),
    };
    assert_eq!(sent, [crate::fixtures::start(delete.clone())]);
    crate::fixtures::done(&mut s, delete, Ok(()));
    says(&s, "Deleted version sha256:3a7b55e1 of ghtui");
    assert!(s.following.is_some());
    let mut detail = fixtures::package_detail();
    detail.versions.items.retain(|v| v.id != 101);
    let key = DataKey::Package(fixtures::package_ref());
    fetched(&mut s, key, Data::from(detail));
    assert!(s.following.is_none());
}

/// Without a token's read:packages scope, the page says which, and the
/// command that adds it.
#[test]
fn packages_say_what_the_token_lacks() {
    let mut s = state();
    let of = PackagesOf::Owner("octocat".into());
    let _ = s.push(Route::Site(Page::Packages(of.clone())));
    let why = "Packages take your token's read:packages scope, which it lacks: `gh auth refresh -s read:packages` adds it";
    update(
        &mut s,
        Msg::fetched(
            DataKey::Packages(of),
            Err(crate::api::ApiError::Denied(why.into())),
        ),
    );
    let text = page_text(&s);
    assert!(text.contains("gh auth refresh -s read:packages"), "{text}");
    let tabs = s.chrome();
    let active = tabs
        .active
        .and_then(|i| tabs.tabs.get(i))
        .map(|(t, _)| t.label.clone());
    assert_eq!(active.as_deref(), Some("Packages"));
}
