//! Offline: the header says so, a comment GitHub can't be reached for
//! waits in the queue and shows on its page, and what GitHub refused is
//! sent again, copied or thrown away from there.

use super::*;
use crate::offline::queue::{Entry, Op, Status, Told, Write};
use crate::offline::{Cmd as Off, Msg as OffMsg};
use crate::route::app;

fn offline(state: &mut State, msg: OffMsg) -> Vec<Cmd> {
    update(state, Msg::Offline(msg))
}

#[test]
fn a_comment_github_can_t_be_reached_for_waits_on_its_page_and_goes_when_it_s_back() {
    let mut state = with_issue();
    let cmds = offline(&mut state, OffMsg::Reach(Some(100)));
    assert_eq!(cmds, [Cmd::Timer(Timer::Probe, 2_000)]);
    assert_eq!(crate::offline::header(&state), ["⚠ Offline"]);

    press(&mut state, "c");
    press(&mut state, "Thanks!");
    let sent = press(&mut state, "<C-s>");
    let offline_err = Err(ApiError::Offline("not sent while offline".into()));
    let cmds = answer(&mut state, &sent, offline_err);
    assert!(state.overlay.is_none(), "the composer is done with it");
    says(&state, "a comment is queued");
    let [Cmd::Offline(Off::Queue(Op::Add(entry)))] = cmds.as_slice() else {
        panic!("{cmds:?}")
    };
    let comment = Write::Change(Change::Comment {
        subject: NodeId::new("I_14"),
        body: "Thanks!".into(),
    });
    assert_eq!(entry.write, comment);
    assert_eq!(entry.page, issue(14).url());

    // The queue says it has it: the page and the header show it.
    let queued = Entry {
        id: 1,
        ..*entry.clone()
    };
    offline(&mut state, OffMsg::Queue(Told::All(vec![queued.clone()])));
    let shown = page_text(&state);
    assert!(shown.contains("Queued for GitHub  1"), "{shown}");
    assert!(shown.contains("Thanks!"), "{shown}");
    assert_eq!(crate::offline::header(&state), ["⚠ Offline", "1 queued"]);

    // Still away: asked again, later.
    let cmds = update(&mut state, Msg::Timer(Timer::Probe));
    assert_eq!(cmds, [Cmd::Offline(Off::Probe)]);
    let cmds = offline(&mut state, OffMsg::Probed(false));
    assert_eq!(cmds, [Cmd::Timer(Timer::Probe, 4_000)]);

    // Back: the queue's sent, and the page follows GitHub once it's in.
    let cmds = offline(&mut state, OffMsg::Reach(None));
    says(&state, "GitHub is back");
    assert!(
        cmds.contains(&Cmd::Offline(Off::Queue(Op::Flush))),
        "{cmds:?}"
    );
    offline(&mut state, OffMsg::Queue(Told::All(Vec::new())));
    let cmds = offline(&mut state, OffMsg::Queue(Told::Taken(Box::new(queued))));
    says(&state, "Sent to GitHub: A comment");
    let refetch = Cmd::Api(Api::Fetch {
        key: DataKey::Issue(ghtui(), 14),
        cached: false,
    });
    assert!(cmds.contains(&refetch), "{cmds:?}");
    assert!(crate::offline::header(&state).is_empty());
}

#[test]
fn what_can_t_wait_isn_t_queued() {
    let mut state = with_issue();
    let merge = Change::Merge {
        pr: NodeId::new("PR_1"),
        method: crate::api::change::MergeMethod::Squash,
        head: "abc".into(),
    };
    let err = Err(ApiError::Offline("not sent while offline".into()));
    let sent = [fixtures::change(
        merge,
        act::By::Act(act::Subject::Issue(ghtui(), 14)),
    )];
    let cmds = answer(&mut state, &sent, err);
    assert!(cmds.is_empty(), "{cmds:?}");
    warns(&state, "can't be reached");
}

#[test]
fn a_refused_write_is_sent_again_copied_or_thrown_away_from_its_row() {
    let mut state = with_issue();
    let refused = Entry {
        id: 3,
        at: 0,
        write: Write::Change(Change::Comment {
            subject: NodeId::new("I_14"),
            body: "Too late".into(),
        }),
        page: issue(14).url(),
        title: "gold-silver-copper/ghtui#14".into(),
        status: Status::Refused("the conversation is locked".into()),
    };
    offline(&mut state, OffMsg::Queue(Told::All(vec![refused.clone()])));
    assert_eq!(crate::offline::header(&state), ["⚠ 1 not sent"]);
    assert!(page_text(&state).contains("GitHub said: the conversation is locked"));
    let _ = act(&mut state, Action::Offline);
    assert_eq!(route(&state), Route::App(app::Offline));
    while page(&state).selected_link() != Some(&crate::ui::page::Link::Queued(3)) {
        press(&mut state, "j");
    }
    assert_eq!(
        press(&mut state, ",R"),
        [Cmd::Offline(Off::Queue(Op::Retry(3)))]
    );
    assert_eq!(press(&mut state, "y"), [Cmd::Copy("Too late".into())]);
    assert_eq!(
        press(&mut state, "<Del>"),
        [Cmd::Offline(Off::Queue(Op::Discard(3)))]
    );
    offline(&mut state, OffMsg::Queue(Told::All(Vec::new())));
    let cmds = press(&mut state, "<C-z>");
    assert_eq!(cmds, [Cmd::Offline(Off::Queue(Op::Add(Box::new(refused))))]);
}

#[test]
fn refreshing_while_github_is_away_asks_whether_it_s_back() {
    let mut state = with_issue();
    offline(&mut state, OffMsg::Reach(Some(100)));
    let cmds = press(&mut state, "r");
    assert!(cmds.contains(&Cmd::Offline(Off::Probe)), "{cmds:?}");
}
