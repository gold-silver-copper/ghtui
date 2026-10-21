//! Watching pages other than a pull request's review: a running job, the
//! Actions tab, an issue.

use super::*;
use std::collections::HashSet;

use crate::api::Polled;
use crate::api::browse::{CheckOutcome, JobLog};
use crate::live::Ask;
use crate::live::page::Probe;

/// The asks the page watch's next timer sends, and how long it waits.
fn next_ask(s: &mut State) -> (Vec<Ask<Probe>>, u64) {
    let (cmds, ms) = next_watch(s);
    match &cmds[..] {
        [Cmd::Api(Api::Watch { asks, .. })] => (asks.clone(), ms),
        _ => panic!("{cmds:?}"),
    }
}

fn answer_with(s: &mut State, answers: Vec<(Probe, Result<Polled, ApiError>)>) -> Vec<Cmd> {
    let what = s.page_watch.as_ref().expect("a watch").what.clone();
    update(s, Msg::Watched(what, answers))
}

/// The fixture's issue as REST has it, with `comments` comments.
fn rest_issue(comments: u64) -> String {
    let issue = crate::fixtures::issue();
    serde_json::json!({
        "title": issue.title,
        "body": issue.body,
        "state": "open",
        "state_reason": null,
        "comments": comments,
        "labels": [{"name": "ui"}, {"name": "bug"}],
        "assignees": [{"login": "hubot"}],
    })
    .to_string()
}

fn on_issue() -> (State, DataKey) {
    (at(with_repo(), issue(14)), DataKey::Issue(ghtui(), 14))
}

/// An issue on screen is asked about with ETags: the first answers are
/// only what the page has; a change fetches it again, and another
/// author's new comment is marked New and said, yours not.
#[test]
fn an_issue_is_watched_and_another_authors_comment_is_new() {
    let (mut s, key) = on_issue();
    let (asks, wait) = next_ask(&mut s);
    assert_eq!(wait, crate::live::FIRST_MS);
    assert!(matches!(
        &asks[..],
        [
            Ask {
                probe: Probe::Issue(_, 14),
                etag: None
            },
            Ask {
                probe: Probe::IssueComments(_, 14, _),
                etag: None
            },
        ]
    ));
    let comments = asks[1].probe.clone();
    let first = vec![
        (Probe::Issue(ghtui(), 14), changed("\"i1\"", &rest_issue(1))),
        (comments.clone(), changed("\"c1\"", "[]")),
    ];
    assert_eq!(
        crate::fixtures::asked(&answer_with(&mut s, first)),
        Vec::new()
    );

    let (asks, wait) = next_ask(&mut s);
    assert!(wait > crate::live::FIRST_MS);
    assert_eq!(asks[0].etag.as_deref(), Some("\"i1\""));
    let came = vec![
        (Probe::Issue(ghtui(), 14), changed("\"i2\"", &rest_issue(3))),
        (comments, Ok(Polled::Same)),
    ];
    assert_eq!(
        crate::fixtures::asked(&answer_with(&mut s, came)),
        std::slice::from_ref(&key)
    );

    let mut issue = crate::fixtures::issue();
    let comment = |id, author: &str| crate::api::browse::Comment {
        id: Some(id),
        author: author.into(),
        body: format!("from {author}"),
        created_at: "2026-10-03T12:00:00Z".into(),
        editable: None,
        hiding: crate::api::manage::Hiding::default(),
    };
    let first = issue.comments().next().cloned().expect("a comment");
    issue.timeline = vec![
        crate::api::timeline::Item::Comment(first),
        crate::api::timeline::Item::Comment(comment(2, "monalisa")),
        crate::api::timeline::Item::Comment(comment(3, "octocat")),
    ]
    .into();
    issue.comment_count = 3;
    fetched(&mut s, key, Data::Issue(Some(Box::new(issue))));
    says(&s, "New on gold-silver-copper/ghtui#14: 1 comment");
    let fresh = crate::live::page::fresh(&s, &route(&s)).cloned();
    assert_eq!(fresh, Some(HashSet::from([2])));
    s.sync_page();
    // The badge is on the right of the comment's header.
    let lines = &page(&s).page().lines;
    let badged: Vec<String> = (lines.iter())
        .filter(|l| l.right.iter().any(|r| r.text().contains("New")))
        .map(crate::ui::page::PageLine::text)
        .collect();
    assert_eq!(badged, ["monalisa commented just now"]);
}

/// A first answer newer than the page (a comment came in between
/// fetching it and the first ask) is a change: it's fetched again.
#[test]
fn a_first_answer_newer_than_the_issue_is_fetched() {
    let (mut s, key) = on_issue();
    let (asks, _) = next_ask(&mut s);
    let comments = asks[1].probe.clone();
    let first = vec![
        (Probe::Issue(ghtui(), 14), changed("\"i1\"", &rest_issue(2))),
        (comments, changed("\"c1\"", "[]")),
    ];
    assert_eq!(crate::fixtures::asked(&answer_with(&mut s, first)), [key]);
}

/// A running job's page follows the job: it asks after the job, stays on
/// the step running, and once the job ends, fetches its log, opens on its
/// failure and says how it ended. Then the asking stops.
#[test]
fn a_running_job_is_followed_to_its_end() {
    let mut s = with_job(None);
    let (done, log) = crate::fixtures::job();
    let mut running = done.clone();
    running.outcome = CheckOutcome::Pending;
    running.run.outcome = CheckOutcome::Pending;
    for step in &mut running.steps[2..] {
        step.outcome = CheckOutcome::Pending;
        step.completed_at = None;
    }
    running.steps[3].started_at = None;
    fetched(&mut s, DataKey::Job(ghtui(), 2), Data::from(running));
    let waiting = JobLog {
        running: true,
        ..JobLog::default()
    };
    fetched(
        &mut s,
        DataKey::JobLog(ghtui(), 2),
        Data::Log(Arc::new(waiting)),
    );
    let (asks, _) = next_ask(&mut s);
    let shown = |s: &State| {
        let p = page(s).page();
        p.jump
            .and_then(|j| p.lines.get(j))
            .map(crate::ui::page::PageLine::text)
            .unwrap_or_default()
    };
    assert!(shown(&s).contains("cargo test"), "{}", shown(&s));
    assert_eq!(asks[0].probe, Probe::Job(ghtui(), 2));
    let body = serde_json::json!({
        "id": 2, "run_id": 7, "name": "test (ubuntu)", "status": "completed",
        "conclusion": "failure", "started_at": null, "completed_at": null, "steps": [],
    });
    let cmds = answer_with(
        &mut s,
        vec![(asks[0].probe.clone(), changed("\"j\"", &body.to_string()))],
    );
    let keys = crate::fixtures::asked(&cmds);
    assert!(keys.contains(&DataKey::Job(ghtui(), 2)), "{keys:?}");
    assert!(keys.contains(&DataKey::JobLog(ghtui(), 2)), "{keys:?}");

    fetched(&mut s, DataKey::Job(ghtui(), 2), Data::from(done));
    says(&s, "test (ubuntu) failed");
    fetched(
        &mut s,
        DataKey::JobLog(ghtui(), 2),
        Data::Log(Arc::new(log)),
    );
    s.sync_page();
    assert!(shown(&s).contains("exit code 101"), "{}", shown(&s));
    let asking = timers(&mut s)
        .into_iter()
        .any(|cmd| matches!(cmd, Cmd::Timer(Timer::Watch(_), _)));
    assert!(!asking);
}

/// The Actions tab is asked about, and a run that wasn't there is said.
#[test]
fn new_runs_on_the_actions_tab_are_said() {
    let mut s = at(with_repo(), Route::actions(ghtui()));
    let query = String::new();
    let key = DataKey::Runs(ghtui(), query.clone());
    let (_, runs) = crate::fixtures::workflow_runs();
    let (asks, _) = next_ask(&mut s);
    assert_eq!(asks[0].probe, Probe::Runs(ghtui(), query));
    let wire = serde_json::json!({ "total_count": 1, "workflow_runs": [{
        "id": 999, "name": "CI", "display_title": "New", "run_number": 99, "event": "push",
        "head_branch": "main", "head_sha": "abc", "status": "queued", "conclusion": null,
        "actor": {"login": "monalisa"}, "run_started_at": null, "created_at": null,
        "updated_at": null,
    }]});
    let cmds = answer_with(
        &mut s,
        vec![(asks[0].probe.clone(), changed("\"r\"", &wire.to_string()))],
    );
    assert_eq!(crate::fixtures::asked(&cmds), std::slice::from_ref(&key));
    let mut more = runs;
    let mut new = more.items[0].clone();
    (new.id, new.actor) = (999, Some("monalisa".into()));
    more.items.insert(0, new);
    fetched(&mut s, key, Data::from(more));
    says(&s, "New on the Actions tab: 1 run");
}
