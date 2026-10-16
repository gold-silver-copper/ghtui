//! Changing a release: editing it, deleting it (then, asked apart, its
//! tag), and its assets: saving one where you say, uploading one, and
//! deleting one. Each asks where GitHub does, sends one change, and the
//! page follows GitHub until it shows it.

use crate::api::release::{Made, NewRelease};
use crate::release::assets::{self, Of};
use crate::release::{Delete, DeleteTag, Save};
use crate::route::releases;
use ratatui_textarea::CursorMove;

use super::*;
use crate::fixtures::{self};
use crate::ui::page::Link;

const ID: u64 = 272_100_001;

fn page() -> Route {
    Route::Releases(releases::Release {
        repo: ghtui(),
        tag: "v0.2.0".into(),
    })
}

fn select_link(s: &mut State, link: &Link) {
    let height = s.page_height();
    let Screen::Page(p) = s.screen_mut() else {
        panic!("not on a page")
    };
    let page = p.page();
    let i = (0..page.items.len())
        .find(|&i| page.target(page.items[i].link) == Some(link))
        .unwrap_or_else(|| panic!("no {link:?} row"));
    p.select(i, height);
}

/// `,e` on a release opens it as the form, filled from it; saving asks
/// whether to update it or make it a draft again, sends the whole of it
/// to the release's ID, and its page follows until it shows the edit.
#[test]
fn a_release_is_edited_and_its_page_follows() {
    let mut s = with_release();
    press(&mut s, "e");
    let form = Route::Releases(releases::EditRelease {
        repo: ghtui(),
        tag: "v0.2.0".into(),
    });
    assert_eq!(s.route(), Some(&form));
    assert!(page_text(&s).contains("Editing v0.2.0"));
    assert!(page_text(&s).contains("Scope names in hunk headers"));
    press(&mut s, "c");
    let Some(Overlay::Compose(c)) = &mut s.overlay else {
        panic!("no composer")
    };
    c.input.move_cursor(CursorMove::Top);
    c.input.move_cursor(CursorMove::End);
    c.input.insert_str(", fixed");
    press(&mut s, "<Esc>");
    press(&mut s, "<C-s>");
    let choices: Vec<&str> = (overlay!(s, Ask).choices.iter())
        .map(|(l, _)| l.as_str())
        .collect();
    assert_eq!(choices, ["Update release", "Make it a draft again"]);
    let sent = fixtures::sent::<Save>(&press(&mut s, "<Enter>"));
    let new = NewRelease {
        repo: ghtui(),
        tag: "v0.2.0".into(),
        target: None,
        name: "ghtui 0.2.0, fixed".into(),
        notes: "## Highlights\n\n- Scope names in hunk headers\n- Commits open in ghtui".into(),
        draft: false,
        prerelease: false,
        latest: true,
    };
    let want = Save {
        route: form,
        id: Some(ID),
        new,
    };
    assert_eq!(sent, want);
    let made = Made {
        tag_name: "v0.2.0".into(),
        draft: false,
        html_url: String::new(),
    };
    fixtures::done(&mut s, sent, Ok(made));
    assert_eq!(s.route(), Some(&page()));
    says(&s, "Updated v0.2.0");
    // GitHub still shows the old title: the page waits.
    let key = DataKey::Release(ghtui(), "v0.2.0".into());
    fetched(
        &mut s,
        key.clone(),
        Data::Release(Box::new(fixtures::release(true))),
    );
    assert!(s.following.is_some());
    let mut edited = fixtures::release(true);
    edited.name = "ghtui 0.2.0, fixed".into();
    fetched(&mut s, key, Data::Release(Box::new(edited)));
    assert!(s.following.is_none(), "GitHub shows it");
}

/// `,D` on a release asks first; once it's deleted, the releases follow
/// until it's gone, and deleting its tag too is asked apart.
#[test]
fn a_release_is_deleted_then_its_tag_asked_about() {
    let mut s = with_release();
    press(&mut s, "D");
    assert_eq!(overlay!(s, Ask).title, "Delete the release ghtui 0.2.0?");
    assert!(
        (overlay!(s, Ask).facts.iter()).any(|(f, _)| f.contains("2 assets go with it")),
        "{:?}",
        overlay!(s, Ask).facts
    );
    let sent = fixtures::sent::<Delete>(&press(&mut s, "<Enter>"));
    let want = Delete {
        repo: ghtui(),
        tag: "v0.2.0".into(),
        id: ID,
    };
    assert_eq!(sent, want);
    fixtures::done(&mut s, sent, Ok(true));
    assert_eq!(
        s.route(),
        Some(&Route::Releases(releases::Releases(ghtui())))
    );
    assert_eq!(overlay!(s, Ask).title, "Delete the tag v0.2.0 as well?");
    let sent = fixtures::sent::<DeleteTag>(&press(&mut s, "<Enter>"));
    let tag = "v0.2.0".to_owned();
    assert_eq!(sent, DeleteTag { repo: ghtui(), tag });
    fixtures::done(&mut s, sent, Ok(()));
    assert!(s.overlay.is_none());
    says(&s, "Deleted the tag v0.2.0");
    // The releases still list it until GitHub drops it.
    fetched(
        &mut s,
        DataKey::Releases(ghtui()),
        Data::Releases(Box::new(fixtures::releases())),
    );
    assert!(s.following.is_some());
    let mut gone = fixtures::releases();
    gone.items.retain(|r| r.tag != "v0.2.0");
    fetched(
        &mut s,
        DataKey::Releases(ghtui()),
        Data::Releases(Box::new(gone)),
    );
    assert!(s.following.is_none());
}

/// Enter on an asset asks where to save it: by its name in the current
/// folder unless told otherwise, and over a file that's there only when
/// the row says it replaces it.
#[test]
fn an_asset_is_saved_where_you_say() {
    let mut s = with_release();
    let name = "ghtui-x86_64-linux.tar.gz";
    let asset = Link::Asset(name.into());
    select_link(&mut s, &asset);
    press(&mut s, "<Enter>");
    let sent = fixtures::sent::<assets::Save>(&press(&mut s, "<Enter>"));
    let of = Of {
        repo: ghtui(),
        tag: "v0.2.0".into(),
        id: ID,
        name: name.into(),
    };
    let save = assets::Save {
        of,
        path: std::path::Path::new(".").join(name),
        replace: false,
    };
    assert_eq!(sent, save);
    select_link(&mut s, &asset);
    press(&mut s, "<Enter>");
    press(&mut s, "Cargo.toml");
    let Some(Overlay::Picker(p)) = &s.overlay else {
        panic!("no picker")
    };
    assert_eq!(s.picker_rows(p)[0].0.label, "Replace Cargo.toml");
    let sent = fixtures::sent::<assets::Save>(&press(&mut s, "<Enter>"));
    assert!(sent.replace);
}

/// `,U` uploads a file from a path, named as it is; the page follows
/// until the release lists it. `,D` on an asset deletes it, after
/// asking, and the page follows until it's gone.
#[test]
fn assets_are_uploaded_and_deleted() {
    let mut s = with_release();
    act(&mut s, Action::UploadAsset);
    press(&mut s, "Cargo.toml");
    let sent = fixtures::sent::<assets::Upload>(&press(&mut s, "<Enter>"));
    let of = Of {
        repo: ghtui(),
        tag: "v0.2.0".into(),
        id: ID,
        name: "Cargo.toml".into(),
    };
    let upload = assets::Upload {
        of,
        path: "Cargo.toml".into(),
    };
    assert_eq!(sent, upload);
    fixtures::done(&mut s, sent, Ok(()));
    says(&s, "Uploaded Cargo.toml");
    assert!(s.following.is_some(), "it follows the upload");
    let mut with = fixtures::release(true);
    let mut toml = with.assets.items[0].clone();
    toml.name = "Cargo.toml".into();
    with.assets.items.push(toml);
    let key = DataKey::Release(ghtui(), "v0.2.0".into());
    fetched(&mut s, key.clone(), Data::Release(Box::new(with)));
    assert!(s.following.is_none());

    let name = "ghtui-aarch64-macos.tar.gz";
    select_link(&mut s, &Link::Asset(name.into()));
    press(&mut s, "D");
    assert_eq!(overlay!(s, Ask).title, format!("Delete {name}?"));
    let sent = fixtures::sent::<assets::Delete>(&press(&mut s, "<Enter>"));
    let of = Of {
        repo: ghtui(),
        tag: "v0.2.0".into(),
        id: ID,
        name: name.into(),
    };
    assert_eq!(sent, assets::Delete(of));
    fixtures::done(&mut s, sent, Ok(()));
    assert!(s.overlay.is_none());
    let mut without = fixtures::release(true);
    without.assets.items.retain(|a| a.name != name);
    fetched(&mut s, key, Data::Release(Box::new(without)));
    assert!(s.following.is_none());
}
