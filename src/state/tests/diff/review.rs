use super::*;

fn to_line(s: &mut State, text: &str) {
    press(s, "/");
    press(s, &format!("{text}<Enter>"));
}

#[test]
fn threads_show_navigate_and_resolve() {
    let (mut s, pr) = diff_state(120);
    diff_msg(
        &mut s,
        &pr,
        DiffMsg::ThreadsLoaded(Ok(vec![
            thread("done", Some(1), true, false),
            thread("open", Some(14), false, false),
        ])),
    );
    assert_eq!(s.diffs[&DiffOf::Pr(pr.clone())].doc.annotations().len(), 2);
    press(&mut s, "gg");
    act(&mut s, Action::NextThread);
    let at = s.diffs[&DiffOf::Pr(pr.clone())]
        .doc
        .annotation_at(screen(&s).cursor)
        .unwrap();
    assert_eq!(
        s.diffs[&DiffOf::Pr(pr.clone())].doc.annotations()[at as usize].key,
        AnnotationKey::Thread(NodeId::new("open"))
    );
    // The status bar leads with what fits a thread.
    let hints = s.key_hints();
    assert_eq!(
        hints.get(1..3),
        Some(
            &[
                ("c".into(), "reply".into()),
                (",t".into(), "resolve".into())
            ][..]
        )
    );

    // Resolve: optimistic, rolled back on failure.
    let cmds = act(&mut s, Action::ResolveThread);
    assert!(matches!(
        &cmds[..],
        [Cmd::Api(Api::SetResolved { resolved: true, .. }), ..]
    ));
    assert!(s.diffs[&DiffOf::Pr(pr.clone())].inputs().thread_list()[1].resolved);
    diff_msg(
        &mut s,
        &pr,
        DiffMsg::ResolvedSet {
            thread_id: NodeId::new("open"),
            resolved: true,
            result: Err(ApiError::Network("down".into())),
        },
    );
    assert!(!s.diffs[&DiffOf::Pr(pr.clone())].inputs().thread_list()[1].resolved);
    warns(&s, "");
}

#[test]
fn comment_draft_edit_and_delete() {
    let (mut s, pr) = diff_state(120);
    to_line(&mut s, "origin");
    press(&mut s, "c");
    let compose = overlay!(s, Compose);
    assert!(matches!(compose.target, ComposeTarget::Line { .. }));
    press(&mut s, "Use a constant");
    let cmds = press(&mut s, "<C-s>");
    assert!(s.overlay.is_none());
    let Some(Cmd::SaveReview(_, review)) = cmds.first() else {
        panic!("{cmds:?}")
    };
    assert_eq!(review.pending.len(), 1);
    let draft = &review.pending[0];
    assert_eq!(
        (draft.line, draft.commit.as_str()),
        (Some(14), &*"e".repeat(40))
    );
    assert!(
        s.diffs[&DiffOf::Pr(pr.clone())]
            .doc
            .annotations()
            .iter()
            .any(crate::ui::annotations::Annotation::is_draft)
    );

    // The cursor is on the commented line; Enter edits the draft.
    press(&mut s, "<Enter>");
    assert!(
        matches!(&s.overlay, Some(Overlay::Compose(c)) if matches!(c.target, ComposeTarget::Draft { .. }))
    );
    press(&mut s, "!");
    press(&mut s, "<C-s>");
    assert_eq!(
        s.diffs[&DiffOf::Pr(pr.clone())].inputs().review.pending[0].body,
        "Use a constant!"
    );

    let cmds = press(&mut s, "<Delete>");
    assert!(matches!(&cmds[..], [Cmd::SaveReview(_, r)] if r.pending.is_empty()));
    says(&s, "ctrl-z");

    // Deleting can be undone.
    let cmds = press(&mut s, "<C-z>");
    assert!(matches!(&cmds[..], [Cmd::SaveReview(_, r)] if r.pending[0].body == "Use a constant!"));
    assert!(
        s.diffs[&DiffOf::Pr(pr)]
            .doc
            .annotations()
            .iter()
            .any(crate::ui::annotations::Annotation::is_draft)
    );
    press(&mut s, "<C-z>");
    says(&s, "No deleted");
}

#[test]
fn lines_outside_githubs_diff_become_file_comments() {
    let (mut s, pr) = diff_state(120);
    // GitHub's diff only covers line 1.
    diff_msg(
        &mut s,
        &pr,
        DiffMsg::PatchesLoaded(Ok(vec![PatchFile {
            filename: "src/point.rs".into(),
            previous_filename: None,
            patch: Some("@@ -1 +1 @@\n-a\n+b".into()),
        }])),
    );
    to_line(&mut s, "origin");
    press(&mut s, "c");
    let compose = overlay!(s, Compose);
    assert!(matches!(
        &compose.target,
        ComposeTarget::File {
            reason: Some(_),
            ..
        }
    ));
}

#[test]
fn selections_across_hunks_are_refused() {
    let (mut s, pr) = diff_state(120);
    diff_msg(
        &mut s,
        &pr,
        DiffMsg::PatchesLoaded(Ok(vec![PatchFile {
            filename: "src/point.rs".into(),
            previous_filename: None,
            patch: Some("@@ -1,6 +1,6 @@\n@@ -10,4 +10,8 @@".into()),
        }])),
    );
    to_line(&mut s, "A point in 2D");
    press(&mut s, "x");
    to_line(&mut s, "origin");
    press(&mut s, "c");
    assert!(s.overlay.is_none());
    warns(&s, "one hunk");
}

#[test]
fn cancelling_a_comment_keeps_the_selection() {
    let (mut s, _) = diff_state(120);
    to_line(&mut s, "A point in 2D");
    press(&mut s, "Vjc<Esc>");
    assert!(s.overlay.is_none());
    assert!(screen(&s).selection.is_some(), "the selection survives");
    // Again, and the comment covers the same lines.
    press(&mut s, "c");
    let compose = overlay!(s, Compose);
    assert!(matches!(&compose.target, ComposeTarget::Line { start, end, .. } if start != end));
    press(&mut s, "Fine<C-s>");
    assert!(screen(&s).selection.is_none(), "an added comment ends it");
}

#[test]
fn typed_text_survives_one_press_of_each_discard_key() {
    let (mut s, _) = diff_state(120);
    to_line(&mut s, "origin");
    press(&mut s, "cnit<C-c>");
    assert!(!s.quit);
    assert!(matches!(&s.overlay, Some(Overlay::Compose(c)) if c.text() == "nit"));
    press(&mut s, "<C-c>");
    assert!(s.quit);

    // Each key warns for itself: esc then ctrl-c still asks.
    let (mut s, _) = diff_state(120);
    to_line(&mut s, "origin");
    press(&mut s, "cnit<Esc><C-c>");
    assert!(!s.quit);
    assert!(matches!(&s.overlay, Some(Overlay::Compose(c)) if c.text() == "nit"));

    let (mut s, _) = diff_state(120);
    press(&mut s, ",slooks good<Esc>");
    assert!(matches!(&s.overlay, Some(Overlay::Submit(d)) if d.input.lines() == ["looks good"]));
    press(&mut s, "!<Esc><Esc>");
    assert!(s.overlay.is_none());
}

#[test]
fn submit_with_rejection_then_fix_and_resubmit() {
    let (mut s, pr) = diff_state(120);
    to_line(&mut s, "origin");
    press(&mut s, "c");
    press(&mut s, "nit");
    press(&mut s, "<C-s>");
    let id = s.diffs[&DiffOf::Pr(pr.clone())].inputs().review.pending[0].id;

    press(&mut s, ",s");
    assert!(matches!(s.overlay, Some(Overlay::Submit(_))));
    press(&mut s, "<Tab>");
    let cmds = press(&mut s, "<C-s>");
    let Some(Cmd::Api(Api::SubmitReview { drafts, event, .. })) = cmds.first() else {
        panic!("{cmds:?}")
    };
    assert_eq!((drafts.len(), *event), (1, ReviewEvent::Approve));

    diff_msg(
        &mut s,
        &pr,
        DiffMsg::ReviewSubmitted(SubmitOutcome {
            rejected: vec![(id, "line must be part of the diff".into())],
            ..SubmitOutcome::default()
        }),
    );
    assert!(matches!(&s.overlay, Some(Overlay::Submit(d)) if d.error.is_some() && !d.sending));
    assert_eq!(
        s.diffs[&DiffOf::Pr(pr.clone())].inputs().review.pending[0]
            .error
            .as_deref(),
        Some("line must be part of the diff")
    );

    // Make it a file comment and submit again.
    press(&mut s, "<Esc>");
    act(&mut s, Action::FileComment);
    assert_eq!(
        s.diffs[&DiffOf::Pr(pr.clone())].inputs().review.pending[0].line,
        None
    );
    press(&mut s, ",s");
    press(&mut s, "<C-s>");
    diff_msg(
        &mut s,
        &pr,
        DiffMsg::ReviewSubmitted(SubmitOutcome {
            accepted: vec![id],
            submitted: true,
            ..SubmitOutcome::default()
        }),
    );
    assert!(s.overlay.is_none());
    assert!(
        s.diffs[&DiffOf::Pr(pr.clone())]
            .inputs()
            .review
            .pending
            .is_empty()
    );
    assert_eq!(
        s.diffs[&DiffOf::Pr(pr.clone())]
            .inputs()
            .review
            .last_reviewed_head
            .as_deref(),
        Some(&*"e".repeat(40))
    );
}

#[test]
fn request_changes_needs_a_summary() {
    let (mut s, _) = diff_state(120);
    press(&mut s, ",s");
    press(&mut s, "<Tab><Tab>");
    assert!(press(&mut s, "<C-s>").is_empty());
    assert!(matches!(&s.overlay, Some(Overlay::Submit(d)) if d.error.is_some()));
}

#[test]
fn replies_post_right_away() {
    let (mut s, pr) = diff_state(120);
    diff_msg(
        &mut s,
        &pr,
        DiffMsg::ThreadsLoaded(Ok(vec![thread("t", Some(14), false, false)])),
    );
    press(&mut s, "gg");
    act(&mut s, Action::NextThread);
    press(&mut s, "c");
    press(&mut s, "Fixed");
    let cmds = press(&mut s, "<C-s>");
    assert!(
        matches!(fixtures::sent_change(&cmds), (Change::Reply { body, .. }, _) if body == "Fixed")
    );
    answer(&mut s, &cmds, Err(ApiError::Network("down".into())));
    assert!(
        matches!(&s.overlay, Some(Overlay::Compose(c)) if c.error.is_some() && c.text() == "Fixed")
    );
    let cmds = press(&mut s, "<C-s>");
    let cmds = answer(&mut s, &cmds, Ok(()));
    assert!(s.overlay.is_none());
    assert_eq!(cmds, vec![Cmd::Api(Api::FetchThreads(pr))]);
}

#[test]
fn suggestions_go_through_the_editor_with_a_preview() {
    let (mut s, _) = diff_state(120);
    to_line(&mut s, "origin");
    let cmds = act(&mut s, Action::Suggest);
    let Some(Cmd::Edit { purpose, text }) = cmds.into_iter().next() else {
        panic!()
    };
    assert!(text.contains("pub fn origin()"));
    update(
        &mut s,
        Msg::Edited(purpose, Ok("    pub fn zero() -> Self {\n".into())),
    );
    let compose = overlay!(s, Compose);
    assert!(
        compose
            .text()
            .starts_with("```suggestion\n    pub fn zero()")
    );
    let preview = compose.preview.as_ref().unwrap();
    assert_eq!(preview.suggested, ["    pub fn zero() -> Self {"]);
    assert_eq!(preview.original, ["    pub fn origin() -> Self {"]);
    // Unchanged text isn't a suggestion.
    s.overlay = None;
    let cmds = act(&mut s, Action::Suggest);
    let Some(Cmd::Edit { purpose, text }) = cmds.into_iter().next() else {
        panic!()
    };
    update(&mut s, Msg::Edited(purpose, Ok(text)));
    assert!(s.overlay.is_none());
    let _ = EditPurpose::Compose;
}

#[test]
fn outdated_threads_are_mapped_forward() {
    let (mut s, pr) = diff_state(120);
    let cmds = diff_msg(
        &mut s,
        &pr,
        DiffMsg::ThreadsLoaded(Ok(vec![thread("old", None, false, true)])),
    );
    assert!(cmds.iter().any(|c| matches!(c, Cmd::Git(Git::MapOutdated(j)) if j.get().3.len() == 1 && *j.get().2 == "e".repeat(40))));
    let ann = &s.diffs[&DiffOf::Pr(pr.clone())].doc.annotations()[0];
    assert_eq!(ann.place, Place::Outdated(Some(3)), "unplaced until mapped");
    let job = s.diffs[&DiffOf::Pr(pr.clone())].job;
    let mapped = JobMsg::Mapped(vec![(NodeId::new("old"), Some(4))]);
    diff_msg(&mut s, &pr, DiffMsg::Job(job, mapped));
    let ann = &s.diffs[&DiffOf::Pr(pr.clone())].doc.annotations()[0];
    assert!(matches!(ann.place, Place::Moved(at) if at.line == 4));
}

/// Outdated threads are mapped onto each job's head: another
/// range drops the old lines, and a late reply for the old job
/// doesn't land on the new one.
#[test]
fn an_outdated_mapping_is_for_its_own_job() {
    let (mut s, pr) = diff_state(120);
    let of = DiffOf::Pr(pr.clone());
    let thread = DiffMsg::ThreadsLoaded(Ok(vec![thread("old", None, false, true)]));
    let _ = diff_msg(&mut s, &pr, thread);
    let old = s.diffs[&of].job;
    let line = |n| JobMsg::Mapped(vec![(NodeId::new("old"), Some(n))]);
    diff_msg(&mut s, &pr, DiffMsg::Job(old, line(4)));
    s.diffs.get_mut(&of).unwrap().restart(None);
    assert!(s.diffs[&of].mapped().is_empty(), "the old job's lines kept");
    diff_msg(&mut s, &pr, DiffMsg::Job(old, line(9)));
    assert!(s.diffs[&of].mapped().is_empty(), "a late reply landed");
    let new = s.diffs[&of].job;
    diff_msg(&mut s, &pr, DiffMsg::Job(new, line(7)));
    assert_eq!(s.diffs[&of].mapped()[&NodeId::new("old")], Some(7));
}
