//! Lists of issues, pull requests, repositories and people, and
//! search.

use super::*;

pub(super) fn repo_row(page: &mut Page, r: &RepoSummary, now: u64, show_owner: bool) {
    let name = if show_owner {
        r.repo.to_string()
    } else {
        r.repo.name.clone()
    };
    item(page, url::repo(&r.repo), |page, link| {
        let mut segs = vec![Seg::linked(name, Role::Link, link)];
        for (on, text) in [
            (r.private, "Private"),
            (r.fork, "Fork"),
            (r.archived, "Archived"),
        ] {
            if on {
                segs.push(space());
                segs.push(chip(text, Bg::SecondaryContainer));
            }
        }
        let right = vec![meta(format!("☆ {}", compact(r.stars)))];
        page.box_line(segs, right, 0);
        if page.compact {
            return;
        }
        if let Some(d) = &r.description {
            page.wrapped(vec![Seg::new(d.clone(), Role::Body)], 0, Frame::Body);
        }
        let mut subline = Vec::new();
        if let Some(lang) = &r.language {
            subline.push(Seg::new("● ", Role::Accent));
            subline.push(meta(format!("{lang}   ")));
        }
        subline.push(meta(format!("⑂ {}", compact(r.forks))));
        if let Some(p) = &r.pushed_at {
            let updated = format!("   Updated {}", time::ago_iso(p, now));
            subline.push(meta(updated));
        }
        body(page, subline);
    });
}

/// Whether a list's search orders it by when its items were updated, so
/// its rows say that time rather than when they were opened.
pub(super) fn by_update(query: &str) -> bool {
    query
        .split_whitespace()
        .any(|w| w.starts_with("sort:updated"))
}

/// An issue's or pull request's row; with `updated`, it says when it was
/// last updated (the list's order) rather than opened.
pub(super) fn issue_row(
    page: &mut Page,
    i: &IssueSummary,
    shown: (bool, bool),
    icons: Icons,
    now: u64,
) {
    issue_row_to(page, i, shown, icons, now, false);
}

/// An issue's or pull request's row; with `review`, a pull request's opens
/// its Files changed, to review it.
pub(super) fn issue_row_to(
    page: &mut Page,
    i: &IssueSummary,
    (show_repo, updated): (bool, bool),
    icons: Icons,
    now: u64,
    review: bool,
) {
    let (icon, role) = issue_icon(icons, i.status());
    let pr = PrRef {
        repo: i.repo.clone(),
        number: i.number,
    };
    let target = match i.kind {
        IssueKind::Pr { .. } if review => url::pull_tab(&pr, "files"),
        IssueKind::Pr { .. } => url::pull(&pr),
        IssueKind::Issue(_) => url::issue(&i.repo, i.number),
    };
    let place = if show_repo {
        format!("{}#{}", i.repo, i.number)
    } else {
        format!("#{}", i.number)
    };
    item(page, target, |page, link| {
        if page.compact {
            // One row: the title, then where it is and its signals.
            let segs = vec![
                Seg::new(format!("{icon} "), role),
                Seg::linked(i.title.clone(), Role::Strong, link),
            ];
            let mut right = vec![meta(format!("{place}  "))];
            right.extend(issue_signals(i, icons));
            page.box_line(segs, right, 0);
            return;
        }
        let mut segs = vec![Seg::linked(i.title.clone(), Role::Strong, link)];
        labels(&mut segs, &i.labels);
        hanging(page, Seg::new(format!("{icon} "), role), segs, Frame::Body);
        // GitHub's second line: `#12 opened 3 days ago by octocat · Approved`.
        let (verb, at) = if updated {
            ("updated", &i.updated_at)
        } else {
            ("opened", &i.created_at)
        };
        let when = format!("{place} {verb} {} by {}", time::ago_iso(at, now), i.author);
        let mut subline = vec![meta(when)];
        if let IssueKind::Pr {
            review, approvals, ..
        } = i.kind
        {
            if let Some(review) = review {
                let (text, role) = match review {
                    ReviewDecision::Approved => ("Approved", Role::Success),
                    ReviewDecision::ChangesRequested => ("Changes requested", Role::Error),
                    ReviewDecision::ReviewRequired => ("Review required", Role::Meta),
                };
                subline.extend([meta(" · "), Seg::new(text, role)]);
            }
            if approvals > 0 {
                let text = format!(" · {}", plural(approvals, "approval"));
                subline.push(meta(text));
            }
        }
        page.box_line(subline, issue_signals(i, icons), 2);
    });
}

/// Checks and comment count, for the right of an issue's row.
pub(super) fn issue_signals(i: &IssueSummary, icons: Icons) -> Vec<Seg> {
    let mut right = Vec::new();
    if let IssueKind::Pr {
        checks: Some(checks),
        ..
    } = i.kind
    {
        let role = match checks {
            ChecksState::Passing => Role::Success,
            ChecksState::Failing => Role::Error,
            ChecksState::Pending => Role::Accent,
        };
        right.push(Seg::new(format!("{}  ", icons.checks(checks)), role));
    }
    if i.comments > 0 {
        right.push(meta(format!("{} {}", icons.comment(), i.comments)));
    }
    right
}

pub(super) fn user_row(page: &mut Page, u: &UserSummary) {
    item(page, url::user(&u.login), |page, link| {
        let mut segs = vec![Seg::linked(u.login.clone(), Role::Link, link)];
        if let Some(name) = &u.name {
            segs.push(strong(format!("  {name}")));
        }
        if u.is_org {
            segs.push(space());
            segs.push(chip("Organization", Bg::SecondaryContainer));
        }
        body(page, segs);
        if let Some(bio) = u.bio.as_ref().filter(|_| !page.compact) {
            page.wrapped(vec![meta(bio.clone())], 0, Frame::Body);
        }
    });
}

/// A list of people (stargazers, watchers, followers…) that loads more.
pub fn people_list(
    page: &mut Page,
    title: &str,
    people: Fetched<'_, Results<UserSummary>>,
    empty: &str,
) {
    paged_list(page, title, people, empty, user_row);
}

/// A list of repositories (forks…) that loads more.
pub fn repos(page: &mut Page, title: &str, repos: Fetched<'_, Results<RepoSummary>>, now: u64) {
    paged_list(page, title, repos, "None yet.", |page, r| {
        repo_row(page, r, now, true);
    });
}

/// The filter field above a list: `⌕ is:open label:bug`.
pub(super) fn filter_field(page: &mut Page, query: &str, keys: Keys<'_>, verb: &str) {
    let width = usize::from(page.room(Frame::None, 0));
    let hint = format!("{} to {verb} ", keys.filter);
    let text = format!(" ⌕ {query}");
    let pad = width.saturating_sub(crate::ui::text::width(&text) + crate::ui::text::width(&hint));
    let link = page.link(Link::Filter);
    page.line(vec![
        Seg::linked(
            format!("{text}{}", " ".repeat(pad)),
            Role::Chip(Bg::ContainerHigh),
            link,
        ),
        Seg::linked(hint, Role::Chip(Bg::ContainerHigh), link),
    ]);
}

/// Which list state a query shows.
pub fn list_state(query: &str) -> &'static str {
    let has = |state: &str| {
        query
            .split_whitespace()
            .any(|w| w.strip_prefix("is:").or_else(|| w.strip_prefix("state:")) == Some(state))
    };
    if has("closed") {
        "closed"
    } else if has("open") {
        "open"
    } else if has("unread") {
        "unread"
    } else if has("read") {
        "read"
    } else {
        "all"
    }
}

/// The sort a query asks for, by name.
pub fn sort_label(query: &str) -> &'static str {
    let sort = query
        .split_whitespace()
        .find_map(|w| w.strip_prefix("sort:"))
        .unwrap_or("created-desc");
    SORTS
        .iter()
        .find(|(q, _)| *q == sort)
        .map_or("Best match", |(_, l)| l)
}

/// GitHub's sorts: `(qualifier, label)`.
pub const SORTS: [(&str, &str); 5] = [
    ("created-desc", "Newest"),
    ("created-asc", "Oldest"),
    ("comments-desc", "Most commented"),
    ("updated-desc", "Recently updated"),
    ("reactions-desc", "Most reactions"),
];

/// A repository's issue or pull request list.
pub fn issue_list(
    page: &mut Page,
    query: &str,
    counts: Option<(u64, u64)>,
    is_pr: bool,
    results: Fetched<'_, Results<IssueSummary>>,
    cx: PageCtx<'_>,
) {
    let PageCtx { icons, keys, now } = cx;
    filter_field(page, query, keys, "filter");
    let state = list_state(query);
    let mut title = Vec::new();
    let (open_state, closed_state) = if is_pr {
        (Status::Pr(PrState::Open), Status::Pr(PrState::Merged))
    } else {
        (
            Status::Issue(IssueState::Open),
            Status::Issue(IssueState::Closed),
        )
    };
    let (open, closed) = counts.unwrap_or_default();
    for (name, kind, n, label) in [
        ("open", open_state, open, "Open"),
        ("closed", closed_state, closed, "Closed"),
    ] {
        let (icon, _) = issue_icon(icons, kind);
        let role = if state == name {
            Role::Strong
        } else {
            Role::Meta
        };
        let text = match counts {
            Some(_) => format!("{icon} {} {label}", compact(n)),
            None => format!("{icon} {label}"),
        };
        let link = page.link(Link::State(name.to_owned()));
        if !title.is_empty() {
            title.push(Seg::new("   ", Role::Body));
        }
        title.push(Seg::linked(text, role, link));
    }
    let sort = link_seg(
        page,
        format!("Sort: {} ▾", sort_label(query)),
        Link::Sort,
        Role::Meta,
    );
    let Some(r) = results.show_or(page, "the list", |page| {
        page.box_top(title.clone(), vec![sort.clone()]);
        skeleton(page);
        page.box_bottom();
    }) else {
        return;
    };
    let empty = "No results matched your search.";
    let updated = by_update(query);
    list_box(page, title, vec![sort], r, empty, |page, i| {
        issue_row(page, i, (false, updated), icons, now);
    });
}

pub fn search(
    page: &mut Page,
    kind: SearchKind,
    query: &str,
    results: Fetched<'_, SearchResults>,
    icons: Icons,
    keys: Keys<'_>,
    now: u64,
) {
    filter_field(page, query, keys, "filter");
    let what = match kind {
        SearchKind::Repos => "repository",
        SearchKind::Issues => "issue",
        SearchKind::Pulls => "pull request",
        SearchKind::Users => "user",
        SearchKind::Discussions => "discussion",
        SearchKind::Commits => "commit",
        SearchKind::Code => "code",
    };
    let searching = |page: &mut Page| loading_box(page, vec![meta("Searching…")]);
    let Some(results) = results.show_or(page, "results", searching) else {
        return;
    };
    let (total, shown) = results.counts();
    page.box_top(
        vec![strong(format!(
            "{} {what} {}",
            compact(total),
            noun(total, "result")
        ))],
        Vec::new(),
    );
    if shown == 0 {
        empty_row(page, "Your search did not match anything.");
    }
    match results {
        SearchResults::Repos(r) => box_rows(page, r, |page, r| repo_row(page, r, now, true)),
        SearchResults::Issues(r) => box_rows(page, r, |page, i| {
            issue_row(page, i, (true, by_update(query)), icons, now);
        }),
        SearchResults::Users(r) => box_rows(page, r, user_row),
        SearchResults::Discussions(r) => box_rows(page, r, |page, hit| {
            let d = &hit.summary;
            let target = format!("{}/discussions/{}", url::repo(&hit.repo), d.number);
            item(page, target, |page, link| {
                let mut segs = vec![Seg::linked(d.title.clone(), Role::Strong, link)];
                if d.answered {
                    segs.push(space());
                    segs.push(chip("✓ Answered", Bg::SuccessContainer));
                }
                body(page, segs);
                let subline = format!(
                    "{}#{} · {} · {} · {} · updated {}",
                    hit.repo,
                    d.number,
                    d.category,
                    d.author,
                    plural(d.comments, "comment"),
                    time::ago_iso(&d.updated_at, now)
                );
                body(page, vec![meta(subline)]);
            });
        }),
        SearchResults::Commits(r) => box_rows(page, r, |page, hit| {
            let c = &hit.commit;
            item(page, url::commit(&hit.repo, &c.oid), |page, link| {
                let right = vec![Seg::new(crate::ui::text::short_sha(&c.oid), Role::Code)];
                page.box_line(
                    vec![Seg::linked(c.headline.clone(), Role::Strong, link)],
                    right,
                    0,
                );
                let subline = format!(
                    "{} · {} committed {}",
                    hit.repo,
                    c.author.name(),
                    time::ago_iso(&c.date, now)
                );
                body(page, vec![meta(subline)]);
            });
        }),
        SearchResults::Code(r) => box_rows(page, r, |page, hit| {
            // Git says where it searched, and which lines matched.
            let rev = hit.rev.as_deref().unwrap_or("HEAD");
            let file = url::blob(&hit.repo, rev, &hit.path);
            item(page, file.clone(), |page, link| {
                page.box_line(
                    vec![Seg::linked(hit.path.clone(), Role::Code, link)],
                    Vec::new(),
                    0,
                );
                body(page, vec![meta(hit.repo.to_string())]);
            });
            for (n, text) in &hit.lines {
                item(page, format!("{file}#L{n}"), |page, link| {
                    let number = Seg::linked(format!("{n:>5}  "), Role::Meta, link);
                    let text = crate::ui::text::truncate(text.trim(), 200);
                    body(page, vec![number, Seg::new(text, Role::Code)]);
                });
            }
        }),
    }
    page.box_bottom();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_queries_name_their_state_and_sort() {
        assert_eq!(list_state("is:open label:bug"), "open");
        assert_eq!(list_state("is:closed"), "closed");
        assert_eq!(list_state("label:bug"), "all");
        assert_eq!(sort_label("is:open sort:comments-desc"), "Most commented");
        assert_eq!(sort_label("is:open"), "Newest");
        assert_eq!(day("2026-10-01T12:00:00Z"), "Oct 1, 2026");
    }
}
