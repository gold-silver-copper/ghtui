//! Commits: lists of them, and one commit.

use super::*;

pub fn pr_commits(
    page: &mut Page,
    pr: &PrRef,
    d: &PrDetail,
    activity: Fetched<'_, PrActivity>,
    now: u64,
) {
    pr_summary(page, pr, d, now);
    let Some(a) = activity.show(page, "commits") else { return };
    commit_rows(page, &pr.repo, Some(pr), &a.commits, now);
}

/// Commits in boxes by day, each linked to its page: first how many
/// earlier ones were left out, which opens the history from the oldest
/// here back (oldest first, as a comparison and a PR list them), last
/// "Load more" if another page loads. A pull request's open as its
/// commits, as GitHub links them.
pub(super) fn commit_rows(
    page: &mut Page,
    repo: &RepoId,
    pr: Option<&PrRef>,
    commits: &impl Rows<CommitInfo>,
    now: u64,
) {
    let rest = match (commits.rest(), commits.rows().first()) {
        (Rest::OnGitHub(n), Some(oldest)) => {
            let earlier = url::commits(repo, &oldest.oid, "");
            let text = format!("… {n} earlier {}", noun(n, "commit"));
            item(page, earlier, |page, link| {
                page.wrapped(vec![Seg::linked(text, Role::Link, link)], 2, Frame::None);
            });
            Rest::All
        }
        (rest, _) => rest,
    };
    let mut current_day = String::new();
    for c in commits.rows() {
        let date = day(&c.date);
        if date == current_day {
            page.box_rule();
        } else {
            if !current_day.is_empty() {
                page.box_bottom();
            }
            page.box_top(vec![meta(format!("◷ Commits on {date}"))], Vec::new());
            current_day = date;
        }
        let commit = match pr {
            Some(pr) => url::pull_tab(pr, &format!("commits/{}", c.oid)),
            None => url::commit(repo, &c.oid),
        };
        item(page, commit, |page, link| {
            let sha = vec![Seg::new(short_sha(&c.oid), Role::Code)];
            link_line(page, c.headline.clone(), Role::Strong, link, sha);
            let who = c.author.name();
            let committed = format!("{who} committed {}", time::ago_iso(&c.date, now));
            body(page, vec![meta(committed)]);
        });
    }
    if !current_day.is_empty() {
        rest_row(page, commits.rows().len(), rest);
        page.box_bottom();
    }
}

/// A revision's commits (of `path`, if it isn't empty), newest first.
pub fn commit_history(
    page: &mut Page,
    repo: &RepoId,
    rev: &str,
    path: &str,
    history: Fetched<'_, Results<CommitInfo>>,
    now: u64,
) {
    let mut title = vec![Seg::new("Commits", Role::Title)];
    let at = if path.is_empty() {
        format!("  {rev}")
    } else {
        format!("  {rev} · {path}")
    };
    title.push(meta(at));
    page.wrapped(title, 0, Frame::None);
    page.blank();
    let Some(h) = history.show(page, "commits") else { return };
    if h.items.is_empty() {
        empty_box(page, meta("◷ Commits"), "No commits here.");
        return;
    }
    commit_rows(page, repo, None, h, now);
}

/// A pull request's commit: which of its commits it is, and links to the
/// ones either side.
pub struct PrCommitAt {
    pub n: u64,
    pub of: u64,
    pub pr: String,
    pub number: u64,
    pub previous: Option<String>,
    pub next: Option<String>,
}

/// The line above a pull request's commit: `Commit 3 of 7 in #149`, and
/// the previous and next commits.
pub fn pr_commit_nav(page: &mut Page, at: &PrCommitAt) {
    let pr = link_seg(page, format!("#{}", at.number), at.pr.clone(), Role::Link);
    let mut segs = vec![meta(format!("Commit {} of {} in ", at.n, at.of)), pr];
    for (label, url) in [("‹ Previous", &at.previous), ("Next ›", &at.next)] {
        segs.push(meta("   "));
        segs.push(match url {
            Some(url) => link_seg(page, label, url.clone(), Role::Link),
            None => meta(label),
        });
    }
    page.line(segs);
    page.blank();
}

/// A commit: its message, who and when, its parents, and a link to its
/// files (`files` is that link's target).
pub fn commit(page: &mut Page, repo: &RepoId, d: &CommitDetail, files: &str, now: u64) {
    say(page, d.headline.clone(), Role::Title);
    if !d.body.is_empty() {
        page.blank();
        let base = LinkBase::new(repo, &d.oid, "");
        markdown::render(page, &d.body, Some(&base), Frame::None);
    }
    page.blank();
    let mut who = vec![
        person_seg(page, &d.author, Role::Strong),
        meta(format!(" authored {}", time::ago_iso(&d.authored_at, now))),
    ];
    if let Some(committer) = &d.committer {
        who.push(meta(" · "));
        who.push(person_seg(page, committer, Role::Strong));
        who.push(meta(format!(
            " committed {}",
            time::ago_iso(&d.committed_at, now)
        )));
    }
    match d.verified {
        Some(true) => who.extend([space(), chip("Verified", Bg::SuccessContainer)]),
        Some(false) => who.extend([space(), chip("Unverified", Bg::ErrorContainer)]),
        None => {}
    }
    page.wrapped(who, 0, Frame::None);
    let mut ids = vec![meta(format!("commit {}", short_sha(&d.oid)))];
    if !d.parents.is_empty() {
        let noun = if d.parents.total() == 1 {
            "parent"
        } else {
            "parents"
        };
        ids.push(meta(format!(" · {noun} ")));
        for (i, parent) in d.parents.iter().enumerate() {
            if i > 0 {
                ids.push(meta(" + "));
            }
            ids.push(sha_seg(page, repo, parent, Role::Code));
        }
        more_chips(&mut ids, &d.parents);
    }
    page.wrapped(ids, 0, Frame::None);
    page.blank();
    page.box_top(vec![meta("± Files changed")], Vec::new());
    item(page, files, |page, link| {
        let what = match d.changed_files {
            Some(n) => plural(n, "file"),
            None => "Files".to_owned(),
        };
        let mut segs = vec![Seg::linked(what, Role::Link, link), space()];
        segs.extend(changes(d.additions, d.deletions));
        page.box_line(segs, vec![meta("↵ show the diff")], 0);
    });
    page.box_bottom();
}
