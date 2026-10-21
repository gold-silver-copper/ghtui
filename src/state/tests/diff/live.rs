use super::*;
use crate::api::Polled;
use crate::live::{Ask, Probe};

/// The ask the watch's next timer sends, and how long it waits first.
fn next_ask(s: &mut State) -> (Vec<Ask>, u64) {
    let (cmds, ms) = next_watch(s);
    match &cmds[..] {
        [Cmd::Api(Api::Poll { asks, .. })] => (asks.clone(), ms),
        _ => panic!("{cmds:?}"),
    }
}

/// While Files changed is on screen, ghtui asks GitHub whether the pull
/// request changed, with the ETags of its last answers: new review
/// comments fetch the threads again, and what's new is marked and said.
#[test]
fn polling_asks_with_etags_and_marks_new_threads() {
    let (mut s, pr) = diff_state(120);
    s.viewer = Some("octocat".into());
    let old = thread("old", Some(1), false, false);
    diff_msg(&mut s, &pr, DiffMsg::ThreadsLoaded(Ok(vec![old.clone()])));

    let (asks, wait) = next_ask(&mut s);
    assert_eq!(wait, crate::live::FIRST_MS);
    let probes: Vec<&Probe> = asks.iter().map(|a| &a.probe).collect();
    assert_eq!(probes, [&Probe::Pull, &Probe::ReviewComments]);
    assert!(asks.iter().all(|a| a.etag.is_none()));
    // The first answers are only what's there.
    let first = vec![
        (Probe::Pull, changed("\"p1\"", r#"{"head":{"sha":"abc"}}"#)),
        (Probe::ReviewComments, changed("\"c1\"", "[]")),
    ];
    let cmds = update(&mut s, Msg::Polled(pr.clone(), first));
    assert!(cmds.is_empty(), "{cmds:?}");

    // Nothing changed yet: the next ask waits longer.
    let (asks, wait) = next_ask(&mut s);
    assert!(wait > crate::live::FIRST_MS);
    let etag = |probe: &Probe| asks.iter().find(|a| a.probe == *probe)?.etag.clone();
    assert_eq!(etag(&Probe::Pull).as_deref(), Some("\"p1\""));
    assert_eq!(etag(&Probe::ReviewComments).as_deref(), Some("\"c1\""));
    assert!(
        asks.iter()
            .any(|a| a.probe == Probe::CheckRuns("abc".into()))
    );
    let answers = vec![
        (Probe::Pull, Ok(Polled::Same)),
        (Probe::ReviewComments, changed("\"c2\"", "[]")),
    ];
    let cmds = update(&mut s, Msg::Polled(pr.clone(), answers));
    assert!(
        cmds.contains(&Cmd::Api(Api::FetchThreads(pr.clone()))),
        "{cmds:?}"
    );

    let new = thread("new", Some(14), false, false);
    diff_msg(&mut s, &pr, DiffMsg::ThreadsLoaded(Ok(vec![old, new])));
    let anns = s.diffs[&DiffOf::Pr(pr.clone())].doc.annotations();
    let marked = |id: &str| {
        let ann = anns
            .iter()
            .find(|a| a.key == AnnotationKey::Thread(NodeId::new(id)));
        ann.is_some_and(|a| a.new && a.comments.iter().all(|c| c.new))
    };
    assert!(marked("new") && !marked("old"));
    says(&s, "New on o/r#7: 1 thread");
}

/// A comment that came in between fetching the threads and the first
/// ask isn't taken for what was there: the threads are fetched again.
#[test]
fn a_comment_before_the_first_answer_is_fetched() {
    let (mut s, pr) = diff_state(120);
    diff_msg(
        &mut s,
        &pr,
        DiffMsg::ThreadsLoaded(Ok(vec![thread("old", Some(1), false, false)])),
    );
    let _ = next_ask(&mut s);
    let seen = vec![(
        Probe::ReviewComments,
        changed("\"c1\"", r#"[{"node_id":"old-c"}]"#),
    )];
    assert!(update(&mut s, Msg::Polled(pr.clone(), seen)).is_empty());
    let (mut s, pr) = diff_state(120);
    diff_msg(
        &mut s,
        &pr,
        DiffMsg::ThreadsLoaded(Ok(vec![thread("old", Some(1), false, false)])),
    );
    let _ = next_ask(&mut s);
    let came = vec![(
        Probe::ReviewComments,
        changed("\"c1\"", r#"[{"node_id":"new-c"}]"#),
    )];
    let cmds = update(&mut s, Msg::Polled(pr.clone(), came));
    assert!(cmds.contains(&Cmd::Api(Api::FetchThreads(pr))), "{cmds:?}");
}

/// Leaving the pull request stops the asking.
#[test]
fn nothing_else_is_polled() {
    let (mut s, _) = diff_state(120);
    next_ask(&mut s);
    press(&mut s, "h");
    let scheduled = timers(&mut s)
        .into_iter()
        .any(|cmd| matches!(cmd, Cmd::Timer(Timer::Watch(_), _)));
    assert!(!scheduled && s.watch.is_none());
}
