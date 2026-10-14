use crate::api::browse::CheckOutcome;
use crate::api::change::MergeMethod;
use crate::api::model::MergeState;
use crate::api::notifications::ThreadAt;
use crate::route::{app, runs, search};
use crate::store::DraftSide;

use super::*;
use crate::act::{By, Subject};
use crate::forms::{About, Answer};

fn pr() -> PrRef {
    PrRef::parse("gold-silver-copper/ghtui#12").unwrap()
}

/// A pull request whose state is `state`.
fn pr_in(state: PrState) -> State {
    pr_with(|d| d.state = state)
}

/// The pull request, changed by `f`.
fn pr_with(f: impl FnOnce(&mut crate::api::model::PrDetail)) -> State {
    let mut s = with_pr();
    crate::fixtures::pr_fetched(&mut s, &pr(), Ok(mergeable(f)));
    s
}

/// The pull request without its conflicts (GitHub would merge it),
/// changed by `f`.
fn mergeable(f: impl FnOnce(&mut crate::api::model::PrDetail)) -> crate::api::model::PrDetail {
    let mut detail = pr_detail();
    detail.mergeable = crate::api::model::Mergeable::Yes;
    detail.merge_state = MergeState::Unstable;
    f(&mut detail);
    detail
}

/// Merging asks first, saying what stands in the way; a refusal stays
/// in the dialog, with the permission it needs; once GitHub says it's
/// merged, the pull request is fetched again.
#[test]
fn merging_asks_first_and_ends_up_as_github_has_it() {
    let mut s = pr_with(|_| {});
    assert!(press(&mut s, ",m").is_empty());
    let c = overlay!(s, Ask);
    let problems: Vec<&str> = (c.facts.iter())
        .filter(|(_, p)| *p)
        .map(|(f, _)| f.as_str())
        .collect();
    assert_eq!(problems, ["Some checks failed"], "{:?}", c.facts);
    assert!(
        c.facts
            .iter()
            .any(|(f, _)| f == "Its branch is 2 commits behind main (,u updates it)"),
        "{:?}",
        c.facts
    );
    let methods: Vec<&str> = c.choices.iter().map(|(l, _)| l.as_str()).collect();
    assert_eq!(methods, ["Squash and merge", "Create a merge commit"]);
    press(&mut s, "<Tab>");
    let merge = Change::Merge {
        pr: NodeId::new("PR_12"),
        method: MergeMethod::Merge,
        head: pr_detail().head_oid,
    };
    let sent = press(&mut s, "<Enter>");
    assert_eq!(
        sent,
        vec![fixtures::change(merge, By::Confirm(Subject::Pr(pr())))]
    );
    assert!(overlay!(s, Ask).sent.sending);
    // Keys wait while it's sent.
    assert!(press(&mut s, "<Enter>").is_empty());

    let refused = ApiError::GraphQl(vec!["Must have push access to repository".into()]);
    answer(&mut s, &sent, Err(refused));
    let error = overlay!(s, Ask).sent.error.clone().unwrap_or_default();
    assert!(error.contains("Must have push access"), "{error}");
    assert!(error.contains("Contents: write"), "{error}");
    assert!(!overlay!(s, Ask).sent.sending);

    press(&mut s, "<Enter>");
    let cmds = answer(&mut s, &sent, Ok(()));
    assert!(s.overlay.is_none());
    says(&s, "Merged");
    assert!(
        crate::fixtures::asks_pr(&cmds, &pr()),
        "the pull request is fetched again: {cmds:?}"
    );
    // Until GitHub shows it merged, nothing else is offered.
    press(&mut s, ",x");
    says(&s, "hasn't shown the last change");
    let merged = mergeable(|d| d.state = PrState::Merged);
    crate::fixtures::pr_fetched(&mut s, &pr(), Ok(merged));
    assert!(s.following.is_none());
}

/// On Home, an action works on the selected row: the pull request
/// loads, then the dialog opens about it, and stays about it when the
/// list changes under it. Once GitHub shows the merge, Home is fetched
/// again.
#[test]
fn acting_on_a_homes_row_is_about_that_row() {
    let mut s = super::with_home();
    s.data.retain(|key, _| !matches!(key, DataKey::Pr(_)));
    press(&mut s, "jj");
    let cmds = press(&mut s, ",m");
    assert_eq!(cmds, vec![crate::fixtures::fetch_pr(&pr())]);
    says(&s, "Loading gold-silver-copper/ghtui#12");
    crate::fixtures::pr_fetched(&mut s, &pr(), Ok(mergeable(|_| {})));
    assert_eq!(
        overlay!(s, Ask).about,
        About::Act(Action::Merge, Subject::Pr(pr()))
    );
    assert!(
        overlay!(s, Ask)
            .title
            .starts_with("Merge gold-silver-copper/ghtui#12")
    );
    // The list moves; the dialog is still about #12.
    press(&mut s, "<Esc>");
    press(&mut s, ",m");
    let mine = crate::fixtures::found_prs(Vec::new(), 0);
    crate::fixtures::section(&mut s, 1, mine);
    assert_eq!(
        overlay!(s, Ask).about,
        About::Act(Action::Merge, Subject::Pr(pr()))
    );
    let sent = press(&mut s, "<Enter>");
    answer(&mut s, &sent, Ok(()));
    assert_eq!(s.route(), Some(&Route::App(app::Home)));
    let w = crate::act::awaiting_here(&s).unwrap();
    assert_eq!(w.about, crate::act::Subject::Pr(pr()));
    // Home asked again at once; GitHub's search may not show it yet.
    for i in 0..2 {
        let none = crate::fixtures::found_prs(Vec::new(), 0);
        crate::fixtures::section(&mut s, i, none);
    }
    let merged = mergeable(|d| d.state = PrState::Merged);
    let cmds = crate::fixtures::pr_fetched(&mut s, &pr(), Ok(merged));
    assert!(s.following.is_none());
    let home = s.home[1].search.clone().unwrap().key();
    assert!(crate::fixtures::asks(&cmds, &home), "{cmds:?}");
    // An issue row offers closing, not merging.
    crate::fixtures::section(
        &mut s,
        1,
        crate::fixtures::found_prs(
            vec![crate::fixtures::issue_summary(
                3,
                Status::Issue(IssueState::Open),
            )],
            1,
        ),
    );
    press(&mut s, "ggjj");
    press(&mut s, ",m");
    says(&s, "That works on a pull request");
}

/// Updating the branch, the page follows GitHub until it shows the new
/// head (GitHub moves it after it answers), and merging then names that
/// head: merging the old one is refused as "Head branch was modified".
#[test]
fn after_updating_the_branch_merging_waits_for_the_new_head() {
    // Behind, though the repository doesn't require it up to date.
    let mut s = pr_with(|_| {});
    press(&mut s, ",u");
    let update_branch = Change::UpdateBranch {
        pr: NodeId::new("PR_12"),
        rebase: false,
        head: pr_detail().head_oid,
    };
    assert_eq!(overlay!(s, Ask).choices[0].1, Answer::Change(update_branch));
    assert_eq!(
        overlay!(s, Ask).facts[0].0,
        "Its branch, syntax-palette, is 2 commits behind main; updating pushes to it"
    );
    let sent = press(&mut s, "<Enter>");
    answer(&mut s, &sent, Ok(()));
    // GitHub still shows the old head.
    crate::fixtures::pr_fetched(&mut s, &pr(), Ok(mergeable(|_| {})));
    press(&mut s, ",m");
    says(&s, "hasn't shown the last change");
    let cmds = timers(&mut s);
    assert!(cmds.contains(&Cmd::Timer(Timer::Live, 5_000)), "{cmds:?}");
    let cmds = update(&mut s, Msg::Timer(Timer::Live));
    assert!(crate::fixtures::asks_pr(&cmds, &pr()), "{cmds:?}");
    let moved = mergeable(|d| {
        d.head_oid = "1".repeat(40);
        d.behind_by = Some(0);
    });
    crate::fixtures::pr_fetched(&mut s, &pr(), Ok(moved));
    press(&mut s, ",m");
    let head = match &overlay!(s, Ask).choices[0].1 {
        Answer::Change(Change::Merge { head, .. }) => head.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(head, "1".repeat(40));
    press(&mut s, "<Esc>");
    press(&mut s, ",u");
    says(&s, "It's up to date with main");
}

/// What GitHub says you may not do is refused, saying why, and what
/// GitHub says stops a merge is refused before asking.
#[test]
fn what_github_forbids_is_refused() {
    let mut s = pr_with(|d| d.may = crate::api::model::PrPermits::default());
    press(&mut s, ",m");
    says(&s, "Merging takes write access to gold-silver-copper/ghtui");
    press(&mut s, ",x");
    says(&s, "GitHub doesn't let you close it");
    press(&mut s, ",u");
    says(&s, "which you can't push to");

    let mut s = with_pr();
    press(&mut s, ",m");
    says(&s, "It conflicts with main: resolve that on GitHub");
    let mut s = pr_with(|d| d.merge_state = MergeState::Blocked);
    press(&mut s, ",m");
    says(&s, "blocked");
    let mut s = pr_with(|d| {
        d.merge_state = MergeState::Behind;
        d.may.enable_auto_merge = false;
    });
    press(&mut s, ",m");
    says(&s, ",u updates it");
    // Behind, with auto-merge allowed: it merges once updated.
    let mut s = pr_with(|d| {
        d.merge_state = MergeState::Behind;
        (d.may.enable_auto_merge, d.auto_merge_allowed) = (true, true);
    });
    press(&mut s, ",m");
    let c = overlay!(s, Ask);
    assert!(
        c.choices
            .iter()
            .all(|(label, _)| label.ends_with("when ready")),
        "{:?}",
        c.choices
    );
    assert!(
        c.facts.iter().any(|(f, _)| f.contains(",u updates it")),
        "{:?}",
        c.facts
    );

    let mut s = with_run();
    let mut repo = crate::fixtures::overview();
    repo.can_write = false;
    fetched(&mut s, DataKey::Repo(ghtui()), Data::Repo(Box::new(repo)));
    press(&mut s, ",R");
    says(
        &s,
        "Re-running and cancelling take write access to gold-silver-copper/ghtui",
    );
}

/// An action that can't apply says why, on the key and in the menu
/// alike, and sends nothing.
#[test]
fn what_cant_be_done_says_why() {
    let mut s = pr_in(PrState::Merged);
    assert!(press(&mut s, ",m").is_empty());
    assert!(s.overlay.is_none());
    says(&s, "It's merged");
    let merge = (s.doables().into_iter())
        .find(|d| d.action == Action::Merge)
        .unwrap();
    assert_eq!(merge.unavailable.as_deref(), Some("It's merged"));
    press(&mut s, ",x");
    says(&s, "can't be reopened");
    press(&mut s, ",r");
    says(&s, "It's merged");

    let mut s = with_pr();
    s.viewer = Some(pr_detail().author);
    press(&mut s, ",a");
    says(&s, "You can't approve your own pull request");

    let mut s = with_repo();
    press(&mut s, ",m");
    says(&s, "That works on a pull request (or its row in a list)");
    press(&mut s, ",R");
    says(&s, "Re-running works on a workflow run or a job");
}

/// Marking a draft ready asks nothing: it's sent at once. GitHub's
/// answer goes back to what sent it: a dialog or a comment opened while
/// it's on its way is left as it is.
#[test]
fn a_draft_is_marked_ready_at_once_and_hears_back_alone() {
    let mut s = pr_in(PrState::Draft);
    let ready = Change::ReadyForReview {
        pr: NodeId::new("PR_12"),
    };
    let sent = press(&mut s, ",r");
    assert_eq!(
        sent,
        vec![fixtures::change(ready, By::Act(Subject::Pr(pr())))]
    );
    press(&mut s, ",x");
    let refused = ApiError::GraphQl(vec!["Not now".into()]);
    answer(&mut s, &sent, Err(refused));
    assert_eq!(overlay!(s, Ask).sent.error, None);
    warns(&s, "Not now");
    press(&mut s, "<Esc>WcLGTM");
    answer(&mut s, &sent, Ok(()));
    says(&s, "Ready for review");
    assert!(
        matches!(&s.overlay, Some(Overlay::Compose(c)) if c.text() == "LGTM"),
        "the comment is still being written"
    );
}

/// A star answered while a re-run from a list's row is on its way, or
/// once GitHub accepted it, leaves the page following that run.
#[test]
fn a_star_leaves_the_wait_for_another_change_alone() {
    let mut s = super::with_workflow();
    let run = Data::Run(Box::new(crate::fixtures::workflow_run()));
    fetched(&mut s, DataKey::Run(ghtui(), 7, None), run);
    let rerun = press(&mut s, ",R<Enter>");
    press(&mut s, "<Esc>");
    let star = press(&mut s, ",*");
    answer(&mut s, &star, Ok(()));
    answer(&mut s, &rerun, Ok(()));
    // The page follows the run, not the list.
    let run = vec![DataKey::Run(ghtui(), 7, None)];
    assert_eq!(crate::follow::poll(&mut s), run);
    answer(&mut s, &star, Ok(()));
    assert_eq!(crate::follow::poll(&mut s), run);
}

/// Approving from the pull request's page reads your saved drafts
/// first, submits them with the approval, and saves what's left.
#[test]
fn approving_from_the_page_takes_your_saved_drafts() {
    let mut s = with_pr();
    s.viewer = Some("me".into());
    assert_eq!(
        press(&mut s, ",a"),
        vec![
            Cmd::Api(Api::FetchPendingReview(pr())),
            Cmd::LoadReview(pr())
        ]
    );
    let Some(Overlay::Submit(dialog)) = &s.overlay else {
        panic!("no dialog");
    };
    assert_eq!(
        (dialog.event, dialog.quick),
        (crate::api::model::ReviewEvent::Approve, true)
    );
    // Enter before the drafts are read waits for them.
    assert!(press(&mut s, "<Enter>").is_empty());

    let draft = crate::store::DraftComment {
        id: 3,
        path: "src/lib.rs".into(),
        body: "nit".into(),
        side: DraftSide::Right,
        line: Some(4),
        start_line: None,
        start_side: None,
        commit: pr_detail().head_oid,
        error: None,
    };
    let saved = ReviewState {
        pending: vec![draft.clone()],
        ..ReviewState::default()
    };
    update(
        &mut s,
        Msg::Diff(pr().into(), DiffMsg::ReviewLoaded(Ok(saved))),
    );
    press(&mut s, "LGTM");
    let cmds = press(&mut s, "<Enter>");
    let submit = crate::review::Submit {
        pr: pr(),
        head: crate::git::Oid::parse(&pr_detail().head_oid).unwrap(),
        drafts: vec![draft],
        event: crate::api::model::ReviewEvent::Approve,
        body: "LGTM".into(),
        seen: 0,
    };
    assert_eq!(cmds, vec![fixtures::start(submit.clone())]);
    let outcome = crate::review::SubmitOutcome {
        accepted: vec![3],
        submitted: true,
        ..Default::default()
    };
    let cmds = fixtures::done(&mut s, submit, Ok(outcome));
    assert!(s.overlay.is_none());
    says(&s, "Approved gold-silver-copper/ghtui#12");
    let left = ReviewState {
        last_reviewed_head: Some(pr_detail().head_oid),
        ..ReviewState::default()
    };
    assert!(cmds.contains(&Cmd::SaveReview(pr(), left)), "{cmds:?}");
}

/// A finished run offers its failed jobs or all of them; a job adds
/// itself; a finished run has nothing to cancel.
#[test]
fn runs_rerun_and_cancel() {
    let mut s = with_run();
    press(&mut s, ",R");
    let choices: Vec<&str> = overlay!(s, Ask)
        .choices
        .iter()
        .map(|(l, _)| l.as_str())
        .collect();
    assert_eq!(choices, ["Re-run failed jobs", "Re-run all jobs"]);
    assert_eq!(
        press(&mut s, "<Enter>"),
        vec![fixtures::change(
            Change::Rerun {
                repo: ghtui(),
                run: 7,
                failed_only: true
            },
            By::Confirm(Subject::Run(ghtui(), 7, None))
        )]
    );
    let mut s = with_run();
    press(&mut s, ",x");
    says(&s, "The run has finished: there's nothing to cancel");

    let mut s = with_job(None);
    press(&mut s, ",R");
    assert_eq!(overlay!(s, Ask).title, "Re-run CI #412?");
    let first = overlay!(s, Ask).choices.first().map(|(l, _)| l.clone());
    assert_eq!(first.as_deref(), Some("Re-run this job"));
}

/// In the inbox, `,n` marks the selected thread read and `,d` done,
/// asking nothing first (as GitHub doesn't); the inbox follows GitHub
/// until it shows the change. Read already, there's nothing to mark.
#[test]
fn notifications_are_marked_read_and_done() {
    let mut s = with_notifications();
    let key = DataKey::Notifications(String::new());
    let thread = |id: &str| Subject::Thread {
        id: id.into(),
        query: String::new(),
    };
    let sent = press(&mut s, ",n");
    let read = Change::MarkRead {
        thread: "101".into(),
    };
    assert_eq!(sent, vec![fixtures::change(read, By::Act(thread("101")))]);
    answer(&mut s, &sent, Ok(()));
    says(&s, "Marked read");
    // Until GitHub shows it read, the inbox is fetched again.
    let mut inbox = crate::fixtures::notifications();
    fetched(
        &mut s,
        key.clone(),
        Data::Notifications(Box::new(inbox.clone())),
    );
    assert!(s.following.is_some());
    if let Some(n) = inbox.list.items.first_mut() {
        n.unread = false;
    }
    fetched(
        &mut s,
        key.clone(),
        Data::Notifications(Box::new(inbox.clone())),
    );
    assert!(s.following.is_none());
    press(&mut s, ",n");
    says(&s, "It's read already");

    // Done, the thread leaves the inbox (GitHub's API lists it on, read,
    // but ghtui keeps it out until GitHub updates it again).
    let sent = press(&mut s, "jj,d");
    let done = Change::MarkDone {
        thread: ThreadAt {
            id: "103".into(),
            updated_at: "2026-10-02T12:00:00Z".into(),
        },
    };
    assert_eq!(sent, vec![fixtures::change(done, By::Act(thread("103")))]);
    answer(&mut s, &sent, Ok(()));
    says(&s, "Marked done");
    assert!(s.following.is_some());
    inbox.list.items.retain(|n| n.id != "103");
    fetched(&mut s, key, Data::Notifications(Box::new(inbox)));
    assert!(s.following.is_none());

    // Elsewhere, there's no notification to mark.
    let mut s = with_repo();
    press(&mut s, ",n");
    says(&s, "works on lists");
}

/// `,N` marks everything the inbox shows read, and `,D` done: asked
/// first, saying how many. Read is one request, up to when the inbox was
/// fetched (GitHub leaves a thread marked up to its own time unread);
/// done, one a thread loaded. The inbox follows GitHub until it shows
/// the change, and the unread count is asked again.
#[test]
fn the_inbox_is_marked_read_and_done_all_at_once() {
    let mut s = with_notifications();
    let key = DataKey::Notifications(String::new());
    let all = Subject::Inbox {
        query: String::new(),
    };
    assert!(press(&mut s, ",N").is_empty());
    let c = overlay!(s, Ask);
    assert_eq!(c.title, "Mark 2 notifications read?");
    let older = "And the older ones further down, not loaded yet";
    assert!(c.facts.iter().any(|(f, _)| f == older), "{:?}", c.facts);
    let read = Change::MarkAllRead {
        repo: None,
        last_read_at: "2026-10-03T12:00:00Z".into(),
    };
    let sent = press(&mut s, "<Enter>");
    assert_eq!(sent, vec![fixtures::change(read, By::Confirm(all.clone()))]);
    let cmds = answer(&mut s, &sent, Ok(()));
    says(&s, "Marked read");
    assert!(fixtures::asks(&cmds, &DataKey::Unread), "{cmds:?}");
    fetched(&mut s, DataKey::Unread, fixtures::sample(&DataKey::Unread));
    // GitHub marks them a moment later: until then, the inbox is asked
    // again.
    let mut inbox = crate::fixtures::notifications();
    let shown = |i: &crate::api::notifications::Inbox| Data::Notifications(Box::new(i.clone()));
    fetched(&mut s, key.clone(), shown(&inbox));
    assert!(s.following.is_some());
    inbox.list.items.iter_mut().for_each(|n| n.unread = false);
    let cmds = fetched(&mut s, key.clone(), shown(&inbox));
    assert!(s.following.is_none());
    assert!(fixtures::asks(&cmds, &DataKey::Unread), "{cmds:?}");
    press(&mut s, ",N");
    says(&s, "Nothing shown is unread");

    // Done: each thread loaded, and only those.
    press(&mut s, ",D");
    let c = overlay!(s, Ask);
    assert_eq!(c.title, "Mark 4 notifications done?");
    let loaded = "Only the 4 loaded: those further down stay as they are";
    assert!(c.facts.iter().any(|(f, _)| f == loaded), "{:?}", c.facts);
    let sent = press(&mut s, "<Enter>");
    let rows = crate::fixtures::notifications().list.items;
    let threads: Vec<ThreadAt> = rows.iter().map(ThreadAt::of).collect();
    let ids: Vec<&str> = threads.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["101", "102", "103", "104"]);
    let done = Change::MarkEach {
        threads: threads.clone(),
        done: true,
    };
    assert_eq!(sent, vec![fixtures::change(done, By::Confirm(all))]);
    answer(&mut s, &sent, Ok(()));
    says(&s, "Marked 4 notifications done");
    (inbox.list.items, inbox.list.next) = (Vec::new(), None);
    fetched(&mut s, key, shown(&inbox));
    assert!(s.following.is_none());

    // REST can't mark read by a reason: what's shown is marked thread
    // by thread.
    let route = Route::Search(search::Notifications {
        query: "reason:mention".to_owned(),
    });
    super::screens::open(&mut s, route, shown(&crate::fixtures::notifications()));
    press(&mut s, ",N");
    let each = Change::MarkEach {
        threads: threads[..2].to_vec(),
        done: false,
    };
    assert_eq!(
        overlay!(s, Ask).choices.first().map(|(_, c)| c),
        Some(&Answer::Change(each))
    );
}

/// A job that ended in a run still going is the run's to cancel, and
/// not to re-run yet.
#[test]
fn a_finished_job_in_a_running_run_cancels_the_run() {
    let mut s = with_job(None);
    let (mut job, _) = crate::fixtures::job();
    job.run.outcome = CheckOutcome::Pending;
    fetched(
        &mut s,
        DataKey::Job(ghtui(), 2),
        Data::Job(Box::new(job.clone())),
    );
    press(&mut s, ",R");
    says(&s, "The run is still going: ,x cancels it");
    press(&mut s, ",x");
    assert_eq!(overlay!(s, Ask).title, "Cancel CI #412?");
    let cancel = Change::CancelRun {
        repo: ghtui(),
        run: 7,
    };
    let sent = press(&mut s, "<Enter>");
    assert_eq!(
        sent,
        vec![fixtures::change(
            cancel,
            By::Confirm(Subject::Job(ghtui(), Some(7), 2))
        )]
    );
    answer(&mut s, &sent, Ok(()));
    // GitHub takes a while to stop it: it's asked for once.
    press(&mut s, ",x");
    says(&s, "hasn't shown the last change");
    assert!(s.busy().is_some_and(|b| b.starts_with("Cancel requested")));
    job.run.outcome = CheckOutcome::Cancelled;
    fetched(&mut s, DataKey::Job(ghtui(), 2), Data::Job(Box::new(job)));
    assert!(s.following.is_none());
}

/// Re-running a job, the page moves to the job GitHub re-runs it as,
/// once GitHub has it, and follows that one.
#[test]
fn a_rerun_job_is_followed_to_its_new_attempt() {
    let mut s = with_job(None);
    let rerun = Change::RerunJob {
        repo: ghtui(),
        job: 2,
    };
    let by = By::Confirm(Subject::Job(ghtui(), Some(7), 2));
    fixtures::changed(&mut s, rerun, by, Ok(()));
    let (mut job, _) = crate::fixtures::job();
    job.run.attempt = 2;
    job.run.outcome = CheckOutcome::Pending;
    fetched(
        &mut s,
        DataKey::Job(ghtui(), 2),
        Data::Job(Box::new(job.clone())),
    );
    assert!(
        matches!(s.route(), Some(Route::Runs(runs::Job { job: 2, .. }))),
        "not there yet"
    );
    job.run.rerun = Some(9);
    let cmds = update(
        &mut s,
        Msg::Fetched {
            key: DataKey::Job(ghtui(), 2),
            result: Ok(Data::Job(Box::new(job))),
            cached_at: None,
            source: None,
        },
    );
    assert!(
        matches!(s.route(), Some(Route::Runs(runs::Job { job: 9, .. }))),
        "{:?}",
        s.route()
    );
    let fetch = Cmd::Api(Api::Fetch {
        key: DataKey::Job(ghtui(), 9),
        cached: true,
    });
    assert!(cmds.contains(&fetch), "{cmds:?}");
}
