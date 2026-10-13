//! A repository's Insights: contributors, commit activity, traffic and
//! the dependency graph, each a tab of GitHub's Insights menu.

use super::charts::{bar, buckets, column_dates, columns, date, month, sparkline};
use super::*;
use crate::api::insights::{
    Activity, Counter, Counted, Dependencies, Popular, Traffic, WEEK, YEAR_WEEKS,
};
use crate::site::{InsightsTab, Page as Site};

/// The columns a chart of weeks takes at most.
const CHART: usize = 104;
/// A chart's height, in rows.
const HEIGHT: usize = 6;

/// Insights' own menu, as GitHub's sidebar: each tab, the one shown
/// strong.
pub fn insights_nav(page: &mut Page, repo: &RepoId, on: InsightsTab) {
    let mut segs = Vec::new();
    for tab in InsightsTab::ALL {
        if !segs.is_empty() {
            segs.push(Seg::new(" · ", Role::Meta));
        }
        let target = crate::route::Route::Site(Site::Insights {
            repo: repo.clone(),
            tab,
        });
        let role = if tab == on { Role::Strong } else { Role::Link };
        segs.push(link_seg(page, tab.name(), target.url(), role));
    }
    page.wrapped(segs, 0, Frame::None);
    page.blank();
}

/// Who counted, said under the chart.
fn counted_by(a: &Activity) -> &'static str {
    match a.by {
        Counter::Git => "Counted by git on the default branch, merges left out",
        Counter::GitHub => "Counted by GitHub on the default branch, merges left out",
    }
}

/// How many columns a chart of `weeks` gets on this page, and how many
/// weeks each column counts.
fn chart_columns(page: &Page, weeks: usize) -> usize {
    let room = usize::from(page.room(Frame::Body, 0)).saturating_sub(10);
    room.min(CHART).min(weeks).max(1)
}

/// Contributors: commits over time, then each author's, most first.
pub fn contributors(page: &mut Page, repo: &RepoId, a: &Activity) {
    let since = month(a.start);
    let title = format!(
        "Commits over time  {} since {since}",
        plural(a.total, "commit")
    );
    page.box_top(vec![Seg::new(title, Role::Strong)], Vec::new());
    if a.total == 0 {
        empty_row(page, "No commits on the default branch.");
        page.box_bottom();
        return;
    }
    let n = chart_columns(page, a.weeks.len());
    let (size, sums) = buckets(&a.weeks, n);
    columns(page, &sums, HEIGHT);
    let last = a.start + (a.weeks.len() as u64).saturating_sub(1) * WEEK;
    column_dates(page, sums.len(), &month(a.start), &month(last));
    let each = match size {
        1 => "Each column is a week".to_owned(),
        n => format!("Each column is {n} weeks"),
    };
    body(
        page,
        vec![Seg::new(format!("{each}. {}.", counted_by(a)), Role::Meta)],
    );
    if let Some(cut) = &a.cut {
        body(page, vec![Seg::new(format!("{cut}."), Role::Meta)]);
    }
    page.box_bottom();
    page.blank();

    let listed = a.authors.len() as u64;
    let count = match a.authors_left_out {
        0 => listed.to_string(),
        n => format!("{listed} of {}", listed + n),
    };
    let title = vec![Seg::new(format!("Contributors  {count}"), Role::Strong)];
    let right = vec![Seg::new("commits · each week, as above", Role::Meta)];
    page.box_top(title, right);
    let width = a.authors.iter().map(|x| x.commits).max().unwrap_or(0);
    let digits = compact(width).len();
    let names = (a.authors.iter())
        .map(|x| crate::ui::text::width(&x.name))
        .max()
        .unwrap_or(0)
        .min(24);
    for (i, author) in a.authors.iter().enumerate() {
        if i > 0 && !page.compact {
            page.box_rule();
        }
        let (_, weeks) = buckets(&author.weeks, n);
        let name = crate::ui::text::truncate(&author.name, names);
        let pad = " ".repeat(names.saturating_sub(crate::ui::text::width(&name)));
        let mut segs = vec![Seg::new(format!("{:>3}  ", i + 1), Role::Meta)];
        let commits = Seg::new(
            format!("  {:>digits$} ", compact(author.commits)),
            Role::Body,
        );
        let line = Seg::new(sparkline(&weeks), Role::Accent);
        match &author.login {
            Some(login) => {
                let link = page.link(url::user(login));
                let start = page.lines.len();
                segs.push(Seg::linked(name, Role::Link, link));
                segs.extend([Seg::new(pad, Role::Body), commits, line]);
                body(page, segs);
                page.item(start, link);
            }
            None => {
                segs.extend([Seg::new(name, Role::Strong), Seg::new(pad, Role::Body)]);
                segs.extend([commits, line]);
                body(page, segs);
            }
        }
    }
    if a.authors_left_out > 0 {
        page.box_rule();
        let more = a.authors_left_out;
        let text = format!("… {more} more, with fewer commits");
        empty_row(page, &text);
    }
    page.box_bottom();
    if a.by == Counter::Git {
        page.blank();
        let note = format!(
            "Authors by their git names (as {}'s .mailmap joins them): linked where a GitHub no-reply address names the account.",
            repo.name
        );
        page.wrapped(vec![Seg::new(note, Role::Meta)], 0, Frame::None);
    }
}

/// Commit activity: the last year's weeks, then its days of the week.
pub fn commit_activity(page: &mut Page, a: &Activity) {
    let (start, weeks) = a.last_year();
    let total: u64 = weeks.iter().map(|&n| u64::from(n)).sum();
    let title = format!(
        "Commits in the last {} weeks  {}",
        weeks.len(),
        compact(total)
    );
    page.box_top(vec![Seg::new(title, Role::Strong)], Vec::new());
    let n = chart_columns(page, weeks.len());
    let (size, sums) = buckets(weeks, n);
    columns(page, &sums, HEIGHT + 2);
    let last = start + (weeks.len() as u64).saturating_sub(1) * WEEK;
    column_dates(page, sums.len(), &date(start), &date(last));
    let each = match size {
        1 => "Each column is a week, from Sunday".to_owned(),
        n => format!("Each column is {n} weeks"),
    };
    body(
        page,
        vec![Seg::new(format!("{each}. {}.", counted_by(a)), Role::Meta)],
    );
    page.box_bottom();
    page.blank();

    let title = format!("By day of the week, in the last {YEAR_WEEKS} weeks");
    page.box_top(vec![Seg::new(title, Role::Strong)], Vec::new());
    let max = a.weekdays.iter().copied().max().map_or(0, u64::from);
    let room = usize::from(page.room(Frame::Body, 0)).saturating_sub(16);
    for (day, n) in ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]
        .iter()
        .zip(a.weekdays)
    {
        let n = u64::from(n);
        body(
            page,
            vec![
                Seg::new(format!("{day}  {:>6}  ", compact(n)), Role::Body),
                Seg::new(bar(n, max, room.min(60)), Role::Accent),
            ],
        );
    }
    page.box_bottom();
    // The weeks, newest first, as numbers.
    page.blank();
    let title = vec![Seg::new("Week by week", Role::Strong)];
    page.box_top(title, vec![Seg::new("newest first", Role::Meta)]);
    let max = weeks.iter().copied().max().map_or(0, u64::from);
    for (i, &n) in weeks.iter().enumerate().rev() {
        let at = start + i as u64 * WEEK;
        let n = u64::from(n);
        body(
            page,
            vec![
                Seg::new(format!("{:<13} {:>6}  ", date(at), compact(n)), Role::Body),
                Seg::new(bar(n, max, room.min(60)), Role::Accent),
            ],
        );
    }
    page.box_bottom();
}

/// Views or clones, day by day.
fn counted(page: &mut Page, what: &str, people: &str, c: &Counted) {
    let title = vec![Seg::new(
        format!(
            "{what}  {} · {} unique {people}",
            compact(c.count),
            compact(c.uniques)
        ),
        Role::Strong,
    )];
    page.box_top(title, vec![Seg::new("the last 14 days", Role::Meta)]);
    if c.days.is_empty() {
        empty_row(page, "None.");
    }
    let max = c.days.iter().map(|d| d.1).max().unwrap_or(0);
    let room = usize::from(page.room(Frame::Body, 0)).saturating_sub(34);
    for (day, count, uniques) in &c.days {
        body(
            page,
            vec![
                Seg::new(
                    format!(
                        "{:<13} {:>5} · {:>4} unique  ",
                        super::day(day),
                        compact(*count),
                        compact(*uniques)
                    ),
                    Role::Body,
                ),
                Seg::new(bar(*count, max, room.min(50)), Role::Accent),
            ],
        );
    }
    page.box_bottom();
    page.blank();
}

/// Referring sites or popular content.
fn popular(page: &mut Page, title: &str, list: &[Popular], paths: bool) {
    let title = vec![Seg::new(title, Role::Strong)];
    page.box_top(title, vec![Seg::new("views · unique", Role::Meta)]);
    if list.is_empty() {
        empty_row(page, "None in the last 14 days.");
    }
    for p in list {
        let counts = Seg::new(
            format!("  {} · {}", compact(p.count), compact(p.uniques)),
            Role::Meta,
        );
        let named = p.title.as_deref().filter(|t| *t != p.name);
        if paths {
            // A path on the website, which ghtui may open.
            let target = format!("{}{}", url::base(), p.name);
            item(page, target, |page, link| {
                let mut segs = vec![Seg::linked(p.name.clone(), Role::Link, link)];
                if let Some(t) = named {
                    segs.push(Seg::new(format!("  {t}"), Role::Meta));
                }
                segs.push(counts);
                body(page, segs);
            });
        } else {
            body(page, vec![Seg::new(p.name.clone(), Role::Body), counts]);
        }
    }
    page.box_bottom();
    page.blank();
}

/// Traffic: views, clones, referring sites and popular content.
pub fn traffic(page: &mut Page, t: &Traffic) {
    counted(page, "Views", "visitors", &t.views);
    counted(page, "Git clones", "cloners", &t.clones);
    popular(page, "Referring sites", &t.referrers, false);
    popular(page, "Popular content", &t.paths, true);
}

/// Who depends on the repository: not in GitHub's API, so a link to
/// GitHub's page of them.
pub fn dependents_note(page: &mut Page, repo: &RepoId) {
    let target = format!("{}/network/dependents", url::repo(repo));
    let segs = vec![
        Seg::new(
            "Dependents (who depends on this repository) aren't in GitHub's API: ",
            Role::Meta,
        ),
        link_seg(page, "see them on github.com ↗", target, Role::Link),
    ];
    page.wrapped(segs, 0, Frame::None);
    page.blank();
}

/// The dependency graph: each manifest and what it depends on.
pub fn dependencies(page: &mut Page, d: &Dependencies) {
    if d.manifests.is_empty() {
        let title = Seg::new("Dependencies", Role::Strong);
        empty_box(
            page,
            title,
            "GitHub found no manifests: the dependency graph may be off, or not yet built.",
        );
        return;
    }
    for m in d.manifests.iter() {
        let count = match m.dependencies.left_out() {
            0 => m.dependencies.len().to_string(),
            _ => format!("{} of {}", m.dependencies.len(), m.dependencies.total()),
        };
        let title = vec![
            Seg::new(m.path.clone(), Role::Strong),
            Seg::new(format!("  {count}"), Role::Meta),
        ];
        page.box_top(title, Vec::new());
        if m.unparseable {
            empty_row(page, "GitHub couldn't read this manifest.");
        } else if m.too_big {
            empty_row(page, "Too big for GitHub to read.");
        } else if m.dependencies.is_empty() {
            empty_row(page, "No dependencies.");
        }
        let names = (m.dependencies.iter())
            .map(|d| crate::ui::text::width(&d.package))
            .max()
            .unwrap_or(0)
            .min(40);
        for dep in m.dependencies.iter() {
            let name = crate::ui::text::truncate(&dep.package, names);
            let pad = " ".repeat(names.saturating_sub(crate::ui::text::width(&name)) + 2);
            let rest = vec![
                Seg::new(pad, Role::Body),
                Seg::new(dep.requirements.clone(), Role::Body),
                Seg::new(format!("  {}", dep.manager.to_lowercase()), Role::Meta),
            ];
            match &dep.repo {
                Some(r) => item(page, url::repo(r), |page, link| {
                    let mut segs = vec![Seg::linked(name, Role::Link, link)];
                    segs.extend(rest);
                    body(page, segs);
                }),
                None => {
                    let mut segs = vec![Seg::new(name, Role::Strong)];
                    segs.extend(rest);
                    body(page, segs);
                }
            }
        }
        if m.dependencies.left_out() > 0 {
            page.box_rule();
            let n = m.dependencies.left_out();
            empty_row(page, &left_out_text(n, "more", "more"));
        }
        page.box_bottom();
        page.blank();
    }
    if d.manifests.left_out() > 0 {
        let n = d.manifests.left_out();
        let text = left_out_text(n, "more manifest", "more manifests");
        page.wrapped(vec![Seg::new(text, Role::Meta)], 0, Frame::None);
    }
}
