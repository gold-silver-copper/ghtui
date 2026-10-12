//! What used to need the browser: making a release, your gists, a
//! wiki's pages and your subscriptions. Each asks where GitHub would,
//! sends one change, and the page follows GitHub until it shows it.

use crate::api::gist::{FileChange, MadeGist};
use crate::api::release::{Made, NewRelease, TagLook};
use crate::api::watch::{Subscription, SubscriptionState, Watched};
use crate::theme::{ColorDepth, Mode};
use ratatui_textarea::CursorMove;

use super::{failures::page_text, ghtui, render, with_gist, with_issue, with_repo, with_wiki};
use crate::browse::{Data, DataKey};
use crate::fixtures::{self, fetched, press, says};
use crate::route::Route;
use crate::state::{Api, Cmd, Msg, Overlay, State, update};
use crate::writes::{Done, Work};

fn composer(s: &mut State) -> &mut crate::review::Compose {
    match &mut s.overlay {
        Some(Overlay::Compose(c)) => c,
        _ => panic!("no composer"),
    }
}

/// Replaces the composer's text with `text`.
fn write(s: &mut State, text: &str) {
    let c = composer(s);
    c.input.select_all();
    c.input.cut();
    c.input.insert_str(text);
}

fn asked(s: &State) -> &crate::create::Ask {
    match &s.overlay {
        Some(Overlay::Ask(a)) => a,
        _ => panic!("nothing asked"),
    }
}

fn asks_for(cmds: &[Cmd], key: &DataKey) -> bool {
    cmds.iter()
        .any(|c| matches!(c, Cmd::Api(Api::Fetch { key: k, .. }) if k == key))
}

/// `,V` opens a repository's new release on its tag; a new tag is looked
/// for on GitHub, then the composer takes the title and notes. Making it
/// asks whether to publish or save a draft; once made, its page opens
/// and follows GitHub until it shows it.
#[test]
fn a_release_is_made_from_a_new_tag_and_its_page_follows() {
    let mut s = with_repo(Mode::Dark, ColorDepth::TrueColor);
    let cmds = press(&mut s, ",V");
    let repo = ghtui();
    assert_eq!(s.route(), Some(&Route::NewRelease(repo.clone())));
    assert!(asks_for(&cmds, &DataKey::Tags(repo.clone())), "{cmds:?}");
    let tags = Data::Tags(Box::new(fixtures::tags()));
    fetched(&mut s, DataKey::Tags(repo.clone()), tags);
    assert!(
        matches!(&s.overlay, Some(Overlay::Picker(_))),
        "the tag first"
    );
    let cmds = press(&mut s, "v9.0<Enter>");
    let look = Cmd::Write(Work::Release(crate::release::Work::Look(
        repo.clone(),
        "v9.0".into(),
    )));
    assert!(cmds.contains(&look), "{cmds:?}");
    let new_tag = TagLook {
        exists: false,
        release: None,
    };
    let looked = crate::release::Done::Looked(repo.clone(), "v9.0".into(), Ok(new_tag));
    update(&mut s, Msg::Write(Done::Release(looked)));
    write(&mut s, "Nine\n\nWhat changed.");
    press(&mut s, "<Esc>");
    assert_snapshot!(render(&s));
    assert!(page_text(&s).contains("v9.0 is a new tag"));

    press(&mut s, ",s");
    assert_eq!(asked(&s).choices.len(), 2, "publish or save a draft");
    let sent = press(&mut s, "<Enter>");
    let new = NewRelease {
        repo: repo.clone(),
        tag: "v9.0".into(),
        target: None,
        name: "Nine".into(),
        notes: "What changed.".into(),
        draft: false,
        prerelease: false,
        latest: true,
    };
    let create = Cmd::Write(Work::Release(crate::release::Work::Create(new)));
    assert_eq!(sent, [create]);

    let made = Made {
        tag_name: "v9.0".into(),
        draft: false,
        html_url: String::new(),
    };
    let done = crate::release::Done::Created(repo.clone(), Ok(made));
    update(&mut s, Msg::Write(Done::Release(done)));
    let page = Route::Release {
        repo: repo.clone(),
        tag: "v9.0".into(),
    };
    assert_eq!(s.route(), Some(&page));
    says(&s, "Published v9.0");
    assert!(s.busy().is_some_and(|b| b.contains("Waiting for GitHub")));
    let release = Data::Release(Box::new(fixtures::release(true)));
    fetched(&mut s, DataKey::Release(repo, "v9.0".into()), release);
    assert_eq!(s.writing.expect, None, "GitHub shows it");
}

/// A tag that has a release already can't have another: the form says
/// so, and nothing is asked.
#[test]
fn a_tag_with_a_release_is_refused() {
    let mut s = with_repo(Mode::Dark, ColorDepth::TrueColor);
    press(&mut s, ",V");
    press(&mut s, "v0.2.0<Enter>");
    let taken = TagLook {
        exists: true,
        release: Some((false, String::new())),
    };
    let looked = crate::release::Done::Looked(ghtui(), "v0.2.0".into(), Ok(taken));
    update(&mut s, Msg::Write(Done::Release(looked)));
    let sent = press(&mut s, "<C-s>");
    assert_eq!(sent, []);
    assert_eq!(
        composer(&mut s).error.as_deref(),
        Some("v0.2.0 has a release already")
    );
}

/// `,w` on an issue lists GitHub's choices with the current one marked;
/// the one chosen is sent, and the subscription read until it shows it.
#[test]
fn a_thread_is_unsubscribed_and_followed() {
    let mut s = with_issue(Mode::Dark);
    let of = Watched::Thread(ghtui(), 14);
    let cmds = press(&mut s, ",w");
    assert!(
        asks_for(&cmds, &DataKey::Subscription(of.clone())),
        "{cmds:?}"
    );
    let now = Data::Subscription(Box::new(fixtures::subscription()));
    fetched(&mut s, DataKey::Subscription(of.clone()), now);
    let Some(Overlay::Picker(p)) = &s.overlay else {
        panic!("no picker");
    };
    let rows = s.picker_rows(p);
    assert!(rows[0].0.hint.starts_with("current"), "{:?}", rows[0].0);
    let sent = press(&mut s, "Unsub<Enter>");
    let work = crate::watch::Work {
        of: of.clone(),
        id: crate::api::model::NodeId::new("I_sub"),
        state: SubscriptionState::Unsubscribed,
    };
    assert_eq!(sent, [Cmd::Write(Work::Watch(work.clone()))]);
    let cmds = update(
        &mut s,
        Msg::Write(Done::Watch(crate::watch::Done(work, Ok(())))),
    );
    says(&s, "Unsubscribed from gold-silver-copper/ghtui#14");
    assert!(
        asks_for(&cmds, &DataKey::Subscription(of.clone())),
        "{cmds:?}"
    );
    let shown = Subscription {
        state: Some(SubscriptionState::Unsubscribed),
        ..fixtures::subscription()
    };
    fetched(
        &mut s,
        DataKey::Subscription(of),
        Data::Subscription(Box::new(shown)),
    );
    assert_eq!(s.writing.expect, None);
}

/// `,e` on your gist opens it as a form; saving sends only the file that
/// changed, and the gist's page follows.
#[test]
fn your_gist_is_edited_and_its_page_follows() {
    let mut s = with_gist(Mode::Light);
    press(&mut s, ",e");
    let route = Route::EditGist {
        owner: "octocat".into(),
        id: "6cad326836d38bd3a7ae".into(),
    };
    assert_eq!(s.route(), Some(&route));
    let text = page_text(&s);
    assert!(
        text.contains("hello.rs") && text.contains("README.md"),
        "{text}"
    );
    press(&mut s, "c");
    composer(&mut s).input.move_cursor(CursorMove::Bottom);
    composer(&mut s).input.move_cursor(CursorMove::End);
    composer(&mut s).input.insert_str("\n// more");
    let sent = press(&mut s, "<C-s>");
    let [Cmd::Write(Work::Gist(crate::gist::Work::Update(r, change)))] = sent.as_slice() else {
        panic!("{sent:?}");
    };
    assert_eq!(*r, route);
    let text = "fn main() {\n    println!(\"hello\");\n}\n// more";
    assert_eq!(
        change.changed,
        [FileChange {
            was: "hello.rs".into(),
            now: Some(("hello.rs".into(), text.into())),
        }]
    );
    assert!(change.added.is_empty());
    let done = crate::gist::Done::Updated(route, change.clone(), Ok(()));
    update(&mut s, Msg::Write(Done::Gist(done)));
    let page = Route::Gist {
        owner: Some("octocat".into()),
        id: "6cad326836d38bd3a7ae".into(),
    };
    assert_eq!(s.route(), Some(&page), "back on the gist");
    assert_eq!(s.screens.len(), 2, "the form isn't left behind");
    assert!(s.writing.expect.is_some());
}

/// Someone else's gist isn't yours to change: `,e` and `,D` say so.
#[test]
fn someone_elses_gist_says_why_it_cant_be_changed() {
    let mut s = with_gist(Mode::Light);
    s.viewer = Some("hubot".into());
    press(&mut s, ",D");
    says(&s, "It's @octocat's gist: only they can change it");
    assert!(s.overlay.is_none());
}

/// A new gist asks whether it's secret or public, as GitHub's buttons
/// do; deleting asks first, then your gists follow.
#[test]
fn a_gist_is_made_then_deleted() {
    let mut s = with_gist(Mode::Light);
    press(&mut s, ",G");
    assert_eq!(s.route(), Some(&Route::NewGist));
    write(&mut s, "hi");
    press(&mut s, "<C-s>");
    let labels: Vec<&str> = asked(&s).choices.iter().map(|(l, _)| l.as_str()).collect();
    assert_eq!(labels, ["Create secret gist", "Create public gist"]);
    let sent = press(&mut s, "l<Enter>");
    let new = crate::api::gist::NewGist {
        description: String::new(),
        public: true,
        files: vec![("gistfile1.txt".into(), "hi".into())],
    };
    let create = crate::gist::Work::Create(Route::NewGist, new);
    assert_eq!(sent, [Cmd::Write(Work::Gist(create))]);
    let made = MadeGist {
        id: "aa5a315d61ae9438b18d".into(),
        owner: None,
    };
    update(
        &mut s,
        Msg::Write(Done::Gist(crate::gist::Done::Created(
            Route::NewGist,
            Ok(made),
        ))),
    );
    assert_eq!(
        s.route(),
        Some(&Route::Gist {
            owner: Some("octocat".into()),
            id: "aa5a315d61ae9438b18d".into(),
        })
    );

    let mut s = with_gist(Mode::Light);
    press(&mut s, ",D");
    assert_eq!(asked(&s).title, "Delete this gist?");
    let sent = press(&mut s, "<Enter>");
    let delete = crate::gist::Work::Delete {
        id: "6cad326836d38bd3a7ae".into(),
        owner: "octocat".into(),
    };
    assert_eq!(sent, [Cmd::Write(Work::Gist(delete))]);
    let deleted = crate::gist::Done::Deleted {
        id: "6cad326836d38bd3a7ae".into(),
        owner: "octocat".into(),
        result: Ok(()),
    };
    update(&mut s, Msg::Write(Done::Gist(deleted)));
    assert_eq!(s.route(), Some(&Route::Gists("octocat".into())));
}

/// `,e` on a wiki page opens it in the composer; saving commits it on
/// the commit it was read at. A page changed meanwhile says so and keeps
/// the text; once saved, the page it came from follows GitHub.
#[test]
fn a_wiki_page_is_edited_never_over_a_newer_one() {
    let mut s = with_wiki(Mode::Light);
    let cmds = press(&mut s, ",e");
    let route = Route::WikiEdit {
        repo: ghtui(),
        page: Some("Home".into()),
    };
    assert_eq!(s.route(), Some(&route));
    let key = DataKey::Wiki(ghtui(), Some("Home".into()));
    assert!(asks_for(&cmds, &key), "{cmds:?}");
    fetched(&mut s, key, Data::Wiki(Box::new(fixtures::wiki())));
    assert!(composer(&mut s).text().starts_with("Welcome to the wiki."));
    write(&mut s, "Welcome, all.");
    let sent = press(&mut s, "<C-s>");
    let wiki = fixtures::wiki();
    let save = crate::wiki::Save {
        repo: ghtui(),
        title: "Home".into(),
        file: wiki.file.clone(),
        base: wiki.commit,
        text: "Welcome, all.".into(),
        login: Some("octocat".into()),
    };
    assert_eq!(sent, [Cmd::Write(Work::Wiki(route.clone(), save))]);
    let changed = "Home changed on GitHub since you opened it".to_owned();
    update(
        &mut s,
        Msg::Write(Done::Wiki(route.clone(), Err(changed.clone()))),
    );
    let c = composer(&mut s);
    assert!(!c.sending);
    assert_eq!(c.error.as_deref(), Some(changed.as_str()));
    assert_eq!(c.text(), "Welcome, all.");

    update(&mut s, Msg::Write(Done::Wiki(route, Ok("Home".into()))));
    let home = Route::Wiki {
        repo: ghtui(),
        page: None,
    };
    assert_eq!(s.route(), Some(&home), "back on the page");
    says(&s, "Saved Home");
    assert!(s.writing.expect.is_some(), "until GitHub shows the text");
}
