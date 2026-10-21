//! A repository's security advisories, wiki, deployments and
//! milestones.

use super::*;
use std::fmt::Write as _;

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
            None => format!("{}/advisories/{}", url::base(), a.ghsa),
        };
        item(page, target, |page, link| {
            let segs = vec![
                severity_chip(a.severity),
                space(),
                Seg::linked(a.summary.clone(), Role::Strong, link),
            ];
            hanging(page, meta(""), segs, Frame::Body);
            let mut subline = a.ghsa.clone();
            if let Some(at) = &a.published_at {
                let _ = write!(subline, " · published {}", time::ago_iso(at, now));
            }
            let packages = unique(a.packages.iter().map(|p| &p.name));
            if !packages.is_empty() {
                let _ = write!(subline, " · {}", packages.join(", "));
            }
            body(page, vec![meta(subline)]);
        });
    });
}

/// A security advisory: what's affected and fixed, how severe, and its
/// description, credits and references.
pub fn advisory(page: &mut Page, a: &Advisory, now: u64) {
    say(page, a.summary.clone(), Role::Title);
    let mut segs = vec![severity_chip(a.severity)];
    if a.withdrawn_at.is_some() {
        tag(&mut segs, "Withdrawn", Bg::SecondaryContainer);
    }
    let mut subline = format!("  {}", a.ghsa);
    if let Some(cve) = &a.cve {
        let _ = write!(subline, " · {cve}");
    }
    if let Some(at) = &a.published_at {
        let _ = write!(subline, " · published {}", time::ago_iso(at, now));
    }
    if let Some(at) = &a.updated_at {
        let _ = write!(subline, " · updated {}", time::ago_iso(at, now));
    }
    segs.push(meta(subline));
    page.wrapped(segs, 0, Frame::None);
    if let Some((score, vector)) = &a.cvss {
        say(page, format!("CVSS {score}  {vector}"), Role::Meta);
    }
    if !a.cwes.is_empty() {
        say(page, a.cwes.join(" · "), Role::Meta);
    }
    page.blank();
    let title = vec![strong(format!("Affected packages  {}", a.packages.len()))];
    list_box(
        page,
        title,
        Vec::new(),
        &Whole(&a.packages),
        "None listed.",
        |page, p| {
            let right = vec![meta(p.ecosystem.clone())];
            page.box_line(vec![strong(p.name.clone())], right, 0);
            let facts = affected(p.vulnerable.as_ref(), p.patched.as_deref());
            body(page, vec![meta(facts)]);
        },
    );
    if !a.description.trim().is_empty() {
        page.blank();
        markdown::render(page, &a.description, None, Frame::None);
    }
    if !a.credits.is_empty() {
        page.blank();
        let mut segs = vec![strong("Credits  ")];
        segs.extend(login_list(page, ", ", &a.credits));
        page.wrapped(segs, 0, Frame::None);
    }
    if !a.references.is_empty() {
        page.blank();
    }
    references(page, &a.references);
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
        let _ = write!(out, "[{label}]({target})");
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
        say(page, title.clone(), Role::Title);
        page.rule(0, Frame::None);
        if w.markdown {
            markdown::render(page, &wiki_links(text), Some(&base), Frame::None);
        } else {
            for line in text.lines() {
                say(page, line.to_owned(), Role::Body);
            }
        }
        page.blank();
    }
    let title = vec![strong(format!("Pages  {}", w.pages.len()))];
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
                link_line(page, title.clone(), Role::Link, link, Vec::new());
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
        let all = ("All".to_owned(), base.clone(), environment.is_none());
        let envs = l.environments.iter().map(|env| {
            let on = environment == Some(env.as_str());
            (env.clone(), filter(env), on)
        });
        let tabs = std::iter::once(all).chain(envs);
        let mut segs = nav_strip(page, &meta(" · "), &Role::Link, tabs);
        more_chips(&mut segs, &l.environments);
        page.wrapped(segs, 0, Frame::None);
        page.blank();
    }
    let r = &l.results;
    let title = counted("Deployments", r);
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
                meta(format!("  {}", d.state)),
            ];
            let right = vec![meta(time::ago_iso(&d.created_at, now))];
            page.box_line(segs, right, 0);
            let mut subline = vec![sha_seg(page, repo, &d.oid, Role::Code)];
            if let Some(branch) = &d.branch {
                subline.push(meta(" · "));
                subline.push(branch_seg(page, repo, branch, Role::Code));
            }
            if let Some(who) = &d.creator {
                subline.push(meta(" · "));
                subline.push(login_seg(page, who, Role::Meta));
            }
            if let Some(at) = &d.environment_url {
                subline.push(meta(" · "));
                subline.push(link_seg(page, at.clone(), at.clone(), Role::Link));
            }
            body(page, subline);
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
        meta("▱".repeat(10 - filled)),
        meta(format!(
            "  {percent}% · {} open · {} closed",
            m.open, m.done
        )),
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
    let tabs = [
        (format!("Open {}", l.open), base.clone(), !closed),
        (
            format!("Closed {}", l.closed),
            format!("{base}?state=closed"),
            closed,
        ),
    ];
    let segs = nav_strip(page, &meta(" · "), &Role::Link, tabs);
    page.wrapped(segs, 0, Frame::None);
    page.blank();
    let r = &l.results;
    let title = counted("Milestones", r);
    list_box(page, title, Vec::new(), r, "No milestones.", |page, m| {
        let target = format!("{}/milestone/{}", url::repo(repo), m.number);
        item(page, target, |page, link| {
            link_line(
                page,
                m.title.clone(),
                Role::Strong,
                link,
                vec![meta(due(m))],
            );
            let mut segs = progress(m);
            segs.push(meta(format!(
                " · updated {}",
                time::ago_iso(&m.updated_at, now)
            )));
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
    say(page, m.title.clone(), Role::Title);
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
    page.wrapped(vec![state, meta(format!("  {when}"))], 0, Frame::None);
    page.wrapped(progress(m), 0, Frame::None);
    if !m.description.trim().is_empty() {
        page.blank();
        let base = LinkBase::new(repo, "HEAD", "");
        markdown::render(page, &m.description, Some(&base), Frame::None);
    }
    page.blank();
    let r = &d.items;
    let title = counted("Issues and pull requests", r);
    let empty = if d.unsearchable {
        "GitHub's search can't look up a title with quotes: open them on GitHub."
    } else {
        "Nothing in this milestone."
    };
    // Most recently updated first.
    list_box(page, title, Vec::new(), r, empty, |page, i| {
        issue_row(page, i, (false, true), icons, now, false);
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
