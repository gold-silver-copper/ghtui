//! A conversation's timeline: comments, reviews with the threads they
//! started, and GitHub's events, in order. Events are one line each, and a
//! run of the same kind by the same person reads as one.

use std::collections::HashSet;

use super::*;
use crate::api::commit_comments::CommitComment;
use crate::api::model::NodeId;
use crate::api::project::InProject;
use crate::api::timeline::{Closer, Event, Item, Latest, Linked, Requested, Review, What};
use crate::ui::page::ReviewLink;

/// Files' lines at commits, by commit and path.
pub type LinesAt<'a> = std::collections::HashMap<(String, String), &'a [String]>;

/// What's marked on a timeline, and what's unfolded.
#[derive(Debug, Clone, Copy)]
pub struct Marks<'a> {
    /// Comments that came in since you started looking, by GitHub's
    /// number.
    pub comments: &'a HashSet<u64>,
    /// Reviews likewise.
    pub reviews: &'a HashSet<u64>,
    /// Review threads and their comments likewise.
    pub threads: &'a HashSet<NodeId>,
    /// Long reviews you unfolded, by their anchors.
    pub unfolded: &'a HashSet<String>,
    /// You: your pending review is yours.
    pub viewer: Option<&'a str>,
    /// Files' lines at commits (by commit and path), read from git for
    /// the threads on them.
    pub lines: &'a LinesAt<'a>,
}

impl Marks<'static> {
    /// Nothing marked or unfolded.
    pub fn none() -> Self {
        static NUMBERS: std::sync::LazyLock<HashSet<u64>> = std::sync::LazyLock::new(HashSet::new);
        static IDS: std::sync::LazyLock<HashSet<NodeId>> = std::sync::LazyLock::new(HashSet::new);
        static NAMES: std::sync::LazyLock<HashSet<String>> = std::sync::LazyLock::new(HashSet::new);
        static LINES: std::sync::LazyLock<LinesAt<'static>> =
            std::sync::LazyLock::new(LinesAt::new);
        Marks {
            comments: &NUMBERS,
            reviews: &NUMBERS,
            threads: &IDS,
            unfolded: &NAMES,
            viewer: None,
            lines: &LINES,
        }
    }
}

/// A review with more threads than this folds the rest away.
pub const FOLD_AFTER: usize = 3;

/// Where the timeline is: the repository its links are in, and the pull
/// request if it's one.
pub(super) struct Place<'a> {
    pub(super) repo: &'a RepoId,
    pub(super) pr: Option<&'a PrRef>,
    /// The pull request's review threads, which its reviews started.
    pub(super) threads: &'a [ReviewThread],
    pub(super) icons: Icons,
}

/// The timeline's items, oldest first, each after a connector.
pub(super) fn entries(
    page: &mut Page,
    talk: &Conversation<'_>,
    place: &Place<'_>,
    items: &[Item],
    marks: Marks<'_>,
) {
    let mut i = 0;
    while let Some(item) = items.get(i) {
        match item {
            Item::Comment(c) => {
                connector(page);
                let new = c.id.is_some_and(|id| marks.comments.contains(&id));
                talk.comment(page, c, new);
                i += 1;
            }
            Item::Review(r) => {
                connector(page);
                review(page, talk, place, r, marks);
                i += 1;
            }
            Item::Event(first) => {
                // The run of events like it that follow.
                let rest = items.get(i + 1..).unwrap_or_default().iter();
                let run: Vec<&Event> = std::iter::once(first)
                    .chain(rest.map_while(|it| match it {
                        Item::Event(e) if same_run(first, e) => Some(e),
                        _ => None,
                    }))
                    .collect();
                i += run.len();
                connector(page);
                events(page, talk, place, &run);
            }
        }
    }
}

/// Whether `e` reads on one line with `first`: the same kind of event
/// by the same person (commits by anyone).
fn same_run(first: &Event, e: &Event) -> bool {
    first.what.kind() == e.what.kind()
        && (first.actor == e.actor || matches!(first.what, What::Committed(_)))
        && !matches!(
            first.what,
            What::CrossReferenced { .. }
                | What::Closed { .. }
                | What::Merged { .. }
                | What::Renamed { .. }
                | What::ForcePushed { .. }
                | What::ReviewDismissed { .. }
                | What::Referenced { .. }
        )
}

/// An event's line: its icon, who, what (wrapped), and when.
fn event_line(page: &mut Page, (icon, role): (&str, Role), segs: Vec<Seg>) {
    let prefix = Seg::new(format!("{icon}  "), role);
    let start = page.lines.len();
    hanging(page, prefix, segs, Frame::None);
    for line in page.lines.iter_mut().skip(start) {
        line.indent = line.indent.saturating_add(2);
    }
}

/// `#12 Title`, linked, with its state's icon.
fn linked_segs(page: &mut Page, icons: Icons, here: &RepoId, l: &Linked) -> Vec<Seg> {
    let (icon, role) = issue_icon(icons, l.status());
    let name = link_seg(page, l.name(here), l.url(), Role::Link);
    vec![
        Seg::new(format!("{icon} "), role),
        name,
        meta(format!(" {}", l.title)),
    ]
}

/// The link of a commit: in the pull request's commits, if it's one.
fn commit_link(place: &Place<'_>, oid: &str) -> String {
    match place.pr {
        Some(pr) => format!("{}/commits/{oid}", url::pull(pr)),
        None => url::commit(place.repo, oid),
    }
}

/// A run of events as one line (commits, a line each under it).
fn events(page: &mut Page, talk: &Conversation<'_>, place: &Place<'_>, run: &[&Event]) {
    let (Some(first), Some(last)) = (run.first(), run.last()) else {
        return;
    };
    let here = place.repo;
    let who = login_seg(page, &first.actor, Role::Strong);
    let when = meta(format!(" {}", time::ago_iso(&last.at, talk.now)));
    let mut link: Option<Link> = None;
    let mut segs = vec![who];
    let icon: (&str, Role);
    match &first.what {
        What::Closed {
            by,
            reason,
            duplicate,
        } => {
            icon = match reason {
                Some(IssueState::NotPlanned) => ("⊘", Role::Meta),
                _ if place.pr.is_some() => ("✗", Role::Error),
                _ => ("✓", Role::Accent),
            };
            let how = match (reason, duplicate) {
                (_, true) => " closed this as a duplicate",
                (Some(IssueState::NotPlanned), _) => " closed this as not planned",
                (Some(_), _) => " closed this as completed",
                (None, _) => " closed this",
            };
            segs.push(meta(how));
            match by {
                Some(Closer::Commit(oid)) => {
                    segs.push(meta(" in "));
                    let target = url::commit(here, oid);
                    segs.push(link_seg(page, short_sha(oid), target, Role::Link));
                }
                Some(Closer::Pr(pr)) => {
                    segs.push(meta(" in"));
                    segs.push(space());
                    segs.extend(linked_segs(page, place.icons, here, pr));
                }
                None => {}
            }
        }
        What::Reopened => {
            icon = ("↺", Role::Success);
            segs.push(meta(" reopened this"));
        }
        What::Merged { commit, into } => {
            icon = ("⑃", Role::Accent);
            segs.push(meta(" merged commit "));
            if let Some(oid) = commit {
                segs.push(link_seg(
                    page,
                    short_sha(oid),
                    url::commit(here, oid),
                    Role::Link,
                ));
            }
            segs.push(meta(" into "));
            segs.push(chip(into, Bg::PrimaryContainer));
        }
        What::CrossReferenced { from, will_close } => {
            icon = ("↗", Role::Meta);
            let what = if *will_close {
                match place.pr {
                    Some(_) => " linked an issue this will close",
                    None if from.is_pr() => " linked a pull request that will close this issue",
                    None => " mentioned this in an issue that will close it",
                }
            } else {
                " mentioned this in"
            };
            segs.push(meta(what));
            segs.push(space());
            segs.extend(linked_segs(page, place.icons, here, from));
            link = Some(Link::Url(from.url()));
        }
        What::Referenced { commit, repo } => {
            icon = ("◆", Role::Meta);
            segs.push(meta(" added a commit that references this "));
            let repo = repo.as_ref().unwrap_or(here);
            let target = url::commit(repo, commit);
            segs.push(link_seg(
                page,
                short_sha(commit),
                target.clone(),
                Role::Link,
            ));
            link = Some(Link::Url(target));
        }
        What::Labeled { .. } => {
            icon = ("◈", Role::Meta);
            let (added, removed): (Vec<&&Event>, Vec<&&Event>) = run
                .iter()
                .partition(|e| matches!(&e.what, What::Labeled { added, .. } if *added));
            for (verb, list) in [(" added", &added), (" removed", &removed)] {
                if list.is_empty() {
                    continue;
                }
                if verb == " removed" && !added.is_empty() {
                    segs.push(meta(" and"));
                }
                segs.push(meta(verb));
                for e in list {
                    if let What::Labeled { label, .. } = &e.what {
                        segs.push(space());
                        segs.push(Seg::new(
                            format!(" {} ", label.name),
                            Role::Label(label.color.clone()),
                        ));
                    }
                }
            }
        }
        What::Assigned { .. } => {
            icon = ("◉", Role::Meta);
            let names = |added: bool| -> Vec<&str> {
                run.iter()
                    .filter_map(|e| match &e.what {
                        What::Assigned { who, added: a } if *a == added => Some(who.as_str()),
                        _ => None,
                    })
                    .collect()
            };
            let (on, off) = (names(true), names(false));
            let mut said = false;
            for (verb, list) in [(" assigned", on), (" unassigned", off)] {
                if list.is_empty() {
                    continue;
                }
                if said {
                    segs.push(meta(" and"));
                }
                said = true;
                if list == [first.actor.as_str()] {
                    segs.push(meta(if verb == " assigned" {
                        " self-assigned this"
                    } else {
                        " removed their assignment"
                    }));
                    continue;
                }
                segs.push(meta(verb));
                for (n, who) in list.into_iter().enumerate() {
                    segs.push(meta(if n == 0 { " " } else { ", " }));
                    segs.push(login_seg(page, who, Role::Link));
                }
            }
        }
        What::Milestoned { .. } => {
            icon = ("◷", Role::Meta);
            let mut said = false;
            for e in run {
                if let What::Milestoned { title, added } = &e.what {
                    if said {
                        segs.push(meta(" and"));
                    }
                    said = true;
                    let (verb, to) = if *added {
                        (" added this to the ", " milestone")
                    } else {
                        (" removed this from the ", " milestone")
                    };
                    segs.push(meta(verb));
                    segs.push(strong(title.clone()));
                    segs.push(meta(to));
                }
            }
        }
        What::Project { .. } | What::ProjectStatus { .. } => {
            icon = ("▦", Role::Meta);
            let Some(e) = run.last() else { return };
            match &e.what {
                What::Project { added: true } => segs.push(meta(" added this to a project")),
                What::Project { added: false } => {
                    segs.push(meta(" removed this from a project"));
                }
                What::ProjectStatus { from, to } if from.is_empty() => {
                    segs.push(meta(" set its status in a project to "));
                    segs.push(strong(to.clone()));
                }
                What::ProjectStatus { from, to } => {
                    segs.push(meta(" moved this in a project from "));
                    segs.push(strong(from.clone()));
                    segs.push(meta(" to "));
                    segs.push(strong(to.clone()));
                }
                _ => {}
            }
        }
        What::ReviewRequested { .. } => {
            icon = ("◎", Role::Meta);
            let mut said = false;
            for added in [true, false] {
                let who: Vec<&Reviewer> = (run.iter())
                    .filter_map(|e| match &e.what {
                        What::ReviewRequested {
                            who: Some(w),
                            added: a,
                        } if *a == added => Some(w),
                        _ => None,
                    })
                    .collect();
                if who.is_empty() {
                    continue;
                }
                if said {
                    segs.push(meta(" and"));
                }
                said = true;
                segs.push(meta(if added {
                    " requested a review from"
                } else {
                    " removed the request for review from"
                }));
                for (n, w) in who.into_iter().enumerate() {
                    segs.push(meta(if n == 0 { " " } else { ", " }));
                    let (name, target) = reviewer_link(w);
                    segs.push(link_seg(page, name, target, Role::Link));
                }
            }
            if !said {
                segs.push(meta(" changed who is asked to review"));
            }
        }
        What::ReviewDismissed { of, message } => {
            icon = ("○", Role::Meta);
            match of {
                Some(of) => {
                    segs.push(meta(" dismissed "));
                    segs.push(login_seg(page, of, Role::Link));
                    segs.push(meta("’s review"));
                }
                None => segs.push(meta(" dismissed a review")),
            }
            if !message.trim().is_empty() {
                segs.push(meta(format!(": {}", message.trim())));
            }
        }
        What::Renamed { from, to } => {
            icon = ("✎", Role::Meta);
            segs.push(meta(" changed the title "));
            segs.push(Seg::new(from.clone(), Role::Strike));
            segs.push(space());
            segs.push(strong(to.clone()));
        }
        What::ReadyForReview | What::ConvertedToDraft => {
            icon = ("◌", Role::Meta);
            let Some(e) = run.last() else { return };
            segs.push(meta(match e.what {
                What::ReadyForReview => " marked this pull request as ready for review",
                _ => " marked this pull request as draft",
            }));
        }
        What::Committed(_) => {
            commits(page, talk, place, run);
            return;
        }
        What::ForcePushed {
            base,
            before,
            after,
        } => {
            icon = ("⇡", Role::Meta);
            segs.push(meta(if *base {
                " force-pushed the base branch"
            } else {
                " force-pushed the branch"
            }));
            if let (Some(before), Some(after)) = (before, after) {
                segs.push(meta(" from "));
                segs.push(Seg::new(short_sha(before).to_owned(), Role::Code));
                segs.push(meta(" to "));
                segs.push(Seg::new(short_sha(after).to_owned(), Role::Code));
                if crate::git::Oid::parse(before).is_ok() && crate::git::Oid::parse(after).is_ok() {
                    let target = url::files(&url::compare(here, before, after));
                    segs.push(meta(" · "));
                    segs.push(link_seg(page, "Compare", target.clone(), Role::Link));
                    link = Some(Link::Url(target));
                }
            }
        }
        What::BranchDeleted { name } => {
            icon = ("⌥", Role::Meta);
            segs.push(meta(" deleted the "));
            segs.push(chip(name, Bg::PrimaryContainer));
            segs.push(meta(" branch"));
        }
        What::BranchRestored => {
            icon = ("⌥", Role::Meta);
            segs.push(meta(" restored the head branch"));
        }
        What::BaseChanged { from, to } => {
            icon = ("⌥", Role::Meta);
            segs.push(meta(" changed the base branch from "));
            segs.push(chip(from, Bg::PrimaryContainer));
            segs.push(meta(" to "));
            segs.push(chip(to, Bg::PrimaryContainer));
        }
        What::Locked { reason } => {
            icon = ("⚿", Role::Meta);
            segs.push(meta(match reason {
                Some(r) => format!(" locked as {r} and limited conversation to collaborators"),
                None => " locked and limited conversation to collaborators".to_owned(),
            }));
        }
        What::Unlocked => {
            icon = ("⚿", Role::Meta);
            segs.push(meta(" unlocked this conversation"));
        }
        What::Pinned { .. } => {
            icon = ("⚲", Role::Meta);
            let Some(e) = run.last() else { return };
            segs.push(meta(match e.what {
                What::Pinned { pinned: true } => " pinned this issue",
                _ => " unpinned this issue",
            }));
        }
        What::Transferred { from } => {
            icon = ("⇄", Role::Meta);
            segs.push(meta(match from {
                Some(from) => format!(" transferred this issue from {from}"),
                None => " transferred this issue".to_owned(),
            }));
        }
        What::MarkedDuplicate { of, undone } => {
            icon = ("⧉", Role::Meta);
            match (of, undone) {
                (_, true) => segs.push(meta(" marked this as not a duplicate")),
                (Some(of), false) => {
                    segs.push(meta(" marked this as a duplicate of"));
                    segs.push(space());
                    segs.extend(linked_segs(page, place.icons, here, of));
                    link = Some(Link::Url(of.url()));
                }
                (None, false) => segs.push(meta(" marked this as a duplicate")),
            }
        }
        What::AutoMerge { .. } => {
            icon = ("⑃", Role::Meta);
            let Some(e) = run.last() else { return };
            segs.push(meta(match e.what {
                What::AutoMerge { on: true } => " enabled auto-merge",
                _ => " disabled auto-merge",
            }));
        }
        What::MergeQueue { .. } => {
            icon = ("⑃", Role::Meta);
            let Some(e) = run.last() else { return };
            match &e.what {
                What::MergeQueue { added: true, .. } => {
                    segs.push(meta(" added this to the merge queue"));
                }
                What::MergeQueue {
                    reason: Some(r), ..
                } => segs.push(meta(format!(" removed this from the merge queue: {r}"))),
                _ => segs.push(meta(" removed this from the merge queue")),
            }
        }
        What::SubIssue { .. } | What::Parent { .. } => {
            icon = ("⊶", Role::Meta);
            let mut said = false;
            for e in run {
                let (verb, issue) = match &e.what {
                    What::SubIssue { issue, added: true } => (" added a sub-issue", issue),
                    What::SubIssue { issue, .. } => (" removed a sub-issue", issue),
                    What::Parent { issue, added: true } => (" added a parent issue", issue),
                    What::Parent { issue, .. } => (" removed a parent issue", issue),
                    _ => continue,
                };
                if said {
                    segs.push(meta(" and"));
                }
                said = true;
                segs.push(meta(verb));
                if let Some(issue) = issue {
                    segs.push(space());
                    segs.extend(linked_segs(page, place.icons, here, issue));
                    link.get_or_insert_with(|| Link::Url(issue.url()));
                }
            }
        }
        What::IssueType { from, to } => {
            icon = ("◇", Role::Meta);
            match (from, to) {
                (Some(from), Some(to)) => {
                    segs.push(meta(" changed the issue type from "));
                    segs.push(strong(from.clone()));
                    segs.push(meta(" to "));
                    segs.push(strong(to.clone()));
                }
                (None, Some(to)) => {
                    segs.push(meta(" added the issue type "));
                    segs.push(strong(to.clone()));
                }
                (Some(from), None) => {
                    segs.push(meta(" removed the issue type "));
                    segs.push(strong(from.clone()));
                }
                (None, None) => segs.push(meta(" changed the issue type")),
            }
        }
        What::ConvertedToDiscussion => {
            icon = ("◌", Role::Meta);
            segs.push(meta(" converted this issue into a discussion"));
        }
    }
    segs.push(when);
    match link {
        Some(target) => item(page, target, |page, _| event_line(page, icon, segs)),
        None => event_line(page, icon, segs),
    }
}

/// A run of commits: who added how many, then a line for each, each
/// opening the commit (among the pull request's).
fn commits(page: &mut Page, talk: &Conversation<'_>, place: &Place<'_>, run: &[&Event]) {
    let infos: Vec<&CommitInfo> = (run.iter())
        .filter_map(|e| match &e.what {
            What::Committed(c) => Some(c),
            _ => None,
        })
        .collect();
    let (Some(first), Some(last)) = (run.first(), run.last()) else {
        return;
    };
    let one = run.iter().all(|e| e.actor == first.actor) && !first.actor.is_empty();
    let mut segs = Vec::new();
    if one {
        segs.push(login_seg(page, &first.actor, Role::Strong));
        segs.push(meta(format!(
            " added {}",
            plural(infos.len() as u64, "commit")
        )));
    } else {
        segs.push(meta(format!(
            "{} added",
            plural(infos.len() as u64, "commit")
        )));
    }
    segs.push(meta(format!(" {}", time::ago_iso(&last.at, talk.now))));
    event_line(page, ("◆", Role::Meta), segs);
    for c in infos {
        item(page, commit_link(place, &c.oid), |page, link| {
            let room = usize::from(page.room(Frame::None, 5)).saturating_sub(9);
            let headline = crate::ui::text::truncate(&c.headline, room);
            page.add(
                Frame::None,
                5,
                vec![
                    Seg::linked(headline, Role::Body, link),
                    meta("  "),
                    Seg::new(short_sha(&c.oid).to_owned(), Role::Code),
                ],
                Vec::new(),
            );
        });
    }
}

/// Who a reviewer is, and their page.
pub(super) fn reviewer_link(who: &Reviewer) -> (String, String) {
    match who {
        Reviewer::User(login) => (login.clone(), url::user(login)),
        Reviewer::Team(team) => {
            let (org, slug) = team.split_once('/').unwrap_or(("", team));
            (
                team.clone(),
                format!("{}/orgs/{org}/teams/{slug}", url::base()),
            )
        }
    }
}

/// How a review's state reads: its mark, and what its author did.
pub(super) fn review_said(state: ReviewState) -> ((&'static str, Role), &'static str) {
    match state {
        ReviewState::Approved => (("✓", Role::Success), "approved these changes"),
        ReviewState::ChangesRequested => (("✗", Role::Error), "requested changes"),
        ReviewState::Dismissed => (("○", Role::Meta), "reviewed (dismissed)"),
        ReviewState::Commented => (("◉", Role::Meta), "reviewed"),
        ReviewState::Pending => (("◌", Role::Accent), "started a review"),
    }
}

/// A review's state as a chip.
fn state_chip_of(state: ReviewState) -> Seg {
    match state {
        ReviewState::Approved => chip("Approved", Bg::SuccessContainer),
        ReviewState::ChangesRequested => chip("Changes requested", Bg::ErrorContainer),
        ReviewState::Dismissed => chip("Dismissed", Bg::ContainerHigh),
        ReviewState::Commented => chip("Commented", Bg::SecondaryContainer),
        ReviewState::Pending => chip("Pending", Bg::TertiaryContainer),
    }
}

/// The threads a review started, in file order.
pub(super) fn started_by<'a>(
    threads: &'a [ReviewThread],
    review: &Review,
) -> Vec<&'a ReviewThread> {
    let mut out: Vec<&ReviewThread> = (threads.iter())
        .filter(|t| review.id.is_some() && t.comments.first().and_then(|c| c.review) == review.id)
        .collect();
    out.sort_by_key(|t| (&t.path, t.line.or(t.original_line)));
    out
}

/// The threads a review replied in without starting them.
fn replied_in<'a>(threads: &'a [ReviewThread], review: &Review) -> Vec<&'a ReviewThread> {
    (threads.iter())
        .filter(|t| {
            review.id.is_some()
                && t.comments.first().and_then(|c| c.review) != review.id
                && t.comments.iter().skip(1).any(|c| c.review == review.id)
        })
        .collect()
}

/// A review: its state and summary, then each thread it started with
/// the lines it's on, its comments and replies. A long one folds after
/// its first threads, until unfolded.
pub(super) fn review(
    page: &mut Page,
    talk: &Conversation<'_>,
    place: &Place<'_>,
    r: &Review,
    marks: Marks<'_>,
) {
    if let Some(anchor) = r.anchor() {
        page.anchor(anchor);
    }
    let mine = marks.viewer == Some(r.author.as_str());
    let pending = r.state == ReviewState::Pending;
    let started = started_by(place.threads, r);
    let replied = replied_in(place.threads, r);
    let new = r.id.is_some_and(|id| marks.reviews.contains(&id));
    let (icon, what) = review_said(r.state);
    // A review that only decided: one line.
    if r.body.trim().is_empty() && started.is_empty() && replied.is_empty() && !pending {
        let who = login_seg(page, &r.author, Role::Strong);
        let ago = time::ago_iso(&r.at, talk.now);
        let mut segs = vec![who, meta(format!(" {what} {ago}"))];
        if new {
            segs.push(space());
            segs.push(chip("New", Bg::TertiaryContainer));
        }
        event_line(page, icon, segs);
        return;
    }
    let start = page.lines.len();
    let who = login_seg(page, &r.author, Role::Strong);
    let ago = time::ago_iso(&r.at, talk.now);
    let what = if pending && mine {
        format!(" started a review {ago} · not submitted")
    } else {
        format!(" {what} {ago}")
    };
    let mut right = Vec::new();
    if new {
        right.push(chip("New", Bg::TertiaryContainer));
        right.push(space());
    }
    right.push(state_chip_of(r.state));
    page.box_top(
        vec![Seg::new(format!("{}  ", icon.0), icon.1), who, meta(what)],
        right,
    );
    if pending && mine {
        let comments = plural(r.comments, "comment");
        let text = format!(
            "Your pending review on GitHub, with {comments}: submitting your review in Files changed sends it; deleting it here throws it away."
        );
        page.wrapped(vec![meta(text)], 0, Frame::Body);
        page.box_gap();
    }
    if !r.body.trim().is_empty() {
        markdown::render(page, &r.body, Some(&talk.base), Frame::Body);
    }
    reaction_chips(page, &r.editable.reactions);
    // The summary (and the box's top) is the review: `>` quotes it, `,e`
    // edits it.
    let quote = page.link(Link::Quote {
        author: r.author.clone(),
        at: r.at.clone(),
        body: r.body.clone(),
    });
    page.item(start, quote);
    let anchor = r.anchor().unwrap_or_default();
    let folds = started.len() > FOLD_AFTER && !marks.unfolded.contains(&anchor);
    let shown = if folds { FOLD_AFTER - 1 } else { started.len() };
    for t in started.iter().take(shown) {
        page.box_rule();
        thread(page, place, t, marks, talk.now);
    }
    if folds {
        page.box_rule();
        let hidden = started.len().saturating_sub(shown) as u64;
        let link = ReviewLink::Fold(anchor);
        item(page, Link::Review(link), |page, link| {
            body(
                page,
                vec![
                    Seg::linked(
                        format!("▸ {hidden} more {} in this review", noun(hidden, "thread")),
                        Role::Link,
                        link,
                    ),
                    meta(" · ↵ shows them"),
                ],
            );
        });
    } else if started.len() > FOLD_AFTER {
        page.box_rule();
        item(
            page,
            Link::Review(ReviewLink::Fold(anchor)),
            |page, link| {
                body(
                    page,
                    vec![
                        Seg::linked("▾ Fold this review's threads", Role::Link, link),
                        meta(" · ↵"),
                    ],
                );
            },
        );
    }
    if !replied.is_empty() {
        page.box_rule();
        for t in replied {
            let at = super::review::place(&t.path, t.line.or(t.original_line));
            let target = t
                .comments
                .first()
                .map(|c| c.url.clone())
                .unwrap_or_default();
            item(page, target, |page, link| {
                body(
                    page,
                    vec![meta("↳ replied in "), Seg::linked(at, Role::Link, link)],
                );
            });
        }
    }
    page.box_bottom();
}

/// A thread inside a review: where it is (opening it in Files changed),
/// whether it's resolved or outdated, the lines it's on, and its
/// comments. A resolved one shows only its first line, as GitHub folds it.
fn thread(page: &mut Page, place: &Place<'_>, t: &ReviewThread, marks: Marks<'_>, now: u64) {
    let Some(first) = t.comments.first() else {
        return;
    };
    let line = if t.file_level {
        None
    } else {
        t.line.or(t.original_line)
    };
    let at = super::review::place(&t.path, line);
    let target = Some(first.url.clone()).filter(|u| !u.is_empty());
    let target = target.unwrap_or_else(|| match place.pr {
        Some(pr) => url::diff_line(pr, &t.path, line.map(|l| ('R', l))),
        None => url::repo(place.repo),
    });
    item(page, target, |page, link| {
        let mut segs = vec![Seg::linked(at, Role::Link, link)];
        let mut tag = |text: &str, bg| {
            segs.push(space());
            segs.push(chip(text, bg));
        };
        if t.outdated {
            tag("Outdated", Bg::ContainerHigh);
        }
        if t.resolved {
            tag("Resolved", Bg::SuccessContainer);
        }
        if first.pending {
            tag("Pending", Bg::TertiaryContainer);
        }
        if t.comments.iter().any(|c| marks.threads.contains(&c.id)) {
            tag("New", Bg::TertiaryContainer);
        }
        let more = t.comments.total().saturating_sub(1);
        let right = (t.resolved && more > 0).then(|| meta(plural(more, "reply")));
        page.box_line(segs, Vec::from_iter(right), 0);
        if t.resolved {
            let said = super::review::first_line(&first.body, page.room(Frame::Body, 2));
            page.box_line(
                vec![strong(format!("{}: ", first.author)), meta(said)],
                Vec::new(),
                2,
            );
            return;
        }
        if !t.file_level && !git_lines(page, t, marks.lines) {
            hunk_lines(page, t);
        }
        for (n, c) in t.comments.iter().enumerate() {
            if n == 1 && t.comments.left_out() > 0 {
                let text = left_out_text(t.comments.left_out(), "more reply", "more replies");
                page.box_line(vec![meta(text)], Vec::new(), 2);
            }
            let verb = if n == 0 { "" } else { " replied" };
            let mut segs = vec![
                strong(c.author.clone()),
                meta(format!("{verb} {}", time::ago_iso(&c.created_at, now))),
            ];
            if n > 0 && marks.threads.contains(&c.id) {
                segs.push(space());
                segs.push(chip("New", Bg::TertiaryContainer));
            }
            page.box_line(segs, Vec::new(), 2);
            let body_start = page.lines.len();
            markdown::render_comment(page, &c.body, None);
            for l in page.lines.iter_mut().skip(body_start) {
                l.frame = Frame::Body;
                l.indent = l.indent.saturating_add(2);
            }
        }
    });
}

/// The lines a thread is on, as GitHub kept them when it was started:
/// the end of its diff hunk, up to four lines (or the lines it spans).
/// Lines before a one-line thread's line, for context.
const CONTEXT: u32 = 3;

/// The lines a thread is on, numbered, as the file was at the commit its
/// first comment was made on, read from git; a few lines before a
/// one-line thread's for context. False when git hasn't read them (no
/// clone, or not yet), or the thread is on the old side.
fn git_lines(page: &mut Page, t: &ReviewThread, lines: &LinesAt<'_>) -> bool {
    let (Some(end), Some(first)) = (t.original_line, t.comments.first()) else {
        return false;
    };
    if t.side != crate::api::model::Side::Right {
        return false;
    }
    let Some(commit) = &first.original_commit else {
        return false;
    };
    let Some(text) = lines.get(&(commit.clone(), t.path.clone())) else {
        return false;
    };
    let on = t.original_start_line.unwrap_or(end)..=end;
    let start = if on.start() == on.end() {
        end.saturating_sub(CONTEXT).max(1)
    } else {
        *on.start()
    };
    let width = end.to_string().len();
    let room = usize::from(page.room(Frame::Body, 2)).saturating_sub(width + 3);
    for n in start..=end {
        let Some(code) = usize::try_from(n.saturating_sub(1))
            .ok()
            .and_then(|i| text.get(i))
        else {
            return n > start;
        };
        let tone = if on.contains(&n) {
            crate::ui::page::Tone::Marked
        } else {
            crate::ui::page::Tone::Code
        };
        let code = crate::ui::text::truncate(&code.replace('\t', "    "), room);
        page.push(PageLine {
            segs: vec![meta(format!("{n:>width$} │ ")), Seg::new(code, Role::Code)],
            indent: 2,
            right: Vec::new(),
            frame: Frame::Body,
            tone,
            fixed: 1,
        });
    }
    true
}

fn hunk_lines(page: &mut Page, t: &ReviewThread) {
    let lines: Vec<&str> = (t.diff_hunk.lines())
        .filter(|l| !l.starts_with("@@"))
        .collect();
    let span = match (t.original_start_line, t.original_line) {
        (Some(start), Some(end)) if end >= start => (end - start + 1) as usize,
        _ => 1,
    };
    let keep = span.clamp(4, 12);
    let room = usize::from(page.room(Frame::Body, 2));
    for l in lines.iter().skip(lines.len().saturating_sub(keep)) {
        let tone = match l.chars().next() {
            Some('+') => crate::ui::page::Tone::Added,
            Some('-') => crate::ui::page::Tone::Removed,
            _ => crate::ui::page::Tone::Code,
        };
        let text = crate::ui::text::truncate(&l.replace('\t', "    "), room);
        page.push(PageLine {
            segs: vec![Seg::new(text, Role::Code)],
            indent: 2,
            right: Vec::new(),
            frame: Frame::Body,
            tone,
            fixed: 0,
        });
    }
}

// ---- sidebars ---------------------------------------------------------------------

/// The sidebar's reviewers: each one's latest review (an approval of an
/// older head marked stale), then who is asked and hasn't answered,
/// code owners marked.
pub(super) fn reviewers_aside(
    page: &mut Page,
    pr: &PrRef,
    head: &str,
    latest: &Capped<Latest>,
    requested: &Capped<Requested>,
    author: &str,
) {
    aside_heading(page, "Reviewers");
    let latest: Vec<&Latest> = latest.iter().filter(|l| l.author != author).collect();
    if latest.is_empty() && requested.is_empty() {
        say(page, "No reviews", Role::Meta);
    }
    for l in latest {
        let ((mark, role), what) = match l.state {
            ReviewState::Approved => (("✓", Role::Success), "approved"),
            ReviewState::ChangesRequested => (("✗", Role::Error), "requested changes"),
            ReviewState::Dismissed => (("○", Role::Meta), "dismissed"),
            ReviewState::Commented => (("◉", Role::Meta), "commented"),
            ReviewState::Pending => (("◌", Role::Accent), "pending"),
        };
        let target = match l.id {
            Some(id) => format!("{}#pullrequestreview-{id}", url::pull(pr)),
            None => url::user(&l.author),
        };
        let who = link_seg(page, l.author.clone(), target, Role::Link);
        let mut segs = vec![
            Seg::new(format!("{mark} "), role),
            who,
            meta(format!(" {what}")),
        ];
        let stale = l.state == ReviewState::Approved
            && l.commit
                .as_deref()
                .is_some_and(|c| !head.is_empty() && c != head);
        if stale {
            segs.push(meta(" · before the last push"));
        }
        for team in &l.for_teams {
            segs.push(meta(format!(" · for {team}")));
        }
        page.wrapped(segs, 2, Frame::None);
    }
    for r in requested {
        let (name, target) = reviewer_link(&r.who);
        let who = link_seg(page, name, target, Role::Link);
        let mut segs = vec![Seg::new("● ", Role::Accent), who, meta(" asked")];
        if r.code_owner {
            segs.push(meta(" · code owner"));
        }
        page.wrapped(segs, 2, Frame::None);
    }
    if requested.left_out() > 0 {
        let more = format!("and {} more asked, unnamed", requested.left_out());
        say(page, more, Role::Meta);
    }
}

/// A list of issues or pull requests in a sidebar, each linked with its
/// state's icon.
pub(super) fn linked_aside(
    page: &mut Page,
    icons: Icons,
    here: &RepoId,
    heading: &str,
    list: &Capped<Linked>,
    none: &str,
) {
    aside_heading(page, heading);
    if list.is_empty() {
        say(page, none, Role::Meta);
    }
    for l in list {
        let segs = linked_segs(page, icons, here, l);
        page.wrapped(segs, 2, Frame::None);
    }
    if list.left_out() > 0 {
        let more = left_out_text(list.left_out(), "more", "more");
        say(page, more, Role::Meta);
    }
}

/// An issue's place among others: its type, its parent, its sub-issues
/// and how far along they are, and the pull requests and branches for it
/// (GitHub's "Development").
pub(super) fn relations_aside(page: &mut Page, icons: Icons, d: &IssueDetail) {
    if let Some(t) = &d.issue_type {
        aside_heading(page, "Type");
        page.wrapped(vec![chip(t, Bg::SecondaryContainer)], 0, Frame::None);
    }
    aside_heading(page, "Development");
    if d.closed_by.is_empty() && d.branches.is_empty() {
        say(page, "No pull requests or branches", Role::Meta);
    }
    for l in &d.closed_by {
        let segs = linked_segs(page, icons, &d.repo, l);
        page.wrapped(segs, 2, Frame::None);
    }
    for b in &d.branches {
        let target = url::tree(&d.repo, b, "");
        let branch = link_seg(page, b.clone(), target, Role::Link);
        page.wrapped(vec![meta("⌥ "), branch], 2, Frame::None);
    }
    if let Some(parent) = &d.parent {
        aside_heading(page, "Parent issue");
        let segs = linked_segs(page, icons, &d.repo, parent);
        page.wrapped(segs, 2, Frame::None);
    }
    if !d.sub_issues.is_empty() || d.progress.is_some() {
        let heading = match d.progress {
            Some(p) => format!("Sub-issues · {} of {} done", p.done, p.total),
            None => "Sub-issues".to_owned(),
        };
        linked_aside(page, icons, &d.repo, &heading, &d.sub_issues, "None");
    }
    if !d.blocked_by.is_empty() {
        linked_aside(page, icons, &d.repo, "Blocked by", &d.blocked_by, "None");
    }
    if !d.blocking.is_empty() {
        linked_aside(page, icons, &d.repo, "Blocking", &d.blocking, "None");
    }
}

/// A commit's comments, under its page: each on the commit as a whole,
/// or on a line of its diff (which opens there), then the box to write
/// one.
pub fn commit_comments(
    page: &mut Page,
    repo: &RepoId,
    d: &CommitDetail,
    comments: Fetched<'_, Capped<CommitComment>>,
    cx: PageCtx<'_>,
) {
    let loading = |page: &mut Page| {
        page.blank();
        page.line(vec![meta("Loading the comments…")]);
    };
    let Some(comments) = comments.show_or(page, "the commit's comments", loading) else {
        return;
    };
    let talk = Conversation {
        base: LinkBase::new(repo, &d.oid, ""),
        op: "",
        now: cx.now,
        unfolded: Marks::none().unfolded,
    };
    let diff = crate::git::Oid::parse(&d.oid).is_ok();
    page.blank();
    let title = match comments.total() {
        0 => "No comments yet".to_owned(),
        n => format!("Comments · {n}"),
    };
    page.line(vec![strong(title)]);
    for c in comments {
        connector(page);
        let at = c.path.as_ref().map(|path| {
            let target = if diff {
                let line = c.line.map(|l| format!("R{l}")).unwrap_or_default();
                url::diff_file(&url::commit(repo, &d.oid), path, &line)
            } else {
                url::commit(repo, &d.oid)
            };
            (super::review::place(path, c.line), target)
        });
        let verb = match &at {
            Some((place, _)) => format!("commented on {place}"),
            None => "commented".to_owned(),
        };
        talk.said_at(
            page,
            Some(c.anchor()),
            &c.author,
            &verb,
            &c.created_at,
            &c.body,
            None,
            &[],
        );
        if let Some((place, target)) = at {
            item(page, target, |page, link| {
                page.add(
                    Frame::None,
                    5,
                    vec![
                        meta("↳ "),
                        Seg::linked(format!("Open {place} in the diff"), Role::Link, link),
                    ],
                    Vec::new(),
                );
            });
        }
    }
    if comments.left_out() > 0 {
        page.wrapped(vec![meta("… more comments on GitHub")], 2, Frame::None);
    }
    add_comment(page, cx.keys);
}

/// A repository's pinned issues, above its list of issues, each opening
/// the issue; nothing where none are pinned.
pub fn pinned_issues(page: &mut Page, repo: &RepoId, pinned: &Capped<Linked>, icons: Icons) {
    if pinned.is_empty() {
        return;
    }
    page.box_top(vec![strong("Pinned issues")], Vec::new());
    for l in pinned {
        item(page, l.url(), |page, _| {
            let segs = linked_segs(page, icons, repo, l);
            hanging(page, meta("⚲ "), segs, Frame::Body);
        });
    }
    page.box_bottom();
    page.blank();
}

/// The projects an issue or pull request is in, with its status and its
/// other fields' values in each (`,f` sets one here; each opens the
/// project); or why they can't be read (a token without read:project).
pub(super) fn projects_aside(page: &mut Page, projects: Fetched<'_, Capped<InProject>>) {
    aside_heading(page, "Projects");
    #[expect(clippy::disallowed_methods, reason = "said in the sidebar's row")]
    match projects.text("the projects") {
        Ok(list) if list.is_empty() => say(page, "None yet", Role::Meta),
        Ok(list) => {
            for p in list {
                let title = link_seg(page, p.title.clone(), p.url.clone(), Role::Link);
                let mut segs = vec![meta("▦ "), title];
                if let Some(status) = p.status() {
                    segs.push(meta(" · "));
                    segs.push(strong(status.to_owned()));
                }
                page.wrapped(segs, 2, Frame::None);
                let others: Vec<String> = (p.others())
                    .map(|(name, v)| format!("{name}: {}", v.text()))
                    .collect();
                if !others.is_empty() {
                    page.wrapped(vec![meta(others.join(" · "))], 2, Frame::None);
                }
            }
            if list.left_out() > 0 {
                let more = left_out_text(list.left_out(), "more", "more");
                say(page, more, Role::Meta);
            }
        }
        Err(why) => say(page, why, Role::Meta),
    }
}

/// The projects on a page too narrow for a sidebar.
pub(super) fn projects_inline(page: &mut Page, projects: Fetched<'_, Capped<InProject>>) {
    // Said once loaded, or once it failed; while it loads, nothing.
    #[expect(clippy::disallowed_methods, reason = "a failure is said in its row")]
    let list = match projects.text("the projects") {
        Ok(list) => list,
        Err(why) if why.starts_with("Loading") => return,
        Err(why) => return inline(page, "Projects", vec![meta(why)]),
    };
    let mut segs = Vec::new();
    for p in list {
        if !segs.is_empty() {
            segs.push(meta(", "));
        }
        segs.push(link_seg(page, p.title.clone(), p.url.clone(), Role::Link));
        let said: Vec<String> = (p.status().map(str::to_owned).into_iter())
            .chain(p.others().map(|(name, v)| format!("{name}: {}", v.text())))
            .collect();
        if !said.is_empty() {
            segs.push(meta(format!(" ({})", said.join(" · "))));
        }
    }
    inline(page, "Projects", segs);
}

/// `Heading: ` then `segs`, wrapped under it: a sidebar's section on a
/// page too narrow for one.
fn inline(page: &mut Page, heading: &str, segs: Vec<Seg>) {
    if segs.is_empty() {
        return;
    }
    hanging(page, meta(format!("{heading}: ")), segs, Frame::None);
}

/// A pull request's sidebar on a narrow page: its reviewers, and the
/// issues it closes.
pub(super) fn pr_inline(page: &mut Page, icons: Icons, pr: &PrRef, d: &PrDetail, a: &PrActivity) {
    let mut segs = Vec::new();
    let sep = |segs: &mut Vec<Seg>| {
        if !segs.is_empty() {
            segs.push(meta(" · "));
        }
    };
    for l in a.latest.iter().filter(|l| l.author != d.author) {
        sep(&mut segs);
        let ((mark, role), _) = review_said(l.state);
        segs.push(Seg::new(format!("{mark} "), role));
        segs.push(login_seg(page, &l.author, Role::Link));
        let stale = l.state == ReviewState::Approved
            && l.commit.as_deref().is_some_and(|c| c != d.head_oid);
        if stale {
            segs.push(meta(" (before the last push)"));
        }
    }
    for r in &a.requested {
        sep(&mut segs);
        let (name, target) = reviewer_link(&r.who);
        segs.push(Seg::new("● ", Role::Accent));
        segs.push(link_seg(page, name, target, Role::Link));
        segs.push(meta(if r.code_owner {
            " asked (code owner)"
        } else {
            " asked"
        }));
    }
    inline(page, "Reviewers", segs);
    let mut closes = Vec::new();
    for l in &a.closing {
        if !closes.is_empty() {
            closes.push(meta(", "));
        }
        closes.extend(linked_segs(page, icons, &pr.repo, l));
    }
    inline(page, "Closes", closes);
    if let Some(oid) = &a.merge_commit {
        let sha = link_seg(page, short_sha(oid), url::commit(&pr.repo, oid), Role::Link);
        inline(page, "Merge commit", vec![sha]);
    }
}

/// An issue's place among others on a narrow page.
pub(super) fn relations_inline(page: &mut Page, icons: Icons, d: &IssueDetail) {
    if let Some(t) = &d.issue_type {
        inline(page, "Type", vec![chip(t, Bg::SecondaryContainer)]);
    }
    if let Some(parent) = &d.parent {
        let segs = linked_segs(page, icons, &d.repo, parent);
        inline(page, "Parent", segs);
    }
    if let Some(p) = d.progress {
        let mut segs = vec![meta(format!("{} of {} done", p.done, p.total))];
        for l in &d.sub_issues {
            segs.push(meta(" · "));
            segs.extend(linked_segs(page, icons, &d.repo, l));
        }
        inline(page, "Sub-issues", segs);
    }
    for (name, list) in [("Blocked by", &d.blocked_by), ("Blocking", &d.blocking)] {
        if list.is_empty() {
            continue;
        }
        let mut segs = Vec::new();
        for l in list {
            if !segs.is_empty() {
                segs.push(meta(", "));
            }
            segs.extend(linked_segs(page, icons, &d.repo, l));
        }
        if list.left_out() > 0 {
            segs.push(meta(format!(" and {} more", list.left_out())));
        }
        inline(page, name, segs);
    }
    let mut dev = Vec::new();
    for l in &d.closed_by {
        if !dev.is_empty() {
            dev.push(meta(", "));
        }
        dev.extend(linked_segs(page, icons, &d.repo, l));
    }
    for b in &d.branches {
        if !dev.is_empty() {
            dev.push(meta(", "));
        }
        let target = url::tree(&d.repo, b, "");
        dev.push(meta("⌥ "));
        dev.push(link_seg(page, b.clone(), target, Role::Link));
    }
    inline(page, "Development", dev);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::timeline::Event;

    fn ev(actor: &str, what: What) -> Event {
        Event {
            actor: actor.into(),
            at: "2026-01-01T00:00:00Z".into(),
            what,
        }
    }

    /// Labels by one person run together; another's, or a closing, don't.
    #[test]
    fn runs_are_one_kind_by_one_person() {
        let label = |added| What::Labeled {
            label: Label {
                name: "bug".into(),
                color: "ff0000".into(),
            },
            added,
        };
        let a = ev("alice", label(true));
        assert!(same_run(&a, &ev("alice", label(false))));
        assert!(!same_run(&a, &ev("bob", label(true))));
        let closed = What::Closed {
            by: None,
            reason: None,
            duplicate: false,
        };
        assert!(!same_run(
            &ev("alice", closed.clone()),
            &ev("alice", closed)
        ));
    }
}
