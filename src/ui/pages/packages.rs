//! GitHub Packages: an owner's or a repository's packages, and a
//! package's page: how to get a version, and its versions.

use super::*;
use crate::api::packages::{PackageDetail, PackageList, PackagesOf, install};
use std::fmt::Write as _;

fn visibility(v: &str) -> Option<Seg> {
    match v {
        "" | "public" => None,
        v => Some(chip(capitalized(v), Bg::SecondaryContainer)),
    }
}

/// An owner's packages, or a repository's.
pub fn packages(page: &mut Page, of: &PackagesOf, list: &PackageList, now: u64) {
    let title = vec![strong(format!("Packages  {}", list.items.len()))];
    let empty = match of {
        PackagesOf::Owner(_) => "No packages published.",
        PackagesOf::Repo(_) => "This repository hasn't published packages.",
    };
    list_box(
        page,
        title,
        Vec::new(),
        &Whole(&list.items),
        empty,
        |page, p| {
            item(page, url::package(&p.at, None), |page, link| {
                let mut segs = vec![
                    Seg::linked(p.at.name.clone(), Role::Link, link),
                    space(),
                    chip(p.at.kind.name(), Bg::ContainerHigh),
                ];
                if let Some(v) = visibility(&p.visibility) {
                    segs.extend([space(), v]);
                }
                page.box_line(segs, Vec::new(), 0);
                let mut subline = plural(p.versions, "version");
                if let Some(at) = &p.updated_at {
                    let _ = write!(subline, " · updated {}", time::ago_iso(at, now));
                }
                if let (Some(repo), PackagesOf::Owner(_)) = (&p.repo, of) {
                    let _ = write!(subline, " · {repo}");
                }
                body(page, vec![meta(subline)]);
            });
        },
    );
    if !list.cut.is_empty() {
        let kinds: Vec<&str> = list.cut.iter().map(|k| k.name()).collect();
        let text = format!(
            "Only the first 100 {} packages are listed.",
            kinds.join(", ")
        );
        say(page, text, Role::Meta);
    }
}

/// A package: what it is, how to get the version shown (`version`, else
/// the newest), and its versions.
pub fn package(
    page: &mut Page,
    d: &PackageDetail,
    version: Option<u64>,
    delete_key: &str,
    now: u64,
) {
    let p = &d.package;
    let mut title = vec![
        Seg::new(p.at.name.clone(), Role::Title),
        space(),
        chip(p.at.kind.name(), Bg::ContainerHigh),
    ];
    if let Some(v) = visibility(&p.visibility) {
        title.extend([space(), v]);
    }
    page.line(title);
    let mut subline = vec![meta("By "), login_seg(page, &p.at.owner, Role::Link)];
    if let Some(repo) = &p.repo {
        subline.push(meta(" · from "));
        let from = url::repo(repo);
        subline.push(link_seg(page, repo.to_string(), from, Role::Link));
    }
    subline.push(meta(format!(" · {}", plural(p.versions, "version"))));
    page.wrapped(subline, 0, Frame::None);
    page.blank();
    let shown = match version {
        Some(id) => d.versions.items.iter().find(|v| v.id == id),
        None => d.versions.items.first(),
    };
    if let Some(v) = shown {
        let name = v.tags.first().unwrap_or(&v.name);
        let title = vec![strong(format!("Install {name}"))];
        page.box_top(title, Vec::new());
        for line in install(&p.at, p.repo.as_ref(), v) {
            page.wrapped(vec![Seg::new(line, Role::Code)], 0, Frame::Body);
        }
        page.box_bottom();
        page.blank();
    } else if let Some(id) = version {
        let text = format!("Version {id} isn't among those loaded.");
        say(page, text, Role::Meta);
        page.blank();
    }
    let title = counted("Versions", &d.versions);
    let right = vec![meta(format!("{delete_key} deletes one"))];
    list_box(
        page,
        title,
        right,
        &d.versions,
        "No versions.",
        |page, v| {
            item(page, url::package(&p.at, Some(v.id)), |page, link| {
                let role = if shown.is_some_and(|s| s.id == v.id) {
                    Role::Strong
                } else {
                    Role::Link
                };
                let mut segs = vec![Seg::linked(v.name.clone(), role, link)];
                for t in &v.tags {
                    tag(&mut segs, t, Bg::TertiaryContainer);
                }
                page.box_line(segs, Vec::new(), 0);
                if let Some(at) = &v.created_at {
                    let subline = format!("published {}", time::ago_iso(at, now));
                    body(page, vec![meta(subline)]);
                }
            });
        },
    );
}
