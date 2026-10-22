use crate::act::{By, Subject};
use crate::api::edit::{Edit, Named, Reaction, ReactionContent, Written};

use super::*;

fn issue() -> Subject {
    Subject::Issue(ghtui(), 14)
}

fn hooray(count: u64, mine: bool) -> Reaction {
    Reaction {
        content: ReactionContent::Hooray,
        count,
        mine,
    }
}

/// `,e` opens your comment in the composer, where it was and with its
/// text; saving sends the new text, and the page waits until GitHub
/// shows it.
#[test]
fn a_comment_is_edited_in_place_and_followed() {
    let mut s = issue_with(|_| {});
    select_comment(&mut s, "hubot", "");
    let before = screen_text(&s);
    let comment = before.lines().position(|l| l.contains("hubot commented"));
    // Opening it changes nothing on GitHub; it only looks up who can be
    // mentioned.
    let opened = press(&mut s, "e");
    assert!(
        fixtures::asked(&opened) == [DataKey::Mentionable(ghtui())] && opened.len() == 1,
        "{opened:?}"
    );
    let c = fixtures::composer(&mut s);
    assert_eq!(c.text(), "Agreed. 120 columns, centered?");
    let screen = screen_text(&s);
    let editor = screen
        .lines()
        .position(|l| l.contains("Edit hubot's comment"));
    assert!(
        comment.zip(editor).is_some_and(|(c, e)| c.abs_diff(e) <= 1),
        "the editor takes the comment's place: {comment:?} {editor:?}\n{screen}"
    );
    press(&mut s, "!");
    let cmds = press(&mut s, "<C-s>");
    let edit = Edit::Body {
        of: Written::IssueComment,
        id: NodeId::new("IC_1"),
        body: "Agreed. 120 columns, centered?!".into(),
    };
    assert_eq!(
        fixtures::sent_change(&cmds),
        (&Change::Edit(edit), &By::Edit(issue()))
    );

    answer(&mut s, &cmds, Ok(()));
    assert!(s.overlay.is_none());
    says(&s, "Saved");
    // GitHub hasn't shown it: the page waits, and says so.
    issue_shows(&mut s, |_| {});
    assert!(s.following.is_some());
    press(&mut s, "e");
    says(&s, "hasn't shown the last change");
    issue_shows(&mut s, |d| {
        d.first_comment().unwrap().body = "Agreed. 120 columns, centered?!".into();
    });
    assert!(s.following.is_none());
}

/// Editing and deleting are offered only where GitHub lets you, and say
/// why where it doesn't; a description is edited, never deleted.
#[test]
fn edits_are_offered_only_where_github_allows_them() {
    let mut s = issue_with(|d| {
        let mine = d.first_comment().unwrap().editable.as_mut().unwrap();
        (mine.can_update, mine.can_delete) = (false, false);
    });
    select_comment(&mut s, "hubot", "");
    let why = |s: &State, action| {
        let row = s.doables().into_iter().find(|d| d.action == action);
        row.and_then(|d| d.unavailable).unwrap_or_default()
    };
    assert!(why(&s, Action::EditComment).contains("doesn't let you edit hubot's comment"));
    assert!(why(&s, Action::DeleteComment).contains("doesn't let you delete"));
    assert!(
        why(&s, Action::React).is_empty(),
        "reacting is still offered"
    );
    press(&mut s, "e");
    assert!(s.overlay.is_none());
    says(&s, "GitHub doesn't let you edit hubot's comment");

    // The description: nothing selected, it's what's edited.
    press(&mut s, "<Home>");
    if let Screen::Page(p) = s.screen_mut() {
        p.selected = None;
    }
    assert!(why(&s, Action::EditComment).is_empty());
    press(&mut s, "D");
    says(&s, "Select a comment first");
    press(&mut s, "e");
    let c = fixtures::composer(&mut s);
    assert!(c.text().starts_with("On a 200-column terminal"));

    // Elsewhere, they say where they work.
    let mut list = super::with_issues();
    press(&mut list, "e");
    says(&list, "works on a comment, an issue or a pull request");
}

/// Deleting asks first; once GitHub has deleted it, the page shows it
/// gone.
#[test]
fn deleting_a_comment_asks_first() {
    let mut s = issue_with(|_| {});
    select_comment(&mut s, "hubot", "");
    assert!(press(&mut s, "D").is_empty());
    let c = overlay!(s, Ask);
    assert!(c.title.starts_with("Delete hubot's comment"), "{}", c.title);
    // Data arriving doesn't close or change it.
    issue_shows(&mut s, |_| {});
    assert!(matches!(s.overlay, Some(Overlay::Ask(_))));
    let cmds = press(&mut s, "<Enter>");
    let delete = Edit::Delete {
        of: Written::IssueComment,
        id: NodeId::new("IC_1"),
    };
    assert_eq!(
        fixtures::sent_change(&cmds),
        (&Change::Edit(delete), &By::Confirm(issue()))
    );
    answer(&mut s, &cmds, Ok(()));
    assert!(s.overlay.is_none());
    says(&s, "Deleted");
    issue_shows(&mut s, |_| {});
    assert!(s.following.is_some());
    // Meanwhile, another edit waits too, and says so.
    select_comment(&mut s, "hubot", "");
    assert!(press(&mut s, "D").is_empty());
    assert!(s.overlay.is_none());
    says(&s, "GitHub hasn't shown the last change yet");
    issue_shows(&mut s, |d| {
        d.timeline
            .items
            .retain(|i| !matches!(i, crate::api::timeline::Item::Comment(_)));
    });
    assert!(s.following.is_none());
}

/// Reactions show under each comment and the description as GitHub
/// shows them, yours marked.
#[test]
fn reactions_show_under_the_description_and_each_comment() {
    let s = issue_with(|d| {
        d.editable.reactions = vec![Reaction {
            content: ReactionContent::ThumbsUp,
            count: 3,
            mine: false,
        }];
        let comment = d.first_comment().unwrap().editable.as_mut().unwrap();
        comment.reactions = vec![
            hooray(2, true),
            Reaction {
                content: ReactionContent::Eyes,
                count: 1,
                mine: false,
            },
        ];
    });
    let screen = screen_text(&s);
    let row = |text: &str| screen.lines().position(|l| l.contains(text));
    let (yours, hubots) = (row("👍 3"), row("🎉 2   👀 1"));
    let hubot = row("hubot commented");
    assert!(yours < hubot && hubot < hubots, "{screen}");
}

/// `,+` picks one of GitHub's eight reactions: one you gave is taken
/// back, another is given. Without a comment selected, it's the issue's.
#[test]
fn reacting_toggles_yours() {
    let mut s = issue_with(|d| d.editable.reactions = vec![hooray(2, true)]);
    press(&mut s, "<Home>");
    if let Screen::Page(p) = s.screen_mut() {
        p.selected = None;
    }
    press(&mut s, "+");
    let rows = picked_labels(&s);
    assert_eq!(rows.len(), 8);
    assert!(
        rows[3].starts_with('✓') && rows[3].contains("Hooray"),
        "{rows:?}"
    );
    let cmds = press(&mut s, "<Down><Down><Down><Enter>");
    let take_back = Edit::React {
        subject: NodeId::new("I_14"),
        content: ReactionContent::Hooray,
        on: false,
    };
    assert_eq!(
        fixtures::sent_change(&cmds),
        (&Change::Edit(take_back), &By::Edit(issue()))
    );
    assert!(s.overlay.is_none());
    answer(&mut s, &cmds, Ok(()));
    issue_shows(&mut s, |d| d.editable.reactions = vec![hooray(1, false)]);
    assert!(s.following.is_none());

    select_comment(&mut s, "hubot", "");
    press(&mut s, "+");
    let cmds = press(&mut s, "<Enter>");
    let give = Edit::React {
        subject: NodeId::new("IC_1"),
        content: ReactionContent::ThumbsUp,
        on: true,
    };
    assert_eq!(fixtures::sent_change(&cmds).0, &Change::Edit(give));
}

/// The labels picker, as GitHub's sidebar has it: what's set checked and
/// first, then the repository's other labels.
#[test]
fn the_labels_picker_lists_whats_set_first() {
    let mut s = issue_with(|_| {});
    let cmds = act(&mut s, Action::EditLabels);
    let key = DataKey::Sidebar(ghtui(), 14);
    assert!(fixtures::asks(&cmds, &key), "{cmds:?}");
    fixtures::served(&mut s, &cmds);
    press(&mut s, "<Down><Down><Enter><Up><Enter>");
    let screen = screen_text(&s);
    let row = |text: &str| screen.lines().position(|l| l.contains(text));
    let set = row("✓ bug");
    let (added, unset) = (row("✓ documentation"), row("good first issue"));
    assert!(set.is_some() && set < added && added < unset, "{screen}");
    assert!(screen.contains("removing") && screen.contains("adding"));
}

/// Labels are checked and unchecked, then applied at once: what's added
/// and what's removed, and the page waits until GitHub shows both.
#[test]
fn labels_are_checked_then_applied() {
    let mut s = issue_with(|_| {});
    let cmds = act(&mut s, Action::EditLabels);
    fixtures::served(&mut s, &cmds);
    // bug, ui, documentation, good first issue: check documentation,
    // uncheck ui.
    press(&mut s, "<Down><Down><Enter><Up><Enter>");
    let cmds = press(&mut s, "<C-s>");
    let named = |id: &str, name: &str| Named {
        id: NodeId::new(id),
        name: name.into(),
    };
    let labels = Edit::Labels {
        subject: NodeId::new("I_14"),
        add: vec![named("L_docs", "documentation")],
        remove: vec![named("L_ui", "ui")],
    };
    assert_eq!(
        fixtures::sent_change(&cmds),
        (&Change::Edit(labels), &By::Edit(issue()))
    );
    says(&s, "Changing the labels");
    answer(&mut s, &cmds, Ok(()));
    issue_shows(&mut s, |_| {});
    assert!(s.following.is_some());
    let label = |name: &str| crate::api::model::Label {
        name: name.into(),
        color: "0075ca".into(),
    };
    issue_shows(&mut s, |d| d.labels = vec![label("bug")].into());
    assert!(s.following.is_some(), "documentation isn't there yet");
    issue_shows(&mut s, |d| {
        d.labels = vec![label("bug"), label("documentation")].into();
    });
    assert!(s.following.is_none());

    // Unchanged, nothing is sent.
    act(&mut s, Action::EditLabels);
    assert!(press(&mut s, "<C-s>").is_empty());
    says(&s, "Nothing changed");
}

/// Reviewers are people and teams: asking replaces who's asked, so
/// everyone still asked is sent; the conversation shows who's asked.
#[test]
fn reviewers_include_teams() {
    let mut s = with_pr();
    let cmds = act(&mut s, Action::RequestReviews);
    fixtures::served(&mut s, &cmds);
    let rows = picked_labels(&s);
    // Who's asked, then the rest; never the author (octocat).
    let names = [
        "✓ defunkt",
        "✓ octo-org/core",
        "  hubot",
        "  monalisa",
        "  octo-org/docs",
    ];
    assert_eq!(rows, names);
    let cmds = press(&mut s, "<Down><Enter><Down><Enter><C-s>");
    let Change::Edit(Edit::Reviewers { users, teams, .. }) = fixtures::sent_change(&cmds).0 else {
        panic!("{cmds:?}")
    };
    let names = |list: &[Named]| list.iter().map(|n| n.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(users), ["defunkt", "hubot"]);
    assert!(teams.is_empty());
    answer(&mut s, &cmds, Ok(()));
    let mut activity = fixtures::activity();
    activity.requested = ["defunkt", "hubot"]
        .map(|who| crate::api::timeline::Requested {
            who: crate::api::browse::Reviewer::User(who.into()),
            code_owner: false,
        })
        .to_vec()
        .into();
    fetched(&mut s, DataKey::PrActivity(pr()), Data::from(activity));
    assert!(s.following.is_none());
}

/// An issue's type is one of its repository's, or none, set from the
/// picker; the page follows GitHub until it shows it. A pull request has
/// none to set.
#[test]
fn an_issue_type_is_set_and_followed() {
    let mut s = issue_with(|_| {});
    let cmds = act(&mut s, Action::EditIssueType);
    fixtures::served(&mut s, &cmds);
    let rows = picked_labels(&s);
    assert_eq!(rows, ["  No type", "✓ Bug", "  Feature", "  Task"]);
    let cmds = press(&mut s, "Feat<Enter>");
    let set = Edit::IssueType {
        issue: NodeId::new("I_14"),
        issue_type: Some(Named {
            id: NodeId::new("IT_feature"),
            name: "Feature".into(),
        }),
    };
    assert_eq!(
        fixtures::sent_change(&cmds),
        (&Change::Edit(set), &By::Edit(issue()))
    );
    answer(&mut s, &cmds, Ok(()));
    says(&s, "Type set");
    issue_shows(&mut s, |_| {});
    assert!(s.following.is_some(), "GitHub still says Bug");
    issue_shows(&mut s, |d| d.issue_type = Some("Feature".into()));
    assert!(s.following.is_none());

    let mut s = with_pr();
    act(&mut s, Action::EditIssueType);
    says(&s, "a pull request has none");
}

/// One milestone, or none; and a title in a one-line box.
#[test]
fn milestone_and_title() {
    let mut s = issue_with(|_| {});
    let cmds = act(&mut s, Action::EditMilestone);
    fixtures::served(&mut s, &cmds);
    // No milestone, Links (current), 1.0.
    let cmds = press(&mut s, "<Down><Down><Enter>");
    let Change::Edit(Edit::Milestone { milestone, .. }) = fixtures::sent_change(&cmds).0 else {
        panic!("{cmds:?}")
    };
    assert_eq!(milestone.as_ref().map(|m| m.id.as_str()), Some("M_4"));
    answer(&mut s, &cmds, Ok(()));
    issue_shows(&mut s, |d| {
        if let Some(m) = &mut d.milestone {
            m.title = "1.0".into();
        }
    });
    assert!(s.following.is_none());

    act(&mut s, Action::EditTitle);
    let cmds = press(&mut s, "!<Enter>");
    let title = Edit::Title {
        pr: false,
        id: NodeId::new("I_14"),
        title: "Pages wrap at the terminal width!".into(),
    };
    assert_eq!(
        fixtures::sent_change(&cmds),
        (&Change::Edit(title), &By::Edit(issue()))
    );
}
