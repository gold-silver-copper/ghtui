//! Writes made from forms and pickers: making a release, your gists, a
//! wiki's pages and your subscriptions. Each asks where GitHub would,
//! sends one change, and the page follows GitHub until it shows it.

use crate::api::gist::{FileChange, MadeGist};
use crate::api::release::{Made, NewRelease, TagLook};
use crate::api::watch::{Subscription, SubscriptionState, Watched};
use crate::route::{gists, releases, wiki};
use ratatui_textarea::CursorMove;

use super::*;
use crate::fixtures::{self};

/// Replaces the composer's text with `text`.
fn write(s: &mut State, text: &str) {
    let c = fixtures::composer(s);
    c.input.select_all();
    c.input.cut();
    c.input.insert_str(text);
}

/// `,V` opens a repository's new release on its tag; a new tag is looked
/// for on GitHub, then the composer takes the title and notes. Making it
/// asks whether to publish or save a draft; once made, its page opens
/// and follows GitHub until it shows it.
#[test]
fn a_release_is_made_from_a_new_tag_and_its_page_follows() {
    let mut s = with_repo();
    let cmds = act(&mut s, Action::NewRelease);
    let repo = ghtui();
    assert_eq!(
        s.route(),
        Some(&Route::Releases(releases::NewRelease(repo.clone())))
    );
    assert!(
        fixtures::asks(&cmds, &DataKey::Tags(repo.clone())),
        "{cmds:?}"
    );
    let tags = Data::from(fixtures::tags());
    fetched(&mut s, DataKey::Tags(repo.clone()), tags);
    assert!(
        matches!(&s.overlay, Some(Overlay::Picker(_))),
        "the tag first"
    );
    let cmds = press(&mut s, "v9.0<Enter>");
    let look = DataKey::TagLook(repo.clone(), "v9.0".into());
    assert!(fixtures::asks(&cmds, &look), "{cmds:?}");
    let new_tag = TagLook {
        exists: false,
        release: None,
    };
    fixtures::fetched(&mut s, look, Data::from(new_tag));
    assert!(
        page_text(&s).contains("v9.0 is a new tag"),
        "the form behind the composer says what GitHub found"
    );
    write(&mut s, "Nine\n\nWhat changed.");
    press(&mut s, "<Esc>");

    press(&mut s, "<C-s>");
    assert_eq!(overlay!(s, Ask).choices.len(), 2, "publish or save a draft");
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
    let route = Route::Releases(releases::NewRelease(repo.clone()));
    let create = crate::release::Save {
        route,
        id: None,
        new,
    };
    assert_eq!(sent, [fixtures::start(create.clone())]);

    let made = Made {
        tag_name: "v9.0".into(),
        draft: false,
        html_url: String::new(),
    };
    fixtures::done(&mut s, create, Ok(made));
    let page = Route::Releases(releases::Release {
        repo: repo.clone(),
        tag: "v9.0".into(),
    });
    assert_eq!(s.route(), Some(&page));
    says(&s, "Published v9.0");
    assert!(s.busy().is_some_and(|b| b.contains("Waiting for GitHub")));
    let release = Data::from(fixtures::release(true));
    fetched(&mut s, DataKey::Release(repo, "v9.0".into()), release);
    assert!(s.following.is_none(), "GitHub shows it");
}

/// A tag that has a release already can't have another: the form says
/// so, and nothing is asked.
#[test]
fn a_tag_with_a_release_is_refused() {
    let mut s = with_repo();
    act(&mut s, Action::NewRelease);
    press(&mut s, "v0.2.0<Enter>");
    let taken = TagLook {
        exists: true,
        release: Some((false, String::new())),
    };
    let look = DataKey::TagLook(ghtui(), "v0.2.0".into());
    fixtures::fetched(&mut s, look, Data::from(taken));
    let sent = press(&mut s, "<C-s>");
    assert_eq!(sent, []);
    assert_eq!(
        fixtures::composer(&mut s).sent.error.as_deref(),
        Some("v0.2.0 has a release already")
    );
}

/// `,w` on an issue lists GitHub's choices with the current one marked;
/// the one chosen is sent, and the subscription read until it shows it.
#[test]
fn a_thread_is_unsubscribed_and_followed() {
    let mut s = with_issue();
    let of = Watched::Thread(ghtui(), 14);
    let cmds = act(&mut s, Action::Watch);
    assert!(
        fixtures::asks(&cmds, &DataKey::Subscription(of.clone())),
        "{cmds:?}"
    );
    let now = Data::from(fixtures::subscription());
    fetched(&mut s, DataKey::Subscription(of.clone()), now);
    let rows = picked(&s);
    assert!(rows[0].1.starts_with("current"), "{:?}", rows[0]);
    let sent = press(&mut s, "Unsub<Enter>");
    let work = crate::watch::Subscribe {
        of: of.clone(),
        id: crate::api::model::NodeId::new("I_sub"),
        state: SubscriptionState::Unsubscribed,
    };
    assert_eq!(sent, [fixtures::start(work.clone())]);
    let cmds = fixtures::done(&mut s, work, Ok(()));
    says(&s, "Unsubscribed from gold-silver-copper/ghtui#14");
    assert!(
        fixtures::asks(&cmds, &DataKey::Subscription(of.clone())),
        "{cmds:?}"
    );
    let shown = Subscription {
        state: Some(SubscriptionState::Unsubscribed),
        ..fixtures::subscription()
    };
    fetched(&mut s, DataKey::Subscription(of), Data::from(shown));
    assert!(s.following.is_none());
}

/// `,e` on your gist opens it as a form; saving sends only the file that
/// changed, and the gist's page follows.
#[test]
fn your_gist_is_edited_and_its_page_follows() {
    let mut s = with_gist();
    press(&mut s, "e");
    let route = Route::Gists(gists::EditGist {
        owner: "octocat".into(),
        id: "6cad326836d38bd3a7ae".into(),
    });
    assert_eq!(s.route(), Some(&route));
    let text = page_text(&s);
    assert!(
        text.contains("hello.rs") && text.contains("README.md"),
        "{text}"
    );
    press(&mut s, "c");
    fixtures::composer(&mut s)
        .input
        .move_cursor(CursorMove::Bottom);
    fixtures::composer(&mut s)
        .input
        .move_cursor(CursorMove::End);
    fixtures::composer(&mut s).input.insert_str("\n// more");
    let sent = press(&mut s, "<C-s>");
    let update = fixtures::sent::<crate::gist::Update>(&sent);
    let change = &update.change;
    assert_eq!(update.route, route);
    let text = "fn main() {\n    println!(\"hello\");\n}\n// more";
    assert_eq!(
        change.changed,
        [FileChange {
            was: "hello.rs".into(),
            now: Some(("hello.rs".into(), text.into())),
        }]
    );
    assert!(change.added.is_empty());
    fixtures::done(&mut s, update, Ok(()));
    let page = Route::Gists(gists::Gist {
        owner: Some("octocat".into()),
        id: "6cad326836d38bd3a7ae".into(),
    });
    assert_eq!(s.route(), Some(&page), "back on the gist");
    assert_eq!(s.screens.len(), 2, "the form isn't left behind");
    assert!(s.following.is_some());
}

/// Someone else's gist isn't yours to change: `,e` and `,D` say so.
#[test]
fn someone_elses_gist_says_why_it_cant_be_changed() {
    let mut s = with_gist();
    s.viewer = Some("hubot".into());
    press(&mut s, "D");
    says(&s, "It's @octocat's gist: only they can change it");
    assert!(s.overlay.is_none());
}

/// A new gist asks whether it's secret or public, as GitHub's buttons
/// do; deleting asks first, then your gists follow.
#[test]
fn a_gist_is_made_then_deleted() {
    let mut s = with_gist();
    act(&mut s, Action::NewGist);
    assert_eq!(s.route(), Some(&Route::Gists(gists::NewGist)));
    write(&mut s, "hi");
    press(&mut s, "<C-s>");
    let labels = ask_labels(&s);
    assert_eq!(labels, ["Create secret gist", "Create public gist"]);
    let sent = press(&mut s, "l<Enter>");
    let new = crate::api::gist::NewGist {
        description: String::new(),
        public: true,
        files: vec![("gistfile1.txt".into(), "hi".into())],
    };
    let create = crate::gist::Create {
        route: Route::Gists(gists::NewGist),
        new,
    };
    assert_eq!(sent, [fixtures::start(create.clone())]);
    let made = MadeGist {
        id: "aa5a315d61ae9438b18d".into(),
        owner: None,
    };
    fixtures::done(&mut s, create, Ok(made));
    assert_eq!(
        s.route(),
        Some(&Route::Gists(gists::Gist {
            owner: Some("octocat".into()),
            id: "aa5a315d61ae9438b18d".into(),
        }))
    );

    let mut s = with_gist();
    press(&mut s, "D");
    assert_eq!(overlay!(s, Ask).title, "Delete this gist?");
    let sent = press(&mut s, "<Enter>");
    let delete = crate::gist::Delete {
        id: "6cad326836d38bd3a7ae".into(),
        owner: "octocat".into(),
    };
    assert_eq!(sent, [fixtures::start(delete.clone())]);
    fixtures::done(&mut s, delete, Ok(()));
    assert_eq!(
        s.route(),
        Some(&Route::Gists(gists::Gists("octocat".into())))
    );
}

/// `,e` on a wiki page opens it in the composer; saving commits it on
/// the commit it was read at. A page changed meanwhile says so and keeps
/// the text; once saved, the page it came from follows GitHub.
#[test]
fn a_wiki_page_is_edited_never_over_a_newer_one() {
    let mut s = with_wiki();
    let cmds = press(&mut s, "e");
    let route = Route::Wiki(wiki::WikiEdit {
        repo: ghtui(),
        page: Some("Home".into()),
    });
    assert_eq!(s.route(), Some(&route));
    let key = DataKey::Wiki(ghtui(), Some("Home".into()));
    assert!(fixtures::asks(&cmds, &key), "{cmds:?}");
    fetched(&mut s, key, Data::from(fixtures::wiki()));
    assert!(
        fixtures::composer(&mut s)
            .text()
            .starts_with("Welcome to the wiki.")
    );
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
    let save = crate::wiki::form::Save { route, save };
    assert_eq!(sent, [fixtures::start(save.clone())]);
    let changed = "Home changed on GitHub since you opened it";
    fixtures::done(&mut s, save.clone(), Err(fixtures::failure(changed)));
    let c = fixtures::composer(&mut s);
    assert!(!c.sent.sending);
    assert_eq!(c.sent.error.as_deref(), Some(changed));
    assert_eq!(c.text(), "Welcome, all.");

    fixtures::done(&mut s, save, Ok("Home".into()));
    let home = Route::Wiki(wiki::Wiki {
        repo: ghtui(),
        page: None,
    });
    assert_eq!(s.route(), Some(&home), "back on the page");
    says(&s, "Saved Home");
    assert!(s.following.is_some(), "until GitHub shows the text");
}
