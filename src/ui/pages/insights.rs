//! A repository's Insights: contributors, commit activity, traffic and
//! the dependency graph, each a tab of GitHub's Insights menu.

use super::charts::{bar, buckets, column_dates, columns, date, month, sparkline};
use super::*;
use crate::api::insights::{
    Activity, Counted, Counter, Dependencies, Popular, Traffic, WEEK, YEAR_WEEKS,
};

words! {
    /// Insights' tabs, as GitHub's sidebar lists them.
    #[derive(Hash)]
    pub enum InsightsTab {
        pub fn name,
        /// Its path under the repository.
        pub fn path,
    }
    Contributors "Contributors"     "graphs/contributors";
    Commits      "Commit activity"  "graphs/commit-activity";
    Traffic      "Traffic"          "graphs/traffic";
    Dependencies "Dependency graph" "network/dependencies";
}

/// The columns a chart of weeks takes at most.
const CHART: usize = 160;
/// A chart's height, in rows.
const HEIGHT: usize = 6;

/// Insights' own menu, as GitHub's sidebar: each tab, the one shown
/// strong.
pub fn insights_nav(page: &mut Page, repo: &RepoId, on: InsightsTab) {
    let tabs = InsightsTab::ALL
        .into_iter()
        .map(|tab| (tab.name(), url::insights(repo, tab), tab == on));
    let segs = nav_strip(page, &meta(" · "), &Role::Link, tabs);
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

/// A chart of `weeks` (from the Sunday `start`), as wide as the page
/// lets it be: a week a column, several columns a week when there's
/// room, or several weeks a column when there isn't; then the dates at
/// its ends, and what it counts.
fn week_chart(
    page: &mut Page,
    weeks: &[u32],
    start: u64,
    height: usize,
    when: fn(u64) -> String,
    note: &str,
) {
    let room = usize::from(page.room(Frame::Body, 0)).saturating_sub(10);
    let n = room.min(CHART).min(weeks.len()).max(1);
    let (size, sums) = buckets(weeks, n);
    let width = (room.min(CHART) / sums.len().max(1)).clamp(1, 4);
    columns(page, &sums, width, height);
    let last = start + (weeks.len() as u64).saturating_sub(1) * WEEK;
    column_dates(page, sums.len() * width, &when(start), &when(last));
    let each = match size {
        1 => "Each bar is a week, from Sunday".to_owned(),
        n => format!("Each bar is {n} weeks"),
    };
    let text = format!("{each}. {note}.");
    page.wrapped(vec![meta(text)], 0, Frame::Body);
}

/// Room for a bar after `used` cells of a box's line.
fn bar_room(page: &Page, used: usize) -> usize {
    usize::from(page.room(Frame::Body, 0))
        .saturating_sub(used)
        .min(60)
}

/// A box's row: `label`, then a bar of `n` of `max` at most `room` wide.
fn bar_row(page: &mut Page, label: String, n: u64, max: u64, room: usize) {
    let bar = Seg::new(bar(n, max, room), Role::Accent);
    body(page, vec![Seg::new(label, Role::Body), bar]);
}

/// Contributors: commits over time, then each author's, most first.
pub fn contributors(page: &mut Page, repo: &RepoId, a: &Activity) {
    let (commits, since) = (plural(a.total, "commit"), month(a.start));
    let title = format!("Commits over time  {commits} since {since}");
    page.box_top(vec![strong(title)], Vec::new());
    if a.total == 0 {
        empty_row(page, "No commits on the default branch.");
        page.box_bottom();
        return;
    }
    // Months say a long history's ends; days a short one's.
    let when = if a.weeks.len() > YEAR_WEEKS {
        month
    } else {
        date
    };
    week_chart(page, &a.weeks, a.start, HEIGHT, when, counted_by(a));
    if let Some(cut) = &a.cut {
        page.wrapped(vec![meta(format!("{cut}."))], 0, Frame::Body);
    }
    page.box_bottom();
    page.blank();

    let listed = a.authors.len() as u64;
    let count = match a.authors_left_out {
        0 => listed.to_string(),
        n => format!("{listed} of {}", listed + n),
    };
    let title = vec![strong(format!("Contributors  {count}"))];
    let right = vec![meta("commits · each week, as above")];
    page.box_top(title, right);
    let most = a.authors.iter().map(|x| x.commits).max().unwrap_or(0);
    let digits = compact(most).len();
    let names = (a.authors.iter())
        .map(|x| width(&x.name))
        .max()
        .unwrap_or(0)
        .min(24);
    // What's left of the row after its rank, name and count: the weeks
    // are summed to fit it, never cut, so the newest always show.
    let n = usize::from(page.room(Frame::Body, 0))
        .saturating_sub(5 + names + 3 + digits)
        .min(CHART)
        .min(a.weeks.len())
        .max(1);
    for (i, author) in a.authors.iter().enumerate() {
        if i > 0 && !page.compact {
            page.box_rule();
        }
        let (_, weeks) = buckets(&author.weeks, n);
        let name = truncate(&author.name, names);
        let pad = " ".repeat(names.saturating_sub(width(&name)));
        let mut segs = vec![meta(format!("{:>3}  ", i + 1))];
        let commits = Seg::new(
            format!("  {:>digits$} ", compact(author.commits)),
            Role::Body,
        );
        let line = Seg::new(sparkline(&weeks), Role::Accent);
        let profile = author.login.as_deref().map(url::user);
        maybe_item(page, profile, |page, link| {
            let role = link.map_or(Role::Strong, |_| Role::Link);
            segs.extend([Seg::linked(name, role, link), Seg::new(pad, Role::Body)]);
            segs.extend([commits, line]);
            body(page, segs);
        });
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
        say(page, note, Role::Meta);
    }
}

/// Commit activity: the last year's weeks, then its days of the week.
pub fn commit_activity(page: &mut Page, a: &Activity) {
    let (start, weeks) = a.last_year();
    let total: u64 = weeks.iter().map(|&n| u64::from(n)).sum();
    let title = format!("Commits in the last {YEAR_WEEKS} weeks  {}", compact(total));
    page.box_top(vec![strong(title)], Vec::new());
    week_chart(page, &weeks, start, HEIGHT + 2, date, counted_by(a));
    page.box_bottom();
    page.blank();

    let title = format!("By day of the week, in the last {YEAR_WEEKS} weeks");
    page.box_top(vec![strong(title)], Vec::new());
    let max = a.weekdays.iter().copied().max().map_or(0, u64::from);
    let room = bar_room(page, 13);
    for (day, n) in ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]
        .iter()
        .zip(a.weekdays)
    {
        let n = u64::from(n);
        bar_row(page, format!("{day}  {:>6}  ", compact(n)), n, max, room);
    }
    page.box_bottom();
    // The weeks with commits, newest first, as numbers.
    page.blank();
    let title = vec![strong("Week by week")];
    page.box_top(title, vec![meta("newest first")]);
    let max = weeks.iter().copied().max().map_or(0, u64::from);
    let room = bar_room(page, 22);
    let mut any = false;
    for (i, &n) in weeks.iter().enumerate().rev().filter(|(_, n)| **n > 0) {
        any = true;
        let at = start + i as u64 * WEEK;
        let n = u64::from(n);
        let label = format!("{:<13} {:>6}  ", date(at), compact(n));
        bar_row(page, label, n, max, room);
    }
    if !any {
        empty_row(page, "No commits in the last year.");
    }
    page.box_bottom();
}

/// Views or clones, day by day.
fn traffic_box(page: &mut Page, what: &str, people: &str, c: &Counted) {
    let (count, uniques) = (compact(c.count), compact(c.uniques));
    let title = format!("{what}  {count} · {uniques} unique {people}");
    page.box_top(vec![strong(title)], vec![meta("the last 14 days")]);
    if c.days.is_empty() {
        empty_row(page, "None.");
    }
    let max = c.days.iter().map(|d| d.1).max().unwrap_or(0);
    let room = bar_room(page, 35);
    for (day, count, uniques) in &c.days {
        let label = format!(
            "{:<13} {:>5} · {:>4} unique  ",
            super::day(day),
            compact(*count),
            compact(*uniques)
        );
        bar_row(page, label, *count, max, room);
    }
    page.box_bottom();
    page.blank();
}

/// Referring sites or popular content.
fn popular(page: &mut Page, title: &str, list: &[Popular], paths: bool) {
    let title = vec![strong(title)];
    page.box_top(title, vec![meta("views · unique")]);
    if list.is_empty() {
        empty_row(page, "None in the last 14 days.");
    }
    for p in list {
        let counts = vec![meta(format!(
            "{} · {}",
            compact(p.count),
            compact(p.uniques)
        ))];
        // GitHub's title for a path is often the path again.
        let named = (p.title.as_deref())
            .filter(|t| !p.name.ends_with(t.trim_start_matches('/')) && *t != p.name);
        if paths {
            // A path on the website, which ghtui may open.
            let target = format!("{}{}", url::base(), p.name);
            item(page, target, |page, link| {
                let mut segs = vec![Seg::linked(p.name.clone(), Role::Link, link)];
                if let Some(t) = named {
                    segs.push(meta(format!("  {t}")));
                }
                page.box_line(segs, counts, 0);
            });
        } else {
            page.box_line(vec![Seg::new(p.name.clone(), Role::Body)], counts, 0);
        }
    }
    page.box_bottom();
    page.blank();
}

/// Traffic: views, clones, referring sites and popular content.
pub fn traffic(page: &mut Page, t: &Traffic) {
    traffic_box(page, "Views", "visitors", &t.views);
    traffic_box(page, "Git clones", "cloners", &t.clones);
    popular(page, "Referring sites", &t.referrers, false);
    popular(page, "Popular content", &t.paths, true);
}

/// Who depends on the repository: not in GitHub's API, so a link to
/// GitHub's page of them.
pub fn dependents_note(page: &mut Page, repo: &RepoId) {
    let target = format!("{}/network/dependents", url::repo(repo));
    let segs = vec![
        meta("Dependents (who depends on this repository) aren't in GitHub's API: "),
        link_seg(page, "see them on github.com ↗", target, Role::Link),
    ];
    page.wrapped(segs, 0, Frame::None);
    page.blank();
}

/// The dependency graph: each manifest and what it depends on.
pub fn dependencies(page: &mut Page, d: &Dependencies) {
    if d.manifests.is_empty() {
        let why = "GitHub found no manifests: the dependency graph may be off, or not yet built.";
        empty_box(page, strong("Dependencies"), why);
        return;
    }
    for m in &d.manifests {
        let count = match m.dependencies.left_out() {
            0 => m.dependencies.len().to_string(),
            _ => format!("{} of {}", m.dependencies.len(), m.dependencies.total()),
        };
        let title = vec![strong(m.path.clone()), meta(format!("  {count}"))];
        page.box_top(title, Vec::new());
        if m.unparseable {
            empty_row(page, "GitHub couldn't read this manifest.");
        } else if m.too_big {
            empty_row(page, "Too big for GitHub to read.");
        } else if m.dependencies.is_empty() {
            empty_row(page, "No dependencies.");
        }
        let names = (m.dependencies.iter())
            .map(|d| width(&d.package))
            .max()
            .unwrap_or(0)
            .min(40);
        let wants = (m.dependencies.iter())
            .map(|d| width(&d.requirements))
            .max()
            .unwrap_or(0)
            .min(30);
        for dep in &m.dependencies {
            let name = truncate(&dep.package, names);
            let pad = " ".repeat(names.saturating_sub(width(&name)) + 2);
            let want = truncate(&dep.requirements, wants);
            let gap = " ".repeat(wants.saturating_sub(width(&want)) + 2);
            let rest = vec![
                Seg::new(pad, Role::Body),
                Seg::new(want, Role::Body),
                meta(format!("{gap}{}", dep.manager.to_lowercase())),
            ];
            maybe_item(page, dep.repo.as_ref().map(url::repo), |page, link| {
                let role = link.map_or(Role::Strong, |_| Role::Link);
                let mut segs = vec![Seg::linked(name, role, link)];
                segs.extend(rest);
                body(page, segs);
            });
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
        say(page, text, Role::Meta);
    }
}
