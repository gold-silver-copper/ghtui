//! GitHub Packages: an owner's or a repository's packages, and a
//! package's page: how to get a version, and its versions.

use super::*;
use crate::api::packages::{PackageDetail, PackageList, PackagesOf, install};
use crate::route::Route;
use crate::site::Page as Site;

fn visibility(v: &str) -> Option<Seg> {
    match v {
        "" | "public" => None,
        v => {
            let mut v = v.to_owned();
            if let Some(first) = v.get_mut(..1) {
                first.make_ascii_uppercase();
            }
            Some(chip(v, Bg::SecondaryContainer))
        }
    }
}

/// An owner's packages, or a repository's.
pub fn packages(page: &mut Page, of: &PackagesOf, list: &PackageList, now: u64) {
    let title = vec![Seg::new(
        format!("Packages  {}", list.items.len()),
        Role::Strong,
    )];
    let empty = match of {
        PackagesOf::Owner(_) => "No packages published.",
        PackagesOf::Repo(_) => "This repository hasn't published packages.",
    };
    list_box(page, title, Vec::new(), &Whole(&list.items), empty, |page, p| {
        let target = Route::Site(Site::Package {
            at: p.at.clone(),
            version: None,
        });
        item(page, target.url(), |page, link| {
            let mut segs = vec![
                Seg::linked(p.at.name.clone(), Role::Link, link),
                space(),
                chip(p.at.kind.name(), Bg::ContainerHigh),
            ];
            if let Some(v) = visibility(&p.visibility) {
                segs.extend([space(), v]);
            }
            page.box_line(segs, Vec::new(), 0);
            let mut meta = plural(p.versions, "version");
            if let Some(at) = &p.updated_at {
                meta.push_str(&format!(" · updated {}", time::ago_iso(at, now)));
            }
            if let (Some(repo), PackagesOf::Owner(_)) = (&p.repo, of) {
                meta.push_str(&format!(" · {repo}"));
            }
            body(page, vec![Seg::new(meta, Role::Meta)]);
        });
    });
    if !list.cut.is_empty() {
        let kinds: Vec<&str> = list.cut.iter().map(|k| k.name()).collect();
        let text = format!(
            "Only the first 100 {} packages are listed.",
            kinds.join(", ")
        );
        page.wrapped(vec![Seg::new(text, Role::Meta)], 0, Frame::None);
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
    let mut meta = vec![
        Seg::new("By ", Role::Meta),
        login_seg(page, &p.at.owner, Role::Link),
    ];
    if let Some(repo) = &p.repo {
        meta.push(Seg::new(" · from ", Role::Meta));
        meta.push(link_seg(page, repo.to_string(), url::repo(repo), Role::Link));
    }
    meta.push(Seg::new(
        format!(" · {}", plural(p.versions, "version")),
        Role::Meta,
    ));
    page.wrapped(meta, 0, Frame::None);
    page.blank();
    let shown = match version {
        Some(id) => d.versions.items.iter().find(|v| v.id == id),
        None => d.versions.items.first(),
    };
    if let Some(v) = shown {
        let title = vec![Seg::new(format!("Install {}", v.name), Role::Strong)];
        page.box_top(title, Vec::new());
        for line in install(&p.at, p.repo.as_ref(), v) {
            page.wrapped(vec![Seg::new(line, Role::Code)], 0, Frame::Body);
        }
        page.box_bottom();
        page.blank();
    } else if let Some(id) = version {
        let text = format!("Version {id} isn't among those loaded.");
        page.wrapped(vec![Seg::new(text, Role::Meta)], 0, Frame::None);
        page.blank();
    }
    let title = vec![Seg::new(
        format!("Versions  {}", count_of(&d.versions)),
        Role::Strong,
    )];
    let right = vec![Seg::new(format!("{delete_key} deletes one"), Role::Meta)];
    list_box(page, title, right, &d.versions, "No versions.", |page, v| {
        let target = Route::Site(Site::Package {
            at: p.at.clone(),
            version: Some(v.id),
        });
        item(page, target.url(), |page, link| {
            let role = if shown.is_some_and(|s| s.id == v.id) {
                Role::Strong
            } else {
                Role::Link
            };
            let mut segs = vec![Seg::linked(v.name.clone(), role, link)];
            for tag in &v.tags {
                segs.extend([space(), chip(tag, Bg::TertiaryContainer)]);
            }
            page.box_line(segs, Vec::new(), 0);
            if let Some(at) = &v.created_at {
                let meta = format!("published {}", time::ago_iso(at, now));
                body(page, vec![Seg::new(meta, Role::Meta)]);
            }
        });
    });
}
