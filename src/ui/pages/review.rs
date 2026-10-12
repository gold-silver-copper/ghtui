//! A pull request's Review tab.

use super::*;
use crate::api::model::NodeId;
use std::collections::HashSet;

/// What a pull request's Review tab shows of its diff: GitHub's review
/// threads, and your drafts once they're read from disk.
#[derive(Debug, Clone, Copy)]
pub struct Glance<'a> {
    pub threads: Fetched<'a, Vec<ReviewThread>>,
    pub drafts: Option<&'a [DraftComment]>,
    /// Threads and comments that came in since you started looking.
    pub fresh: &'a HashSet<NodeId>,
}

/// A pull request's Review tab: your drafts, the threads still open, who
/// has reviewed and who hasn't yet, and what stops the merge. Each row
/// opens what it's about.
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
    reviewers(page, pr, d, activity, cx.now);
    merge_box(page, pr, d, cx.icons, false);
}

/// The first line of `text` with words in it, cut to `max` columns.
pub(super) fn first_line(text: &str, max: u16) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty());
    crate::ui::text::truncate(line.unwrap_or_default(), usize::from(max))
}

/// `path:line`, or the path alone for a comment on the whole file.
pub(super) fn place(path: &str, line: Option<u32>) -> String {
    line.map_or_else(|| path.to_owned(), |line| format!("{path}:{line}"))
}

/// Your drafts, in file order, each opening its line in Files changed.
pub(super) fn review_drafts(page: &mut Page, pr: &PrRef, drafts: Option<&[DraftComment]>) {
    let title = |n: usize| match n {
        0 => Seg::new("Your drafts", Role::Strong),
        n => Seg::new(format!("Your drafts · {n}"), Role::Strong),
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
    let right = vec![Seg::new("not submitted yet", Role::Meta)];
    page.box_top(vec![title(drafts.len())], right);
    for (n, draft) in drafts.into_iter().enumerate() {
        if n > 0 {
            page.box_rule();
        }
        let side = match draft.side {
            DraftSide::Left => 'L',
            DraftSide::Right => 'R',
        };
        let target = url::diff_line(pr, &draft.path, draft.line.map(|l| (side, l)));
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
        empty_box(page, Seg::new(heading, Role::Strong), "Loading…");
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
    let right = (resolved > 0).then(|| Seg::new(format!("{resolved} resolved"), Role::Meta));
    page.box_top(vec![Seg::new(title, Role::Strong)], Vec::from_iter(right));
    if open.is_empty() {
        empty_row(page, "None.");
    }
    for (n, thread) in open.into_iter().enumerate() {
        if n > 0 {
            page.box_rule();
        }
        let first = thread.comments.first();
        let row = |page: &mut Page, link: Option<u32>| {
            let line = if thread.file_level {
                None
            } else {
                thread.line.or(thread.original_line)
            };
            let at = place(&thread.path, line);
            let mut segs = vec![match link {
                Some(link) => Seg::linked(at, Role::Link, link),
                None => Seg::new(at, Role::Strong),
            }];
            if thread.outdated {
                segs.push(Seg::new(" ", Role::Meta));
                segs.push(chip("Outdated", Bg::ContainerHigh));
            }
            if thread.comments.iter().any(|c| fresh.contains(&c.id)) {
                segs.push(Seg::new(" ", Role::Meta));
                segs.push(chip("New", Bg::TertiaryContainer));
            }
            let mut said = Vec::new();
            if let Some(c) = first {
                said.push(login_seg(page, &c.author, Role::Meta));
                let ago = time::ago_iso(&c.created_at, now);
                said.push(Seg::new(format!(" · {ago}"), Role::Meta));
            }
            let replies = thread.comments.total().saturating_sub(1);
            if replies > 0 {
                said.push(Seg::new(
                    format!(" · {}", plural(replies, "reply")),
                    Role::Meta,
                ));
            }
            page.box_line(segs, said, 0);
            let text = first.map(|c| first_line(&c.body, page.room(Frame::Body, 0)));
            body(page, vec![Seg::new(text.unwrap_or_default(), Role::Body)]);
        };
        match first.map(|c| c.url.clone()).filter(|u| !u.is_empty()) {
            Some(url) => item(page, url, |page, link| row(page, Some(link))),
            None => row(page, None),
        }
    }
    page.box_bottom();
}

/// Who has reviewed and what they said last, then who is asked and
/// hasn't yet. A review opens in the conversation; someone asked, their
/// profile. Replies the author writes in threads are reviews too, which
/// GitHub doesn't count, so neither does this.
pub(super) fn reviewers(
    page: &mut Page,
    pr: &PrRef,
    d: &PrDetail,
    activity: Fetched<'_, PrActivity>,
    now: u64,
) {
    let title = Seg::new("Reviewers", Role::Strong);
    let loading = |page: &mut Page| empty_box(page, title.clone(), "Loading…");
    let Some(a) = activity.show_or(page, "the reviews", loading) else {
        return;
    };
    // Commenting after approving or asking for changes doesn't take it back.
    let mut said: Vec<&crate::api::browse::ReviewSummary> = Vec::new();
    for r in a.reviews.iter().filter(|r| r.author != d.author) {
        let decided =
            |s: ReviewState| matches!(s, ReviewState::Approved | ReviewState::ChangesRequested);
        match said.iter_mut().find(|s| s.author == r.author) {
            Some(s) if r.state == ReviewState::Commented && decided(s.state) => {}
            Some(s) => *s = r,
            None => said.push(r),
        }
    }
    page.box_top(vec![title], Vec::new());
    if said.is_empty() && a.requested.is_empty() && a.requested.left_out() == 0 {
        empty_row(page, "No reviews yet, and no one is asked for one.");
    }
    for r in said {
        let ((mark, role), what) = match r.state {
            ReviewState::Approved => (("✓", Role::Success), "approved"),
            ReviewState::ChangesRequested => (("✗", Role::Error), "requested changes"),
            ReviewState::Dismissed => (("○", Role::Meta), "had a review dismissed"),
            ReviewState::Commented | ReviewState::Pending => (("◉", Role::Meta), "commented"),
        };
        let target = match r.id {
            Some(id) => format!("{}#pullrequestreview-{id}", url::pull(pr)),
            None => url::user(&r.author),
        };
        item(page, target, |page, link| {
            let ago = time::ago_iso(&r.submitted_at, now);
            body(
                page,
                vec![
                    Seg::new(format!("{mark}  "), role),
                    Seg::linked(r.author.clone(), Role::Link, link),
                    Seg::new(format!(" {what} {ago}"), Role::Meta),
                ],
            );
        });
    }
    if a.reviews.left_out() > 0 {
        empty_row(page, "Earlier reviews are on GitHub.");
    }
    for who in &a.requested {
        let (name, target) = match who {
            Reviewer::User(login) => (login.clone(), url::user(login)),
            Reviewer::Team(team) => {
                let (org, slug) = team.split_once('/').unwrap_or(("", team));
                (
                    team.clone(),
                    format!("{}/orgs/{org}/teams/{slug}", url::BASE),
                )
            }
        };
        item(page, target, |page, link| {
            body(
                page,
                vec![
                    Seg::new("●  ", Role::Accent),
                    Seg::linked(name, Role::Link, link),
                    Seg::new(" is asked to review", Role::Meta),
                ],
            );
        });
    }
    if a.requested.left_out() > 0 {
        let more = format!("And {} more asked.", a.requested.left_out());
        empty_row(page, &more);
    }
    page.box_bottom();
}
