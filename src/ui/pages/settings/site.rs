//! A repository's GitHub Pages site and its custom properties.

use super::*;
use crate::api::settings::PropValue;
use crate::api::settings::pages::{Pages, PagesBuild};
use crate::api::settings::properties::Properties;

/// Where a site is built from, said.
pub(crate) fn source(f: &crate::api::settings::PagesForm) -> String {
    if f.workflow {
        "GitHub Actions".into()
    } else {
        format!("the {} branch, {}", f.branch, f.path)
    }
}

pub(super) fn pages(page: &mut Page, p: &Pages, keys: SettingKeys<'_>, now: u64) {
    let Some(site) = &p.site else {
        let right = add_button(page, "Publish a site", keys);
        page.box_top(vec![strong("GitHub Pages")], right);
        let how = format!(
            "No site yet. {} publishes one, from a branch or by a workflow.",
            keys.add
        );
        empty_row(page, &how);
        page.box_bottom();
        return;
    };
    page.box_top(vec![strong("GitHub Pages")], Vec::new());
    let https = match (site.form.https, site.certificate.as_deref()) {
        (true, _) => "enforced".to_owned(),
        (false, Some(state)) => format!("not enforced · certificate {state}"),
        (false, None) => "not enforced".to_owned(),
    };
    let domain = if site.form.domain.is_empty() {
        "none".to_owned()
    } else {
        site.form.domain.clone()
    };
    for (name, value) in [
        ("Site", site.url.clone().unwrap_or_default()),
        (
            "Status",
            site.status
                .clone()
                .unwrap_or_else(|| "not built yet".into()),
        ),
        ("Source", capitalized(&source(&site.form))),
        ("Custom domain", domain),
        ("HTTPS", https),
    ] {
        let name = meta(format!("{name:<14}  "));
        body(page, vec![name, Seg::new(value, Role::Body)]);
    }
    page.box_rule();
    let edit = vec![meta(keys.edit.to_owned())];
    let about = Some("Its source, custom domain and HTTPS".to_owned());
    setting_row(
        page,
        SettingLink::Site,
        "Change the site's settings",
        edit,
        about,
    );
    page.box_rule();
    let delete = vec![meta(keys.delete.to_owned())];
    let about = Some("It's taken down; its source stays as it is".to_owned());
    setting_row(
        page,
        SettingLink::Unpublish,
        "Unpublish the site",
        delete,
        about,
    );
    page.box_bottom();
    page.blank();
    match &p.builds {
        Err(why) => empty_box(page, strong("Latest builds"), &format!("{why}.")),
        Ok(builds) => {
            let none = "No builds from a branch: a workflow's are its runs, in Actions.";
            counted_box(
                page,
                "Latest builds",
                Vec::new(),
                builds,
                none,
                |page, b| {
                    build_row(page, b, now);
                },
            );
        }
    }
}

fn build_row(page: &mut Page, b: &PagesBuild, now: u64) {
    let (mark, role) = match b.status.as_str() {
        "built" => ("✓", Role::Success),
        "errored" => ("✗", Role::Error),
        _ => ("●", Role::Meta),
    };
    let mut about = format!(
        "{} · {}",
        short_sha(&b.commit),
        time::ago_iso(&b.created_at, now)
    );
    if let Some(by) = &b.pusher {
        about.push_str(&format!(" · pushed by {by}"));
    }
    if b.duration > 0 {
        about.push_str(&format!(" · {:.1}s", b.duration as f64 / 1000.0));
    }
    let segs = vec![
        Seg::new(format!("{mark} "), role),
        Seg::new(b.status.clone(), Role::Strong),
        meta(format!("  {about}")),
    ];
    page.box_line(segs, Vec::new(), 0);
    if let Some(error) = &b.error {
        body(page, vec![Seg::new(error.clone(), Role::Error)]);
    }
}

pub(super) fn properties(page: &mut Page, repo: &RepoId, p: &Properties, keys: SettingKeys<'_>) {
    match &p.schema {
        Ok(schema) => {
            let right = match schema.iter().any(|s| s.repo_may) {
                true => vec![meta(format!("{} sets them", keys.edit))],
                false => Vec::new(),
            };
            let none = format!("{} has no custom properties.", repo.owner);
            let schema = Whole(schema);
            let title = vec![strong(format!("Custom properties  {}", schema.0.len()))];
            list_box(page, title, right, &schema, &none, |page, prop| {
                let value = p.value(&prop.name).map(PropValue::text);
                let right = match value {
                    Some(v) => vec![Seg::new(v, Role::Body)],
                    None => vec![meta("unset")],
                };
                let mut about = vec![prop.kind.label().to_owned()];
                if prop.required {
                    about.push("required".into());
                }
                if !prop.repo_may {
                    about.push(format!("set by {}'s admins", repo.owner));
                }
                if let Some(d) = &prop.description {
                    about.push(d.clone());
                }
                let link = SettingLink::Property(prop.name.clone());
                setting_row(
                    page,
                    link,
                    prop.name.clone(),
                    right,
                    Some(about.join(" · ")),
                );
            });
        }
        Err(why) if p.values.is_empty() => {
            empty_box(page, strong("Custom properties"), &format!("{why}."));
        }
        Err(why) => {
            let set = Whole(&p.values);
            let title = vec![strong(format!("Custom properties  {}", p.values.len()))];
            list_box(
                page,
                title,
                Vec::new(),
                &set,
                "None set.",
                |page, (name, v)| {
                    let right = match v {
                        Some(v) => vec![Seg::new(v.text(), Role::Body)],
                        None => vec![meta("unset")],
                    };
                    let link = SettingLink::Property(name.clone());
                    setting_row(page, link, name.clone(), right, None);
                },
            );
            page.blank();
            say(page, format!("{why}."), Role::Meta);
        }
    }
}
