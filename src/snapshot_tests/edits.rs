use crate::act::{By, Subject};
use crate::api::browse::IssueDetail;
use crate::api::change::Change;
use crate::api::edit::{Edit, Named, Reaction, ReactionContent, Written};
use crate::api::model::{NodeId, PrRef};
use crate::theme::Mode;

use super::{ghtui, render, sized, with_issue, with_pr};
use crate::browse::{Data, DataKey};
use crate::fixtures::{self, answer, fetched, press, says};
use crate::keymap::Action;
use crate::state::{Api, Cmd, Overlay, Screen, State};
use crate::ui::page::Link;

fn issue() -> Subject {
    Subject::Issue(ghtui(), 14)
}

fn pr() -> PrRef {
    PrRef::parse("gold-silver-copper/ghtui#12").unwrap()
}

/// The issue as GitHub has it, changed by `f`.
fn deliver(s: &mut State, f: impl FnOnce(&mut IssueDetail)) -> Vec<Cmd> {
    let mut d = fixtures::issue();
    f(&mut d);
    let key = DataKey::Issue(ghtui(), 14);
    fetched(s, key, Data::Issue(Some(Box::new(d))))
}

/// The issue, as GitHub has it changed by `f`, in a tall terminal.
fn issue_with(f: impl FnOnce(&mut IssueDetail)) -> State {
    let mut s = sized(with_issue(Mode::Dark), 100, 40);
    deliver(&mut s, f);
    s
}

/// Selects hubot's comment.
fn on_hubot(s: &mut State) {
    let selected = |s: &State| match s.screen() {
        Screen::Page(p) => {
            matches!(p.selected_link(), Some(Link::Quote { author, .. }) if author == "hubot")
        }
        Screen::Diff(_) => false,
    };
    for _ in 0..10 {
        if selected(s) {
            return;
        }
        press(s, "}");
    }
    assert!(selected(s), "hubot's comment is selected");
}

fn sent(cmds: &[Cmd]) -> (&Change, &By) {
    match cmds {
        [Cmd::Api(Api::Change(change, by))] => (change, by),
        _ => panic!("{cmds:?}"),
    }
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
    on_hubot(&mut s);
    let before = render(&s);
    let comment = before.lines().position(|l| l.contains("hubot commented"));
    // Opening it changes nothing on GitHub; it only looks up who can be
    // mentioned.
    let opened = press(&mut s, ",e");
    assert!(
        opened.iter().all(|c| matches!(
            c,
            crate::state::Cmd::Api(crate::state::Api::FetchMentionable(_))
        )),
        "{opened:?}"
    );
    let Some(Overlay::Compose(c)) = &s.overlay else {
        panic!(",e opens the composer")
    };
    assert_eq!(c.text(), "Agreed. 120 columns, centered?");
    let screen = render(&s);
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
    assert_eq!(sent(&cmds), (&Change::Edit(edit), &By::Edit(issue())));

    answer(&mut s, &cmds, Ok(()));
    assert!(s.overlay.is_none());
    says(&s, "Saved");
    // GitHub hasn't shown it: the page waits, and says so.
    deliver(&mut s, |_| {});
    assert!(s.awaiting.is_some());
    press(&mut s, ",e");
    says(&s, "hasn't shown the last change");
    deliver(&mut s, |d| {
        d.first_comment().unwrap().body = "Agreed. 120 columns, centered?!".into();
    });
    assert!(s.awaiting.is_none());
}

/// Editing and deleting are offered only where GitHub lets you, and say
/// why where it doesn't; a description is edited, never deleted.
#[test]
fn edits_are_offered_only_where_github_allows_them() {
    let mut s = issue_with(|d| {
        let mine = d.first_comment().unwrap().editable.as_mut().unwrap();
        (mine.can_update, mine.can_delete) = (false, false);
    });
    on_hubot(&mut s);
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
    press(&mut s, ",e");
    assert!(s.overlay.is_none());
    says(&s, "GitHub doesn't let you edit hubot's comment");

    // The description: nothing selected, it's what's edited.
    press(&mut s, "gg");
    if let Screen::Page(p) = s.screen_mut() {
        p.selected = None;
    }
    assert!(why(&s, Action::EditComment).is_empty());
    press(&mut s, ",D");
    says(&s, "Select a comment first");
    press(&mut s, ",e");
    let Some(Overlay::Compose(c)) = &s.overlay else {
        panic!("the description opens")
    };
    assert!(c.text().starts_with("On a 200-column terminal"));

    // Elsewhere, they say where they work.
    let mut list = super::with_issues(Mode::Dark);
    press(&mut list, ",e");
    says(&list, "works on a comment, an issue or a pull request");
}

/// Deleting asks first; once GitHub has deleted it, the page shows it
/// gone.
#[test]
fn deleting_a_comment_asks_first() {
    let mut s = issue_with(|_| {});
    on_hubot(&mut s);
    assert!(press(&mut s, ",D").is_empty());
    let Some(Overlay::Confirm(c)) = &s.overlay else {
        panic!(",D asks first")
    };
    assert!(c.title.starts_with("Delete hubot's comment"), "{}", c.title);
    // Data arriving doesn't close or change it.
    deliver(&mut s, |_| {});
    assert!(matches!(s.overlay, Some(Overlay::Confirm(_))));
    let cmds = press(&mut s, "<Enter>");
    let delete = Edit::Delete {
        of: Written::IssueComment,
        id: NodeId::new("IC_1"),
    };
    assert_eq!(sent(&cmds), (&Change::Edit(delete), &By::Confirm(issue())));
    answer(&mut s, &cmds, Ok(()));
    assert!(s.overlay.is_none());
    says(&s, "Deleted");
    deliver(&mut s, |_| {});
    assert!(s.awaiting.is_some());
    // Meanwhile, another edit waits too, and says so.
    on_hubot(&mut s);
    assert!(press(&mut s, ",D").is_empty());
    assert!(s.overlay.is_none());
    says(&s, "GitHub hasn't shown the last change yet");
    deliver(&mut s, |d| {
        d.timeline
            .items
            .retain(|i| !matches!(i, crate::api::timeline::Item::Comment(_)));
    });
    assert!(s.awaiting.is_none());
}

/// Reactions show under each comment and the description as GitHub
/// shows them, yours marked.
#[test]
fn reactions_under_a_comment_dark() {
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
    assert_snapshot!(render(&s));
}

/// `,+` picks one of GitHub's eight reactions: one you gave is taken
/// back, another is given. Without a comment selected, it's the issue's.
#[test]
fn reacting_toggles_yours() {
    let mut s = issue_with(|d| d.editable.reactions = vec![hooray(2, true)]);
    press(&mut s, "gg");
    if let Screen::Page(p) = s.screen_mut() {
        p.selected = None;
    }
    press(&mut s, ",+");
    let rows: Vec<String> = match &s.overlay {
        Some(Overlay::Picker(p)) => s.picker_rows(p).into_iter().map(|(r, _)| r.label).collect(),
        _ => panic!(",+ opens the reactions"),
    };
    assert_eq!(rows.len(), 8);
    assert!(
        rows[3].starts_with('✓') && rows[3].contains("Hooray"),
        "{rows:?}"
    );
    let cmds = press(&mut s, "jjj<Enter>");
    let take_back = Edit::React {
        subject: NodeId::new("I_14"),
        content: ReactionContent::Hooray,
        on: false,
    };
    assert_eq!(sent(&cmds), (&Change::Edit(take_back), &By::Edit(issue())));
    assert!(s.overlay.is_none());
    answer(&mut s, &cmds, Ok(()));
    deliver(&mut s, |d| d.editable.reactions = vec![hooray(1, false)]);
    assert!(s.awaiting.is_none());

    on_hubot(&mut s);
    press(&mut s, ",+");
    let cmds = press(&mut s, "<Enter>");
    let give = Edit::React {
        subject: NodeId::new("IC_1"),
        content: ReactionContent::ThumbsUp,
        on: true,
    };
    assert_eq!(sent(&cmds).0, &Change::Edit(give));
}

/// The labels picker, as GitHub's sidebar has it: what's set checked and
/// first, then the repository's other labels.
#[test]
fn labels_picker_dark() {
    let mut s = issue_with(|_| {});
    let cmds = press(&mut s, ",l");
    let key = DataKey::Sidebar(ghtui(), 14);
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Cmd::Api(Api::Fetch { key: k, .. }) if *k == key)),
        "{cmds:?}"
    );
    fetched(&mut s, key, Data::Sidebar(Box::new(fixtures::sidebar())));
    press(&mut s, "<Down><Down><Enter><Up><Enter>");
    assert_snapshot!(render(&s));
}

/// Labels are checked and unchecked, then applied at once: what's added
/// and what's removed, and the page waits until GitHub shows both.
#[test]
fn labels_are_checked_then_applied() {
    let mut s = issue_with(|_| {});
    press(&mut s, ",l");
    let key = DataKey::Sidebar(ghtui(), 14);
    fetched(&mut s, key, Data::Sidebar(Box::new(fixtures::sidebar())));
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
    assert_eq!(sent(&cmds), (&Change::Edit(labels), &By::Edit(issue())));
    says(&s, "Changing the labels");
    answer(&mut s, &cmds, Ok(()));
    deliver(&mut s, |_| {});
    assert!(s.awaiting.is_some());
    let label = |name: &str| crate::api::model::Label {
        name: name.into(),
        color: "0075ca".into(),
    };
    deliver(&mut s, |d| d.labels = vec![label("bug")].into());
    assert!(s.awaiting.is_some(), "documentation isn't there yet");
    deliver(&mut s, |d| {
        d.labels = vec![label("bug"), label("documentation")].into();
    });
    assert!(s.awaiting.is_none());

    // Unchanged, nothing is sent.
    press(&mut s, ",l");
    assert!(press(&mut s, "<C-s>").is_empty());
    says(&s, "Nothing changed");
}

/// Reviewers are people and teams: asking replaces who's asked, so
/// everyone still asked is sent; the conversation shows who's asked.
#[test]
fn reviewers_include_teams() {
    let mut s = with_pr(Mode::Dark);
    press(&mut s, ",q");
    let key = DataKey::Sidebar(ghtui(), 12);
    fetched(&mut s, key, Data::Sidebar(Box::new(fixtures::sidebar())));
    let rows: Vec<String> = match &s.overlay {
        Some(Overlay::Picker(p)) => s.picker_rows(p).into_iter().map(|(r, _)| r.label).collect(),
        _ => panic!(",q opens the reviewers"),
    };
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
    let Change::Edit(Edit::Reviewers { users, teams, .. }) = sent(&cmds).0 else {
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
    fetched(
        &mut s,
        DataKey::PrActivity(pr()),
        Data::PrActivity(Box::new(activity)),
    );
    assert!(s.awaiting.is_none());
}

/// One milestone, or none; and a title in a one-line box.
#[test]
fn milestone_and_title() {
    let mut s = issue_with(|_| {});
    press(&mut s, ",M");
    let key = DataKey::Sidebar(ghtui(), 14);
    fetched(&mut s, key, Data::Sidebar(Box::new(fixtures::sidebar())));
    // No milestone, Links (current), 1.0.
    let cmds = press(&mut s, "<Down><Down><Enter>");
    let Change::Edit(Edit::Milestone { milestone, .. }) = sent(&cmds).0 else {
        panic!("{cmds:?}")
    };
    assert_eq!(milestone.as_ref().map(|m| m.id.as_str()), Some("M_4"));
    answer(&mut s, &cmds, Ok(()));
    deliver(&mut s, |d| {
        if let Some(m) = &mut d.milestone {
            m.title = "1.0".into();
        }
    });
    assert!(s.awaiting.is_none());

    press(&mut s, ",T");
    let cmds = press(&mut s, "!<Enter>");
    let title = Edit::Title {
        pr: false,
        id: NodeId::new("I_14"),
        title: "Pages wrap at the terminal width!".into(),
    };
    assert_eq!(sent(&cmds), (&Change::Edit(title), &By::Edit(issue())));
}
