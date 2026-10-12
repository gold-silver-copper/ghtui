//! A repository's security advisories, wiki, deployments and
//! milestones.

use super::*;

pub(super) fn severity_chip(severity: Severity) -> Seg {
    let (name, bg) = match severity {
        Severity::Critical => ("Critical", Bg::ErrorContainer),
        Severity::High => ("High", Bg::ErrorContainer),
        Severity::Medium => ("Medium", Bg::TertiaryContainer),
        Severity::Low => ("Low", Bg::SecondaryContainer),
        Severity::Unknown => ("Unknown", Bg::SecondaryContainer),
    };
    chip(name, bg)
}

/// A repository's security advisories, or GitHub's newest reviewed ones.
pub fn advisories(
    page: &mut Page,
    repo: Option<&RepoId>,
    list: Fetched<'_, Results<Advisory>>,
    now: u64,
) {
    let empty = if repo.is_some() {
        "No published advisories."
    } else {
        "No advisories."
    };
    paged_list(page, "Security advisories", list, empty, |page, a| {
        let target = match repo {
            Some(repo) => format!("{}/security/advisories/{}", url::repo(repo), a.ghsa),
            None => format!("{}/advisories/{}", url::BASE, a.ghsa),
        };
        item(page, target, |page, link| {
            let segs = vec![
                severity_chip(a.severity),
                space(),
                Seg::linked(a.summary.clone(), Role::Strong, link),
            ];
            hanging(page, Seg::new("", Role::Meta), segs, Frame::Body);
            let mut meta = a.ghsa.clone();
            if let Some(at) = &a.published_at {
                meta.push_str(&format!(" · published {}", time::ago_iso(at, now)));
            }
            let packages = unique(a.packages.iter().map(|p| &p.name));
            if !packages.is_empty() {
                meta.push_str(&format!(" · {}", packages.join(", ")));
            }
            body(page, vec![Seg::new(meta, Role::Meta)]);
        });
    });
}

/// A security advisory: what's affected and fixed, how severe, and its
/// description, credits and references.
pub fn advisory(page: &mut Page, a: &Advisory, now: u64) {
    page.wrapped(
        vec![Seg::new(a.summary.clone(), Role::Title)],
        0,
        Frame::None,
    );
    let mut segs = vec![severity_chip(a.severity)];
    if a.withdrawn_at.is_some() {
        segs.push(space());
        segs.push(chip("Withdrawn", Bg::SecondaryContainer));
    }
    let mut meta = format!("  {}", a.ghsa);
    if let Some(cve) = &a.cve {
        meta.push_str(&format!(" · {cve}"));
    }
    if let Some(at) = &a.published_at {
        meta.push_str(&format!(" · published {}", time::ago_iso(at, now)));
    }
    if let Some(at) = &a.updated_at {
        meta.push_str(&format!(" · updated {}", time::ago_iso(at, now)));
    }
    segs.push(Seg::new(meta, Role::Meta));
    page.wrapped(segs, 0, Frame::None);
    if let Some((score, vector)) = &a.cvss {
        page.wrapped(
            vec![Seg::new(format!("CVSS {score}  {vector}"), Role::Meta)],
            0,
            Frame::None,
        );
    }
    if !a.cwes.is_empty() {
        page.wrapped(
            vec![Seg::new(a.cwes.join(" · "), Role::Meta)],
            0,
            Frame::None,
        );
    }
    page.blank();
    let title = vec![Seg::new(
        format!("Affected packages  {}", a.packages.len()),
        Role::Strong,
    )];
    list_box(
        page,
        title,
        Vec::new(),
        &Whole(&a.packages),
        "None listed.",
        |page, p| {
            let right = vec![Seg::new(p.ecosystem.clone(), Role::Meta)];
            page.box_line(vec![Seg::new(p.name.clone(), Role::Strong)], right, 0);
            let mut facts = Vec::new();
            if let Some(v) = &p.vulnerable {
                facts.push(format!("Affected {v}"));
            }
            facts.push(format!(
                "Patched {}",
                p.patched.as_deref().unwrap_or("in no version yet")
            ));
            body(page, vec![Seg::new(facts.join(" · "), Role::Meta)]);
        },
    );
    if !a.description.trim().is_empty() {
        page.blank();
        markdown::render(page, &a.description, None, Frame::None);
    }
    if !a.credits.is_empty() {
        page.blank();
        let mut segs = vec![Seg::new("Credits  ", Role::Strong)];
        for (i, login) in a.credits.iter().enumerate() {
            if i > 0 {
                segs.push(Seg::new(", ", Role::Meta));
            }
            segs.push(login_seg(page, login, Role::Link));
        }
        page.wrapped(segs, 0, Frame::None);
    }
    if !a.references.is_empty() {
        page.blank();
        aside_heading(page, "References");
        for r in &a.references {
            let link = link_seg(page, r.clone(), r.clone(), Role::Link);
            page.wrapped(vec![link], 0, Frame::None);
        }
    }
}

/// A wiki's `[[Page]]` and `[[Text|Page]]` links as Markdown links.
pub(super) fn wiki_links(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some((before, after)) = rest.split_once("[[")
        && let Some((inner, tail)) = after.split_once("]]")
    {
        let (label, target) = inner.split_once('|').unwrap_or((inner, inner));
        out.push_str(before);
        let target = url::encode_path(&target.trim().replace(' ', "-"));
        out.push_str(&format!("[{label}]({target})"));
        rest = tail;
    }
    out.push_str(rest);
    out
}
/// A wiki page, then the wiki's pages and its sidebar. Without a page,
/// just the pages.
pub fn wiki(page: &mut Page, repo: &RepoId, w: &WikiPage) {
    let base = LinkBase::wiki(repo);
    if let (Some(title), Some(text)) = (&w.title, &w.text) {
        page.wrapped(vec![Seg::new(title.clone(), Role::Title)], 0, Frame::None);
        page.rule(0, Frame::None);
        if w.markdown {
            markdown::render(page, &wiki_links(text), Some(&base), Frame::None);
        } else {
            for line in text.lines() {
                page.wrapped(vec![Seg::new(line.to_owned(), Role::Body)], 0, Frame::None);
            }
        }
        page.blank();
    }
    let title = vec![Seg::new(format!("Pages  {}", w.pages.len()), Role::Strong)];
    list_box(
        page,
        title,
        Vec::new(),
        &Whole(&w.pages),
        "No pages yet.",
        |page, title| {
            let target = format!(
                "{}/wiki/{}",
                url::repo(repo),
                url::encode_path(&title.replace(' ', "-"))
            );
            item(page, target, |page, link| {
                page.box_line(
                    vec![Seg::linked(title.clone(), Role::Link, link)],
                    Vec::new(),
                    0,
                );
            });
        },
    );
    if let Some(sidebar) = &w.sidebar {
        page.blank();
        markdown::render(page, &wiki_links(sidebar), Some(&base), Frame::None);
    }
}

/// A repository's deployments, newest first, with the environments to
/// filter by.
pub fn deployments(
    page: &mut Page,
    repo: &RepoId,
    list: Fetched<'_, DeploymentList>,
    environment: Option<&str>,
    now: u64,
) {
    let Some(l) = list.show(page, "deployments") else {
        return;
    };
    let base = format!("{}/deployments", url::repo(repo));
    let filter = |env: &str| {
        format!(
            "{base}/activity_log?environments_filter={}",
            url::encode(env)
        )
    };
    if !l.environments.is_empty() {
        let role = |on: bool| if on { Role::Strong } else { Role::Link };
        let mut segs = vec![link_seg(
            page,
            "All",
            base.clone(),
            role(environment.is_none()),
        )];
        for env in &l.environments {
            segs.push(Seg::new(" · ", Role::Meta));
            segs.push(link_seg(
                page,
                env.clone(),
                filter(env),
                role(environment == Some(env.as_str())),
            ));
        }
        more_chips(&mut segs, &l.environments);
        page.wrapped(segs, 0, Frame::None);
        page.blank();
    }
    let r = &l.results;
    let title = vec![Seg::new(
        format!("Deployments  {}", count_of(r)),
        Role::Strong,
    )];
    list_box(page, title, Vec::new(), r, "No deployments.", |page, d| {
        let target = d
            .log_url
            .clone()
            .unwrap_or_else(|| url::commit(repo, &d.oid));
        item(page, target, |page, link| {
            let (mark, role) = outcome_mark(d.outcome);
            let segs = vec![
                Seg::new(format!("{mark} "), role),
                Seg::linked(d.environment.clone(), Role::Strong, link),
                Seg::new(format!("  {}", d.state), Role::Meta),
            ];
            let right = vec![Seg::new(time::ago_iso(&d.created_at, now), Role::Meta)];
            page.box_line(segs, right, 0);
            let sha = crate::ui::text::short_sha(&d.oid);
            let mut meta = vec![link_seg(page, sha, url::commit(repo, &d.oid), Role::Code)];
            if let Some(branch) = &d.branch {
                meta.push(Seg::new(" · ", Role::Meta));
                meta.push(link_seg(
                    page,
                    branch.clone(),
                    url::tree(repo, branch, ""),
                    Role::Code,
                ));
            }
            if let Some(who) = &d.creator {
                meta.push(Seg::new(" · ", Role::Meta));
                meta.push(login_seg(page, who, Role::Meta));
            }
            if let Some(at) = &d.environment_url {
                meta.push(Seg::new(" · ", Role::Meta));
                meta.push(link_seg(page, at.clone(), at.clone(), Role::Link));
            }
            body(page, meta);
        });
    });
}

/// How far along a milestone is: a bar, the share done, and the counts.
pub(super) fn progress(m: &MilestoneInfo) -> Vec<Seg> {
    let all = m.open + m.done;
    let percent = (m.done * 100).checked_div(all).unwrap_or(0);
    let filled = usize::try_from(percent / 10).unwrap_or(0);
    vec![
        Seg::new("▰".repeat(filled), Role::Success),
        Seg::new("▱".repeat(10 - filled), Role::Meta),
        Seg::new(
            format!("  {percent}% · {} open · {} closed", m.open, m.done),
            Role::Meta,
        ),
    ]
}

/// When a milestone is due, or was closed.
pub(super) fn due(m: &MilestoneInfo) -> String {
    match (&m.closed_at, &m.due_on) {
        (Some(at), _) if m.closed => format!("Closed {}", day(at)),
        (_, Some(at)) => format!("Due by {}", day(at)),
        _ => "No due date".to_owned(),
    }
}

/// A repository's open or closed milestones.
pub fn milestones(
    page: &mut Page,
    repo: &RepoId,
    list: Fetched<'_, MilestoneList>,
    closed: bool,
    now: u64,
) {
    let Some(l) = list.show(page, "milestones") else {
        return;
    };
    let base = format!("{}/milestones", url::repo(repo));
    let role = |on: bool| if on { Role::Strong } else { Role::Link };
    let open = link_seg(
        page,
        format!("Open {}", l.open),
        base.clone(),
        role(!closed),
    );
    let shut = link_seg(
        page,
        format!("Closed {}", l.closed),
        format!("{base}?state=closed"),
        role(closed),
    );
    page.wrapped(
        vec![open, Seg::new(" · ", Role::Meta), shut],
        0,
        Frame::None,
    );
    page.blank();
    let r = &l.results;
    let title = vec![Seg::new(
        format!("Milestones  {}", count_of(r)),
        Role::Strong,
    )];
    list_box(page, title, Vec::new(), r, "No milestones.", |page, m| {
        let target = format!("{}/milestone/{}", url::repo(repo), m.number);
        item(page, target, |page, link| {
            let right = vec![Seg::new(due(m), Role::Meta)];
            page.box_line(
                vec![Seg::linked(m.title.clone(), Role::Strong, link)],
                right,
                0,
            );
            let mut segs = progress(m);
            segs.push(Seg::new(
                format!(" · updated {}", time::ago_iso(&m.updated_at, now)),
                Role::Meta,
            ));
            body(page, segs);
            if let Some(first) = m.description.lines().find(|l| !l.trim().is_empty()) {
                body(page, vec![Seg::new(first.trim().to_owned(), Role::Body)]);
            }
        });
    });
}

/// A milestone: how far along it is, its description, and its issues and
/// pull requests.
pub fn milestone(page: &mut Page, repo: &RepoId, d: &MilestoneDetail, icons: Icons, now: u64) {
    let m = &d.info;
    page.wrapped(vec![Seg::new(m.title.clone(), Role::Title)], 0, Frame::None);
    let state = if m.closed {
        chip("Closed", Bg::TertiaryContainer)
    } else {
        chip("Open", Bg::SuccessContainer)
    };
    // The chip says it's closed; the date says when.
    let when = match &m.closed_at {
        Some(at) if m.closed => day(at),
        _ => due(m),
    };
    page.wrapped(
        vec![state, Seg::new(format!("  {when}"), Role::Meta)],
        0,
        Frame::None,
    );
    page.wrapped(progress(m), 0, Frame::None);
    if !m.description.trim().is_empty() {
        page.blank();
        let base = LinkBase::new(repo, "HEAD", "");
        markdown::render(page, &m.description, Some(&base), Frame::None);
    }
    page.blank();
    let r = &d.items;
    let title = vec![Seg::new(
        format!("Issues and pull requests  {}", count_of(r)),
        Role::Strong,
    )];
    let empty = if d.unsearchable {
        "GitHub's search can't look up a title with quotes: open them on GitHub."
    } else {
        "Nothing in this milestone."
    };
    // Most recently updated first.
    list_box(page, title, Vec::new(), r, empty, |page, i| {
        issue_row(page, i, (false, true), icons, now);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wiki_links_become_markdown() {
        assert_eq!(wiki_links("See [[FAQ]]."), "See [FAQ](FAQ).");
        assert_eq!(
            wiki_links("[[Start here|Getting started]]"),
            "[Start here](Getting-started)"
        );
        assert_eq!(wiki_links("[[unclosed"), "[[unclosed");
        assert_eq!(wiki_links("[[C# tips]]"), "[C# tips](C%23-tips)");
    }
}
