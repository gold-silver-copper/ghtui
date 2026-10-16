use super::*;
use crate::api::model::ReviewEvent;
use crate::review::SubmitOutcome;

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
    let job = fixtures::sent::<crate::review::Resolve>(&act(&mut s, Action::ResolveThread));
    assert!(job.resolved && job.thread == NodeId::new("open"));
    assert!(s.diffs[&DiffOf::Pr(pr.clone())].inputs().thread_list()[1].resolved);
    fixtures::done(&mut s, job, Err(fixtures::failure("down")));
    assert!(!s.diffs[&DiffOf::Pr(pr.clone())].inputs().thread_list()[1].resolved);
    warns(&s, "");

    // Resolved, it can be unresolved straight away.
    press(&mut s, "gg");
    act(&mut s, Action::NextThread);
    let job = fixtures::sent::<crate::review::Resolve>(&act(&mut s, Action::ResolveThread));
    fixtures::done(&mut s, job, Ok(()));
    let open = &s.diffs[&DiffOf::Pr(pr.clone())].inputs().thread_list()[1];
    assert!(open.resolved && open.can_unresolve);
}

#[test]
fn comment_draft_edit_and_delete() {
    let (mut s, pr) = diff_state(120);
    to_line(&mut s, "origin");
    press(&mut s, "c");
    let compose = overlay!(s, Compose);
    assert!(matches!(
        compose.target,
        compose::Target::Review(crate::review::Comment::Line { .. })
    ));
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
        matches!(&s.overlay, Some(Overlay::Compose(c)) if matches!(c.target, compose::Target::Review(crate::review::Comment::Draft { .. })))
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
        compose::Target::Review(crate::review::Comment::File {
            reason: Some(_),
            ..
        })
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
    assert!(
        matches!(&compose.target, compose::Target::Review(crate::review::Comment::Line { start, end, .. }) if start != end)
    );
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
    let submit = fixtures::sent::<crate::review::Submit>(&press(&mut s, "<C-s>"));
    assert_eq!(
        (submit.drafts.len(), submit.event),
        (1, ReviewEvent::Approve)
    );
    let outcome = SubmitOutcome {
        rejected: vec![(id, "line must be part of the diff".into())],
        ..SubmitOutcome::default()
    };
    fixtures::done(&mut s, submit, Ok(outcome));
    assert!(
        matches!(&s.overlay, Some(Overlay::Submit(d)) if d.sent.error.is_some() && !d.sent.sending)
    );
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
    let submit = fixtures::sent::<crate::review::Submit>(&press(&mut s, "<C-s>"));
    let outcome = SubmitOutcome {
        accepted: vec![id],
        submitted: true,
        ..SubmitOutcome::default()
    };
    fixtures::done(&mut s, submit, Ok(outcome));
    assert!(s.overlay.is_none());
    assert!(
        s.diffs[&DiffOf::Pr(pr.clone())]
            .inputs()
            .review
            .pending
            .is_empty()
    );
    assert_eq!(
        s.diffs[&DiffOf::Pr(pr)]
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
    assert!(matches!(&s.overlay, Some(Overlay::Submit(d)) if d.sent.error.is_some()));
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
        matches!(&s.overlay, Some(Overlay::Compose(c)) if c.sent.error.is_some() && c.text() == "Fixed")
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

/// The diff on o/r#7 with these drafts read, and the cursor on the
/// first row of the body of the draft `id`.
fn on_draft(drafts: Vec<crate::store::DraftComment>, id: u64) -> (State, PrRef) {
    let (mut s, pr) = unread_diff_state(120);
    let review = ReviewState {
        pending: drafts,
        ..ReviewState::default()
    };
    diff_msg(&mut s, &pr, DiffMsg::ReviewLoaded(Ok(review)));
    let doc = &s.diffs[&DiffOf::Pr(pr.clone())].doc;
    let ann = (doc.annotations().iter())
        .position(|a| a.key == AnnotationKey::Draft(id))
        .unwrap();
    let row = (0..doc.total_rows())
        .map(|g| doc.to_pos(g))
        .find(|p| {
            matches!(doc.row(*p), Some(Row::Thread(_)))
                && doc.annotation_at(*p) == Some(u32::try_from(ann).unwrap())
        })
        .expect("the draft has rows");
    if let Screen::Diff(d) = s.screens.last_mut() {
        d.cursor = row;
    }
    (s, pr)
}

/// `c` on a draft comments where the draft is: on its line, or on its
/// file for a file's draft.
#[test]
fn c_on_a_draft_comments_where_it_is() {
    let line = fixtures::draft(1, 14, "Use a constant");
    let file = crate::store::DraftComment {
        line: None,
        ..fixtures::draft(2, 0, "Split this file")
    };
    let (mut s, _) = on_draft(vec![line.clone(), file.clone()], 1);
    press(&mut s, "c");
    let r14 = crate::diff::anchor::LinePos {
        side: crate::diff::anchor::Side::Right,
        line: 14,
    };
    assert_eq!(
        overlay!(s, Compose).target,
        compose::Target::Review(crate::review::Comment::Line {
            path: "src/point.rs".into(),
            start: r14,
            end: r14,
        })
    );
    let (mut s, _) = on_draft(vec![line, file], 2);
    press(&mut s, "c");
    assert_eq!(
        overlay!(s, Compose).target,
        compose::Target::Review(crate::review::Comment::File {
            path: "src/point.rs".into(),
            reason: None,
        })
    );
}

/// Editing a draft says ctrl-s saves it, and `,e` and `,D` edit and
/// delete it as Enter and del do.
#[test]
fn a_draft_is_edited_and_deleted_by_the_comment_keys() {
    let (mut s, pr) = on_draft(vec![fixtures::draft(1, 14, "Use a constant")], 1);
    press(&mut s, ",e");
    assert!(matches!(
        overlay!(s, Compose).target,
        compose::Target::Review(crate::review::Comment::Draft { id: 1 })
    ));
    let shown = screen_text(&s);
    assert!(shown.contains("save draft"), "{shown}");
    assert!(!shown.contains("add to review"), "{shown}");
    press(&mut s, "!<C-s>");
    let of = DiffOf::Pr(pr);
    assert_eq!(
        s.diffs[&of].inputs().review.pending[0].body,
        "Use a constant!"
    );
    let cmds = press(&mut s, ",D");
    assert!(matches!(&cmds[..], [Cmd::SaveReview(_, r)] if r.pending.is_empty()));
    says(&s, "ctrl-z");
}

/// Resolving a thread folds it, and the cursor stays on it, so the same
/// key, offered as unresolve, unresolves it.
#[test]
fn the_cursor_stays_on_a_thread_it_resolves() {
    let (mut s, pr) = diff_state(120);
    diff_msg(
        &mut s,
        &pr,
        DiffMsg::ThreadsLoaded(Ok(vec![thread("open", Some(14), false, false)])),
    );
    let of = DiffOf::Pr(pr);
    // The thread's last row, below its fold.
    let (doc, ann) = (&s.diffs[&of].doc, 0);
    let last = (0..doc.total_rows())
        .map(|g| doc.to_pos(g))
        .rfind(|p| {
            matches!(doc.row(*p), Some(Row::Thread(_))) && doc.annotation_at(*p) == Some(ann)
        })
        .unwrap();
    if let Screen::Diff(d) = s.screens.last_mut() {
        d.cursor = last;
    }
    for resolved in [true, false] {
        let job = fixtures::sent::<crate::review::Resolve>(&press(&mut s, ",t"));
        assert_eq!(job.resolved, resolved);
        let doc = &s.diffs[&of].doc;
        assert_eq!(
            doc.annotation_at(screen(&s).cursor),
            Some(ann),
            "resolved: {resolved}"
        );
        // The status bar offers what the key does now.
        assert_eq!(screen_text(&s).contains("unresolve"), resolved);
        fixtures::done(&mut s, job, Ok(()));
    }
}

/// `G` goes to the last file's last row, not the space after it, and `c`
/// on the space between two files says so instead of commenting.
#[test]
fn g_ends_in_the_last_file_and_c_between_files_says_why() {
    let (mut s, pr) = diff_state(120);
    press(&mut s, "G");
    let doc = &s.diffs[&DiffOf::Pr(pr)].doc;
    let at = screen(&s).cursor;
    assert_eq!(at.file, doc.files().len() - 1);
    assert_ne!(doc.row(at), Some(Row::Spacer));
    press(&mut s, "j");
    assert_eq!(screen(&s).cursor, at);
    if let Screen::Diff(d) = s.screens.last_mut() {
        d.cursor = Pos { file: 0, row: 0 };
    }
    press(&mut s, "]f");
    press(&mut s, "k");
    let doc = &s.diffs[&DiffOf::Pr(PrRef::parse("o/r#7").unwrap())].doc;
    assert_eq!(doc.row(screen(&s).cursor), Some(Row::Spacer));
    press(&mut s, "c");
    assert!(s.overlay.is_none());
    fixtures::warns(&s, "between files");
}

/// A search's match off screen comes into view with the thread under
/// it, not on the screen's last row with the thread below it.
#[test]
fn a_match_shows_with_its_thread() {
    let (mut s, pr) = diff_state(120);
    s.size.1 = 14;
    diff_msg(
        &mut s,
        &pr,
        DiffMsg::ThreadsLoaded(Ok(vec![thread("open", Some(14), false, false)])),
    );
    press(&mut s, "gg");
    to_line(&mut s, "origin");
    let doc = &s.diffs[&DiffOf::Pr(pr)].doc;
    let screen = screen(&s);
    let height = usize::from(s.layout().content.height) - 1;
    let (top, cursor) = (doc.to_global(screen.top), doc.to_global(screen.cursor));
    let thread_end = (cursor + 1..doc.total_rows())
        .take_while(|g| matches!(doc.row(doc.to_pos(*g)), Some(Row::Thread(_))))
        .last()
        .expect("the thread is under the line");
    assert!(
        top < cursor && thread_end < top + height,
        "{top} {cursor} {thread_end} {height}"
    );
}
