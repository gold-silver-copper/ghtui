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
    let segs = vec![
        author,
        Seg::new(wants, Role::Meta),
        chip(&d.base_ref, Bg::PrimaryContainer),
        Seg::new(" from ", Role::Meta),
        chip(head, Bg::PrimaryContainer),
    ];
    page.wrapped(segs, 0, Frame::None);
    let mut meta = Vec::from(changes(d.additions, d.deletions));
    meta.push(Seg::new(
        format!(
            " · opened {} · updated {}",
            time::ago_iso(&d.created_at, now),
            time::ago_iso(&d.updated_at, now)
        ),
        Role::Meta,
    ));
    labels(&mut meta, &d.labels);
    page.wrapped(meta, 0, Frame::None);
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
        body(
            page,
            vec![
                Seg::new(format!("{icon}  "), role),
                Seg::new(text, Role::Body),
            ],
        );
    }
    page.box_rule();
    item(page, url::pull_tab(pr, "files"), |page, link| {
        let review = match d.changed_files {
            1 => "Review the changed file".to_owned(),
            n => format!("Review the {n} changed files"),
        };
        body(
            page,
            vec![
                Seg::new(format!("{}  ", icons.external()), Role::Meta),
                Seg::linked(review, Role::Link, link),
            ],
        );
    });
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
    format!(
        "Auto-merge enabled by {}: {how} when every requirement is met",
        auto.by
    )
}

/// "#2 in the merge queue for main: waiting for its checks", as the
/// merge box and the dialog say it.
pub fn queue_place(queued: &Queued, base: &str) -> String {
    let place = match queued.position {
        1 => "Next".to_owned(),
        n => format!("#{n}"),
    };
    let state = match queued.state {
        QueueState::AwaitingChecks => "waiting for its checks",
        QueueState::Queued => "waiting its turn",
        QueueState::Mergeable => "ready to merge",
        QueueState::Locked => "merging now",
        QueueState::Unmergeable => "it can't be merged",
    };
    format!("{place} in the merge queue for {base}: {state}")
}

/// Comments in `fresh` (by GitHub's number) came in since you started
/// looking.
pub fn pr_conversation(
    page: &mut Page,
    pr: &PrRef,
    d: &PrDetail,
    activity: Fetched<'_, PrActivity>,
    fresh: &std::collections::HashSet<u64>,
    cx: PageCtx<'_>,
) {
    // Comments and reviews, merged in time order below.
    enum Entry<'a> {
        Comment(&'a Comment),
        Review(&'a crate::api::browse::ReviewSummary),
    }
    let PageCtx { icons, keys, now } = cx;
    // The sidebar doesn't wait for the conversation, which says how it's
    // going; only its reviewers come from it.
    #[expect(
        clippy::disallowed_methods,
        reason = "the conversation says how it's going"
    )]
    let reviews = activity.ready_unchecked().map(|a| &a.reviews);
    page.build_aside(|side| {
        let reviewers = unique(reviews.into_iter().flatten().map(|r| &r.author));
        let earlier = reviews.is_some_and(|r| r.left_out() > 0);
        let none = if reviews.is_some() { "No reviews" } else { "" };
        people(side, "Reviewers", &reviewers, maybe_others(earlier), none);
        let more = (d.assignees.left_out() > 0)
            .then(|| left_out_text(d.assignees.left_out(), "more", "more"));
        people(side, "Assignees", &d.assignees, more, "No one assigned");
        label_list(side, &d.labels);
        milestone_aside(side, &pr.repo, d.milestone.as_ref());
        aside_heading(side, "Size");
        let mut size = Vec::from(changes(d.additions, d.deletions));
        size.push(Seg::new(
            format!(" in {}", plural(d.changed_files, "file")),
            Role::Meta,
        ));
        side.wrapped(size, 0, Frame::None);
    });
    pr_summary(page, pr, d, now);
    let op = &d.author;
    let talk = Conversation {
        base: LinkBase::new(&pr.repo, &d.head_oid, ""),
        op,
        now,
    };
    talk.said(
        page,
        op,
        "opened",
        &d.created_at,
        &d.body,
        true,
        &d.editable.reactions,
    );
    let Some(a) = activity.show_or(page, "the conversation", |page| {
        connector(page);
        page.line(vec![Seg::new("   Loading the conversation…", Role::Meta)]);
    }) else {
        return;
    };
    earlier_here(page, &a.comments, "comment", "comments");
    earlier_here(page, &a.reviews, "review", "reviews");
    let mut entries: Vec<(&str, Entry<'_>)> = a
        .comments
        .iter()
        .map(|c| (c.created_at.as_str(), Entry::Comment(c)))
        .chain(
            a.reviews
                .iter()
                .map(|r| (r.submitted_at.as_str(), Entry::Review(r))),
        )
        .collect();
    entries.sort_by(|x, y| x.0.cmp(y.0));
    for (_, entry) in entries {
        connector(page);
        match entry {
            Entry::Comment(c) => {
                let new = c.id.is_some_and(|id| fresh.contains(&id));
                talk.comment(page, c, new);
            }
            Entry::Review(r) => {
                let (icon, what) = match r.state {
                    ReviewState::Approved => (("✓", Role::Success), "approved these changes"),
                    ReviewState::ChangesRequested => (("✗", Role::Error), "requested changes"),
                    ReviewState::Dismissed => (("○", Role::Meta), "had a review dismissed"),
                    ReviewState::Commented | ReviewState::Pending => {
                        (("◉", Role::Meta), "reviewed")
                    }
                };
                if let Some(id) = r.id {
                    page.anchor(format!("pullrequestreview-{id}"));
                }
                talk.event(page, icon, &r.author, what, &r.submitted_at);
                if !r.body.trim().is_empty() {
                    connector(page);
                    talk.said(
                        page,
                        &r.author,
                        "commented",
                        &r.submitted_at,
                        &r.body,
                        false,
                        &[],
                    );
                }
            }
        }
    }
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
    if !ignored.is_empty() {
        let words = ignored.iter().map(|w| format!("“{w}”")).collect::<Vec<_>>();
        let text = format!(
            "Not filtered by {}: runs filter by workflow:, branch:, is:, event: and actor:",
            words.join(", ")
        );
        page.wrapped(vec![Seg::new(text, Role::Meta)], 0, Frame::None);
    }
    page.blank();
    let empty = if query.is_empty() {
        "No runs yet."
    } else {
        "No runs match this filter."
    };
    run_list(page, repo, runs, empty, cx.now);
}
