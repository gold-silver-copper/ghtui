//! The Security tab beyond advisories: Dependabot's, code scanning's and
//! secret scanning's alerts, as lists and one at a time.

use super::*;
use crate::api::security::{
    Alert, AlertKind, AlertList, AlertState, Detail, SecretLocation, Update,
};

/// The Security tab's own menu, as GitHub's sidebar: advisories, then
/// each kind of alert; `on` is the one shown (`None`: advisories).
pub fn security_nav(page: &mut Page, repo: &RepoId, on: Option<AlertKind>) {
    let base = format!("{}/security", url::repo(repo));
    let role = |here: bool| if here { Role::Strong } else { Role::Link };
    let mut segs = vec![link_seg(
        page,
        "Advisories",
        format!("{base}/advisories"),
        role(on.is_none()),
    )];
    for kind in AlertKind::ALL {
        segs.push(Seg::new(" · ", Role::Meta));
        let target = format!("{base}/{}", kind.slug());
        segs.push(link_seg(page, kind.name(), target, role(on == Some(kind))));
    }
    page.wrapped(segs, 0, Frame::None);
    page.blank();
}

/// An alert's severity: GitHub's rating, else a code scanning rule's
/// level.
fn severity(a: &Alert) -> Option<Seg> {
    match (a.severity, &a.level) {
        (Some(s), _) => Some(super::project::severity_chip(s)),
        (None, Some(level)) => {
            let bg = match level.as_str() {
                "error" => Bg::ErrorContainer,
                "warning" => Bg::TertiaryContainer,
                _ => Bg::SecondaryContainer,
            };
            let mut name = level.clone();
            if let Some(first) = name.get_mut(..1) {
                first.make_ascii_uppercase();
            }
            Some(chip(name, bg))
        }
        (None, None) => None,
    }
}

fn state_chip(state: AlertState) -> Seg {
    let bg = match state {
        AlertState::Open => Bg::SuccessContainer,
        AlertState::Fixed => Bg::PrimaryContainer,
        _ => Bg::SecondaryContainer,
    };
    chip(state.label(), bg)
}

/// A file at lines, linked to it at that commit (ghtui reads it there).
fn file_link(page: &mut Page, repo: &RepoId, path: &str, lines: (u32, u32), at: &str) -> Seg {
    let (start, end) = lines;
    let anchor = if start == end {
        format!("#L{start}")
    } else {
        format!("#L{start}-L{end}")
    };
    let target = format!("{}{anchor}", url::blob(repo, at, path));
    link_seg(page, format!("{path}:{start}"), target, Role::Link)
}

/// Where an alert is, said briefly for its row.
fn row_meta(a: &Alert, now: u64) -> String {
    let mut meta = format!("#{}", a.number);
    match &a.detail {
        Detail::Dependency(d) => {
            meta.push_str(&format!(" · {} ({})", d.package, d.ecosystem));
            if !d.manifest.is_empty() {
                meta.push_str(&format!(" · {}", d.manifest));
            }
        }
        Detail::Code(c) => {
            if let (Some(path), Some((line, _))) = (&c.path, c.lines) {
                meta.push_str(&format!(" · {path}:{line}"));
            }
            if !c.tool.is_empty() {
                meta.push_str(&format!(" · {}", c.tool));
            }
        }
        Detail::Secret(s) => {
            if let Some(SecretLocation::File { path, lines, .. }) = s.locations.first() {
                meta.push_str(&format!(" · {path}:{}", lines.0));
            }
        }
    }
    let when = match (&a.closing, &a.fixed_at) {
        (Some(c), _) => {
            c.at.as_deref()
                .map(|at| format!("closed {}", time::ago_iso(at, now)))
        }
        (None, Some(at)) => Some(format!("fixed {}", time::ago_iso(at, now))),
        (None, None) => Some(format!("opened {}", time::ago_iso(&a.created_at, now))),
    };
    if let Some(when) = when {
        meta.push_str(&format!(" · {when}"));
    }
    meta
}

/// A repository's alerts of one kind, filtered as GitHub's list is.
pub fn alerts(
    page: &mut Page,
    repo: &RepoId,
    kind: AlertKind,
    query: &str,
    list: Fetched<'_, AlertList>,
    cx: PageCtx<'_>,
) {
    let PageCtx { keys, now, .. } = cx;
    filter_field(page, query, keys, "filter");
    let state = list_state(query);
    let mut title = Vec::new();
    for (name, label) in [("open", "Open"), ("closed", "Closed")] {
        let role = if state == name {
            Role::Strong
        } else {
            Role::Meta
        };
        let link = page.link(Link::State(name.to_owned()));
        if !title.is_empty() {
            title.push(Seg::new("   ", Role::Body));
        }
        title.push(Seg::linked(label, role, link));
    }
    let Some(l) = list.show_or(page, &kind.alerts().to_lowercase(), |page| {
        page.box_top(title.clone(), Vec::new());
        skeleton(page);
        page.box_bottom();
    }) else {
        return;
    };
    let ignored = crate::api::security::filter(kind, query).ignored;
    if !ignored.is_empty() {
        page.wrapped(
            vec![Seg::new(
                format!(
                    "Not a filter of {}: {}",
                    kind.alerts().to_lowercase(),
                    ignored.join(" ")
                ),
                Role::Meta,
            )],
            0,
            Frame::None,
        );
        page.blank();
    }
    if let Some(off) = &l.off {
        let text = format!("{off}.");
        page.box_top(title, Vec::new());
        empty_row(page, &text);
        page.box_bottom();
        return;
    }
    let empty = match state {
        "open" => "No open alerts. 🎉",
        _ => "No alerts matched the filter.",
    };
    list_box(page, title, Vec::new(), &l.results, empty, |page, a| {
        let target = format!("{}/security/{}/{}", url::repo(repo), kind.slug(), a.number);
        item(page, target, |page, link| {
            let mut segs = Vec::new();
            if let Some(chip) = severity(a) {
                segs.push(chip);
                segs.push(space());
            }
            segs.push(Seg::linked(a.title.clone(), Role::Strong, link));
            let right = if a.state.is_open() {
                Vec::new()
            } else {
                vec![state_chip(a.state)]
            };
            page.box_line(segs, right, 0);
            body(page, vec![Seg::new(row_meta(a, now), Role::Meta)]);
        });
    });
}

/// One alert: what's wrong, where, how bad, what fixes it, and who
/// closed it and why.
pub fn alert(page: &mut Page, repo: &RepoId, a: &Alert, cx: PageCtx<'_>) {
    let now = cx.now;
    page.wrapped(vec![Seg::new(a.title.clone(), Role::Title)], 0, Frame::None);
    let mut segs = vec![state_chip(a.state)];
    if let Some(chip) = severity(a) {
        segs.push(space());
        segs.push(chip);
    }
    let mut meta = format!(
        "  {} alert #{} · opened {}",
        a.kind().name(),
        a.number,
        time::ago_iso(&a.created_at, now)
    );
    if let Some(at) = &a.fixed_at {
        meta.push_str(&format!(" · fixed {}", time::ago_iso(at, now)));
    }
    segs.push(Seg::new(meta, Role::Meta));
    page.wrapped(segs, 0, Frame::None);
    if let Some(c) = &a.closing {
        let mut segs = vec![Seg::new("Closed", Role::Strong)];
        if let Some(by) = &c.by {
            segs.push(Seg::new(" by ", Role::Meta));
            segs.push(login_seg(page, by, Role::Link));
        }
        if let Some(at) = &c.at {
            segs.push(Seg::new(format!(" {}", time::ago_iso(at, now)), Role::Meta));
        }
        if let Some(reason) = a.reason_label() {
            segs.push(Seg::new(format!(" · {reason}"), Role::Meta));
        }
        page.wrapped(segs, 0, Frame::None);
        if let Some(comment) = &c.comment {
            page.wrapped(vec![Seg::new(comment.clone(), Role::Body)], 2, Frame::None);
        }
    }
    page.blank();
    match &a.detail {
        Detail::Dependency(d) => dependency(page, repo, d),
        Detail::Code(c) => code(page, repo, c),
        Detail::Secret(s) => secret(page, repo, s),
    }
}

fn dependency(page: &mut Page, repo: &RepoId, d: &crate::api::security::Dependency) {
    let title = vec![Seg::new(
        format!("{} ({})", d.package, d.ecosystem),
        Role::Strong,
    )];
    page.box_top(title, Vec::new());
    let mut facts = Vec::new();
    if let Some(v) = &d.vulnerable {
        facts.push(format!("Affected {v}"));
    }
    facts.push(format!(
        "Patched {}",
        d.patched.as_deref().unwrap_or("in no version yet")
    ));
    body(page, vec![Seg::new(facts.join(" · "), Role::Body)]);
    let mut where_ = Vec::new();
    if !d.manifest.is_empty() {
        let target = url::blob(repo, "HEAD", &d.manifest);
        where_.push(link_seg(page, d.manifest.clone(), target, Role::Link));
    }
    let mut about: Vec<&str> = Vec::new();
    if let Some(scope) = &d.scope {
        about.push(scope);
    }
    if let Some(r) = &d.relationship {
        about.push(r);
    }
    if !about.is_empty() {
        where_.push(Seg::new(format!(" · {}", about.join(", ")), Role::Meta));
    }
    if !where_.is_empty() {
        body(page, where_);
    }
    match &d.update {
        Some(Update::Pr {
            number,
            title,
            state,
        }) => {
            let pr = PrRef {
                repo: repo.clone(),
                number: *number,
            };
            let state = state.to_lowercase();
            let link = link_seg(
                page,
                format!("#{number} {title}"),
                url::pull(&pr),
                Role::Link,
            );
            body(
                page,
                vec![
                    Seg::new("Dependabot's pull request ", Role::Meta),
                    link,
                    Seg::new(format!(" · {state}"), Role::Meta),
                ],
            );
        }
        Some(Update::Failed { title, body: why }) => {
            body(
                page,
                vec![Seg::new(
                    format!("Dependabot couldn't open a pull request: {title}"),
                    Role::Meta,
                )],
            );
            if let Some(first) = why.lines().find(|l| !l.trim().is_empty()) {
                body(page, vec![Seg::new(first.trim().to_owned(), Role::Meta)]);
            }
        }
        None => {}
    }
    page.box_bottom();
    page.blank();
    let mut ids = vec![link_seg(
        page,
        d.ghsa.clone(),
        format!("{}/advisories/{}", url::base(), d.ghsa),
        Role::Link,
    )];
    if let Some(cve) = &d.cve {
        ids.push(Seg::new(format!(" · {cve}"), Role::Meta));
    }
    if !d.cwes.is_empty() {
        ids.push(Seg::new(format!(" · {}", d.cwes.join(" · ")), Role::Meta));
    }
    page.wrapped(ids, 0, Frame::None);
    if !d.description.trim().is_empty() {
        page.blank();
        markdown::render(page, &d.description, None, Frame::None);
    }
    if !d.references.is_empty() {
        aside_heading(page, "References");
        for r in &d.references {
            let link = link_seg(page, r.clone(), r.clone(), Role::Link);
            page.wrapped(vec![link], 0, Frame::None);
        }
    }
}

fn code(page: &mut Page, repo: &RepoId, c: &crate::api::security::CodeAlert) {
    let mut title = vec![Seg::new(c.rule.clone(), Role::Strong)];
    if !c.tool.is_empty() {
        title.push(Seg::new(format!("  {}", c.tool), Role::Meta));
    }
    page.box_top(title, Vec::new());
    if !c.message.is_empty() {
        page.box_line(vec![Seg::new(c.message.clone(), Role::Body)], Vec::new(), 0);
    }
    match (&c.path, c.lines, &c.commit) {
        (Some(path), Some(lines), Some(at)) => {
            let mut segs = vec![file_link(page, repo, path, lines, at)];
            let on = c
                .git_ref
                .as_deref()
                .map(|r| r.strip_prefix("refs/heads/").unwrap_or(r))
                .map_or_else(String::new, |r| format!(" on {r}"));
            segs.push(Seg::new(
                format!("{on} at {}", crate::ui::text::short_sha(at)),
                Role::Meta,
            ));
            body(page, segs);
        }
        (Some(path), ..) => body(page, vec![Seg::new(path.clone(), Role::Meta)]),
        _ => {}
    }
    if !c.tags.is_empty() {
        body(page, vec![Seg::new(c.tags.join(" · "), Role::Meta)]);
    }
    page.box_bottom();
    if !c.rule_description.is_empty() {
        page.blank();
        page.wrapped(
            vec![Seg::new(c.rule_description.clone(), Role::Body)],
            0,
            Frame::None,
        );
    }
    if !c.help.trim().is_empty() {
        page.blank();
        markdown::render(page, &c.help, None, Frame::None);
    }
}

fn secret(page: &mut Page, repo: &RepoId, s: &crate::api::security::SecretAlert) {
    let title = vec![Seg::new(s.secret_type.clone(), Role::Strong)];
    let right = match s.validity.as_deref() {
        Some("active") => vec![chip("Active", Bg::ErrorContainer)],
        Some("inactive") => vec![chip("Inactive", Bg::SecondaryContainer)],
        _ => Vec::new(),
    };
    page.box_top(title, right);
    let mut facts = Vec::new();
    if s.publicly_leaked {
        facts.push("Publicly leaked".to_owned());
    }
    if let Some(by) = &s.bypassed_by {
        facts.push(format!("Pushed past push protection by @{by}"));
    }
    if !facts.is_empty() {
        body(page, vec![Seg::new(facts.join(" · "), Role::Meta)]);
    }
    if s.locations.is_empty() {
        empty_row(page, "No locations listed.");
    }
    for l in &s.locations {
        match l {
            SecretLocation::File {
                path,
                lines,
                commit,
            } => {
                let segs = vec![
                    file_link(page, repo, path, *lines, commit),
                    Seg::new(
                        format!(" at {}", crate::ui::text::short_sha(commit)),
                        Role::Meta,
                    ),
                ];
                body(page, segs);
            }
            SecretLocation::Other(what) => {
                body(
                    page,
                    vec![Seg::new(format!("Found in: {what}"), Role::Meta)],
                );
            }
        }
    }
    page.box_bottom();
    page.blank();
    page.wrapped(
        vec![Seg::new(
            "The secret itself isn't shown: revoke it where it was issued, then close the alert as revoked.",
            Role::Meta,
        )],
        0,
        Frame::None,
    );
}
