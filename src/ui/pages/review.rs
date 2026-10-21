//! A pull request's Review tab.

use super::*;
use crate::api::model::NodeId;
use crate::api::timeline::Review;
use crate::ui::page::ReviewLink;
use std::collections::HashSet;

/// What a pull request's Review tab shows of its diff: GitHub's review
/// threads, and your drafts once they're read from disk.
#[derive(Debug, Clone, Copy)]
pub struct Glance<'a> {
    pub threads: Fetched<'a, Vec<ReviewThread>>,
    pub drafts: Option<&'a [DraftComment]>,
    /// Threads and comments that came in since you started looking.
    pub fresh: &'a HashSet<NodeId>,
    /// All its reviews, oldest first.
    pub reviews: Fetched<'a, Capped<Review>>,
    /// Reviews that came in since you started looking.
    pub new_reviews: &'a HashSet<u64>,
}

/// A pull request's Review tab: your drafts, the threads still open, who
/// has reviewed and who hasn't yet, every review, and what stops the
/// merge. Each row opens what it's about.
pub fn pr_review(
    page: &mut Page,
    pr: &PrRef,
    d: &PrDetail,
    activity: Fetched<'_, PrActivity>,
    glance: Glance<'_>,
    cx: PageCtx<'_>,
) {
    pr_summary(page, pr, d, cx.now);
    review_drafts(page, pr, glance.drafts);
    open_threads(page, glance.threads, glance.fresh, cx.now);
    reviewers(page, pr, d, activity);
    all_reviews(page, pr, d, glance, cx.now);
    merge_box(page, pr, d, cx.icons, false);
}

/// Who has reviewed and what they said last (an approval of an older
/// head marked), then who is asked and hasn't yet (code owners marked).
/// A review opens its page; someone asked, their profile.
fn reviewers(page: &mut Page, pr: &PrRef, d: &PrDetail, activity: Fetched<'_, PrActivity>) {
    let title = strong("Reviewers");
    let loading = |page: &mut Page| empty_box(page, title.clone(), "Loading…");
    let Some(a) = activity.show_or(page, "the reviews", loading) else {
        return;
    };
    let latest: Vec<_> = a.latest.iter().filter(|l| l.author != d.author).collect();
    page.box_top(vec![title], Vec::new());
    if latest.is_empty() && a.requested.is_empty() && a.requested.left_out() == 0 {
        empty_row(page, "No reviews yet, and no one is asked for one.");
    }
    for l in latest {
        let ((mark, role), what) = timeline::review_said(l.state);
        let target = match l.id {
            Some(id) => Link::Review(ReviewLink::Open { pr: pr.clone(), id }),
            None => Link::Url(url::user(&l.author)),
        };
        item(page, target, |page, link| {
            let mut segs = vec![
                Seg::new(format!("{mark}  "), role),
                Seg::linked(l.author.clone(), Role::Link, link),
                meta(format!(" {what}")),
            ];
            if timeline::stale(l, &d.head_oid) {
                segs.push(meta(" · before the last push"));
            }
            for team in &l.for_teams {
                segs.push(meta(format!(" · for {team}")));
            }
            body(page, segs);
        });
    }
    for r in &a.requested {
        let (name, target) = timeline::reviewer_link(&r.who);
        item(page, target, |page, link| {
            let what = if r.code_owner {
                " is asked to review, as a code owner"
            } else {
                " is asked to review"
            };
            body(
                page,
                vec![
                    Seg::new("●  ", Role::Accent),
                    Seg::linked(name, Role::Link, link),
                    meta(what),
                ],
            );
        });
    }
    if a.requested.left_out() > 0 {
        let more = format!(
            "And {} more asked, that GitHub's API doesn't name (a team you can't see, say).",
            a.requested.left_out()
        );
        empty_row(page, &more);
    }
    page.box_bottom();
}

/// Every review, newest first: who, what they decided, when, on which
/// commit, how many comments, and the start of the summary. Each opens
/// its page.
fn all_reviews(page: &mut Page, pr: &PrRef, d: &PrDetail, glance: Glance<'_>, now: u64) {
    let heading = "Reviews";
    let loading = |page: &mut Page| empty_box(page, strong(heading), "Loading…");
    let Some(reviews) = glance.reviews.show_or(page, "the reviews", loading) else {
        return;
    };
    let title = match reviews.total() {
        0 => heading.to_owned(),
        n => format!("{heading} · {n}"),
    };
    page.box_top(vec![strong(title)], Vec::new());
    if reviews.is_empty() {
        empty_row(page, "None yet.");
    }
    for (n, r) in reviews.iter().rev().enumerate() {
        if n > 0 {
            page.box_rule();
        }
        let Some(id) = r.id else { continue };
        let link = Link::Review(ReviewLink::Open { pr: pr.clone(), id });
        item(page, link, |page, link| {
            let ((mark, role), what) = timeline::review_said(r.state);
            let mut segs = vec![
                Seg::new(format!("{mark}  "), role),
                Seg::linked(r.author.clone(), Role::Link, link),
                meta(format!(" {what}")),
            ];
            if glance.new_reviews.contains(&id) {
                tag(&mut segs, "New", Bg::TertiaryContainer);
            }
            let mut right = vec![meta(time::ago_iso(&r.at, now))];
            if r.comments > 0 {
                right.insert(0, meta(format!("{} · ", plural(r.comments, "comment"))));
            }
            page.box_line(segs, right, 0);
            let mut on = Vec::new();
            if let Some(c) = &r.commit {
                let at = if *c == d.head_oid {
                    format!("on {} (the head)", short_sha(c))
                } else {
                    format!("on {}", short_sha(c))
                };
                on.push(meta(at));
            }
            let said = first_line(&r.body, page.room(Frame::Body, 0));
            if !said.is_empty() {
                sep(&mut on, " · ");
                on.push(Seg::new(said, Role::Body));
            }
            if !on.is_empty() {
                page.wrapped(on, 0, Frame::Body);
            }
        });
    }
    if reviews.left_out() > 0 {
        let text = left_out_text(reviews.left_out(), "more review", "more reviews");
        empty_row(page, &text);
    }
    page.box_bottom();
}

/// One review: its summary and every thread it started or replied in,
/// then the changes it reviewed (as they were at its commit) and what
/// changed since.
#[expect(clippy::too_many_arguments, reason = "a page's data and context")]
pub fn pr_reviewed(
    page: &mut Page,
    pr: &PrRef,
    d: &PrDetail,
    id: u64,
    activity: Fetched<'_, PrActivity>,
    reviews: Fetched<'_, Capped<Review>>,
    marks: Marks<'_>,
    cx: PageCtx<'_>,
) {
    pr_summary(page, pr, d, cx.now);
    let Some(a) = activity.show(page, "the review's comments") else {
        return;
    };
    // From all of them, or else from the conversation's newest.
    #[expect(clippy::disallowed_methods, reason = "either says where it is")]
    let all = reviews.ready_unchecked();
    let found = all
        .and_then(|all| all.iter().find(|r| r.id == Some(id)))
        .or_else(|| a.reviews().find(|r| r.id == Some(id)));
    let Some(r) = found else {
        let what = if all.is_some() {
            "GitHub has no such review on this pull request."
        } else {
            "Loading the review…"
        };
        say(page, what, Role::Meta);
        return;
    };
    let talk = Conversation {
        base: LinkBase::new(&pr.repo, &d.head_oid, ""),
        op: &d.author,
        now: cx.now,
        unfolded: marks.unfolded,
    };
    let place = timeline::Place {
        repo: &pr.repo,
        pr: Some(pr),
        threads: timeline::by_review(&a.threads),
        icons: cx.icons,
    };
    // All of it: nothing folds here.
    let unfolded: HashSet<String> = r.anchor().into_iter().collect();
    let marks = Marks {
        unfolded: &unfolded,
        ..marks
    };
    timeline::review(page, &talk, &place, r, marks);
    let Some(commit) = &r.commit else {
        return;
    };
    page.box_top(vec![strong("Its changes")], Vec::new());
    let reviewed = Link::Review(ReviewLink::AsReviewed {
        pr: pr.clone(),
        commit: commit.clone(),
    });
    item(page, reviewed, |page, link| {
        let text = format!(
            "The changes it reviewed, as they were at {}",
            short_sha(commit)
        );
        body(page, vec![Seg::linked(text, Role::Link, link)]);
    });
    if *commit == d.head_oid {
        empty_row(page, "Nothing changed since: it reviewed the head.");
    } else {
        let since = Link::Review(ReviewLink::Since {
            pr: pr.clone(),
            commit: commit.clone(),
        });
        item(page, since, |page, link| {
            let text = format!(
                "What changed since, from {} to {}",
                short_sha(commit),
                short_sha(&d.head_oid)
            );
            body(page, vec![Seg::linked(text, Role::Link, link)]);
        });
    }
    page.box_bottom();
}

/// The first line of `text` with words in it, cut to `max` columns.
pub(super) fn first_line(text: &str, max: u16) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty());
    truncate(line.unwrap_or_default(), usize::from(max))
}

/// `path:line`, or the path alone for a comment on the whole file.
pub(super) fn place(path: &str, line: Option<u32>) -> String {
    line.map_or_else(|| path.to_owned(), |line| format!("{path}:{line}"))
}

/// Your drafts, in file order, each opening its line in Files changed.
pub(super) fn review_drafts(page: &mut Page, pr: &PrRef, drafts: Option<&[DraftComment]>) {
    let title = |n: usize| match n {
        0 => strong("Your drafts"),
        n => strong(format!("Your drafts · {n}")),
    };
    let Some(drafts) = drafts else {
        empty_box(page, title(0), "Reading your drafts…");
        return;
    };
    if drafts.is_empty() {
        let none = "None. Comments you write in Files changed wait here until you submit them.";
        empty_box(page, title(0), none);
        return;
    }
    let mut drafts = Vec::from_iter(drafts);
    drafts.sort_by_key(|d| (&d.path, d.line));
    let right = vec![meta("not submitted yet")];
    page.box_top(vec![title(drafts.len())], right);
    for (n, draft) in drafts.into_iter().enumerate() {
        if n > 0 {
            page.box_rule();
        }
        let side = match draft.side {
            DraftSide::Left => 'L',
            DraftSide::Right => 'R',
        };
        let target = Link::Review(ReviewLink::Draft {
            pr: pr.clone(),
            id: draft.id,
            url: url::diff_line(pr, &draft.path, draft.line.map(|l| (side, l))),
        });
        item(page, target, |page, link| {
            let at = place(&draft.path, draft.line);
            body(page, vec![Seg::linked(at, Role::Link, link)]);
            let text = first_line(&draft.body, page.room(Frame::Body, 0));
            body(page, vec![Seg::new(text, Role::Body)]);
            if let Some(err) = &draft.error {
                let why = format!("GitHub refused it: {err}");
                body(page, vec![Seg::new(why, Role::Error)]);
            }
        });
    }
    page.box_bottom();
}

/// The unresolved review threads, in file order (as `]t` walks them),
/// each opening Files changed at its thread.
pub(super) fn open_threads(
    page: &mut Page,
    threads: Fetched<'_, Vec<ReviewThread>>,
    fresh: &HashSet<NodeId>,
    now: u64,
) {
    let heading = "Unresolved threads";
    let loading = |page: &mut Page| {
        empty_box(page, strong(heading), "Loading…");
    };
    let Some(threads) = threads.show_or(page, "the review threads", loading) else {
        return;
    };
    let mut open: Vec<&ReviewThread> = threads.iter().filter(|t| !t.resolved).collect();
    open.sort_by_key(|t| (&t.path, t.line.or(t.original_line)));
    let resolved = threads.len() - open.len();
    let title = match open.len() {
        0 => heading.to_owned(),
        n => format!("{heading} · {n}"),
    };
    let right = (resolved > 0).then(|| meta(format!("{resolved} resolved")));
    page.box_top(vec![strong(title)], Vec::from_iter(right));
    if open.is_empty() {
        empty_row(page, "None.");
    }
    for (n, thread) in open.into_iter().enumerate() {
        if n > 0 {
            page.box_rule();
        }
        let first = thread.comments.first();
        let target = first.map(|c| c.url.clone()).filter(|u| !u.is_empty());
        maybe_item(page, target, |page, link| {
            let line = if thread.file_level {
                None
            } else {
                thread.line.or(thread.original_line)
            };
            let at = place(&thread.path, line);
            let mut segs = vec![match link {
                Some(link) => Seg::linked(at, Role::Link, link),
                None => strong(at),
            }];
            if thread.outdated {
                tag(&mut segs, "Outdated", Bg::ContainerHigh);
            }
            if thread.comments.iter().any(|c| fresh.contains(&c.id)) {
                tag(&mut segs, "New", Bg::TertiaryContainer);
            }
            let mut said = Vec::new();
            if let Some(c) = first {
                said.push(login_seg(page, &c.author, Role::Meta));
                let ago = time::ago_iso(&c.created_at, now);
                said.push(meta(format!(" · {ago}")));
            }
            let replies = thread.comments.total().saturating_sub(1);
            if replies > 0 {
                said.push(meta(format!(" · {}", plural(replies, "reply"))));
            }
            page.box_line(segs, said, 0);
            let text = first.map(|c| first_line(&c.body, page.room(Frame::Body, 0)));
            body(page, vec![Seg::new(text.unwrap_or_default(), Role::Body)]);
        });
    }
    page.box_bottom();
}
