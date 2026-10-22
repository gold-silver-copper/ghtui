//! Following people and setting your status: each is sent once, and the
//! profile is read again until GitHub shows it.

use crate::api::browse::{Profile, UserStatus};
use crate::api::social::NewStatus;
use crate::social::{Follow, SetStatus};
use crate::ui::pages::ProfileTab;

use super::*;

fn octocat() -> DataKey {
    DataKey::Profile("octocat".into())
}

/// `:follow` on someone's profile follows them; GitHub's answer is
/// followed until the profile says so, and the profile then shows it.
/// Your own can't be followed.
#[test]
fn a_person_is_followed_and_the_profile_follows() {
    let mut s = with_profile(ProfileTab::Overview);
    act(&mut s, Action::Follow);
    says(&s, "That's you");

    s.viewer = Some("hubot".into());
    let cmds = act(&mut s, Action::Follow);
    let follow = Follow {
        login: "octocat".into(),
        id: NodeId::new("U_octocat"),
        org: false,
        on: true,
    };
    assert_eq!(cmds, [fixtures::start(follow.clone())]);
    let cmds = fixtures::done(&mut s, follow, Ok(()));
    says(&s, "Following @octocat");
    assert!(fixtures::asks(&cmds, &octocat()), "{cmds:?}");
    fetched(&mut s, octocat(), Data::from(fixtures::profile()));
    assert!(s.following.is_some(), "GitHub doesn't show it yet");
    let followed = Profile {
        followed: Some(true),
        ..fixtures::profile()
    };
    fetched(&mut s, octocat(), Data::from(followed));
    assert!(s.following.is_none());
    assert!(page_text(&s).contains("Following"));
}

/// An organization you follow is unfollowed; a refusal for a token
/// without the scope names it and how to add it.
#[test]
fn an_organization_is_unfollowed_and_a_missing_scope_is_named() {
    let mut s = with_org();
    let cmds = act(&mut s, Action::Follow);
    let unfollow = fixtures::sent::<Follow>(&cmds);
    assert!(unfollow.org && !unfollow.on, "{unfollow:?}");
    let why = "Your token has not been granted the required scopes to execute this query. The 'unfollowOrganization' field requires one of the following scopes: ['user:follow']";
    fixtures::done(&mut s, unfollow, Err(fixtures::failure(why)));
    warns(&s, "`gh auth refresh -s user:follow` adds it");
}

/// `:set status` starts from your status; what's typed is set, busy or
/// not, to clear when chosen; the profile follows until it shows it.
#[test]
fn your_status_is_set_busy_and_followed() {
    let mut s = with_profile(ProfileTab::Overview);
    let cmds = act(&mut s, Action::SetStatus);
    fixtures::served(&mut s, &cmds);
    let rows = picked(&s);
    assert_eq!(
        rows[0],
        (
            "Set “🐙 Reviewing pull requests”".to_owned(),
            "doesn't clear".to_owned()
        )
    );
    // Busy, then clearing in 30 minutes: what was typed stays.
    press(&mut s, " today");
    let busy = picked_labels(&s)
        .iter()
        .position(|l| l.contains("Busy"))
        .unwrap();
    press(&mut s, &"<Down>".repeat(busy));
    let cmds = press(&mut s, "<Enter>");
    fixtures::served(&mut s, &cmds);
    let rows = picked(&s);
    assert_eq!(rows[1].0, "Set “🐙 Reviewing pull requests today · Busy”");
    let cmds = press(&mut s, "<Down><Enter>");
    let status = NewStatus {
        emoji: Some("🐙".into()),
        message: "Reviewing pull requests today".into(),
        busy: true,
        expires_at: Some(crate::ui::time::iso8601(NOW + 1800)),
    };
    let set = SetStatus {
        login: "octocat".into(),
        status: Some(status),
    };
    assert_eq!(cmds, [fixtures::start(set.clone())]);
    fixtures::done(&mut s, set, Ok(()));
    says(&s, "Status set");
    assert!(s.following.is_some());
    let shown = Profile {
        status: Some(UserStatus {
            emoji: Some(":octopus:".into()),
            message: Some("Reviewing pull requests today".into()),
            busy: true,
            expires_at: Some(crate::ui::time::iso8601(NOW + 1800)),
        }),
        ..fixtures::profile()
    };
    fetched(&mut s, octocat(), Data::from(shown));
    assert!(s.following.is_none(), "its emoji is GitHub's to write");
    assert!(page_text(&s).contains("Busy · clears in 30m"));
}

/// Clearing your status sends none, and a token without the user scope
/// says how to add it.
#[test]
fn your_status_is_cleared() {
    let mut s = with_profile(ProfileTab::Overview);
    let cmds = act(&mut s, Action::SetStatus);
    fixtures::served(&mut s, &cmds);
    let clear = picked_labels(&s)
        .iter()
        .position(|l| l == "Clear your status")
        .unwrap();
    press(&mut s, &"<Down>".repeat(clear));
    let cmds = press(&mut s, "<Enter>");
    let clear = SetStatus {
        login: "octocat".into(),
        status: None,
    };
    assert_eq!(cmds, [fixtures::start(clear.clone())]);
    let why = "The 'changeUserStatus' field requires one of the following scopes: ['user'], but your token has not been granted the required scopes";
    fixtures::done(&mut s, clear, Err(fixtures::failure(why)));
    warns(&s, "`gh auth refresh -s user` adds it");
}
