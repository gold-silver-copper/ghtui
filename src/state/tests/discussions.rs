use crate::act::{By, Subject};
use crate::api::browse::{Comment, DiscussionDetail, DiscussionsOf};
use crate::api::discussion::Discuss;
use crate::api::edit::{Edit, ReactionContent};

use super::*;
use crate::fixtures::{self};
use crate::ui::page::Link;

fn about() -> Subject {
    Subject::Discussion(DiscussionsOf::Repo(ghtui()), 14603)
}

/// The discussion as GitHub has it, changed by `f`.
fn deliver(s: &mut State, f: impl FnOnce(&mut DiscussionDetail)) -> Vec<Cmd> {
    let mut d = fixtures::discussion();
    f(&mut d);
    let key = DataKey::Discussion(DiscussionsOf::Repo(ghtui()), 14603);
    fetched(s, key, Data::Discussion(Box::new(d)))
}

/// Selects the comment `author` wrote that starts with `text`.
fn select(s: &mut State, author: &str, text: &str) {
    let on = |s: &State| match s.screen() {
        Screen::Page(p) => matches!(p.selected_link(),
            Some(Link::Quote { author: a, body, .. }) if a == author && body.starts_with(text)),
        Screen::Diff(_) => false,
    };
    press(s, "gg");
    for _ in 0..12 {
        if on(s) {
            return;
        }
        press(s, "}");
    }
    panic!("{author}'s “{text}” isn't selected");
}

fn sent(cmds: &[Cmd]) -> (&Change, &By) {
    fixtures::sent_change(cmds)
}

/// `c` with no comment selected writes an answer (it's a Q&A); on a
/// comment or one of its replies, a reply in that comment's thread. The
/// page follows GitHub until it shows what you wrote.
#[test]
fn c_answers_or_replies_in_the_thread() {
    let mut s = with_discussion();
    press(&mut s, "c");
    let Some(Overlay::Compose(c)) = &s.overlay else {
        panic!("no composer");
    };
    assert_eq!(
        c.target.title(&s),
        "Answer on “How do I review a PR offline?”"
    );
    let cmds = press(&mut s, "Use git<C-s>");
    let comment = Change::Discussion(Discuss::Comment {
        discussion: NodeId::new("D_14603"),
        reply_to: None,
        body: "Use git".into(),
    });
    assert_eq!(sent(&cmds), (&comment, &By::Edit(about())));
    answer(&mut s, &cmds, Ok(()));
    says(&s, "Comment posted");
    assert!(s.following.is_some(), "until GitHub shows it");
    deliver(&mut s, |d| {
        let mut mine = d.comments.items.last().cloned().unwrap();
        mine.comment = Comment {
            author: "octocat".into(),
            body: "Use git".into(),
            ..mine.comment
        };
        d.comments.items.push(mine);
    });
    assert!(s.following.is_none());

    // On a reply, the reply goes in its comment's thread.
    for (author, text) in [("hubot", "Fetch"), ("octocat", "That works")] {
        select(&mut s, author, text);
        press(&mut s, "c");
        let cmds = press(&mut s, "Thanks<C-s>");
        let reply = Change::Discussion(Discuss::Comment {
            discussion: NodeId::new("D_14603"),
            reply_to: Some(NodeId::new("DC_18765024")),
            body: "Thanks".into(),
        });
        assert_eq!(sent(&cmds).0, &reply, "on {author}'s");
        press(&mut s, "<Esc><Esc>");
        s.overlay = None;
    }
}

/// `,!` marks a comment the answer, or unmarks the one that is; a reply
/// can't be one, and GitHub's word on who may decides.
#[test]
fn the_answer_is_marked_and_unmarked() {
    let mut s = with_discussion();
    select(&mut s, "monalisa", "Drafts");
    let cmds = press(&mut s, ",!");
    let mark = Change::Discussion(Discuss::Answer {
        comment: NodeId::new("DC_18765100"),
        on: true,
    });
    assert_eq!(sent(&cmds), (&mark, &By::Act(about())));
    answer(&mut s, &cmds, Ok(()));
    says(&s, "Marked as the answer");
    deliver(&mut s, |d| {
        let monalisa = d.comments.items.last().cloned().unwrap();
        d.answer = Some(Box::new(monalisa));
    });
    assert!(s.following.is_none());

    let mut s = with_discussion();
    select(&mut s, "hubot", "Fetch");
    let cmds = press(&mut s, ",!");
    let unmark = Change::Discussion(Discuss::Answer {
        comment: NodeId::new("DC_18765024"),
        on: false,
    });
    assert_eq!(sent(&cmds).0, &unmark);

    select(&mut s, "octocat", "That works");
    assert!(press(&mut s, ",!").is_empty());
    says(&s, "A reply can't be the answer, only a comment");
    deliver(&mut s, |d| d.answerable = false);
    select(&mut s, "monalisa", "Drafts");
    press(&mut s, ",!");
    says(&s, "Q&A doesn't take answers");
}

/// `,+` reacts to the selected comment (or the post), and the page
/// follows GitHub until it shows the reaction; `,e` and `,D` go by what
/// GitHub lets you do.
#[test]
fn comments_are_reacted_to_edited_and_deleted() {
    let mut s = with_discussion();
    select(&mut s, "monalisa", "Drafts");
    press(&mut s, ",+");
    assert!(matches!(s.overlay, Some(Overlay::Picker(_))));
    let cmds = press(&mut s, "<Enter>");
    let react = Change::Edit(Edit::React {
        subject: NodeId::new("DC_18765100"),
        content: ReactionContent::ThumbsUp,
        on: true,
    });
    assert_eq!(sent(&cmds), (&react, &By::Edit(about())));
    answer(&mut s, &cmds, Ok(()));
    assert!(s.following.is_some());
    deliver(&mut s, |d| {
        if let Some(e) = d
            .comments
            .items
            .last_mut()
            .and_then(|c| c.comment.editable.as_mut())
        {
            e.reactions.push(crate::api::edit::Reaction {
                content: ReactionContent::ThumbsUp,
                count: 1,
                mine: true,
            });
        }
    });
    assert!(s.following.is_none());

    press(&mut s, ",e");
    says(&s, "GitHub doesn't let you edit monalisa's comment");
    select(&mut s, "octocat", "That works");
    press(&mut s, ",e");
    let Some(Overlay::Compose(c)) = &s.overlay else {
        panic!("no composer");
    };
    assert_eq!(c.text(), "That works, thanks!");
    s.overlay = None;
    press(&mut s, ",D");
    let Some(Overlay::Ask(c)) = &s.overlay else {
        panic!("not asked");
    };
    assert_eq!(c.title, "Delete octocat's reply?");
    let cmds = press(&mut s, "<Enter>");
    let delete = Change::Discussion(Discuss::Delete {
        id: NodeId::new("DC_18765030"),
    });
    assert_eq!(sent(&cmds), (&delete, &By::Confirm(about())));
}
