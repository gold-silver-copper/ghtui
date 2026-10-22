//! Committing someone's suggestion from Files changed, alone or in a
//! batch: what's asked, what's sent, and the diff following GitHub to the
//! new head.

use super::*;
use crate::forms::Answer;
use crate::suggest::{Commit, Committed, Suggestion};

/// A thread on new line `line` of the fixture diff's `src/point.rs`,
/// alice's comment suggesting `lines`.
fn suggesting(id: &str, line: u32, lines: &str) -> crate::api::model::ReviewThread {
    let mut t = thread(id, Some(line), false, false);
    t.comments.items[0].body = format!("Better:\n```suggestion\n{lines}```");
    t
}

/// The fixture diff of o/r#7 at its head, its pull request open on a
/// branch you may push to, with `threads`.
fn diff_with(threads: Vec<crate::api::model::ReviewThread>) -> (State, PrRef) {
    let (mut s, pr) = super::diff::diff_state(120);
    let mut d = pr_detail();
    d.head_oid = "e".repeat(40);
    d.head_ref = "topic".into();
    d.head_repo = Some("o/r".into());
    fixtures::pr_cached(&mut s, &pr, d);
    threads_loaded(&mut s, &pr, threads);
    press(&mut s, "<Home>");
    act(&mut s, Action::NextThread);
    (s, pr)
}

/// The menu's row for `action`: its label, and why it can't be done.
fn menu_row(s: &State, action: Action) -> Option<(String, Option<String>)> {
    let row = s.doables().into_iter().find(|d| d.action == action)?;
    Some((row.label.into_owned(), row.unavailable))
}

/// The job the question's choice sends.
fn chosen(s: &mut State) -> Commit {
    let ask = overlay!(s, Ask);
    let Some((_, Answer::CommitSuggestions(job))) = ask.choices.first() else {
        panic!()
    };
    let job = (**job).clone();
    let cmds = press(s, "<Enter>");
    assert_eq!(fixtures::sent::<Commit>(&cmds), job);
    job
}

/// On a thread whose comment suggests, the menu offers to commit it;
/// committing asks first, sends the suggestion with the head its lines
/// are on, resolves the thread once it's pushed, and Files changed loads
/// the new head once GitHub shows it.
#[test]
fn a_suggestion_is_committed_and_the_diff_follows_github() {
    let new = "    pub fn origin() -> Point {\n";
    let (mut s, pr) = diff_with(vec![suggesting("s", 14, new)]);
    let (label, why) = menu_row(&s, Action::CommitSuggestion).unwrap();
    assert_eq!((label.as_str(), why), ("Commit the suggestion", None));

    act(&mut s, Action::CommitSuggestion);
    assert_eq!(overlay!(s, Ask).title, "Commit the suggestion to topic?");
    assert_eq!(ask_labels(&s), ["Commit suggestion"]);
    let job = chosen(&mut s);
    let one = Suggestion {
        thread: NodeId::new("s"),
        can_resolve: true,
        author: "alice".into(),
        path: "src/point.rs".into(),
        start: 14,
        end: 14,
        lines: new.into(),
    };
    assert_eq!(
        (
            job.suggestions.as_slice(),
            job.seen.as_str(),
            job.branch.as_str()
        ),
        (std::slice::from_ref(&one), &*"e".repeat(40), "topic")
    );

    let made = "f".repeat(40);
    let done = Committed {
        oid: made.clone(),
        unresolved: Vec::new(),
    };
    let cmds = fixtures::done(&mut s, job, Ok(done));
    assert!(s.overlay.is_none());
    says(&s, "Committed the suggestion to topic as fffffff");
    assert!(fixtures::asks_pr(&cmds, &pr));
    let of = DiffOf::Pr(pr.clone());
    assert!(s.diffs[&of].inputs().thread_list()[0].resolved);

    // GitHub shows the new head: the diff starts again at it.
    let mut d = pr_detail();
    d.head_oid = made;
    let cmds = fixtures::pr_fetched(&mut s, &pr, Ok(d));
    fixtures::diff_job(&cmds);
    says(&s, "Files changed shows o/r#7 at its new head");
}

/// Suggestions batched are committed together; one on lines the batch
/// changes already can't join it, and one can be taken out again.
#[test]
fn suggestions_are_batched_and_committed_together() {
    let (mut s, _) = diff_with(vec![
        suggesting("a", 14, "    pub fn zero() -> Self {\n"),
        suggesting("b", 15, "        Self::new(0, 0)\n"),
    ]);
    act(&mut s, Action::BatchSuggestion);
    says(&s, "Added to the batch (1 in it)");
    let (label, _) = menu_row(&s, Action::BatchSuggestion).unwrap();
    assert_eq!(label, "Take the suggestion out of the batch");
    act(&mut s, Action::NextThread);
    act(&mut s, Action::BatchSuggestion);
    says(&s, "Added to the batch (2 in it)");
    let (label, why) = menu_row(&s, Action::CommitSuggestion).unwrap();
    assert_eq!(
        (label.as_str(), why),
        ("Commit the 2 suggestions in the batch", None)
    );
    act(&mut s, Action::BatchSuggestion);
    says(&s, "Taken out of the batch (1 in it)");
    act(&mut s, Action::BatchSuggestion);

    act(&mut s, Action::CommitSuggestion);
    assert_eq!(ask_labels(&s), ["Commit 2 suggestions"]);
    let job = chosen(&mut s);
    let threads: Vec<&NodeId> = job.suggestions.iter().map(|s| &s.thread).collect();
    assert_eq!(threads, [&NodeId::new("a"), &NodeId::new("b")]);
    // GitHub refuses: the question says why, and the batch stays.
    fixtures::done(&mut s, job, Err(fixtures::failure("src/point.rs changed")));
    assert_eq!(
        overlay!(s, Ask).sent.error.as_deref(),
        Some("Not committed: src/point.rs changed")
    );
}

/// What GitHub wouldn't commit isn't offered, and says why: an outdated
/// suggestion, and a branch you can't push to.
#[test]
fn what_github_wouldnt_commit_says_why() {
    let mut old = suggesting("old", 14, "x\n");
    old.outdated = true;
    old.line = None;
    let (s, _) = diff_with(vec![old]);
    let (_, why) = menu_row(&s, Action::CommitSuggestion).unwrap();
    assert!(why.unwrap().contains("outdated"));

    let (mut s, pr) = diff_with(vec![suggesting("s", 14, "x\n")]);
    let mut d = pr_detail();
    d.head_oid = "e".repeat(40);
    d.may.push_head = false;
    d.may.merge = false;
    fixtures::pr_cached(&mut s, &pr, d);
    act(&mut s, Action::CommitSuggestion);
    says(&s, "which you can't push to");
    assert!(s.overlay.is_none());

    // A thread without one has nothing to offer.
    let (s, _) = diff_with(vec![thread("plain", Some(14), false, false)]);
    assert_eq!(menu_row(&s, Action::CommitSuggestion), None);
}
