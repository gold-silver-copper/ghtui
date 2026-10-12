//! Profiles, teams and gists.

use super::*;

pub(super) fn team_row(page: &mut Page, org: &str, t: &TeamSummary) {
    item(
        page,
        format!("{}/orgs/{org}/teams/{}", url::base(), t.slug),
        |page, link| {
            let mut segs = vec![Seg::linked(t.name.clone(), Role::Strong, link)];
            if t.secret {
                segs.push(space());
                segs.push(chip("Secret", Bg::SecondaryContainer));
            }
            let counts = format!(
                "{} · {}",
                plural(t.members, "member"),
                plural(t.repos, "repository")
            );
            page.box_line(segs, vec![Seg::new(counts, Role::Meta)], 0);
            if !t.description.is_empty() {
                body(page, vec![Seg::new(t.description.clone(), Role::Meta)]);
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
        Seg::new(" / ", Role::Meta),
        Seg::new(t.name.clone(), Role::Title),
    ];
    if t.secret {
        title.push(space());
        title.push(chip("Secret", Bg::SecondaryContainer));
    }
    page.wrapped(title, 0, Frame::None);
    if !t.description.is_empty() {
        page.wrapped(
            vec![Seg::new(t.description.clone(), Role::Body)],
            0,
            Frame::None,
        );
    }
    if let Some(parent) = &d.parent {
        let target = format!("{}/orgs/{org}/teams/{}", url::base(), parent.slug);
        let link = link_seg(page, parent.name.clone(), target, Role::Link);
        page.wrapped(vec![Seg::new("Part of ", Role::Meta), link], 0, Frame::None);
    }
    page.blank();
    let title = vec![Seg::new(
        format!("Members  {}", count_of(&d.members)),
        Role::Strong,
    )];
    list_box(page, title, Vec::new(), &d.members, "No members.", user_row);
    page.blank();
    let title = vec![Seg::new(
        format!("Repositories  {}", count_of(&d.repos)),
        Role::Strong,
    )];
    list_box(
        page,
        title,
        Vec::new(),
        &d.repos,
        "No repositories.",
        |page, r| {
            item(page, url::repo(&r.repo), |page, link| {
                let right = vec![Seg::new(format!("★ {}", compact(r.stars)), Role::Meta)];
                page.box_line(
                    vec![Seg::linked(r.repo.to_string(), Role::Link, link)],
                    right,
                    0,
                );
                if !r.description.is_empty() {
                    body(page, vec![Seg::new(r.description.clone(), Role::Meta)]);
                }
            });
        },
    );
    if !d.children.is_empty() {
        page.blank();
        let title = vec![Seg::new(
            format!("Child teams  {}", count_of(&d.children)),
            Role::Strong,
        )];
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
        title.push(Seg::new(" / ", Role::Meta));
    }
    title.push(Seg::new(first.to_owned(), Role::Title));
    if !g.public {
        title.push(space());
        title.push(chip("Secret", Bg::SecondaryContainer));
    }
    page.wrapped(title, 0, Frame::None);
    let meta = format!(
        "created {} · updated {} · {} · {}",
        time::ago_iso(&g.created_at, now),
        time::ago_iso(&g.updated_at, now),
        plural(g.files.len() as u64, "file"),
        plural(g.comments, "comment"),
    );
    page.wrapped(vec![Seg::new(meta, Role::Meta)], 0, Frame::None);
    if !g.description.is_empty() {
        page.wrapped(
            vec![Seg::new(g.description.clone(), Role::Body)],
            0,
            Frame::None,
        );
    }
    for f in &g.files {
        page.blank();
        let mut info = vec![Seg::new(f.name.clone(), Role::Strong)];
        let mut facts = vec![crate::ui::text::size(f.size)];
        if let Some(language) = &f.language {
            facts.push(language.clone());
        }
        info.push(Seg::new(format!("  {}", facts.join(" · ")), Role::Meta));
        page.box_top(info, Vec::new());
        match &f.text {
            Some(text) if is_markdown(&f.name) => {
                markdown::render(page, text, None, Frame::Body);
            }
            Some(text) => {
                let lines = markdown::highlighted(text, crate::diff::Language::from_path(&f.name));
                let width = lines.len().to_string().len();
                for (i, mut segs) in lines.into_iter().enumerate() {
                    segs.insert(
                        0,
                        Seg::new(
                            format!("{:>width$}  ", i + 1),
                            Role::Syntax(Syntax::Comment),
                        ),
                    );
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
            body(
                page,
                vec![Seg::new("Only the beginning is shown.", Role::Meta)],
            );
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
                let right = vec![Seg::new(time::ago_iso(&g.updated_at, now), Role::Meta)];
                page.box_line(vec![Seg::linked(name, Role::Strong, link)], right, 0);
                if !g.description.is_empty() {
                    body(page, vec![Seg::new(g.description.clone(), Role::Body)]);
                }
                let meta = format!(
                    "{} · {} · ★ {}",
                    plural(g.files.len() as u64, "file"),
                    plural(g.comments, "comment"),
                    compact(g.stars)
                );
                body(page, vec![Seg::new(meta, Role::Meta)]);
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
    People(Fetched<'a, Results<UserSummary>>),
    /// The overview's are in the profile.
    None,
}

pub fn profile(page: &mut Page, p: &Profile, tab: ProfileTab, list: ProfileList<'_>, now: u64) {
    let mut segs = Vec::new();
    match &p.name {
        Some(name) => {
            segs.push(Seg::new(name.clone(), Role::Title));
            segs.push(Seg::new(format!("  {}", p.login), Role::Meta));
        }
        None => segs.push(Seg::new(p.login.clone(), Role::Title)),
    }
    if p.is_org {
        segs.push(space());
        segs.push(chip("Organization", Bg::SecondaryContainer));
    }
    if p.verified {
        segs.push(space());
        segs.push(chip("✓ Verified", Bg::SuccessContainer));
    }
    page.line(segs);
    if let Some(status) = &p.status {
        page.wrapped(vec![Seg::new(status.clone(), Role::Meta)], 0, Frame::None);
    }
    if let Some(bio) = &p.bio {
        page.wrapped(vec![Seg::new(bio.clone(), Role::Body)], 0, Frame::None);
    }
    let mut facts = Vec::new();
    if let (Some(followers), Some(following)) = (p.followers, p.following) {
        facts.push(Seg::new(
            format!(
                "⚇ {} · {} following",
                plural(followers, "follower"),
                compact(following)
            ),
            Role::Meta,
        ));
    }
    // Spaced apart, with nothing before the first.
    let gap = |segs: &Vec<Seg>| if segs.is_empty() { "" } else { "   " };
    for (icon, value) in [("◆ ", &p.company), ("⌖ ", &p.location), ("", &p.pronouns)] {
        if let Some(v) = value {
            facts.push(Seg::new(format!("{}{icon}{v}", gap(&facts)), Role::Meta));
        }
    }
    if let Some(site) = &p.website {
        facts.push(Seg::new(format!("{}↗ ", gap(&facts)), Role::Meta));
        facts.push(link_seg(page, site.clone(), site.clone(), Role::Link));
    }
    if !facts.is_empty() {
        page.wrapped(facts, 0, Frame::None);
    }
    let mut links = Vec::new();
    for (shown, target) in &p.socials {
        links.push(Seg::new(format!("{}↗ ", gap(&links)), Role::Meta));
        links.push(link_seg(page, shown.clone(), target.clone(), Role::Link));
    }
    if !links.is_empty() {
        page.wrapped(links, 0, Frame::None);
    }
    if !p.orgs.is_empty() {
        let mut orgs = vec![Seg::new("Organizations  ", Role::Meta)];
        for (i, org) in p.orgs.iter().enumerate() {
            if i > 0 {
                orgs.push(Seg::new(" · ", Role::Meta));
            }
            orgs.push(login_seg(page, org, Role::Link));
        }
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
        let title = Seg::new(format!("{repo} / {path}"), Role::Meta);
        page.box_top(vec![title], Vec::new());
        if let Some(repo) = RepoId::parse(&repo) {
            let base = LinkBase::new(&repo, "HEAD", path);
            markdown::render(page, readme, Some(&base), Frame::Body);
        }
        page.box_bottom();
    }
    page.blank();
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
                    let sort = Seg::new("Recently starred", Role::Meta);
                    let title = format!("Starred  {}", compact(p.star_count));
                    (title, sort, true, "Nothing starred yet.")
                }
            };
            if let Some(r) = repos.show(page, "repositories") {
                let title = vec![Seg::new(title, Role::Strong)];
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
            people_list(page, title, people);
            return;
        }
        (_, ProfileList::None) => return,
    }
    // The overview: pinned (or popular) repositories, then the rest.
    let (title, repos, show_owner) = if p.pinned.is_empty() {
        let mut popular = p.repos.items.clone();
        popular.sort_by_key(|r| std::cmp::Reverse(r.stars));
        popular.truncate(6);
        ("Popular repositories", popular, false)
    } else {
        ("Pinned", p.pinned.clone(), true)
    };
    let title = vec![Seg::new(title, Role::Strong)];
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
            Seg::new("People", Role::Strong),
            Seg::new(format!("  {}", compact(p.people.total())), Role::Meta),
        ];
        page.box_top(title, Vec::new());
        if p.people.is_empty() {
            empty_row(page, "No public members.");
        } else {
            let mut people = Vec::new();
            for (i, login) in p.people.iter().enumerate() {
                if i > 0 {
                    people.push(Seg::new(" · ", Role::Meta));
                }
                people.push(login_seg(page, login, Role::Link));
            }
            page.wrapped(people, 0, Frame::Body);
        }
        // The rest are on the People tab.
        let all = format!("{}/orgs/{}/people", url::base(), p.login);
        rest_row(page, p.people.len(), At(&p.people, Some(all)).rest());
        page.box_bottom();
        let languages = top_languages(&p.repos);
        if !languages.is_empty() {
            page.blank();
            let mut segs = vec![Seg::new("Top languages  ", Role::Strong)];
            for (lang, n) in languages {
                segs.push(Seg::new("● ", Role::Accent));
                segs.push(Seg::new(format!("{lang} {n}   "), Role::Meta));
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
    let month: usize = date.get(5..7)?.parse().ok()?;
    MONTHS.get(month.checked_sub(1)?).copied()
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
    page.line(vec![Seg::new(title, Role::Strong)]);
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
    page.line(vec![Seg::new(months, Role::Meta)]);
    for day in 0..7 {
        let label = match day {
            1 => "Mon ",
            3 => "Wed ",
            5 => "Fri ",
            _ => "    ",
        };
        let mut segs = vec![Seg::new(label, Role::Meta)];
        for week in weeks {
            segs.push(match week.days.get(day) {
                Some(&level) => Seg::new("■", Role::Heat(level.min(4))),
                None => Seg::new(" ", Role::Meta),
            });
        }
        page.line(segs);
    }
    let mut legend = vec![Seg::new(format!("{}Less ", " ".repeat(LABELS)), Role::Meta)];
    legend.extend((0..=4).map(|level| Seg::new("■", Role::Heat(level))));
    legend.push(Seg::new(" More", Role::Meta));
    page.line(legend);
}

/// What someone did, month by month: commits by repository, and the pull
/// requests and issues they opened and reviewed.
/// "20+ pull requests" when GitHub may have more than `n`.
pub(super) fn at_least(n: u64, word: &str, short: bool) -> String {
    if short {
        format!("{n}+ {}", noun(2, word))
    } else {
        plural(n, word)
    }
}

pub(super) fn contribution_activity(page: &mut Page, c: &Contributions) {
    page.line(vec![Seg::new("Contribution activity", Role::Strong)]);
    if c.activity.is_empty() {
        page.line(vec![Seg::new("No recent activity.", Role::Meta)]);
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
                    Seg::new("    ", Role::Meta),
                    name,
                    Seg::new(format!("  {}", plural(*n, "commit")), Role::Meta),
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
                page.wrapped(
                    vec![Seg::new(format!("{place}  "), Role::Meta), link],
                    4,
                    Frame::None,
                );
            }
        }
    }
    let short = &c.short;
    if [&short.commits, &short.pulls, &short.issues, &short.reviews]
        .iter()
        .any(|s| s.is_some())
    {
        page.blank();
        page.line(vec![Seg::new(
            "Counts with + are at least that: GitHub has more",
            Role::Meta,
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
