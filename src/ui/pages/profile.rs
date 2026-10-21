//! Profiles, teams and gists.

use super::*;

pub(super) fn team_row(page: &mut Page, org: &str, t: &TeamSummary) {
    item(
        page,
        format!("{}/orgs/{org}/teams/{}", url::base(), t.slug),
        |page, link| {
            let mut segs = vec![Seg::linked(t.name.clone(), Role::Strong, link)];
            if t.secret {
                tag(&mut segs, "Secret", Bg::SecondaryContainer);
            }
            let counts = format!(
                "{} · {}",
                plural(t.members, "member"),
                plural(t.repos, "repository")
            );
            page.box_line(segs, vec![meta(counts)], 0);
            if !t.description.is_empty() {
                body(page, vec![meta(t.description.clone())]);
            }
        },
    );
}

/// An organization's teams, the ones you can see.
pub fn teams(page: &mut Page, org: &str, list: Fetched<'_, Results<TeamSummary>>) {
    paged_list(
        page,
        "Teams",
        list,
        "No teams you can see: an organization's teams show to its members.",
        |page, t| team_row(page, org, t),
    );
}

/// A team: its description and parent, then its members, repositories
/// and child teams.
pub fn team(page: &mut Page, org: &str, d: &TeamDetail) {
    let t = &d.team;
    let mut title = vec![
        link_seg(page, format!("@{org}"), url::user(org), Role::Link),
        meta(" / "),
        Seg::new(t.name.clone(), Role::Title),
    ];
    if t.secret {
        tag(&mut title, "Secret", Bg::SecondaryContainer);
    }
    page.wrapped(title, 0, Frame::None);
    if !t.description.is_empty() {
        say(page, t.description.clone(), Role::Body);
    }
    if let Some(parent) = &d.parent {
        let target = format!("{}/orgs/{org}/teams/{}", url::base(), parent.slug);
        let link = link_seg(page, parent.name.clone(), target, Role::Link);
        page.wrapped(vec![meta("Part of "), link], 0, Frame::None);
    }
    page.blank();
    let title = counted("Members", &d.members);
    list_box(page, title, Vec::new(), &d.members, "No members.", user_row);
    page.blank();
    let title = counted("Repositories", &d.repos);
    list_box(
        page,
        title,
        Vec::new(),
        &d.repos,
        "No repositories.",
        |page, r| {
            item(page, url::repo(&r.repo), |page, link| {
                let right = vec![meta(format!("★ {}", compact(r.stars)))];
                page.box_line(
                    vec![Seg::linked(r.repo.to_string(), Role::Link, link)],
                    right,
                    0,
                );
                if !r.description.is_empty() {
                    body(page, vec![meta(r.description.clone())]);
                }
            });
        },
    );
    if !d.children.is_empty() {
        page.blank();
        let title = counted("Child teams", &d.children);
        list_box(page, title, Vec::new(), &d.children, "", |page, c| {
            team_row(page, org, c);
        });
    }
}

/// A gist: who and when, its description, and each of its files.
pub fn gist(page: &mut Page, g: &Gist, now: u64) {
    let first = g.files.first().map_or(g.id.as_str(), |f| f.name.as_str());
    let mut title = Vec::new();
    if let Some(owner) = &g.owner {
        title.push(link_seg(
            page,
            owner.clone(),
            format!("{}/{owner}", url::gists()),
            Role::Link,
        ));
        title.push(meta(" / "));
    }
    title.push(Seg::new(first.to_owned(), Role::Title));
    if !g.public {
        tag(&mut title, "Secret", Bg::SecondaryContainer);
    }
    page.wrapped(title, 0, Frame::None);
    let subline = format!(
        "created {} · updated {} · {} · {}",
        time::ago_iso(&g.created_at, now),
        time::ago_iso(&g.updated_at, now),
        plural(g.files.len() as u64, "file"),
        plural(g.comments, "comment"),
    );
    say(page, subline, Role::Meta);
    if !g.description.is_empty() {
        say(page, g.description.clone(), Role::Body);
    }
    for f in &g.files {
        page.blank();
        let mut info = vec![strong(f.name.clone())];
        let mut facts = vec![size(f.size)];
        if let Some(language) = &f.language {
            facts.push(language.clone());
        }
        info.push(meta(format!("  {}", facts.join(" · "))));
        page.box_top(info, Vec::new());
        match &f.text {
            Some(text) if is_markdown(&f.name) => {
                markdown::render(page, text, None, Frame::Body);
            }
            Some(text) => {
                let lines =
                    markdown::highlighted_file(text, crate::diff::Language::from_path(&f.name));
                let width = lines.len().to_string().len();
                for (i, mut segs) in lines.into_iter().enumerate() {
                    segs.insert(0, line_number(i + 1, width));
                    page.push(PageLine {
                        segs,
                        frame: Frame::Body,
                        tone: Tone::Code,
                        ..PageLine::default()
                    });
                }
            }
            None => empty_row(page, "Too large to show here: open it on GitHub."),
        }
        if f.truncated {
            body(page, vec![meta("Only the beginning is shown.")]);
        }
        page.box_bottom();
    }
}

/// Someone's public gists, most recently updated first.
pub fn gists(page: &mut Page, login: &str, list: Fetched<'_, Results<GistSummary>>, now: u64) {
    paged_list(page, "Gists", list, "No public gists.", |page, g| {
        item(
            page,
            format!("{}/{login}/{}", url::gists(), g.id),
            |page, link| {
                let name = g.files.first().cloned().unwrap_or_else(|| g.id.clone());
                let right = vec![meta(time::ago_iso(&g.updated_at, now))];
                page.box_line(vec![Seg::linked(name, Role::Strong, link)], right, 0);
                if !g.description.is_empty() {
                    body(page, vec![Seg::new(g.description.clone(), Role::Body)]);
                }
                let subline = format!(
                    "{} · {} · ★ {}",
                    plural(g.files.len() as u64, "file"),
                    plural(g.comments, "comment"),
                    compact(g.stars)
                );
                body(page, vec![meta(subline)]);
            },
        );
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ProfileTab {
    #[default]
    Overview,
    Repositories(RepoSort),
    Stars,
    Followers,
    Following,
    /// An organization's public members.
    People,
}

/// The list a profile tab shows, once loaded.
#[derive(Debug, Clone, Copy)]
pub enum ProfileList<'a> {
    Repos(Fetched<'a, Results<RepoSummary>>),
    /// What a user starred, and their lists.
    Starred(
        Fetched<'a, Capped<crate::api::lists::StarList>>,
        Fetched<'a, Results<RepoSummary>>,
    ),
    People(Fetched<'a, Results<UserSummary>>),
    /// The overview's are in the profile.
    None,
}

pub fn profile(page: &mut Page, p: &Profile, tab: ProfileTab, list: ProfileList<'_>, now: u64) {
    let mut segs = Vec::new();
    match &p.name {
        Some(name) => {
            segs.push(Seg::new(name.clone(), Role::Title));
            segs.push(meta(format!("  {}", p.login)));
        }
        None => segs.push(Seg::new(p.login.clone(), Role::Title)),
    }
    if p.is_org {
        tag(&mut segs, "Organization", Bg::SecondaryContainer);
    }
    if p.verified {
        tag(&mut segs, "✓ Verified", Bg::SuccessContainer);
    }
    page.line(segs);
    if let Some(status) = &p.status {
        say(page, status.clone(), Role::Meta);
    }
    if let Some(bio) = &p.bio {
        say(page, bio.clone(), Role::Body);
    }
    let mut facts = Vec::new();
    if let (Some(followers), Some(following)) = (p.followers, p.following) {
        facts.push(meta(format!(
            "⚇ {} · {} following",
            plural(followers, "follower"),
            compact(following)
        )));
    }
    // Spaced apart, with nothing before the first.
    let gap = |segs: &Vec<Seg>| if segs.is_empty() { "" } else { "   " };
    for (icon, value) in [("◆ ", &p.company), ("⌖ ", &p.location), ("", &p.pronouns)] {
        if let Some(v) = value {
            facts.push(meta(format!("{}{icon}{v}", gap(&facts))));
        }
    }
    if let Some(site) = &p.website {
        facts.push(meta(format!("{}↗ ", gap(&facts))));
        facts.push(link_seg(page, site.clone(), site.clone(), Role::Link));
    }
    if !facts.is_empty() {
        page.wrapped(facts, 0, Frame::None);
    }
    let mut links = Vec::new();
    for (shown, target) in &p.socials {
        links.push(meta(format!("{}↗ ", gap(&links))));
        links.push(link_seg(page, shown.clone(), target.clone(), Role::Link));
    }
    if !links.is_empty() {
        page.wrapped(links, 0, Frame::None);
    }
    if !p.orgs.is_empty() {
        let mut orgs = vec![meta("Organizations  ")];
        orgs.extend(login_list(page, " · ", &p.orgs));
        more_chips(&mut orgs, &p.orgs);
        page.wrapped(orgs, 0, Frame::None);
    }
    if tab == ProfileTab::Overview
        && let Some(readme) = &p.readme
    {
        page.blank();
        let (repo, path) = if p.is_org {
            (format!("{}/.github", p.login), "profile/README.md")
        } else {
            (format!("{0}/{0}", p.login), "README.md")
        };
        let title = meta(format!("{repo} / {path}"));
        page.box_top(vec![title], Vec::new());
        if let Some(repo) = RepoId::parse(&repo) {
            let base = LinkBase::new(&repo, "HEAD", path);
            markdown::render(page, readme, Some(&base), Frame::Body);
        }
        page.box_bottom();
    }
    page.blank();
    let list = match list {
        ProfileList::Starred(lists, repos) => {
            super::star_lists(page, &p.login, lists);
            ProfileList::Repos(repos)
        }
        list => list,
    };
    match (tab, list) {
        (ProfileTab::Overview, _) => {}
        (tab, ProfileList::Repos(repos)) => {
            let (title, sort, show_owner, empty) = match tab {
                ProfileTab::Repositories(sort) => {
                    let sort = format!("Sort: {}", sort.label());
                    let sort = link_seg(page, sort, Link::Sort, Role::Link);
                    let title = format!("Repositories  {}", compact(p.repos.total()));
                    (title, sort, false, "No public repositories yet.")
                }
                _ => {
                    let sort = meta("Recently starred");
                    let title = format!("Starred  {}", compact(p.star_count));
                    (title, sort, true, "Nothing starred yet.")
                }
            };
            if let Some(r) = repos.show(page, "repositories") {
                let title = vec![strong(title)];
                list_box(page, title, vec![sort], r, empty, |page, repo| {
                    repo_row(page, repo, now, show_owner);
                });
            }
            return;
        }
        (tab, ProfileList::People(people)) => {
            let title = match tab {
                ProfileTab::Followers => "Followers",
                ProfileTab::Following => "Following",
                _ => "People",
            };
            people_list(page, title, people, "Nobody here yet.");
            return;
        }
        (_, ProfileList::None | ProfileList::Starred(..)) => return,
    }
    // The overview: pinned (or popular) repositories, then the rest.
    let (title, repos, show_owner) = if p.pinned.is_empty() {
        let mut popular: Vec<&RepoSummary> = p.repos.items.iter().collect();
        popular.sort_by_key(|r| std::cmp::Reverse(r.stars));
        popular.truncate(6);
        ("Popular repositories", popular, false)
    } else {
        ("Pinned", p.pinned.iter().collect(), true)
    };
    let title = vec![strong(title)];
    list_box(
        page,
        title,
        Vec::new(),
        &Whole(&repos),
        "No public repositories yet.",
        |page, r| {
            repo_row(page, r, now, show_owner);
        },
    );
    if let Some(c) = &p.contributions {
        page.blank();
        contribution_graph(page, c);
        page.blank();
        contribution_activity(page, c);
    }
    if p.is_org {
        page.blank();
        let title = vec![
            strong("People"),
            meta(format!("  {}", compact(p.people.total()))),
        ];
        page.box_top(title, Vec::new());
        if p.people.is_empty() {
            empty_row(page, "No public members.");
        } else {
            let people = login_list(page, " · ", &p.people);
            page.wrapped(people, 0, Frame::Body);
        }
        // The rest are on the People tab.
        let all = format!("{}/orgs/{}/people", url::base(), p.login);
        rest_row(page, p.people.len(), At(&p.people, Some(all)).rest());
        page.box_bottom();
        let languages = top_languages(&p.repos);
        if !languages.is_empty() {
            page.blank();
            let mut segs = vec![strong("Top languages  ")];
            for (lang, n) in languages {
                segs.push(Seg::new("● ", Role::Accent));
                segs.push(meta(format!("{lang} {n}   ")));
            }
            page.wrapped(segs, 0, Frame::None);
        }
    }
}

/// 1234567 → "1,234,567".
pub(super) fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// "Sep" for `2026-09-…`.
pub(super) fn month_name(date: &str) -> Option<&'static str> {
    month_abbr(date.get(5..7)?.parse().ok()?)
}

/// The last year's contributions as GitHub draws them: a column per week,
/// a row per day, shaded by how much happened. Narrow pages show the most
/// recent weeks that fit.
pub(super) fn contribution_graph(page: &mut Page, c: &Contributions) {
    const LABELS: usize = 4;
    let title = format!(
        "{} {} in the last year",
        thousands(c.total),
        noun(c.total, "contribution")
    );
    page.line(vec![strong(title)]);
    let room = usize::from(page.room(Frame::None, 0)).saturating_sub(LABELS);
    let weeks = c
        .weeks
        .get(c.weeks.len().saturating_sub(room)..)
        .unwrap_or_default();
    // Month names over the week each month starts in, where they fit.
    let mut row = vec![' '; weeks.len()];
    let (mut free, mut last) = (0, None);
    for (i, week) in weeks.iter().enumerate() {
        let month = month_name(&week.start);
        if month != last
            && i >= free
            && let Some(name) = month
            && i + name.len() <= row.len()
        {
            for (cell, ch) in row.iter_mut().skip(i).zip(name.chars()) {
                *cell = ch;
            }
            free = i + name.len() + 1;
        }
        last = month;
    }
    let months: String = " ".repeat(LABELS).chars().chain(row).collect();
    page.line(vec![meta(months)]);
    for day in 0..7 {
        let label = match day {
            1 => "Mon ",
            3 => "Wed ",
            5 => "Fri ",
            _ => "    ",
        };
        let mut segs = vec![meta(label)];
        for week in weeks {
            segs.push(match week.days.get(day) {
                Some(&level) => Seg::new("■", Role::Heat(level.min(4))),
                None => meta(" "),
            });
        }
        page.line(segs);
    }
    let mut legend = vec![meta(format!("{}Less ", " ".repeat(LABELS)))];
    legend.extend((0..=4).map(|level| Seg::new("■", Role::Heat(level))));
    legend.push(meta(" More"));
    page.line(legend);
}

/// "20+ pull requests" when GitHub may have more than `n`.
pub(super) fn at_least(n: u64, word: &str, short: bool) -> String {
    if short {
        format!("{n}+ {}", noun(2, word))
    } else {
        plural(n, word)
    }
}

/// What someone did, month by month: commits by repository, and the pull
/// requests and issues they opened and reviewed.
pub(super) fn contribution_activity(page: &mut Page, c: &Contributions) {
    page.line(vec![strong("Contribution activity")]);
    if c.activity.is_empty() {
        page.line(vec![meta("No recent activity.")]);
        return;
    }
    for m in &c.activity {
        page.blank();
        let year = m.month.get(..4).unwrap_or_default();
        let name = month_name(&format!("{}-01", m.month)).unwrap_or_default();
        page.line(vec![Seg::new(format!("{name} {year}"), Role::Heading)]);
        if !m.commits.is_empty() {
            let total: u64 = m.commits.iter().map(|(_, n)| n).sum();
            let repos = plural(m.commits.len() as u64, "repository");
            let short = Short::in_month(c.short.commits.as_ref(), &m.month);
            let created = format!("◷ Created {} in {repos}", at_least(total, "commit", short));
            page.line(vec![Seg::new(created, Role::Body)]);
            for (repo, n) in m.commits.iter().take(5) {
                let name = link_seg(
                    page,
                    repo.clone(),
                    format!("{}/{repo}", url::base()),
                    Role::Link,
                );
                page.line(vec![
                    meta("    "),
                    name,
                    meta(format!("  {}", plural(*n, "commit"))),
                ]);
            }
        }
        for (items, icon, verb, short) in [
            (&m.pulls, "⇄", "Opened", &c.short.pulls),
            (&m.issues, "◉", "Opened", &c.short.issues),
            (&m.reviews, "◎", "Reviewed", &c.short.reviews),
        ] {
            if items.is_empty() {
                continue;
            }
            let noun = if icon == "◉" {
                "issue"
            } else {
                "pull request"
            };
            let what = at_least(
                items.len() as u64,
                noun,
                Short::in_month(short.as_ref(), &m.month),
            );
            page.line(vec![Seg::new(format!("{icon} {verb} {what}"), Role::Body)]);
            for item in items.iter().take(5) {
                let target = if icon == "◉" {
                    format!("{}/{}/issues/{}", url::base(), item.repo, item.number)
                } else {
                    format!("{}/{}/pull/{}", url::base(), item.repo, item.number)
                };
                let place = format!("{}#{}", item.repo, item.number);
                let link = link_seg(page, item.title.clone(), target, Role::Link);
                page.wrapped(vec![meta(format!("{place}  ")), link], 4, Frame::None);
            }
        }
    }
    let short = &c.short;
    if [&short.commits, &short.pulls, &short.issues, &short.reviews]
        .iter()
        .any(|s| s.is_some())
    {
        page.blank();
        page.line(vec![meta(
            "Counts with + are at least that: GitHub has more",
        )]);
    }
}

/// An organization's most used languages, across the repositories loaded.
pub(super) fn top_languages(repos: &[RepoSummary]) -> Vec<(String, usize)> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for lang in repos.iter().filter_map(|r| r.language.as_ref()) {
        match counts.iter_mut().find(|(l, _)| l == lang) {
            Some((_, n)) => *n += 1,
            None => counts.push((lang.clone(), 1)),
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    counts.truncate(5);
    counts
}
