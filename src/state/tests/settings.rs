use crate::api::settings::{Enforcement, Permission, Settings, SettingsChange, SettingsPage};
use crate::route::settings;

use super::*;
use crate::act::{By, Subject};
use crate::settings::fixtures::page;
use crate::ui::page::Link;
use crate::ui::pages::SettingLink;

/// A settings page of a repository you administer, with its data in.
pub(super) fn on(at: SettingsPage) -> State {
    let mut s = sized(state(), 100, 40);
    let _ = s.push(Route::Settings(settings::Page {
        repo: ghtui(),
        page: at,
    }));
    let overview = crate::api::browse::RepoOverview {
        can_admin: true,
        ..fixtures::overview()
    };
    fetched(&mut s, DataKey::Repo(ghtui()), Data::from(overview));
    deliver(&mut s, at, page(at));
    s
}

fn deliver(s: &mut State, at: SettingsPage, data: Settings) -> Cmds {
    fetched(s, DataKey::Settings(ghtui(), at), Data::from(data))
}

/// The one change `cmds` sends.
fn sent(cmds: &[Cmd]) -> (&SettingsChange, &By) {
    match fixtures::sent_change(cmds) {
        (Change::Settings(c), by) => (c, by),
        _ => panic!("{cmds:?}"),
    }
}

/// What the confirmation says, its facts one a line.
fn asked(s: &State) -> String {
    let c = overlay!(s, Ask);
    let facts: Vec<&str> = c.facts.iter().map(|(f, _)| f.as_str()).collect();
    format!("{}\n{}", c.title, facts.join("\n"))
}

/// Selects the first row that `is`.
fn select(s: &mut State, is: impl Fn(&SettingLink) -> bool) {
    fixtures::select_by(s, |l| matches!(l, Link::Setting(l) if is(l)));
}

/// Writes `text` in the open composer, in place of what's there.
fn write(s: &mut State, text: &str) {
    let c = fixtures::composer(s);
    let target = c.target.clone();
    let _ = s.compose(target, text);
}

fn composed(s: &State) -> String {
    match &s.overlay {
        Some(Overlay::Compose(c)) => c.text(),
        _ => panic!("no composer"),
    }
}

fn about(at: SettingsPage) -> Subject {
    Subject::Settings(ghtui(), at)
}

/// The Settings tab shows only to admins, as on GitHub, and its menu
/// leads to each page.
#[test]
fn the_settings_tab_is_for_admins() {
    let s = on(SettingsPage::General);
    let tabs: Vec<String> = s
        .chrome()
        .tabs
        .iter()
        .map(|(t, _)| t.label.clone())
        .collect();
    assert_eq!(tabs.last().map(String::as_str), Some("Settings"));
    let text = page_text(&s);
    assert!(
        text.contains("General · Collaborators · Branches · Webhooks · Pages · Deploy keys · Secrets · Variables"),
        "{text}"
    );
    assert!(text.contains("never deletes a repository"), "{text}");
    let mut repo = super::with_repo();
    let tabs = repo.chrome().tabs;
    assert!(tabs.iter().all(|(t, _)| t.label != "Settings"));
    press(&mut repo, "x");
    says(&repo, "Closing works on");
}

/// `,e` writes a rule's form in the composer; what changed is named
/// before it's sent, only that is sent, and the page follows GitHub
/// until it shows it.
#[test]
fn a_rule_is_edited_as_a_form_and_named_before_saving() {
    let mut s = on(SettingsPage::Branches);
    select(&mut s, |l| matches!(l, SettingLink::Rule(_)));
    press(&mut s, "e");
    let text = composed(&s);
    assert!(text.contains("Required approvals: 1"), "{text}");
    write(
        &mut s,
        &text.replace("Required approvals: 1", "Required approvals: 2"),
    );
    press(&mut s, "<C-s>");
    assert_eq!(
        asked(&s),
        "Change the protection rule for main?\nRequired approvals: 1 → 2"
    );
    let cmds = press(&mut s, "<Enter>");
    let (change, by) = sent(&cmds);
    let SettingsChange::UpdateRule { id, to, .. } = change else {
        panic!("{change:?}")
    };
    assert_eq!(id, &NodeId::new("BPR_1"));
    assert_eq!(by, &By::Confirm(about(SettingsPage::Branches)));
    let to = (**to).clone();
    answer(&mut s, &cmds, Ok(()));
    assert!(s.following.is_some(), "it waits for GitHub to show it");
    let mut shown = crate::settings::fixtures::rule();
    shown.form = to;
    let Settings::Branches(mut b) = page(SettingsPage::Branches) else { panic!() };
    b.rules = vec![shown].into();
    deliver(&mut s, SettingsPage::Branches, Settings::Branches(b));
    assert!(s.following.is_none());
    assert!(page_text(&s).contains("2 approvals"));
}

/// A new rule comes from an empty form; one GitHub refuses opens again
/// with its text, saying why. Deleting asks first.
#[test]
fn a_new_rule_refused_comes_back_to_edit() {
    let mut s = on(SettingsPage::Branches);
    press(&mut s, "a");
    let text = composed(&s).replace("Branch name pattern: ", "Branch name pattern: release/*");
    write(&mut s, &text);
    press(&mut s, "<C-s>");
    assert!(asked(&s).starts_with("Protect branches matching release/*?"));
    let cmds = press(&mut s, "<Enter>");
    assert!(matches!(sent(&cmds).0, SettingsChange::CreateRule { .. }));
    let refused = crate::api::ApiError::GraphQl(vec!["Name already protected: release/*".into()]);
    answer(&mut s, &cmds, Err(refused));
    let c = fixtures::composer(&mut s);
    assert!(c.text().contains("release/*"));
    assert!(
        c.sent
            .error
            .as_deref()
            .is_some_and(|e| e.contains("already protected"))
    );

    let mut s = on(SettingsPage::Branches);
    select(&mut s, |l| matches!(l, SettingLink::Rule(_)));
    press(&mut s, "D");
    assert!(asked(&s).contains(
        "Delete the protection rule for main?\nBranches matching main are then unprotected"
    ));
}

/// `,x` turns a ruleset off (or on) after asking; an organization's is
/// the organization's.
#[test]
fn a_ruleset_is_turned_off_after_asking() {
    let mut s = on(SettingsPage::Branches);
    select(&mut s, |l| *l == SettingLink::Ruleset(7));
    press(&mut s, "x");
    assert!(asked(&s).starts_with("Disable ruleset “Tags are kept”?"));
    let cmds = press(&mut s, "<Enter>");
    assert!(matches!(
        sent(&cmds).0,
        SettingsChange::SetRuleset {
            id: 7,
            enforcement: Enforcement::Disabled,
            ..
        }
    ));
    let mut s = on(SettingsPage::Branches);
    select(&mut s, |l| *l == SettingLink::Ruleset(8));
    press(&mut s, "x");
    says(&s, "It's octo-org's ruleset");
}

/// Inviting: a login, then a role, then the question naming both.
/// A role changes the same way; the owner's can't.
#[test]
fn a_collaborator_is_invited_with_a_role() {
    let mut s = on(SettingsPage::Access);
    press(&mut s, "a");
    press(&mut s, "hubot2<Enter>");
    press(&mut s, "write<Enter>");
    assert!(asked(&s).starts_with("Invite @hubot2 to gold-silver-copper/ghtui with Write access?"));
    let cmds = press(&mut s, "<Enter>");
    assert_eq!(
        sent(&cmds).0,
        &SettingsChange::SetCollaborator {
            repo: ghtui(),
            login: "hubot2".into(),
            permission: Permission::Write,
            invite: true,
        }
    );

    let mut s = on(SettingsPage::Access);
    select(&mut s, |l| *l == SettingLink::Person("octocat".into()));
    press(&mut s, "e");
    press(&mut s, "maintain<Enter>");
    assert!(asked(&s).contains("From Write to Maintain"));
    press(&mut s, "<Esc>");
    press(&mut s, "D");
    assert!(asked(&s).starts_with("Remove @octocat from gold-silver-copper/ghtui?"));
    press(&mut s, "<Esc>");
    select(&mut s, |l| {
        *l == SettingLink::Person("gold-silver-copper".into())
    });
    press(&mut s, "D");
    says(&s, "owns gold-silver-copper/ghtui");
}

/// A webhook from the form; on its page, `,x` disables it and `,R`
/// redelivers a delivery, each after asking; deleting it there goes
/// back to the list.
#[test]
fn webhooks_are_added_disabled_redelivered_and_deleted() {
    let mut s = on(SettingsPage::Hooks);
    press(&mut s, "a");
    let text = composed(&s).replace("Payload URL: ", "Payload URL: https://example.com/new");
    write(&mut s, &text);
    press(&mut s, "<C-s>");
    assert!(asked(&s).starts_with(
        "Add a webhook to gold-silver-copper/ghtui?\nPayload URL: https://example.com/new"
    ));
    let cmds = press(&mut s, "<Enter>");
    assert!(matches!(sent(&cmds).0, SettingsChange::CreateHook { .. }));

    let mut s = on(SettingsPage::Hook(11));
    press(&mut s, "x");
    assert!(asked(&s).starts_with("Disable this webhook?\nhttps://example.com/hook/11"));
    let cmds = press(&mut s, "<Enter>");
    let SettingsChange::UpdateHook { to, .. } = sent(&cmds).0 else { panic!() };
    assert!(!to.active);
    press(&mut s, "<Esc>");
    select(&mut s, |l| *l == SettingLink::Delivery(902));
    act(&mut s, Action::Redeliver);
    assert!(asked(&s).starts_with("Redeliver the pull_request delivery?"));
    let cmds = press(&mut s, "<Enter>");
    assert!(matches!(
        sent(&cmds).0,
        SettingsChange::Redeliver { delivery: 902, .. }
    ));

    let mut s = on(SettingsPage::Hook(11));
    press(&mut s, "D");
    let cmds = press(&mut s, "<Enter>");
    assert_eq!(sent(&cmds).1, &By::Confirm(about(SettingsPage::Hooks)));
    answer(&mut s, &cmds, Ok(()));
    assert_eq!(
        s.route(),
        Some(&Route::Settings(settings::Page {
            repo: ghtui(),
            page: SettingsPage::Hooks
        }))
    );
}

/// A new webhook's form starts on its first field: what's typed is its
/// payload URL.
#[test]
fn a_new_webhooks_form_starts_on_its_payload_url() {
    let mut s = on(SettingsPage::Hooks);
    press(&mut s, "a");
    press(&mut s, "https://example.com/new");
    let text = composed(&s);
    assert_eq!(
        text.lines().next(),
        Some("Payload URL: https://example.com/new"),
        "{text}"
    );
}

/// A deploy key with write access says so before it's added; a key
/// can't be edited, only deleted.
#[test]
fn a_deploy_key_with_write_access_says_so() {
    let mut s = on(SettingsPage::Keys);
    press(&mut s, "a");
    write(
        &mut s,
        "Title: deploy\nAllow write access: yes\nKey: ssh-ed25519 AAAAC3Nz me@x",
    );
    press(&mut s, "<C-s>");
    let c = overlay!(s, Ask);
    assert_eq!(
        c.title,
        "Add deploy key “deploy” to gold-silver-copper/ghtui?"
    );
    assert!(c.facts.contains(&(
        "With write access: it can push to gold-silver-copper/ghtui".into(),
        true
    )));
    press(&mut s, "<Esc>");
    select(&mut s, |l| *l == SettingLink::Key(41));
    press(&mut s, "e");
    says(&s, "delete it and add it again");
    press(&mut s, "D");
    assert!(asked(&s).starts_with("Delete deploy key “CI”?"));
}

/// Archiving asks twice: for the repository's name, then once more.
/// Deleting it is github.com's.
#[test]
fn archiving_asks_twice() {
    let mut s = on(SettingsPage::General);
    press(&mut s, "x");
    press(&mut s, "ghtui");
    assert!(press(&mut s, "<Enter>").is_empty());
    assert!(
        !matches!(s.overlay, Some(Overlay::Ask(_))),
        "the wrong name asks nothing"
    );
    press(&mut s, "<Esc>,x");
    press(&mut s, "gold-silver-copper/ghtui<Enter>");
    assert!(asked(&s).starts_with("Archive gold-silver-copper/ghtui?\nThe last ask"));
    let cmds = press(&mut s, "<Enter>");
    assert_eq!(
        sent(&cmds).0,
        &SettingsChange::Archive {
            repo: ghtui(),
            archived: true
        }
    );
    answer(&mut s, &cmds, Ok(()));
    let Settings::General(mut g) = page(SettingsPage::General) else { panic!() };
    g.archived = true;
    deliver(&mut s, SettingsPage::General, Settings::General(g));
    assert!(s.following.is_none());
    assert!(page_text(&s).contains("Unarchive this repository"));
    press(&mut s, "D");
    says(&s, "ghtui never deletes one");
}

/// A new secret is written in the form, named in the question, and sent
/// sealed: its value is nowhere in what's sent. The page follows GitHub
/// until it lists it.
#[test]
fn a_secret_is_sent_sealed() {
    use crate::api::settings::ActionsChange;
    let mut s = on(SettingsPage::Secrets);
    let text = page_text(&s);
    assert!(text.contains("Environment secrets · production"), "{text}");
    assert!(text.contains("ORG_TOKEN"), "{text}");
    press(&mut s, "a");
    write(&mut s, "Name: new_token\nValue: hunter2\nline two");
    press(&mut s, "<C-s>");
    assert!(asked(&s).starts_with("Set secret NEW_TOKEN in gold-silver-copper/ghtui?"));
    let cmds = press(&mut s, "<Enter>");
    let (change, _) = sent(&cmds);
    assert!(!format!("{change:?}").contains("hunter2"), "{change:?}");
    let SettingsChange::Actions(ActionsChange::SetSecret {
        env, name, key_id, ..
    }) = change
    else {
        panic!("{change:?}")
    };
    assert_eq!((env, name.as_str()), (&None, "NEW_TOKEN"));
    assert_eq!(key_id, "568250167242549743");
    answer(&mut s, &cmds, Ok(()));
    assert!(s.following.is_some());
    let mut shown = crate::settings::fixtures::secrets();
    let mut made = shown.repo.items[0].clone();
    made.name = "NEW_TOKEN".into();
    shown.repo.items.push(made);
    deliver(
        &mut s,
        SettingsPage::Secrets,
        Settings::Secrets(Box::new(shown)),
    );
    assert!(s.following.is_none());
}

/// `e` on an environment's secret asks only for its value; `D` deletes it
/// after asking; the organization's are its own.
#[test]
fn an_environments_secret_is_set_and_deleted_there() {
    use crate::api::settings::ActionsChange;
    let mut s = on(SettingsPage::Secrets);
    let deploy = SettingLink::Secret(Some("production".into()), "DEPLOY_KEY".into());
    select(&mut s, |l| *l == deploy);
    press(&mut s, "e");
    assert_eq!(composed(&s), "Value: ");
    write(&mut s, "Value: s3");
    press(&mut s, "<C-s>");
    assert!(
        asked(&s).starts_with(
            "Set secret DEPLOY_KEY in the production environment?\nIt replaces its value"
        ),
        "{}",
        asked(&s)
    );
    let cmds = press(&mut s, "<Enter>");
    assert!(matches!(
        sent(&cmds).0,
        SettingsChange::Actions(ActionsChange::SetSecret { env: Some(e), .. }) if e == "production"
    ));

    let mut s = on(SettingsPage::Secrets);
    select(&mut s, |l| *l == deploy);
    press(&mut s, "D");
    assert!(asked(&s).starts_with("Delete secret DEPLOY_KEY from the production environment?"));
    press(&mut s, "<Esc>");
    select(&mut s, |l| *l == SettingLink::OrgItem("ORG_TOKEN".into()));
    press(&mut s, "e");
    says(&s, "the organization's settings");
}

/// A variable's edit names what changed, sends it, and the page follows.
#[test]
fn a_variable_is_edited() {
    use crate::api::settings::ActionsChange;
    let mut s = on(SettingsPage::Variables);
    select(&mut s, |l| {
        *l == SettingLink::Variable(None, "REGION".into())
    });
    press(&mut s, "e");
    let text = composed(&s);
    assert_eq!(text, "Name: REGION\nValue: eu");
    write(&mut s, &text.replace("eu", "us"));
    press(&mut s, "<C-s>");
    assert_eq!(asked(&s), "Change variable REGION?\nValue: eu → us");
    let cmds = press(&mut s, "<Enter>");
    assert_eq!(
        sent(&cmds).0,
        &SettingsChange::Actions(ActionsChange::SetVariable {
            repo: ghtui(),
            env: None,
            from: Some("REGION".into()),
            name: "REGION".into(),
            value: "us".into(),
        })
    );
    answer(&mut s, &cmds, Ok(()));
    let mut shown = crate::settings::fixtures::variables();
    shown.repo.items[0].value = "us".into();
    deliver(
        &mut s,
        SettingsPage::Variables,
        Settings::Variables(Box::new(shown)),
    );
    assert!(s.following.is_none());
    assert!(page_text(&s).contains("us"));
}

/// A cache is deleted alone or with every cache of its key, after
/// asking; it takes write access.
#[test]
fn caches_are_deleted_one_or_by_key() {
    use crate::api::settings::ActionsChange;
    let mut s = on(SettingsPage::Caches);
    let text = page_text(&s);
    assert!(text.contains("3 caches · 900.0 MB of 10.0 GB"), "{text}");
    select(&mut s, |l| *l == SettingLink::Cache(1));
    press(&mut s, "D");
    let labels: Vec<String> = overlay!(s, Ask)
        .choices
        .iter()
        .map(|(l, _)| l.clone())
        .collect();
    assert_eq!(labels, ["Delete this cache", "Delete all 2 with this key"]);
    let cmds = press(&mut s, "<Tab><Enter>");
    assert!(matches!(
        sent(&cmds).0,
        SettingsChange::Actions(ActionsChange::DeleteCaches { count: 2, .. })
    ));
    answer(&mut s, &cmds, Ok(()));
    let mut shown = crate::settings::fixtures::caches();
    shown.items.items.retain(|c| c.key != "cargo-linux");
    deliver(&mut s, SettingsPage::Caches, Settings::Caches(shown));
    assert!(s.following.is_none());

    let mut s = on(SettingsPage::Caches);
    let overview = crate::api::browse::RepoOverview {
        can_write: false,
        ..fixtures::overview()
    };
    fetched(&mut s, DataKey::Repo(ghtui()), Data::from(overview));
    select(&mut s, |l| *l == SettingLink::Cache(3));
    press(&mut s, "D");
    says(&s, "takes write access");
}

/// The Pages site's form changes its source after asking; `D`
/// unpublishes it.
#[test]
fn the_pages_site_is_changed_and_unpublished() {
    use crate::api::settings::PagesChange;
    let mut s = on(SettingsPage::Pages);
    let text = page_text(&s);
    assert!(text.contains("The main branch, /docs"), "{text}");
    assert!(text.contains("Page build failed."), "{text}");
    press(&mut s, "e");
    let text = composed(&s);
    write(&mut s, &text.replace("Folder: /docs", "Folder: /"));
    press(&mut s, "<C-s>");
    assert_eq!(
        asked(&s),
        "Change gold-silver-copper/ghtui's site?\nFolder: /docs → /"
    );
    let cmds = press(&mut s, "<Enter>");
    assert!(matches!(
        sent(&cmds).0,
        SettingsChange::Pages(PagesChange::Update { to, .. }) if to.path == "/"
    ));
    let mut s = on(SettingsPage::Pages);
    press(&mut s, "D");
    assert!(asked(&s).starts_with("Unpublish gold-silver-copper/ghtui's site?"));
    press(&mut s, "<Esc>a");
    says(&s, "It has a site");
}

/// Custom properties: the form holds those the repository may set; one
/// only the organization sets says so.
#[test]
fn custom_properties_are_set_in_a_form() {
    let mut s = on(SettingsPage::Properties);
    select(&mut s, |l| *l == SettingLink::Property("team".into()));
    press(&mut s, "e");
    says(&s, "team is set by gold-silver-copper's admins");
    select(&mut s, |l| *l == SettingLink::Property("env".into()));
    press(&mut s, "e");
    assert_eq!(composed(&s), "env: Dev");
    write(&mut s, "env: prod");
    press(&mut s, "<C-s>");
    assert_eq!(
        asked(&s),
        "Set gold-silver-copper/ghtui's custom properties?\nenv: Dev → Prod"
    );
    let cmds = press(&mut s, "<Enter>");
    assert_eq!(
        sent(&cmds).0,
        &SettingsChange::SetProperties {
            repo: ghtui(),
            values: vec![(
                "env".into(),
                Some(crate::api::settings::PropValue::One("Prod".into()))
            )],
        }
    );
}
