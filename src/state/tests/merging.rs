use crate::api::change::MergeMethod;
use crate::api::model::{AutoMerge, MergeState, QueueState, Queued};

use super::*;
use crate::act::{By, Subject};
use crate::forms::Answer;

fn choices(s: &State) -> Vec<(&str, &Change)> {
    match &s.overlay {
        Some(Overlay::Ask(c)) => (c.choices.iter())
            .filter_map(|(l, a)| match a {
                Answer::Change(c) => Some((l.as_str(), c)),
                _ => None,
            })
            .collect(),
        _ => panic!("no confirmation"),
    }
}

/// The Space menu's row for merging: its label, and why it can't be done.
fn merge_row(s: &State) -> (String, Option<String>) {
    let row = s.doables().into_iter().find(|d| d.action == Action::Merge);
    let row = row.unwrap();
    (row.label, row.unavailable)
}

/// Required checks still running.
fn checks_running(d: &mut PrDetail) {
    d.checks = Some(ChecksState::Pending);
    d.merge_state = MergeState::Blocked;
}

fn alice_squashes() -> Option<AutoMerge> {
    Some(AutoMerge {
        by: "alice".into(),
        method: MergeMethod::Squash,
    })
}

/// Auto-merge, once enabled, says who enabled it.
#[test]
fn auto_merge_says_who_enabled_it() {
    let mut s = pr_with(|d| {
        checks_running(d);
        d.auto_merge = alice_squashes();
    });
    press(&mut s, "<End>");
    let screen = screen_text(&s);
    assert!(screen.contains("Auto-merge enabled by alice"), "{screen}");
}

/// A queued pull request says where it is in the merge queue.
#[test]
fn a_queued_pull_request_says_its_place() {
    let mut s = pr_with(|d| {
        d.checks = Some(ChecksState::Passing);
        d.merge_queue = true;
        d.queued = Some(Queued {
            position: 2,
            state: QueueState::AwaitingChecks,
        });
    });
    press(&mut s, "<End>");
    let screen = screen_text(&s);
    assert!(
        screen.contains("#2 in the merge queue for main"),
        "{screen}"
    );
}

/// While required checks run, `,m` offers to merge when they pass,
/// rather than refusing; the page follows GitHub until it shows
/// auto-merge on, and then `,m` turns it off.
#[test]
fn merging_while_checks_run_offers_merge_when_ready() {
    // Without leave to enable auto-merge, it says why.
    let s = pr_with(checks_running);
    let (_, why) = merge_row(&s);
    assert_eq!(
        why.as_deref(),
        Some("GitHub says it's blocked: a required review or check is missing")
    );

    let mut s = pr_with(|d| {
        checks_running(d);
        d.may.enable_auto_merge = true;
    });
    press(&mut s, "M");
    let labels: Vec<&str> = choices(&s).into_iter().map(|(l, _)| l).collect();
    assert_eq!(
        labels,
        [
            "Squash and merge when ready",
            "Create a merge commit when ready"
        ]
    );
    let enable = Change::EnableAutoMerge {
        pr: NodeId::new("PR_12"),
        method: MergeMethod::Squash,
        head: pr_detail().head_oid,
    };
    let sent = press(&mut s, "<Enter>");
    assert_eq!(
        sent,
        vec![fixtures::change(enable, By::Confirm(Subject::Pr(pr())))]
    );
    answer(&mut s, &sent, Ok(()));
    says(&s, "Auto-merge enabled");
    // GitHub doesn't show it yet: the page waits.
    pr_shows(&mut s, |d| {
        checks_running(d);
        d.may.enable_auto_merge = true;
    });
    assert!(s.following.is_some());
    pr_shows(&mut s, |d| {
        checks_running(d);
        d.auto_merge = alice_squashes();
        d.may.disable_auto_merge = true;
    });
    assert!(s.following.is_none());

    assert_eq!(merge_row(&s), ("Disable auto-merge".to_owned(), None));
    press(&mut s, "M");
    let disable = Change::DisableAutoMerge {
        pr: NodeId::new("PR_12"),
    };
    assert_eq!(choices(&s), [("Disable auto-merge", &disable)]);
    let sent = press(&mut s, "<Enter>");
    answer(&mut s, &sent, Ok(()));
    pr_shows(&mut s, checks_running);
    assert!(s.following.is_none());

    // Mergeable now, with checks that aren't required still running:
    // waiting for them comes first, merging now after.
    let mut s = pr_with(|d| {
        d.checks = Some(ChecksState::Pending);
        d.may.enable_auto_merge = true;
    });
    press(&mut s, "M");
    let labels: Vec<&str> = choices(&s).into_iter().map(|(l, _)| l).collect();
    assert_eq!(labels.len(), 4);
    assert_eq!(labels[0], "Squash and merge when ready");
    assert_eq!(labels[2], "Squash and merge");
}

/// Where the base branch has a merge queue, merging adds it to the
/// queue; once GitHub shows it queued, `,m` takes it out again.
#[test]
fn a_merge_queue_takes_it_in_and_lets_it_go() {
    let queue = |d: &mut PrDetail| d.merge_queue = true;
    let mut s = pr_with(queue);
    assert_eq!(merge_row(&s), ("Add to the merge queue".to_owned(), None));
    press(&mut s, "M");
    let enqueue = Change::Enqueue {
        pr: NodeId::new("PR_12"),
        head: pr_detail().head_oid,
    };
    assert_eq!(choices(&s), [("Add to the merge queue", &enqueue)]);
    let sent = press(&mut s, "<Enter>");
    answer(&mut s, &sent, Ok(()));
    says(&s, "Added to the merge queue");
    assert!(s.following.is_some());
    let queued = |d: &mut PrDetail| {
        d.merge_queue = true;
        d.queued = Some(Queued {
            position: 1,
            state: QueueState::Queued,
        });
    };
    pr_shows(&mut s, queued);
    assert!(s.following.is_none());

    assert_eq!(
        merge_row(&s),
        ("Remove from the merge queue".to_owned(), None)
    );
    press(&mut s, "M");
    let dequeue = Change::Dequeue {
        pr: NodeId::new("PR_12"),
    };
    assert_eq!(choices(&s), [("Remove from the queue", &dequeue)]);
    let sent = press(&mut s, "<Enter>");
    answer(&mut s, &sent, Ok(()));
    pr_shows(&mut s, queued);
    assert!(s.following.is_some());
    pr_shows(&mut s, queue);
    assert!(s.following.is_none());

    // Taking it out takes write access, and the menu says so.
    let s = pr_with(|d| {
        queued(d);
        d.may.merge = false;
    });
    let (_, why) = merge_row(&s);
    assert_eq!(
        why.as_deref(),
        Some("Removing it from the merge queue takes write access to gold-silver-copper/ghtui")
    );
}
