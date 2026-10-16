use super::*;
use crate::api::model::{ViewedFiles, ViewedState};
use crate::diff_screen::DiffScreen;
use crate::route::pulls;
use crate::ui::diff_doc::{Doc, Pos, Row, Viewed};

/// A diff screen on o/r#7 built from the fixture diff, before
/// its saved review is read.
fn unread_diff_state(width: u16) -> (State, PrRef) {
    let mut s = state();
    s.size = (width, 40);
    let pr = PrRef::parse("o/r#7").unwrap();
    s.diffs.insert(DiffOf::Pr(pr.clone()), fixture_diff());
    s.screens.push(Screen::Diff(Box::new(DiffScreen::new(
        DiffOf::Pr(pr.clone()),
        DiffPrefs::fit(width),
    ))));
    let _ = s.settle_diff();
    (s, pr)
}

/// The same, with an empty saved review read.
pub(in crate::state) fn diff_state(width: u16) -> (State, PrRef) {
    let (mut s, pr) = unread_diff_state(width);
    diff_msg(
        &mut s,
        &pr,
        DiffMsg::ReviewLoaded(Ok(ReviewState::default())),
    );
    (s, pr)
}

fn screen(s: &State) -> &DiffScreen {
    match s.screen() {
        Screen::Diff(screen) => screen,
        Screen::Page(_) => panic!("not on the diff"),
    }
}

fn viewed_files(pr_id: &str) -> ViewedFiles {
    ViewedFiles {
        pull_request_id: NodeId::new(pr_id),
        states: [("src/point.rs".to_owned(), ViewedState::Viewed)].into(),
    }
}

#[test]
fn split_is_automatic_by_width_and_toggles() {
    let (mut narrow, _) = diff_state(120);
    assert!(!narrow.diffs.values().next().unwrap().doc.opts().split);
    act(&mut narrow, Action::ToggleSplit);
    assert!(narrow.diffs.values().next().unwrap().doc.opts().split);
    let (wide, _) = diff_state(220);
    assert!(wide.diffs.values().next().unwrap().doc.opts().split);
}

/// Where the cursor is, and the document it's in.
fn cursor(s: &State) -> (Pos, &Doc) {
    let screen = screen(s);
    (screen.cursor, &s.diffs[&screen.of].doc)
}

/// The cursor on a removed line stays on it when the view turns
/// split, though the split row also shows the added line beside it.
#[test]
fn a_removed_line_keeps_the_cursor_across_split() {
    let (mut s, _) = diff_state(120);
    let (_, doc) = cursor(&s);
    let rows = doc.files()[0].rows().len();
    let removed = (0..rows)
        .map(|row| Pos { file: 0, row })
        .find(|p| doc.row_text(*p) == "/// A point.")
        .expect("the unified view shows the removed line");
    if let Screen::Diff(d) = s.screens.last_mut() {
        d.cursor = removed;
    }
    act(&mut s, Action::ToggleSplit);
    let (pos, doc) = cursor(&s);
    assert!(doc.opts().split);
    let text = doc.row_text(pos);
    assert!(
        text.lines().any(|l| l == "/// A point."),
        "the cursor left the removed line for {pos:?}: {text:?}"
    );
}

/// A pull request's Review tab reads the drafts and threads its
/// diff does, without the diff open. It lists the drafts and the
/// unresolved threads, and a thread opens Files changed on it.
#[test]
fn the_review_tab_lists_drafts_and_threads_and_opens_them() {
    let mut s = state();
    let pr = PrRef::parse("o/r#7").unwrap();
    let of = DiffOf::Pr(pr.clone());
    let _ = s.push(Route::Pulls(pulls::Pr {
        pr: pr.clone(),
        tab: PrTab::Review,
    }));
    let cmds = crate::fixtures::pr_fetched(&mut s, &pr, Ok(crate::fixtures::pr_detail()));
    assert!(cmds.contains(&Cmd::LoadReview(pr.clone())), "{cmds:?}");
    assert!(cmds.contains(&Cmd::Api(Api::FetchThreads(pr.clone()))));
    // The diff job lists the files.
    s.diffs.insert(of.clone(), fixture_diff());
    let review = ReviewState {
        pending: vec![crate::fixtures::draft(1, 5, "Name it for what it holds")],
        ..ReviewState::default()
    };
    diff_msg(&mut s, &pr, DiffMsg::ReviewLoaded(Ok(review)));
    let url = "https://github.com/o/r/pull/7#discussion_r42";
    let mut open = thread("open", Some(14), false, false);
    open.comments[0].url = url.into();
    let threads = vec![thread("done", Some(3), true, false), open];
    diff_msg(&mut s, &pr, DiffMsg::ThreadsLoaded(Ok(threads)));
    let text = super::page_text(&s);
    assert!(text.contains("src/point.rs:5"), "{text}");
    assert!(text.contains("Name it for what it holds"), "{text}");
    assert!(text.contains("Thread open"), "{text}");
    assert!(!text.contains("Thread done"), "resolved: {text}");
    for _ in 0..9 {
        if page(&s)
            .selected_link()
            .and_then(crate::ui::page::Link::url)
            == Some(url)
        {
            break;
        }
        press(&mut s, "<Down>");
    }
    press(&mut s, "<Enter>");
    let screen = screen(&s);
    assert_eq!(screen.of, of);
    let doc = &s.diffs[&of].doc;
    assert_eq!(doc.annotation_at(screen.cursor), Some(1), "the open thread");
}

/// The Review tab on o/r#7 with these drafts read, the diff listed.
fn review_tab(drafts: Vec<crate::store::DraftComment>) -> (State, PrRef) {
    let mut s = state();
    s.size = (120, 14);
    let pr = PrRef::parse("o/r#7").unwrap();
    let _ = s.push(Route::Pulls(pulls::Pr {
        pr: pr.clone(),
        tab: PrTab::Review,
    }));
    let _ = crate::fixtures::pr_fetched(&mut s, &pr, Ok(crate::fixtures::pr_detail()));
    s.diffs.insert(DiffOf::Pr(pr.clone()), fixture_diff());
    let review = ReviewState {
        pending: drafts,
        ..ReviewState::default()
    };
    diff_msg(&mut s, &pr, DiffMsg::ReviewLoaded(Ok(review)));
    diff_msg(&mut s, &pr, DiffMsg::ThreadsLoaded(Ok(Vec::new())));
    (s, pr)
}

/// Selects the Review tab's row of draft `id`.
fn select_draft(s: &mut State, id: u64) {
    for _ in 0..9 {
        if matches!(page(s).selected_link(), Some(crate::ui::page::Link::Review(
            crate::ui::page::ReviewLink::Draft { id: at, .. }
        )) if *at == id)
        {
            return;
        }
        press(s, "<Down>");
    }
    panic!("no row for draft {id}");
}

/// A draft on the Review tab is deleted (`,D` or del) and brought back
/// there, and `,e` edits it in Files changed.
#[test]
fn the_review_tab_edits_and_deletes_drafts() {
    let (mut s, pr) = review_tab(vec![crate::fixtures::draft(
        1,
        5,
        "Name it for what it holds",
    )]);
    select_draft(&mut s, 1);
    let cmds = press(&mut s, "D");
    assert!(
        matches!(&cmds[..], [Cmd::SaveReview(_, r)] if r.pending.is_empty()),
        "{cmds:?}"
    );
    let text = super::page_text(&s);
    assert!(!text.contains("Name it for"), "{text}");
    let cmds = press(&mut s, "<C-z>");
    assert!(
        matches!(&cmds[..], [Cmd::SaveReview(_, r)] if r.pending.len() == 1),
        "{cmds:?}"
    );
    select_draft(&mut s, 1);
    let cmds = press(&mut s, "<Delete>");
    assert!(
        matches!(&cmds[..], [Cmd::SaveReview(_, r)] if r.pending.is_empty()),
        "{cmds:?}"
    );
    press(&mut s, "<C-z>");
    select_draft(&mut s, 1);
    press(&mut s, "e");
    assert_eq!(screen(&s).of, DiffOf::Pr(pr));
    assert!(matches!(
        overlay!(s, Compose).target,
        compose::Target::Review(crate::review::Comment::Draft { id: 1 })
    ));
}

/// A file's draft opened from the Review tab shows under its file's
/// header, not below the screen.
#[test]
fn a_files_draft_opens_in_view() {
    let draft = crate::store::DraftComment {
        path: "zz/pending.rs".into(),
        line: None,
        ..crate::fixtures::draft(1, 0, "Is this file needed?")
    };
    let (mut s, pr) = review_tab(vec![draft]);
    select_draft(&mut s, 1);
    press(&mut s, "<Enter>");
    let _ = s.settle_diff();
    let doc = &s.diffs[&DiffOf::Pr(pr)].doc;
    let screen = screen(&s);
    let height = usize::from(s.layout().content.height) - 1;
    let top = doc.to_global(screen.top);
    let body = (0..doc.total_rows())
        .rfind(|g| doc.annotation_at(doc.to_pos(*g)) == Some(0))
        .expect("the draft has rows");
    assert_eq!(doc.files()[screen.cursor.file].meta.path(), "zz/pending.rs");
    assert!(body < top + height, "{top} {body} {height}");
}

/// Threads arriving above the cursor push its line down, and the
/// cursor goes with it.
#[test]
fn threads_arriving_keep_the_cursor_on_its_line() {
    let (mut s, pr) = diff_state(120);
    let (_, doc) = cursor(&s);
    let rows = doc.files()[0].rows().len();
    let last = (0..rows)
        .map(|row| Pos { file: 0, row })
        .rfind(|p| matches!(doc.row(*p), Some(Row::Line(_) | Row::Split { .. })))
        .expect("the file shows lines");
    let text = doc.row_text(last);
    if let Screen::Diff(d) = s.screens.last_mut() {
        d.cursor = last;
    }
    let threads = vec![thread("t", Some(14), false, false)];
    diff_msg(&mut s, &pr, DiffMsg::ThreadsLoaded(Ok(threads)));
    let (pos, doc) = cursor(&s);
    assert!(pos.row > last.row, "the thread is above the cursor");
    assert_eq!(doc.row_text(pos), text);
}

/// The cursor on a review thread stays on that thread when the
/// rows are rebuilt (here: the view turns split).
#[test]
fn a_thread_row_keeps_the_cursor_across_a_rebuild() {
    let (mut s, pr) = diff_state(120);
    diff_msg(
        &mut s,
        &pr,
        DiffMsg::ThreadsLoaded(Ok(vec![thread("t", Some(14), false, false)])),
    );
    let (_, doc) = cursor(&s);
    let rows = doc.files()[0].rows().len();
    let on_thread = (0..rows)
        .map(|row| Pos { file: 0, row })
        .find(|p| matches!(doc.row(*p), Some(Row::Thread(_))))
        .expect("the thread has a row");
    if let Screen::Diff(d) = s.screens.last_mut() {
        d.cursor = on_thread;
    }
    act(&mut s, Action::ToggleSplit);
    let (pos, doc) = cursor(&s);
    assert!(doc.opts().split);
    assert!(
        matches!(doc.row(pos), Some(Row::Thread(_))),
        "the cursor left the thread for {pos:?}: {:?}",
        doc.row(pos)
    );
}

#[test]
fn viewed_state_applies_and_rolls_back_on_failure() {
    let (mut s, pr) = diff_state(120);
    // Before GitHub's state arrives, toggling explains why not.
    assert!(press(&mut s, "v").is_empty());
    warns(&s, "");

    diff_msg(
        &mut s,
        &pr,
        DiffMsg::ViewedLoaded(Box::new(Ok(viewed_files("PR_1")))),
    );
    assert_eq!(
        s.diffs[&DiffOf::Pr(pr.clone())].doc.files()[0].viewed,
        Viewed::Viewed
    );
    assert!(
        s.diffs[&DiffOf::Pr(pr.clone())].doc.files()[0].collapsed(),
        "viewed files collapse"
    );

    // Next unviewed skips the viewed file.
    press(&mut s, "<Home>");
    act(&mut s, Action::NextUnviewed);
    assert_eq!(screen(&s).cursor.file, 1);

    // Toggle file 1 viewed: optimistic, then GitHub refuses.
    let job = fixtures::sent::<crate::review::SetViewed>(&press(&mut s, "v"));
    assert!(job.viewed && job.previous == Viewed::Unviewed);
    assert_eq!(job.pull_request_id.as_str(), "PR_1");
    assert_eq!(
        s.diffs[&DiffOf::Pr(pr.clone())].doc.files()[1].viewed,
        Viewed::Viewed
    );
    fixtures::done(&mut s, job, Err(fixtures::failure("offline")));
    assert_eq!(
        s.diffs[&DiffOf::Pr(pr.clone())].doc.files()[1].viewed,
        Viewed::Unviewed
    );
    warns(&s, "");
}

/// Marks made before the saved review arrives aren't saved over
/// it; they're merged into it once it's in.
#[test]
fn nothing_is_saved_before_the_saved_review_is_read() {
    let (mut s, pr) = unread_diff_state(120);
    press(&mut s, "]<Down><Down><Down>");
    let cmds = press(&mut s, "m");
    assert!(!cmds.iter().any(|c| matches!(c, Cmd::SaveReview(..))));

    let saved = ReviewState {
        reviewed_hunks: vec!["older".into()],
        ..ReviewState::default()
    };
    let cmds = diff_msg(&mut s, &pr, DiffMsg::ReviewLoaded(Ok(saved)));
    let [Cmd::SaveReview(_, review)] = &cmds[..] else {
        panic!("{cmds:?}")
    };
    assert_eq!(review.reviewed_hunks.len(), 2);
    assert!(review.reviewed_hunks.contains(&"older".to_owned()));
}

/// A saved review read before anything was done is not written back.
#[test]
fn reading_the_saved_review_alone_saves_nothing() {
    let (mut s, pr) = unread_diff_state(120);
    let saved = ReviewState {
        reviewed_hunks: vec!["older".into()],
        ..ReviewState::default()
    };
    let cmds = diff_msg(&mut s, &pr, DiffMsg::ReviewLoaded(Ok(saved)));
    assert!(cmds.is_empty(), "{cmds:?}");
}

#[test]
fn unreadable_drafts_raise_a_lasting_problem() {
    let (mut s, pr) = diff_state(120);
    diff_msg(
        &mut s,
        &pr,
        DiffMsg::ReviewLoaded(Err(Failure::msg("disk full"))),
    );
    assert!(s.problems[&Problem::Drafts].contains("disk full"));
    assert!(s.layout().problem.is_some());
    update(&mut s, Msg::Problem(Problem::Drafts, None));
    assert!(s.layout().problem.is_none());
}

#[test]
fn review_marks_toggle_and_persist() {
    let (mut s, pr) = diff_state(120);
    // Off a changed line: nothing to mark.
    assert!(press(&mut s, "<Home>").is_empty());
    assert!(press(&mut s, "m").is_empty());
    // Onto the first change.
    press(&mut s, "]<Down><Down><Down>");
    let cmds = press(&mut s, "m");
    let Some(Cmd::SaveReview(saved_pr, review)) = cmds.first() else {
        panic!("{cmds:?}")
    };
    assert_eq!(saved_pr, &pr);
    assert_eq!(review.reviewed_hunks.len(), 1);
    assert_eq!(
        s.diffs[&DiffOf::Pr(pr)]
            .inputs()
            .review
            .reviewed_hunks
            .len(),
        1
    );
    let cmds = press(&mut s, "m");
    let Some(Cmd::SaveReview(_, review)) = cmds.first() else {
        panic!()
    };
    assert!(review.reviewed_hunks.is_empty());
}

#[test]
fn search_and_repeat() {
    let (mut s, pr) = diff_state(120);
    press(&mut s, "/");
    assert!(matches!(s.overlay, Some(Overlay::Find(_))));
    press(&mut s, "origin<Enter>");
    assert!(s.overlay.is_none());
    let cursor = screen(&s).cursor;
    assert!(
        s.diffs[&DiffOf::Pr(pr)]
            .doc
            .row_text(cursor)
            .contains("origin")
    );
    says(&s, "1 line");
    press(&mut s, "n");
    assert_eq!(screen(&s).cursor, cursor, "single match wraps to itself");
    press(&mut s, "/");
    press(&mut s, "zzzz<Enter>");
    warns(&s, "");
}

/// A number in the palette goes to that new line of the file.
#[test]
fn a_line_number_in_the_palette_goes_to_that_line() {
    let (mut s, pr) = diff_state(120);
    press(&mut s, ":5<Enter>");
    let at = screen(&s).cursor;
    let doc = &s.diffs[&DiffOf::Pr(pr)].doc;
    let shown: Vec<_> = (doc.files().first().zip(doc.row(at)))
        .map(|(f, row)| f.shows(row).map(|p| p.line).collect())
        .unwrap_or_default();
    assert!(shown.iter().any(|&l| l >= 5), "{at:?}: {shown:?}");
    press(&mut s, ":9999<Enter>");
    says(&s, "no line 9999");
}

#[test]
fn n_and_shift_n_follow_a_search_and_esc_ends_it() {
    let (mut s, _) = diff_state(120);
    press(&mut s, "/");
    press(&mut s, "Point<Enter>");
    let first = screen(&s).cursor;
    press(&mut s, "n");
    assert_ne!(screen(&s).cursor, first, "n: the next match");
    press(&mut s, "N");
    assert_eq!(screen(&s).cursor, first, "N: back");
    let depth = s.screens.len();
    press(&mut s, "<Esc>");
    assert!(screen(&s).search.is_none(), "esc ends the search");
    assert_eq!(s.screens.len(), depth, "and stays");
    press(&mut s, "<Esc>");
    assert_eq!(s.screens.len(), depth - 1, "then goes back");
}

#[test]
fn file_finder_jumps_to_files() {
    let (mut s, _) = diff_state(120);
    press(&mut s, "fgone");
    let p = overlay!(s, Picker);
    assert_eq!(s.picker_rows(p)[0].0.label, "gone.py");
    press(&mut s, "<Enter>");
    assert!(s.overlay.is_none());
    assert_eq!(screen(&s).cursor.file, 3);
    assert_eq!(screen(&s).cursor.row, 0);
}

#[test]
fn whitespace_and_full_file_keep_the_cursor_line() {
    let (mut s, pr) = diff_state(120);
    press(&mut s, "]<Down><Down><Down>");
    let text = s.diffs[&DiffOf::Pr(pr.clone())]
        .doc
        .row_text(screen(&s).cursor);
    act(&mut s, Action::FullFile);
    assert!(s.diffs[&DiffOf::Pr(pr.clone())].doc.files()[0].full);
    assert_eq!(
        s.diffs[&DiffOf::Pr(pr.clone())]
            .doc
            .row_text(screen(&s).cursor),
        text
    );
    act(&mut s, Action::IgnoreWhitespace);
    assert_eq!(
        s.diffs[&DiffOf::Pr(pr)].doc.opts().whitespace,
        crate::diff::Whitespace::Ignore
    );
    press(&mut s, "e");
}
pub(in crate::state) mod screen;
pub(in crate::state) use screen::{commit_of, fixture_diff, screen_state};

mod better;

/// Watching the pull request under review for what's new.
mod live;

mod review;

/// The comment box: pastes, wrapping, and completing mentions.
mod typing;

/// What a diff is built from (GitHub's patches, viewed marks, the
/// screen's toggles) outlasts each rebuild of what's shown.
mod inputs_survive;
