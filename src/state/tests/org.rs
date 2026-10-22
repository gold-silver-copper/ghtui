//! An organization's people and teams, changed by its owners and its
//! teams' maintainers; the invitations sent to you; and a repository's
//! draft security advisory. Each asks first where GitHub's page does,
//! sends one change, and the page follows GitHub until it shows it.

use crate::act::{By, Subject};
use crate::api::org::{OrgChange, OrgRole};
use crate::api::settings::Permission;
use crate::housekeeping::{FormOf, advisory};
use crate::org::At;
use crate::route::{people, security};

use super::*;

fn org_people() -> Route {
    Route::People(people::User {
        login: "ratatui".into(),
        tab: ProfileTab::People,
    })
}

/// The organization's People tab, as one of its owners (or not).
fn people(owner: bool) -> State {
    let mut s = super::screens::at(sized(state(), 120, 40), org_people());
    let access = crate::org::fixtures::access(owner);
    fetched(
        &mut s,
        DataKey::OrgAccess("ratatui".into()),
        Data::from(access),
    );
    s
}

/// The one organization change `cmds` sends.
fn sent(cmds: &[Cmd]) -> (&OrgChange, &By) {
    match fixtures::sent_change(cmds) {
        (Change::Org(c), by) => (c, by),
        _ => panic!("{cmds:?}"),
    }
}

/// What the question asks, its facts one a line.
fn asked(s: &State) -> String {
    let c = overlay!(s, Ask);
    let facts: Vec<&str> = c.facts.iter().map(|(f, _)| f.as_str()).collect();
    format!("{}\n{}", c.title, facts.join("\n"))
}

fn select(s: &mut State, url: &str) {
    let link = crate::ui::page::Link::Url(url.into());
    fixtures::select_by(s, |l| *l == link);
}

/// An owner sees each member's role and the invitations waiting; `a`
/// invites someone with a role, after asking, and the page follows
/// GitHub until the invitation shows. Someone who isn't an owner is told
/// why they can't.
#[test]
fn an_owner_invites_a_member_and_the_page_follows() {
    let mut s = people(true);
    let text = screen_text(&s);
    assert!(text.contains("octocat") && text.contains("Owner"), "{text}");
    assert!(
        text.contains("Pending invitations  1") && text.contains("@newbie"),
        "{text}"
    );
    press(&mut s, "a");
    press(&mut s, "hubot<Enter>");
    assert!(picked_labels(&s)[0].contains("is a member already"));
    press(&mut s, "<C-u>hubot2<Enter>");
    press(&mut s, "owner<Enter>");
    assert!(
        asked(&s).starts_with("Invite @hubot2 to @ratatui as an owner?"),
        "{}",
        asked(&s)
    );
    let cmds = press(&mut s, "<Enter>");
    let (change, by) = sent(&cmds);
    assert_eq!(
        change,
        &OrgChange::SetMember {
            org: "ratatui".into(),
            login: "hubot2".into(),
            role: OrgRole::Owner,
            invite: true,
        }
    );
    assert_eq!(by, &By::Confirm(Subject::Org(At::People("ratatui".into()))));
    let cmds = answer(&mut s, &cmds, Ok(()));
    let again = fixtures::asked(&cmds);
    assert!(
        again.contains(&DataKey::OrgAccess("ratatui".into())),
        "{again:?}"
    );
    assert!(s.following.is_some(), "it waits for GitHub to show it");
    let mut access = crate::org::fixtures::access(true);
    if let Some(Ok(list)) = &mut access.invitations {
        list.items[0].login = Some("hubot2".into());
    }
    fetched(
        &mut s,
        DataKey::OrgAccess("ratatui".into()),
        Data::from(access),
    );
    assert!(s.following.is_none());

    let mut s = people(false);
    press(&mut s, "a");
    says(&s, "Only @ratatui's owners change its people");
    // By email too.
    let mut s = people(true);
    press(&mut s, "a");
    press(&mut s, "mona@example.com<Enter>member<Enter>");
    assert!(asked(&s).starts_with("Invite mona@example.com to @ratatui as a member?"));
    let cmds = press(&mut s, "<Enter>");
    assert!(matches!(
        sent(&cmds).0,
        OrgChange::InviteEmail {
            role: OrgRole::Member,
            ..
        }
    ));
}

/// `e` changes a member's role (the one they have is marked), `D`
/// removes a member or cancels an invitation, each after asking; a token
/// without the scope says which, and how to add it.
#[test]
fn an_owner_changes_roles_removes_and_cancels() {
    let mut s = people(true);
    select(&mut s, "https://github.com/hubot");
    press(&mut s, "e");
    assert!(picked_labels(&s).contains(&"✓ Member (now)".to_owned()));
    press(&mut s, "owner<Enter>");
    assert!(asked(&s).starts_with("Make @hubot an owner of @ratatui?"));
    let cmds = press(&mut s, "<Enter>");
    assert!(matches!(
        sent(&cmds).0,
        OrgChange::SetMember {
            role: OrgRole::Owner,
            invite: false,
            ..
        }
    ));
    press(&mut s, "<Esc>");

    let mut s = people(true);
    select(&mut s, "https://github.com/hubot");
    press(&mut s, "D");
    assert!(asked(&s).starts_with("Remove @hubot from @ratatui?"));
    press(&mut s, "<Esc>");
    let invitation = crate::ui::page::Link::Setting(crate::ui::pages::SettingLink::OrgInvitation(
        41,
        Some("newbie".into()),
    ));
    fixtures::select_by(&mut s, |l| *l == invitation);
    press(&mut s, "e");
    says(&s, "GitHub doesn't change an invitation's role");
    press(&mut s, "D");
    assert!(asked(&s).starts_with("Cancel the invitation to @newbie?"));
    let cmds = press(&mut s, "<Enter>");
    assert!(matches!(
        sent(&cmds).0,
        OrgChange::CancelInvitation { id: 41, .. }
    ));

    let mut s = people(true);
    let mut access = crate::org::fixtures::access(true);
    access.lacks = Some(crate::api::scope_missing(
        "Changing @ratatui's people and teams",
        "admin:org",
    ));
    fetched(
        &mut s,
        DataKey::OrgAccess("ratatui".into()),
        Data::from(access),
    );
    press(&mut s, "a");
    says(&s, "gh auth refresh -s admin:org");
}

/// On the Teams page, `a` makes a team (its name, then who can see it)
/// after asking, where GitHub lets you; on a team's page its maintainer
/// adds someone or a repository, with a role, and removes them.
#[test]
fn teams_are_made_and_their_members_and_repositories_changed() {
    let mut s = super::screens::with_teams();
    fetched(
        &mut s,
        DataKey::OrgAccess("github".into()),
        Data::from(crate::org::fixtures::access(true)),
    );
    press(&mut s, "a");
    press(&mut s, "core<Enter>");
    assert!(picked_labels(&s)[0].contains("has a team named core already"));
    press(&mut s, "<C-u>Docs<Enter>");
    press(&mut s, "secret<Enter>");
    assert!(asked(&s).starts_with("Make the team “Docs” in @github?\nSecret"));
    let cmds = press(&mut s, "<Enter>");
    assert_eq!(
        sent(&cmds).0,
        &OrgChange::CreateTeam {
            org: "github".into(),
            name: "Docs".into(),
            secret: true,
        }
    );

    let mut s = super::screens::with_team();
    press(&mut s, "a");
    says(&s, "takes maintaining it");
    let mut team = fixtures::team();
    team.can_admin = true;
    // All its members listed: one added shows, or not.
    team.members = crate::api::model::Capped::new(team.members.to_vec(), 2);
    let key = DataKey::Team("github".into(), "core".into());
    fetched(&mut s, key.clone(), Data::from(team.clone()));
    assert!(
        screen_text(&s).contains("Maintainer"),
        "{}",
        screen_text(&s)
    );
    press(&mut s, "a");
    press(&mut s, "mona<Enter>");
    press(&mut s, "maintainer<Enter>");
    assert!(asked(&s).starts_with("Add @mona to @github/core as a maintainer?"));
    let cmds = press(&mut s, "<Enter>");
    assert_eq!(
        sent(&cmds).0,
        &OrgChange::SetTeamMember {
            org: "github".into(),
            team: "core".into(),
            login: "mona".into(),
            maintainer: true,
            add: true,
        }
    );
    answer(&mut s, &cmds, Ok(()));
    assert!(s.following.is_some());
    team.members.items.push(crate::api::browse::UserSummary {
        login: "mona".into(),
        ..Default::default()
    });
    team.maintainers.push("mona".into());
    fetched(&mut s, key, Data::from(team));
    assert!(s.following.is_none(), "GitHub shows them");

    press(&mut s, "a");
    press(&mut s, "github/linguist");
    assert_eq!(picked_labels(&s), ["Give it access to github/linguist"]);
    press(&mut s, "<Enter>write<Enter>");
    let cmds = press(&mut s, "<Enter>");
    assert!(matches!(
        sent(&cmds).0,
        OrgChange::SetTeamRepo {
            permission: Permission::Write,
            add: true,
            ..
        }
    ));
    press(&mut s, "<Esc>");
    select(&mut s, "https://github.com/gold-silver-copper/ghtui");
    press(&mut s, "D");
    assert!(asked(&s).starts_with("Remove gold-silver-copper/ghtui from @github/core?"));
}

fn inbox_with_invitations() -> State {
    let mut s = super::screens::with_notifications();
    let all = crate::invited::fixtures::invitations();
    fetched(&mut s, DataKey::Invitations, Data::from(all));
    s
}

/// The inbox lists the invitations sent to you first; answering one asks
/// whether to accept or decline it, and the inbox follows GitHub until
/// it's gone. Marking a notification read still finds its thread below.
#[test]
fn an_invitation_is_answered_from_the_inbox() {
    let mut s = inbox_with_invitations();
    let text = page_text(&s);
    assert!(
        text.contains("@octocat invited you to collaborate on octocat/secret-plans"),
        "{text}"
    );
    assert!(text.contains("@octo-org invited you to join"), "{text}");
    select(
        &mut s,
        "https://github.com/octocat/secret-plans/invitations",
    );
    act(&mut s, Action::AnswerInvitation);
    assert!(asked(&s).starts_with("Join octocat/secret-plans?\nYou were invited by @octocat"));
    assert_eq!(ask_labels(&s), ["Accept invitation", "Decline"]);
    let cmds = press(&mut s, "<Enter>");
    assert_eq!(
        sent(&cmds).0,
        &OrgChange::AnswerRepo {
            id: 77,
            repo: RepoId::new("octocat", "secret-plans"),
            accept: true,
        }
    );
    answer(&mut s, &cmds, Ok(()));
    says(&s, "You're a collaborator on octocat/secret-plans now");
    assert!(s.following.is_some());
    let mut all = crate::invited::fixtures::invitations();
    all.repos.clear();
    fetched(&mut s, DataKey::Invitations, Data::from(all));
    assert!(s.following.is_none());

    let mut s = inbox_with_invitations();
    select(
        &mut s,
        "https://github.com/ratatui/ratatui/issues/1820#issuecomment-7",
    );
    let cmds = press(&mut s, "m");
    assert!(matches!(
        fixtures::sent_change(&cmds).0,
        Change::MarkRead { .. }
    ));
}

/// An invitation's own page has GitHub's buttons; Decline asks with
/// declining first. An organization's can only be accepted here.
#[test]
fn an_invitation_page_accepts_or_declines() {
    let page = Route::People(people::Invitation(people::Invite::Repo(RepoId::new(
        "octocat",
        "secret-plans",
    ))));
    let mut s = super::screens::at(state(), page);
    fetched(
        &mut s,
        DataKey::Invitations,
        Data::from(crate::invited::fixtures::invitations()),
    );
    let text = page_text(&s);
    assert!(
        text.contains("Accept invitation") && text.contains("Decline"),
        "{text}"
    );
    let decline = crate::invited::button(false);
    fixtures::select_by(&mut s, |l| *l == decline);
    press(&mut s, "<Enter>");
    assert_eq!(ask_labels(&s), ["Decline", "Accept invitation"]);
    let cmds = press(&mut s, "<Enter>");
    assert!(matches!(
        sent(&cmds).0,
        OrgChange::AnswerRepo { accept: false, .. }
    ));

    let org = Route::People(people::Invitation(people::Invite::Org("octo-org".into())));
    let mut s = super::screens::at(state(), org);
    fetched(
        &mut s,
        DataKey::Invitations,
        Data::from(crate::invited::fixtures::invitations()),
    );
    act(&mut s, Action::AnswerInvitation);
    assert_eq!(ask_labels(&s), ["Join @octo-org"]);
    assert!(asked(&s).contains("GitHub's API can't decline"));
}

/// On the Security tab, `:new here` drafts an advisory in GitHub's form
/// (its title first); ctrl-s makes the draft, and its page opens. Without
/// admin access, it says why.
#[test]
fn an_advisory_is_drafted_from_the_security_tab() {
    let mut s = super::screens::with_advisories();
    act(&mut s, Action::NewHere);
    says(&s, "takes admin access");
    let overview = crate::api::browse::RepoOverview {
        can_admin: true,
        ..fixtures::overview()
    };
    fetched(&mut s, DataKey::Repo(ghtui()), Data::from(overview));
    act(&mut s, Action::NewHere);
    let form = Route::Housekeeping(FormOf::Advisory(ghtui()));
    assert_eq!(s.route(), Some(&form));
    press(&mut s, "Path traversal<Enter>");
    for (row, value) in [
        ("Ecosystem", "rust"),
        ("Package", "ghtui"),
        ("Severity", "high"),
    ] {
        fixtures::select_where(&mut s, |p| {
            let item = p.selected.and_then(|i| p.page().items.get(i));
            item.and_then(|item| p.page().lines.get(item.start))
                .is_some_and(|line| line.text().starts_with(row))
        });
        press(&mut s, &format!("<Enter>{value}<Enter>"));
    }
    let text = page_text(&s);
    assert!(text.contains("Rust") && text.contains("High"), "{text}");
    assert!(text.contains("Describe the vulnerability"), "{text}");
    s.forms.edit::<crate::housekeeping::Form>(&form).text = "It escapes.".into();
    let cmds = press(&mut s, "<C-s>");
    let work = fixtures::sent::<advisory::Draft>(&cmds);
    assert_eq!(work.advisory.summary, "Path traversal");
    assert_eq!(work.advisory.ecosystem, "rust");
    fixtures::done(&mut s, work, Ok("GHSA-aaaa-bbbb-cccc".to_owned()));
    assert_eq!(
        s.route(),
        Some(&Route::Security(security::Advisory {
            repo: Some(ghtui()),
            ghsa: "GHSA-aaaa-bbbb-cccc".into(),
        }))
    );
    says(&s, "Drafted GHSA-aaaa-bbbb-cccc");
}
