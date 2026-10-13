use crate::api::change::Change;
use crate::api::model::NodeId;
use crate::api::settings::{Enforcement, Permission, Settings, SettingsChange, SettingsPage};
use crate::theme::{ColorDepth, Mode};

use super::{ghtui, page_text, render, sized, state};
use crate::act::{By, Confirm, Subject};
use crate::browse::{Data, DataKey};
use crate::fixtures::{self, answer, fetched, press, says};
use crate::route::Route;
use crate::settings::fixtures::page;
use crate::state::{Api, Cmd, Overlay, Screen, State};
use crate::ui::page::Link;
use crate::ui::pages::SettingLink;

/// A settings page of a repository you administer, with its data in.
fn on(at: SettingsPage, mode: Mode) -> State {
    let mut s = sized(state(mode, ColorDepth::TrueColor), 100, 40);
    let _ = s.push(Route::Settings {
        repo: ghtui(),
        page: at,
    });
    let overview = crate::api::browse::RepoOverview {
        can_admin: true,
        ..fixtures::overview()
    };
    fetched(
        &mut s,
        DataKey::Repo(ghtui()),
        Data::Repo(Box::new(overview)),
    );
    deliver(&mut s, at, page(at));
    s
}

fn deliver(s: &mut State, at: SettingsPage, data: Settings) -> Vec<Cmd> {
    fetched(
        s,
        DataKey::Settings(ghtui(), at),
        Data::Settings(Box::new(data)),
    )
}

/// The one change `cmds` sends.
fn sent(cmds: &[Cmd]) -> (&SettingsChange, &By) {
    match cmds {
        [Cmd::Api(Api::Change(Change::Settings(c), by))] => (c, by),
        _ => panic!("{cmds:?}"),
    }
}

fn confirm(s: &State) -> &Confirm {
    match &s.overlay {
        Some(Overlay::Confirm(c)) => c,
        other => panic!("no confirmation: {}", other.is_some()),
    }
}

/// What the confirmation says, its facts one a line.
fn asked(s: &State) -> String {
    let c = confirm(s);
    let facts: Vec<&str> = c.facts.iter().map(|(f, _)| f.as_str()).collect();
    format!("{}\n{}", c.title, facts.join("\n"))
}

/// Selects the first row that `is`.
fn select(s: &mut State, is: impl Fn(&SettingLink) -> bool) {
    let selected = |s: &State| match s.screen() {
        Screen::Page(p) => matches!(p.selected_link(), Some(Link::Setting(l)) if is(l)),
        Screen::Diff(_) => false,
    };
    press(s, "gg");
    for _ in 0..12 {
        if selected(s) {
            return;
        }
        press(s, "}");
    }
    assert!(selected(s), "nothing selected is what's wanted");
}

/// Writes `text` in the open composer, in place of what's there.
fn write(s: &mut State, text: &str) {
    let Some(Overlay::Compose(c)) = &s.overlay else {
        panic!("no composer");
    };
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

#[test]
fn settings_branches_dark() {
    assert_snapshot!(render(&on(SettingsPage::Branches, Mode::Dark)));
}

#[test]
fn settings_hook_light() {
    assert_snapshot!(render(&on(SettingsPage::Hook(11), Mode::Light)));
}

/// The Settings tab shows only to admins, as on GitHub, and its menu
/// leads to each page.
#[test]
fn the_settings_tab_is_for_admins() {
    let s = on(SettingsPage::General, Mode::Dark);
    let tabs: Vec<String> = s
        .chrome()
        .tabs
        .iter()
        .map(|(t, _)| t.label.clone())
        .collect();
    assert_eq!(tabs.last().map(String::as_str), Some("Settings"));
    let text = page_text(&s);
    assert!(
        text.contains("General · Collaborators · Branches · Webhooks · Deploy keys"),
        "{text}"
    );
    assert!(text.contains("never deletes a repository"), "{text}");
    let mut repo = super::with_repo(Mode::Dark, ColorDepth::TrueColor);
    let tabs = repo.chrome().tabs;
    assert!(tabs.iter().all(|(t, _)| t.label != "Settings"));
    press(&mut repo, ",x");
    says(&repo, "Closing works on");
}

/// `,e` writes a rule's form in the composer; what changed is named
/// before it's sent, only that is sent, and the page follows GitHub
/// until it shows it.
#[test]
fn a_rule_is_edited_as_a_form_and_named_before_saving() {
    let mut s = on(SettingsPage::Branches, Mode::Dark);
    select(&mut s, |l| matches!(l, SettingLink::Rule(_)));
    press(&mut s, ",e");
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
        panic!("{change:?}");
    };
    assert_eq!(id, &NodeId::new("BPR_1"));
    assert_eq!(by, &By::Confirm(about(SettingsPage::Branches)));
    let to = (**to).clone();
    answer(&mut s, &cmds, Ok(()));
    assert!(s.awaiting.is_some(), "it waits for GitHub to show it");
    let mut shown = crate::settings::fixtures::rule();
    shown.form = to;
    let Settings::Branches(mut b) = page(SettingsPage::Branches) else {
        panic!()
    };
    b.rules = vec![shown].into();
    deliver(&mut s, SettingsPage::Branches, Settings::Branches(b));
    assert!(s.awaiting.is_none());
    assert!(page_text(&s).contains("2 approvals"));
}

/// A new rule comes from an empty form; one GitHub refuses opens again
/// with its text, saying why. Deleting asks first.
#[test]
fn a_new_rule_refused_comes_back_to_edit() {
    let mut s = on(SettingsPage::Branches, Mode::Dark);
    press(&mut s, ",n");
    let text = composed(&s).replace("Branch name pattern: ", "Branch name pattern: release/*");
    write(&mut s, &text);
    press(&mut s, "<C-s>");
    assert!(
        asked(&s).starts_with("Protect branches matching release/* in gold-silver-copper/ghtui?")
    );
    let cmds = press(&mut s, "<Enter>");
    assert!(matches!(sent(&cmds).0, SettingsChange::CreateRule { .. }));
    let refused = crate::api::ApiError::GraphQl(vec!["Name already protected: release/*".into()]);
    answer(&mut s, &cmds, Err(refused));
    let Some(Overlay::Compose(c)) = &s.overlay else {
        panic!("the form isn't back");
    };
    assert!(c.text().contains("release/*"));
    assert!(
        c.error
            .as_deref()
            .is_some_and(|e| e.contains("already protected"))
    );

    let mut s = on(SettingsPage::Branches, Mode::Dark);
    select(&mut s, |l| matches!(l, SettingLink::Rule(_)));
    press(&mut s, ",D");
    assert!(asked(&s).contains(
        "Delete the protection rule for main?\nBranches matching main are then unprotected"
    ));
}

/// `,x` turns a ruleset off (or on) after asking; an organization's is
/// the organization's.
#[test]
fn a_ruleset_is_turned_off_after_asking() {
    let mut s = on(SettingsPage::Branches, Mode::Dark);
    select(&mut s, |l| *l == SettingLink::Ruleset(7));
    press(&mut s, ",x");
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
    let mut s = on(SettingsPage::Branches, Mode::Dark);
    select(&mut s, |l| *l == SettingLink::Ruleset(8));
    press(&mut s, ",x");
    says(&s, "It's octo-org's ruleset");
}

/// Inviting: a login, then a role, then the question naming both.
/// A role changes the same way; the owner's can't.
#[test]
fn a_collaborator_is_invited_with_a_role() {
    let mut s = on(SettingsPage::Access, Mode::Dark);
    press(&mut s, ",n");
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

    let mut s = on(SettingsPage::Access, Mode::Dark);
    select(&mut s, |l| *l == SettingLink::Person("octocat".into()));
    press(&mut s, ",e");
    press(&mut s, "maintain<Enter>");
    assert!(asked(&s).contains("From Write to Maintain"));
    press(&mut s, "<Esc>");
    press(&mut s, ",D");
    assert!(asked(&s).starts_with("Remove @octocat from gold-silver-copper/ghtui?"));
    press(&mut s, "<Esc>");
    select(&mut s, |l| {
        *l == SettingLink::Person("gold-silver-copper".into())
    });
    press(&mut s, ",D");
    says(&s, "owns gold-silver-copper/ghtui");
}

/// A webhook from the form; on its page, `,x` disables it and `,R`
/// redelivers a delivery, each after asking; deleting it there goes
/// back to the list.
#[test]
fn webhooks_are_added_disabled_redelivered_and_deleted() {
    let mut s = on(SettingsPage::Hooks, Mode::Dark);
    press(&mut s, ",n");
    let text = composed(&s).replace("Payload URL: ", "Payload URL: https://example.com/new");
    write(&mut s, &text);
    press(&mut s, "<C-s>");
    assert!(
        asked(&s).starts_with("Send gold-silver-copper/ghtui's events to https://example.com/new?")
    );
    let cmds = press(&mut s, "<Enter>");
    assert!(matches!(sent(&cmds).0, SettingsChange::CreateHook { .. }));

    let mut s = on(SettingsPage::Hook(11), Mode::Dark);
    press(&mut s, ",x");
    assert!(asked(&s).starts_with("Disable the webhook to https://example.com/hook/11?"));
    let cmds = press(&mut s, "<Enter>");
    let SettingsChange::UpdateHook { to, .. } = sent(&cmds).0 else {
        panic!()
    };
    assert!(!to.active);
    press(&mut s, "<Esc>");
    select(&mut s, |l| *l == SettingLink::Delivery(902));
    press(&mut s, ",R");
    assert!(asked(&s).starts_with("Redeliver the pull_request delivery?"));
    let cmds = press(&mut s, "<Enter>");
    assert!(matches!(
        sent(&cmds).0,
        SettingsChange::Redeliver { delivery: 902, .. }
    ));

    let mut s = on(SettingsPage::Hook(11), Mode::Dark);
    press(&mut s, ",D");
    let cmds = press(&mut s, "<Enter>");
    assert_eq!(sent(&cmds).1, &By::Confirm(about(SettingsPage::Hooks)));
    answer(&mut s, &cmds, Ok(()));
    assert_eq!(
        s.route(),
        Some(&Route::Settings {
            repo: ghtui(),
            page: SettingsPage::Hooks
        })
    );
}

/// A deploy key with write access says so before it's added; a key
/// can't be edited, only deleted.
#[test]
fn a_deploy_key_with_write_access_says_so() {
    let mut s = on(SettingsPage::Keys, Mode::Dark);
    press(&mut s, ",n");
    write(
        &mut s,
        "Title: deploy\nAllow write access: yes\nKey: ssh-ed25519 AAAAC3Nz me@x",
    );
    press(&mut s, "<C-s>");
    let c = confirm(&s);
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
    press(&mut s, ",e");
    says(&s, "delete it and add it again");
    press(&mut s, ",D");
    assert!(asked(&s).starts_with("Delete deploy key “CI”?"));
}

/// Archiving asks twice: for the repository's name, then once more.
/// Deleting it is github.com's.
#[test]
fn archiving_asks_twice() {
    let mut s = on(SettingsPage::General, Mode::Dark);
    press(&mut s, ",x");
    press(&mut s, "ghtui");
    assert!(press(&mut s, "<Enter>").is_empty());
    assert!(
        !matches!(s.overlay, Some(Overlay::Confirm(_))),
        "the wrong name asks nothing"
    );
    press(&mut s, "<Esc>,x");
    press(&mut s, "gold-silver-copper/ghtui<Enter>");
    assert!(asked(&s).starts_with("Archive gold-silver-copper/ghtui? This is the last ask"));
    let cmds = press(&mut s, "<Enter>");
    assert_eq!(
        sent(&cmds).0,
        &SettingsChange::Archive {
            repo: ghtui(),
            archived: true
        }
    );
    answer(&mut s, &cmds, Ok(()));
    let Settings::General(mut g) = page(SettingsPage::General) else {
        panic!()
    };
    g.archived = true;
    deliver(&mut s, SettingsPage::General, Settings::General(g));
    assert!(s.awaiting.is_none());
    assert!(page_text(&s).contains("Unarchive this repository"));
    press(&mut s, ",D");
    says(&s, "ghtui never deletes one");
}
