use super::*;

#[test]
fn moves_are_detected_when_every_file_is_diffed() {
    let (mut s, pr) = diff_state(120);
    // The fixture has a loading file; finishing it triggers detection.
    let pending = s.diffs[&DiffOf::Pr(pr.clone())]
        .doc
        .files()
        .iter()
        .position(|f| f.diff.is_none())
        .unwrap();
    let job = s.diffs[&DiffOf::Pr(pr.clone())].job;
    let cmds = diff_msg(
        &mut s,
        &pr,
        DiffMsg::Job(
            job,
            JobMsg::File(
                pending,
                Arc::new(FileDiff::compute(
                    "zz/pending.rs",
                    Some(b"a\n"),
                    Some(b"b\n"),
                )),
            ),
        ),
    );
    assert!(cmds.iter().any(
        |c| matches!(c, Cmd::Git(Git::DetectMoves(m)) if matches!(m.get(), (p, j, files) if *p == DiffOf::Pr(pr.clone()) && *j == job && files.len() == 8))
    ));
    // Only once.
    let again = diff_msg(&mut s, &pr, DiffMsg::Job(job, JobMsg::Moves(Vec::new())));
    assert!(
        !again
            .iter()
            .any(|c| matches!(c, Cmd::Git(Git::DetectMoves(..))))
    );
    assert!(act(&mut s, Action::JumpMove).is_empty());
    says(&s, "moved");
}

/// After a restart (a refresh, another commit range), the old
/// job's results don't land in the new diff.
#[test]
fn a_restarted_job_ignores_the_old_one() {
    let (mut s, pr) = diff_state(120);
    let old = s.diffs[&DiffOf::Pr(pr.clone())].job;
    s.diffs
        .get_mut(&DiffOf::Pr(pr.clone()))
        .unwrap()
        .restart(None);
    let stale = Arc::new(FileDiff::compute("a.rs", Some(b"a\n"), Some(b"b\n")));
    diff_msg(&mut s, &pr, DiffMsg::Job(old, JobMsg::File(0, stale)));
    diff_msg(
        &mut s,
        &pr,
        DiffMsg::Job(old, JobMsg::Failed(Failure::msg("old"))),
    );
    let diff = &s.diffs[&DiffOf::Pr(pr.clone())];
    assert_ne!(diff.job, old);
    assert!(diff.doc.is_empty());
    assert_eq!(diff.error, None);
    // The current job's messages still count.
    let job = diff.job;
    diff_msg(
        &mut s,
        &pr,
        DiffMsg::Job(job, JobMsg::Failed(Failure::msg("new"))),
    );
    assert_eq!(
        s.diffs[&DiffOf::Pr(pr.clone())].error.as_deref(),
        Some("new")
    );
}

#[test]
fn since_review_asks_github_then_compares() {
    let (mut s, pr) = diff_state(120);
    s.viewer = Some("me".into());
    let cmds = act(&mut s, Action::ToggleSinceReview);
    assert!(matches!(&cmds[..], [Cmd::Api(Api::FetchLastReview { login, .. })] if login == "me"));
    let cmds = diff_msg(&mut s, &pr, DiffMsg::LastReview(Ok(Some("a".repeat(40)))));
    assert!(matches!(&cmds[..], [Cmd::Git(Git::SinceReview(j))] if j.get().2 == "a".repeat(40)));

    // Nothing in the old diff matched: everything is new.
    let job = s.diffs[&DiffOf::Pr(pr.clone())].job;
    diff_msg(
        &mut s,
        &pr,
        DiffMsg::Job(job, JobMsg::Since("a".repeat(40), Ok(Default::default()))),
    );
    assert!(s.diffs[&DiffOf::Pr(pr.clone())].doc.since_active());
    assert_eq!(s.chrome().tabs[3].0.label, "Files · since your review");
    // Toggling back needs no new lookups.
    assert!(
        act(&mut s, Action::ToggleSinceReview)
            .iter()
            .all(|c| matches!(c, Cmd::Git(Git::Prioritize(..))))
    );
    assert!(!s.diffs[&DiffOf::Pr(pr.clone())].doc.since_active());
}

#[test]
fn since_review_finishing_after_you_left_is_dropped() {
    let (mut s, pr) = diff_state(120);
    s.viewer = Some("me".into());
    act(&mut s, Action::ToggleSinceReview);
    diff_msg(&mut s, &pr, DiffMsg::LastReview(Ok(Some("old".into()))));
    let _ = s.back();
    s.notices.dismiss();
    let job = s.diffs[&DiffOf::Pr(pr.clone())].job;
    diff_msg(
        &mut s,
        &pr,
        DiffMsg::Job(job, JobMsg::Since("old".into(), Ok(Default::default()))),
    );
    assert!(!s.diffs[&DiffOf::Pr(pr.clone())].doc.since_active());
    assert_eq!(s.notices.shown(), None);
}

#[test]
fn since_review_without_a_review_says_so() {
    let (mut s, pr) = diff_state(120);
    s.viewer = Some("me".into());
    act(&mut s, Action::ToggleSinceReview);
    diff_msg(&mut s, &pr, DiffMsg::LastReview(Ok(None)));
    says(&s, "haven't reviewed");
    // Reviewing the current head means nothing's new.
    s.diffs
        .get_mut(&DiffOf::Pr(pr.clone()))
        .unwrap()
        .since_requested = true;
    diff_msg(&mut s, &pr, DiffMsg::LastReview(Ok(Some("e".repeat(40)))));
    says(&s, "current head");
}

#[test]
fn a_failed_review_lookup_isnt_taken_for_no_review() {
    let (mut s, pr) = diff_state(120);
    s.viewer = Some("me".into());
    act(&mut s, Action::ToggleSinceReview);
    diff_msg(
        &mut s,
        &pr,
        DiffMsg::LastReview(Err(ApiError::Network("offline".into()))),
    );
    assert!(
        !matches!(s.notices.shown(), Some(Notice::Info(m)) if m.contains("haven't reviewed")),
        "{:?}",
        s.notices.shown()
    );
    // Asking again asks GitHub again.
    let cmds = act(&mut s, Action::ToggleSinceReview);
    assert!(
        matches!(&cmds[..], [Cmd::Api(Api::FetchLastReview { .. })]),
        "{cmds:?}"
    );
}

/// Going by the review made here when GitHub can't be asked
/// says so: GitHub may know a later one.
#[test]
fn a_failed_review_lookup_says_so_beside_the_local_one() {
    let (mut s, pr) = diff_state(120);
    s.viewer = Some("me".into());
    s.diffs
        .get_mut(&DiffOf::Pr(pr.clone()))
        .unwrap()
        .edit(|i| i.review.last_reviewed_head = Some("old".into()));
    act(&mut s, Action::ToggleSinceReview);
    let cmds = diff_msg(
        &mut s,
        &pr,
        DiffMsg::LastReview(Err(ApiError::Network("offline".into()))),
    );
    assert!(
        matches!(&cmds[..], [Cmd::Git(Git::SinceReview(j))] if j.get().2 == "old"),
        "{cmds:?}"
    );
    says(&s, "offline");
    says(&s, "made here");
}

#[test]
fn commit_picker_loads_a_range_and_blocks_comments() {
    let (mut s, pr) = diff_state(120);
    // `ghtui pr` would have loaded metadata; the picker needs the base.
    crate::fixtures::pr_cached(&mut s, &pr, crate::fixtures::pr_detail());
    assert!(matches!(
        &act(&mut s, Action::PickCommits)[..],
        [Cmd::Git(Git::ListCommits(_))]
    ));
    diff_msg(
        &mut s,
        &pr,
        DiffMsg::CommitsListed(Ok(vec![
            crate::git::repo::Commit {
                oid: Oid::parse(&"a".repeat(40)).unwrap(),
                subject: "first".into(),
            },
            crate::git::repo::Commit {
                oid: Oid::parse(&"b".repeat(40)).unwrap(),
                subject: "second".into(),
            },
        ])),
    );
    assert!(matches!(s.overlay, Some(Overlay::Picker(_))));
    // Mark the first commit, select the second: a range.
    press(&mut s, "jj<Space>j");
    let cmds = press(&mut s, "<Enter>");
    let [
        Cmd::Git(Git::LoadDiff {
            range: Some((from, to)),
            ..
        }),
    ] = &cmds[..]
    else {
        panic!("{cmds:?}")
    };
    assert_eq!(from, &format!("{}^", "a".repeat(40)));
    assert_eq!(to, &"b".repeat(40));
    assert_eq!(s.chrome().tabs[3].0.label, "Files · aaaaaaa..bbbbbbb");
    // Commenting needs the whole PR.
    press(&mut s, "c");
    assert!(s.overlay.is_none());
    warns(&s, "All changes");
    // Back to everything.
    act(&mut s, Action::PickCommits);
    let cmds = press(&mut s, "<Enter>");
    assert!(matches!(
        &cmds[..],
        [Cmd::Git(Git::LoadDiff { range: None, .. })]
    ));
}
