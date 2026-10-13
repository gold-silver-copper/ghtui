use crate::api::explore::Since;
use crate::api::model::NodeId;
use crate::api::packages::PackagesOf;
use crate::theme::{ColorDepth, Mode};

use super::{failures::page_text, ghtui, open, render, sized, state, with_repo};
use crate::browse::{Data, DataKey};
use crate::fixtures::{fetched, press, says};
use crate::route::Route;
use crate::site::{InsightsTab, Key, Page, Value, fixtures, site};
use crate::state::{Api, Cmd, Msg, Overlay, State, update};
use crate::writes::{Done, Work};

fn insights(mode: Mode, tab: InsightsTab) -> State {
    let mut s = with_repo(mode, ColorDepth::TrueColor);
    let route = Route::Site(Page::Insights { repo: ghtui(), tab });
    let key = match tab {
        InsightsTab::Contributors | InsightsTab::Commits => Key::Activity(ghtui()),
        InsightsTab::Traffic => Key::Traffic(ghtui()),
        InsightsTab::Dependencies => Key::Dependencies(ghtui()),
    };
    open(&mut s, route, fixtures::sample(&key));
    s
}

fn explore(mode: Mode) -> State {
    let mut s = state(mode, ColorDepth::TrueColor);
    let page = Page::Explore {
        language: Some("rust".into()),
        since: Since::Weekly,
    };
    let Some(key) = page.paged() else {
        panic!("Explore lists repositories");
    };
    let _ = s.push(Route::Site(page));
    let repos = Data::Search(Box::new(crate::fixtures::repo_results()));
    fetched(&mut s, key, repos);
    fetched(
        &mut s,
        site(Key::Featured),
        fixtures::sample(&Key::Featured),
    );
    s
}

fn package(mode: Mode) -> State {
    let mut s = state(mode, ColorDepth::TrueColor);
    let route = Route::Site(Page::Package {
        at: fixtures::package_ref(),
        version: None,
    });
    open(
        &mut s,
        route,
        fixtures::sample(&Key::Package(fixtures::package_ref())),
    );
    s
}

fn asks_for(cmds: &[Cmd], key: &DataKey) -> bool {
    cmds.iter()
        .any(|c| matches!(c, Cmd::Api(Api::Fetch { key: k, .. }) if k == key))
}

#[test]
fn insights_contributors_dark() {
    assert_snapshot!(render(&insights(Mode::Dark, InsightsTab::Contributors)));
}

#[test]
fn explore_light() {
    assert_snapshot!(render(&explore(Mode::Light)));
}

#[test]
fn package_dark() {
    assert_snapshot!(render(&package(Mode::Dark)));
}

/// Each Insights tab says its numbers beside its bars: the year's
/// commits and their days, the traffic's views and clones, and each
/// manifest's dependencies (dependents link to github.com, which alone
/// has them). At 90 columns nothing is cut without saying so.
#[test]
fn insights_tabs_say_their_numbers() {
    let s = sized(insights(Mode::Dark, InsightsTab::Commits), 90, 60);
    let text = page_text(&s);
    for part in [
        "Commits in the last 52 weeks  105",
        "Tue      25",
        "Week by week",
    ] {
        assert!(text.contains(part), "{part} in {text}");
    }
    let s = insights(Mode::Dark, InsightsTab::Traffic);
    let text = page_text(&s);
    for part in [
        "Views  54 · 12 unique visitors",
        "Git clones  7 · 2 unique cloners",
        "news.ycombinator.com",
    ] {
        assert!(text.contains(part), "{part} in {text}");
    }
    let s = insights(Mode::Dark, InsightsTab::Dependencies);
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
    let mut s = state(Mode::Dark, ColorDepth::TrueColor);
    let page = Page::Topic("rust".into());
    let _ = s.push(Route::Site(page.clone()));
    fetched(
        &mut s,
        site(Key::Topic("rust".into())),
        fixtures::sample(&Key::Topic("rust".into())),
    );
    let Some(key) = page.paged() else {
        panic!("a topic lists repositories");
    };
    fetched(
        &mut s,
        key,
        Data::Search(Box::new(crate::fixtures::repo_results())),
    );
    let text = page_text(&s);
    for part in [
        "Created by Graydon Hoare · Released July 7, 2010 · ☆ 1.2k stars",
        "#cargo · #webassembly",
    ] {
        assert!(text.contains(part), "{part} in {text}");
    }

    let mut s = state(Mode::Dark, ColorDepth::TrueColor);
    let route = Route::Site(Page::StarList {
        login: "octocat".into(),
        slug: "terminal-tools".into(),
    });
    let key = Key::StarList("octocat".into(), "terminal-tools".into());
    open(&mut s, route, fixtures::sample(&key));
    let text = page_text(&s);
    assert!(text.contains("Terminal tools  2 repositories"), "{text}");
    assert!(text.contains("Repositories  2"), "{text}");

    let mut s = state(Mode::Dark, ColorDepth::TrueColor);
    let _ = s.push(Route::User {
        login: "octocat".into(),
        tab: crate::ui::pages::ProfileTab::Stars,
    });
    let profile = Data::Profile(Box::new(crate::fixtures::profile()));
    fetched(&mut s, DataKey::Profile("octocat".into()), profile);
    let lists = Key::StarLists("octocat".into());
    fetched(&mut s, site(lists.clone()), fixtures::sample(&lists));
    let text = page_text(&s);
    assert!(text.contains("Lists  2"), "{text}");
    assert!(text.contains("To read  0 repositories  Private"), "{text}");
}

/// `,L` lists your lists, the repository's checked; choosing one puts
/// it in (as GitHub's one mutation sets every list it's in), and your
/// lists are read until GitHub shows it. A name no list has makes one.
#[test]
fn a_repository_is_put_in_a_list_and_followed() {
    let mut s = with_repo(Mode::Dark, ColorDepth::TrueColor);
    let key = site(Key::MyLists(ghtui()));
    let cmds = press(&mut s, ",L");
    assert!(asks_for(&cmds, &key), "{cmds:?}");
    fetched(
        &mut s,
        key.clone(),
        fixtures::sample(&Key::MyLists(ghtui())),
    );
    let Some(Overlay::Picker(p)) = &s.overlay else {
        panic!("no picker");
    };
    let labels: Vec<String> = s.picker_rows(p).into_iter().map(|(r, _)| r.label).collect();
    assert_eq!(labels, ["✓ Terminal tools", "  To read"]);
    let sent = press(&mut s, "read<Enter>");
    let work = crate::site::lists::Work {
        repo: ghtui(),
        item: NodeId::new("R_ghtui"),
        lists: vec![NodeId::new("UL_terminal-tools"), NodeId::new("UL_to-read")],
        create: None,
        list: "To read".into(),
        add: true,
    };
    let lists = crate::site::Work::Lists(work.clone());
    assert_eq!(sent, [Cmd::Write(Work::Site(lists))]);
    let done = crate::site::Done::Lists(work, Ok(()));
    let cmds = update(&mut s, Msg::Write(Done::Site(done)));
    says(&s, "Added gold-silver-copper/ghtui to “To read”");
    assert!(asks_for(&cmds, &key), "{cmds:?}");
    assert!(s.writing.expect.is_some());
    let mut mine = fixtures::my_lists();
    for l in &mut mine.lists.iter_mut() {
        l.has = Some(true);
    }
    fetched(&mut s, key, Data::Site(Box::new(Value::MyLists(mine))));
    assert_eq!(s.writing.expect, None);

    // A new name makes the list, and puts the repository in it too.
    press(&mut s, ",L");
    let sent = press(&mut s, "Rust<Enter>");
    let [Cmd::Write(Work::Site(crate::site::Work::Lists(w)))] = sent.as_slice() else {
        panic!("{sent:?}");
    };
    assert_eq!(w.create.as_deref(), Some("Rust"));
}

/// `,D` on a version asks first, saying what goes with it; the package
/// follows GitHub until the version's gone.
#[test]
fn a_packages_version_is_deleted_after_asking() {
    let mut s = package(Mode::Dark);
    press(&mut s, "/sha256:3a7b<Enter>");
    press(&mut s, ",D");
    let Some(Overlay::Ask(ask)) = &s.overlay else {
        panic!("no question");
    };
    assert_eq!(ask.title, "Delete version sha256:3a7b55e1 of ghtui?");
    let sent = press(&mut s, "<Enter>");
    let delete = crate::site::packages::Delete {
        at: fixtures::package_ref(),
        id: 101,
        name: "sha256:3a7b55e1".into(),
    };
    let work = crate::site::Work::DeleteVersion(delete.clone());
    assert_eq!(sent, [Cmd::Write(Work::Site(work))]);
    let done = crate::site::Done::DeleteVersion(delete, Ok(()));
    update(&mut s, Msg::Write(Done::Site(done)));
    says(&s, "Deleted version sha256:3a7b55e1 of ghtui");
    assert!(s.writing.expect.is_some());
    let mut detail = fixtures::package_detail();
    detail.versions.items.retain(|v| v.id != 101);
    let key = site(Key::Package(fixtures::package_ref()));
    fetched(&mut s, key, Data::Site(Box::new(Value::Package(detail))));
    assert_eq!(s.writing.expect, None);
}

/// Without a token's read:packages scope, the page says which, and the
/// command that adds it.
#[test]
fn packages_say_what_the_token_lacks() {
    let mut s = state(Mode::Dark, ColorDepth::TrueColor);
    let of = PackagesOf::Owner("octocat".into());
    let _ = s.push(Route::Site(Page::Packages(of.clone())));
    let why = "Packages take your token's read:packages scope, which it lacks: `gh auth refresh -s read:packages` adds it";
    update(
        &mut s,
        Msg::Fetched {
            key: site(Key::Packages(of)),
            result: Err(crate::api::ApiError::Denied(why.into())),
            cached_at: None,
        },
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
