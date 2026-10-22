//! A pull request's conversation, checks and actions.

use super::*;
use crate::api::change::MergeMethod;
use crate::api::model::{AutoMerge, QueueState, Queued};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrTab {
    Conversation,
    Commits,
    Checks,
    /// Where its review stands: drafts, open threads, reviewers, what
    /// blocks the merge. ghtui's own: GitHub has no page for it.
    Review,
    /// One review, by GitHub's number for it: its summary and comments,
    /// the changes it reviewed and what changed since. ghtui's own too.
    Reviewed(u64),
}

/// The lines under a PR's title: branches, size (the state is in the
/// sticky title above).
pub(super) fn pr_summary(page: &mut Page, pr: &PrRef, d: &PrDetail, now: u64) {
    let author = login_seg(page, &d.author, Role::Strong);
    let head = match &d.head_repo {
        Some(repo) if *repo != pr.repo.to_string() => {
            format!("{}:{}", repo.split('/').next().unwrap_or(repo), d.head_ref)
        }
        _ => d.head_ref.clone(),
    };
    let wants = match d.state {
        PrState::Merged => " merged into ",
        PrState::Closed => " wanted to merge into ",
        PrState::Open | PrState::Draft => " wants to merge into ",
    };
    let mut segs = vec![
        author,
        meta(wants),
        chip(&d.base_ref, Bg::PrimaryContainer),
        meta(" from "),
        chip(head, Bg::PrimaryContainer),
    ];
    if d.head_gone {
        segs.push(meta(" · branch deleted"));
    }
    page.wrapped(segs, 0, Frame::None);
    let mut subline = Vec::from(changes(d.additions, d.deletions));
    subline.push(meta(format!(
        " · opened {} · updated {}",
        time::ago_iso(&d.created_at, now),
        time::ago_iso(&d.updated_at, now)
    )));
    if let Some(locked) = d.locked {
        let locked = super::conversation::locked_said(locked);
        subline.extend([meta("  "), chip(locked, Bg::TertiaryContainer)]);
    }
    labels(&mut subline, &d.labels);
    page.wrapped(subline, 0, Frame::None);
    page.rule(0, Frame::None);
}

/// GitHub's merge box: checks, reviews, conflicts, auto-merge and the
/// merge queue. On a timeline, it's joined to what's above.
pub(super) fn merge_box(page: &mut Page, pr: &PrRef, d: &PrDetail, icons: Icons, timeline: bool) {
    let readiness = d.readiness();
    if matches!(readiness, Readiness::Merged | Readiness::Closed) {
        return;
    }
    if timeline {
        connector(page);
    }
    let (headline, role) = match (readiness, d.checks, d.review) {
        _ if d.queued.is_some() => ("Queued to merge", Role::Accent),
        (Readiness::Draft, ..) => ("This pull request is still a draft", Role::Meta),
        (Readiness::Conflicts, ..) => (
            "This branch has conflicts that must be resolved",
            Role::Error,
        ),
        (_, Some(ChecksState::Failing), _) => ("Some checks were not successful", Role::Error),
        (_, Some(ChecksState::Pending), _) => ("Some checks haven't completed yet", Role::Accent),
        (_, _, Some(ReviewDecision::ChangesRequested)) => ("Changes requested", Role::Error),
        (_, _, Some(ReviewDecision::ReviewRequired)) => ("Review required", Role::Accent),
        (Readiness::Checking, ..) => ("Checking whether it can be merged…", Role::Accent),
        (Readiness::Behind, ..) => (
            "This branch is out-of-date with the base branch",
            Role::Error,
        ),
        (Readiness::Blocked, ..) => ("Merging is blocked", Role::Error),
        _ => ("This branch can be merged", Role::Success),
    };
    page.box_top(vec![Seg::new(headline, role)], Vec::new());
    // Each row: passed, failed, or neither yet.
    let review = d.review.map(|r| match r {
        ReviewDecision::Approved => (Some(true), "Changes approved"),
        ReviewDecision::ChangesRequested => (Some(false), "Changes requested"),
        ReviewDecision::ReviewRequired => (None, "Review required"),
    });
    let checks = d.checks.map(|c| match c {
        ChecksState::Passing => (Some(true), "All checks have passed"),
        ChecksState::Failing => (Some(false), "Some checks were not successful"),
        ChecksState::Pending => (None, "Some checks haven't completed yet"),
    });
    let conflicts = match d.mergeable {
        Mergeable::Yes => (Some(true), "No conflicts with the base branch".to_owned()),
        Mergeable::Conflicting => (Some(false), "This branch has conflicts".to_owned()),
        Mergeable::Unknown => (None, "Checking for conflicts…".to_owned()),
    };
    // Behind is a problem only where the repository requires it up to date.
    let behind = d.behind_by.filter(|n| *n > 0).map(|n| {
        let mark = if readiness == Readiness::Behind {
            ("✗", Role::Error)
        } else {
            ("◦", Role::Meta)
        };
        (
            mark,
            format!("{} behind {}", plural(n, "commit"), d.base_ref),
        )
    });
    // Where it's headed: the queue, auto-merge, or a queue it can join.
    let queue = d.queued.map(|q| {
        let ok = (q.state == QueueState::Unmergeable).then_some(false);
        (ok, queue_place(&q, &d.base_ref))
    });
    let auto = (d.auto_merge.as_ref()).map(|a| (None, auto_merging(a, d.merge_queue)));
    let rows = [review, checks].into_iter().flatten();
    let rows = (queue.into_iter().chain(auto))
        .chain(rows.map(|(ok, text)| (ok, text.to_owned())))
        .chain([conflicts]);
    let rows = rows.map(|(ok, text)| {
        let mark = match ok {
            Some(true) => ("✓", Role::Success),
            Some(false) => ("✗", Role::Error),
            None => ("●", Role::Accent),
        };
        (mark, text)
    });
    let joins = (d.merge_queue && d.queued.is_none()).then(|| {
        (
            ("◦", Role::Meta),
            format!("{} merges through a merge queue", d.base_ref),
        )
    });
    for ((icon, role), text) in rows.chain(behind).chain(joins) {
        let mark = Seg::new(format!("{icon}  "), role);
        body(page, vec![mark, Seg::new(text, Role::Body)]);
    }
    page.box_rule();
    item(page, url::pull_tab(pr, "files"), |page, link| {
        let review = match d.changed_files {
            1 => "Review the changed file".to_owned(),
            n => format!("Review the {n} changed files"),
        };
        let external = meta(format!("{}  ", icons.external()));
        body(page, vec![external, Seg::linked(review, Role::Link, link)]);
    });
    if d.merge_queue {
        let queue = url::merge_queue(&pr.repo, &d.base_ref);
        item(page, queue, |page, link| {
            let text = format!("See the merge queue for {}", d.base_ref);
            body(page, vec![meta("   "), Seg::linked(text, Role::Link, link)]);
        });
    }
    page.box_bottom();
}

/// "Auto-merge enabled by alice: squashed and merged when every
/// requirement is met", as the merge box and the dialog say it.
pub fn auto_merging(auto: &AutoMerge, queue: bool) -> String {
    let how = if queue {
        "added to the merge queue"
    } else {
        match auto.method {
            MergeMethod::Merge => "merged with a merge commit",
            MergeMethod::Squash => "squashed and merged",
            MergeMethod::Rebase => "rebased and merged",
        }
    };
    let by = &auto.by;
    format!("Auto-merge enabled by {by}: {how} when every requirement is met")
}

/// "#2 in the merge queue for main: waiting for its checks", as the
/// merge box and the dialog say it.
pub fn queue_place(queued: &Queued, base: &str) -> String {
    let place = match queued.position {
        1 => "Next".to_owned(),
        n => format!("#{n}"),
    };
    let state = queue_state(queued.state);
    format!("{place} in the merge queue for {base}: {state}")
}

/// Where an entry of a merge queue stands.
fn queue_state(state: QueueState) -> &'static str {
    match state {
        QueueState::AwaitingChecks => "waiting for its checks",
        QueueState::Queued => "waiting its turn",
        QueueState::Mergeable => "ready to merge",
        QueueState::Locked => "merging now",
        QueueState::Unmergeable => "it can't be merged",
    }
}

/// A branch's merge queue, next first: each pull request, where it
/// stands, who queued it and when, and when GitHub expects it merged.
pub fn merge_queue(
    page: &mut Page,
    repo: &RepoId,
    branch: &str,
    q: &crate::api::queue::MergeQueue,
    now: u64,
) {
    let mut right = Vec::new();
    if let Some(method) = &q.method {
        right.push(meta(format!("{} merges", method.to_lowercase())));
    }
    let title = counted(&format!("Merge queue for {branch}"), &q.entries);
    list_box(
        page,
        title,
        right,
        &q.entries,
        "The queue is empty.",
        |page, e| {
            let pr = |number| PrRef {
                repo: repo.clone(),
                number,
            };
            let target = e.number.map(|n| url::pull(&pr(n)));
            maybe_item(page, target, |page, link| {
                let ok = match e.state {
                    QueueState::Unmergeable => Role::Error,
                    QueueState::Mergeable | QueueState::Locked => Role::Success,
                    QueueState::AwaitingChecks | QueueState::Queued => Role::Accent,
                };
                let place = match e.position {
                    1 => "Next".to_owned(),
                    n => format!("#{n}"),
                };
                let title = match e.number {
                    Some(n) => format!("{} #{n}", e.title),
                    None => "A pull request you can't see".to_owned(),
                };
                let segs = vec![
                    Seg::new(format!("● {place}  "), ok),
                    Seg::linked(title, Role::Strong, link),
                ];
                let right = vec![meta(queue_state(e.state))];
                page.box_line(segs, right, 0);
                let mut sub = Vec::new();
                if let Some(by) = &e.by {
                    sub.push(format!("queued by {by}"));
                }
                sub.push(time::ago_iso(&e.enqueued_at, now));
                if let Some(eta) = e.eta {
                    let mins = eta.div_ceil(60);
                    let (h, m) = (mins / 60, mins % 60);
                    let when = if h == 0 {
                        format!("{m}m")
                    } else {
                        format!("{h}h {m}m")
                    };
                    sub.push(format!("merges in about {when}"));
                }
                if e.jump {
                    sub.push("jumped the queue".to_owned());
                }
                if e.solo {
                    sub.push("merges alone".to_owned());
                }
                body(page, vec![meta(sub.join(" · "))]);
            });
        },
    );
}

/// A pull request's conversation: its description, then its timeline
/// (comments, reviews with their threads, events), the merge box and the
/// comment box. The sidebar has its reviewers, what it will close and the
/// rest of its fields.
pub fn pr_conversation(
    page: &mut Page,
    pr: &PrRef,
    d: &PrDetail,
    activity: Fetched<'_, PrActivity>,
    projects: Fetched<'_, Capped<crate::api::project::InProject>>,
    marks: Marks<'_>,
    cx: PageCtx<'_>,
) {
    let PageCtx { icons, keys, now } = cx;
    // The sidebar doesn't wait for the conversation, which says how it's
    // going; only its reviewers and links come from it.
    #[expect(
        clippy::disallowed_methods,
        reason = "the conversation says how it's going"
    )]
    let a = activity.ready_unchecked();
    let aside = page.build_aside(|side| {
        match a {
            Some(a) => {
                let (head, author) = (&d.head_oid, &d.author);
                timeline::reviewers_aside(side, pr, head, &a.latest, &a.requested, author);
            }
            None => people(side, "Reviewers", &[], None, ""),
        }
        assignees(side, &d.assignees);
        label_list(side, &d.labels);
        milestone_aside(side, &pr.repo, d.milestone.as_ref());
        timeline::projects_aside(side, projects);
        if let Some(a) = a {
            let none = "No issues linked";
            timeline::linked_aside(side, icons, &pr.repo, "Closes", &a.closing, none);
            if let Some(oid) = &a.merge_commit {
                aside_heading(side, "Merge commit");
                let sha = sha_seg(side, &pr.repo, oid, Role::Link);
                side.wrapped(vec![sha], 0, Frame::None);
            }
        }
        aside_heading(side, "Size");
        let mut size = Vec::from(changes(d.additions, d.deletions));
        size.push(meta(format!(" in {}", plural(d.changed_files, "file"))));
        side.wrapped(size, 0, Frame::None);
    });
    pr_summary(page, pr, d, now);
    // Without a sidebar, what it would say is said up here.
    if !aside && let Some(a) = a {
        timeline::pr_inline(page, icons, pr, d, a);
        timeline::projects_inline(page, projects);
    }
    let talk = Conversation::new(&pr.repo, &d.head_oid, &d.author, now, marks.unfolded);
    let reactions = &d.editable.reactions;
    talk.opened(page, "opened", &d.created_at, &d.body, reactions);
    let Some(a) = activity.show_or(page, "the conversation", |page| {
        connector(page);
        page.line(vec![meta("   Loading the conversation…")]);
    }) else {
        return;
    };
    earlier_here(page, &a.timeline, "comment or event", "comments and events");
    let place = timeline::Place {
        repo: &pr.repo,
        pr: Some(pr),
        threads: timeline::by_review(&a.threads),
        icons,
    };
    timeline::entries(page, &talk, &place, &a.timeline, marks);
    merge_box(page, pr, d, icons, true);
    add_comment(page, keys);
}

/// A pull request's Checks tab: the checks on its head.
pub fn pr_checks(page: &mut Page, pr: &PrRef, d: &PrDetail, c: Fetched<'_, Checks>, now: u64) {
    pr_summary(page, pr, d, now);
    page.blank();
    checks(page, c, now);
}

/// A repository's Actions tab: its workflow runs, newest first, as
/// `query` filters them. Words runs can't be filtered by are said.
pub fn actions(
    page: &mut Page,
    repo: &RepoId,
    query: &str,
    runs: Fetched<'_, Results<RunSummary>>,
    cx: PageCtx<'_>,
) {
    filter_field(page, query, cx.keys, "filter");
    let ignored = RunFilter::parse(query).ignored;
    let knows = "runs filter by workflow:, branch:, is:, event: and actor:";
    ignored_note(page, &ignored, knows);
    let caches = format!("{}/actions/caches", url::repo(repo));
    let caches = link_seg(page, "Caches", caches, Role::Link);
    page.wrapped(vec![meta("Management: "), caches], 0, Frame::None);
    page.blank();
    let empty = if query.is_empty() {
        "No runs yet."
    } else {
        "No runs match this filter."
    };
    run_list(page, repo, runs, empty, cx.now);
}
