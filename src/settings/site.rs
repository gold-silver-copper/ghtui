//! A repository's GitHub Pages site (its source, custom domain and HTTPS,
//! written as a form) and its custom properties (each property's value,
//! a line of a form, checked against what the property allows).

use super::{Asks, Form, ask, differences, flag, read, text_of, write, yes};
use crate::act::Step;
use crate::api::model::RepoId;
use crate::api::settings::properties::{PropKind, Property};
use crate::api::settings::{
    PagesChange, PagesForm, PropValue, Properties, PropertyValues, Settings, SettingsChange,
    SettingsPage,
};
use crate::keymap::Action;
use crate::state::State;
use crate::ui::pages::SettingLink;

/// What a key does on the Pages or Custom properties page, or `None` on
/// another.
pub fn plan(
    state: &State,
    action: Action,
    repo: &RepoId,
    settings: &Settings,
    row: Option<&SettingLink>,
) -> Option<Result<Step, String>> {
    Some(match settings {
        Settings::Pages(p) => pages(state, action, repo, p.site.as_ref()),
        Settings::Properties(p) => properties(action, repo, p, row),
        _ => return None,
    })
}

fn pages(
    state: &State,
    action: Action,
    repo: &RepoId,
    site: Option<&crate::api::settings::pages::Site>,
) -> Result<Step, String> {
    match (action, site) {
        (Action::AddSetting, None) => {
            let form = Form::Pages {
                repo: repo.clone(),
                from: None,
            };
            write(form, pages_text(&PagesForm::default()))
        }
        (Action::AddSetting, Some(_)) => {
            let edit = state.first_key(Action::EditSetting);
            Err(format!("It has a site: {edit} changes it"))
        }
        (Action::EditSetting | Action::DeleteSetting, None) => {
            let add = state.first_key(Action::AddSetting);
            Err(format!("It has no site: {add} publishes one"))
        }
        (Action::EditSetting, Some(site)) => {
            let form = Form::Pages {
                repo: repo.clone(),
                from: Some(Box::new(site.form.clone())),
            };
            write(form, pages_text(&site.form))
        }
        (Action::DeleteSetting, Some(site)) => {
            let url = site.url.clone().unwrap_or_else(|| "The site".into());
            let facts = vec![
                (format!("{url} stops being served"), true),
                ("Its source branch or workflow stays".into(), false),
            ];
            let change = SettingsChange::Pages(PagesChange::Delete { repo: repo.clone() });
            let at = (repo, SettingsPage::Pages);
            let title = format!("Unpublish {repo}'s site?");
            Ok(ask(action, at, title, facts, ("Unpublish", change)))
        }
        _ => Err("A site is published, changed or unpublished, not turned off".into()),
    }
}

const SOURCE: &str = "Source";
const BRANCH: &str = "Branch";
const FOLDER: &str = "Folder";
const DOMAIN: &str = "Custom domain";
const HTTPS: &str = "Enforce HTTPS";

fn pages_lines(f: &PagesForm) -> Vec<(&'static str, String)> {
    let source = if f.workflow { "actions" } else { "branch" };
    vec![
        (SOURCE, source.into()),
        (BRANCH, f.branch.clone()),
        (FOLDER, f.path.clone()),
        (DOMAIN, f.domain.clone()),
        (HTTPS, yes(f.https).into()),
    ]
}

pub fn pages_text(f: &PagesForm) -> String {
    text_of(&pages_lines(f))
}

/// A Pages form from its text; what isn't written stays as in `base`.
pub fn parse_pages(text: &str, base: &PagesForm) -> Result<PagesForm, String> {
    let get = read(text, pages_lines(base))?;
    let workflow = match get(SOURCE).to_ascii_lowercase().as_str() {
        "branch" => false,
        "actions" | "github actions" | "workflow" => true,
        other => return Err(format!("{SOURCE}: branch or actions, not “{other}”")),
    };
    let branch = get(BRANCH);
    if !workflow && branch.is_empty() {
        return Err(format!("{BRANCH}: the branch it's built from"));
    }
    let path = get(FOLDER);
    if !workflow && !matches!(path.as_str(), "/" | "/docs") {
        return Err(format!("{FOLDER}: / or /docs, not “{path}”"));
    }
    let domain = get(DOMAIN).to_ascii_lowercase();
    if domain.contains(['/', ' ', ':']) {
        return Err(format!("{DOMAIN}: a host name, as docs.example.com"));
    }
    Ok(PagesForm {
        workflow,
        branch,
        path,
        domain,
        https: flag(HTTPS, &get(HTTPS))?,
    })
}

/// What a Pages or properties form asks GitHub.
pub(super) fn confirm(form: &Form, text: &str) -> Result<Asks, String> {
    match form {
        Form::Pages { repo, from } => pages_asks(repo, from.as_deref(), text),
        Form::Properties {
            repo,
            schema,
            values,
        } => properties_asks(repo, schema, values, text),
        _ => Err("Not a Pages or properties form".into()),
    }
}

fn pages_asks(repo: &RepoId, from: Option<&PagesForm>, text: &str) -> Result<Asks, String> {
    let base = from.cloned().unwrap_or_default();
    let to = parse_pages(text, &base)?;
    let source = crate::ui::pages::pages_source(&to);
    match from {
        None => {
            let mut facts = vec![(format!("Built from {source}"), false)];
            if !to.domain.is_empty() {
                facts.push((format!("At {}", to.domain), false));
            }
            let change = SettingsChange::Pages(PagesChange::Create {
                repo: repo.clone(),
                form: to,
            });
            let title = format!("Publish a site for {repo}?");
            Ok((Action::AddSetting, title, facts, "Publish", change))
        }
        Some(from) => {
            let facts = differences(&pages_lines(from), &pages_lines(&to));
            if facts.is_empty() {
                return Err("Nothing changed".into());
            }
            let change = SettingsChange::Pages(PagesChange::Update {
                repo: repo.clone(),
                from: Box::new(from.clone()),
                to: Box::new(to),
            });
            let title = format!("Change {repo}'s site?");
            Ok((Action::EditSetting, title, facts, "Save", change))
        }
    }
}

fn properties(
    action: Action,
    repo: &RepoId,
    p: &Properties,
    row: Option<&SettingLink>,
) -> Result<Step, String> {
    if action != Action::EditSetting {
        return Err("A property's value is set or unset in the form, not added or deleted".into());
    }
    let schema = p.schema.as_ref().map_err(Clone::clone)?;
    if let Some(SettingLink::Property(name)) = row
        && let Some(prop) = schema.iter().find(|s| s.name == *name)
        && !prop.repo_may
    {
        return Err(format!(
            "{name} is set by {}'s admins, in its settings",
            repo.owner
        ));
    }
    let editable: Vec<Property> = schema.iter().filter(|s| s.repo_may).cloned().collect();
    if editable.is_empty() {
        return Err(format!(
            "Only {}'s admins set its properties, in its settings",
            repo.owner
        ));
    }
    let text = (editable.iter())
        .map(|s| {
            format!(
                "{}: {}",
                s.name,
                p.value(&s.name).map(PropValue::text).unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let form = Form::Properties {
        repo: repo.clone(),
        values: (editable.iter())
            .map(|s| (s.name.clone(), p.value(&s.name).cloned()))
            .collect(),
        schema: editable,
    };
    write(form, text)
}

/// A property's value as written, checked against what it allows.
fn prop_value(prop: &Property, written: &str) -> Result<Option<PropValue>, String> {
    let name = &prop.name;
    if written.is_empty() {
        if prop.required {
            return Err(format!("{name} is required"));
        }
        return Ok(None);
    }
    let allowed = |v: &str| -> Result<String, String> {
        (prop.allowed.iter())
            .find(|a| a.eq_ignore_ascii_case(v))
            .cloned()
            .ok_or_else(|| format!("{name}: one of {}, not “{v}”", prop.allowed.join(", ")))
    };
    Ok(Some(match prop.kind {
        PropKind::Text => PropValue::One(written.to_owned()),
        PropKind::Url if written.starts_with("https://") || written.starts_with("http://") => {
            PropValue::One(written.to_owned())
        }
        PropKind::Url => return Err(format!("{name}: an http(s):// address")),
        PropKind::Bool => match written.to_ascii_lowercase().as_str() {
            "true" | "yes" => PropValue::One("true".into()),
            "false" | "no" => PropValue::One("false".into()),
            _ => return Err(format!("{name}: true or false, not “{written}”")),
        },
        PropKind::Select => PropValue::One(allowed(written)?),
        PropKind::Multi => PropValue::Many(
            (written.split(','))
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(allowed)
                .collect::<Result<_, _>>()?,
        ),
    }))
}

/// The properties' form read: each value that changed.
pub fn parse_properties(
    text: &str,
    schema: &[Property],
    values: &PropertyValues,
) -> Result<PropertyValues, String> {
    let mut changed = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(format!("Line {}: write it as “Name: value”", n + 1));
        };
        let name = name.trim();
        let Some(prop) = schema.iter().find(|p| p.name.eq_ignore_ascii_case(name)) else {
            return Err(format!("“{name}” isn't one of the properties you may set"));
        };
        if seen.contains(&prop.name.as_str()) {
            return Err(format!("“{}” is there twice", prop.name));
        }
        seen.push(&prop.name);
        let to = prop_value(prop, value.trim())?;
        let was = (values.iter())
            .find(|(n, _)| *n == prop.name)
            .and_then(|(_, v)| v.as_ref());
        if was != to.as_ref() {
            changed.push((prop.name.clone(), to));
        }
    }
    Ok(changed)
}

fn properties_asks(
    repo: &RepoId,
    schema: &[Property],
    values: &PropertyValues,
    text: &str,
) -> Result<Asks, String> {
    let changed = parse_properties(text, schema, values)?;
    if changed.is_empty() {
        return Err("Nothing changed".into());
    }
    let shown = |v: Option<&PropValue>| v.map_or_else(|| "(unset)".to_owned(), PropValue::text);
    let facts = (changed.iter())
        .map(|(name, to)| {
            let was = (values.iter())
                .find(|(n, _)| n == name)
                .and_then(|(_, v)| v.as_ref());
            (
                format!("{name}: {} → {}", shown(was), shown(to.as_ref())),
                false,
            )
        })
        .collect();
    let change = SettingsChange::SetProperties {
        repo: repo.clone(),
        values: changed,
    };
    let title = format!("Set {repo}'s custom properties?");
    Ok((Action::EditSetting, title, facts, "Save", change))
}

/// Whether the page shows a change to the site or the properties.
pub fn shows(c: &SettingsChange, s: &Settings) -> bool {
    match (c, s) {
        (SettingsChange::Pages(PagesChange::Create { .. }), Settings::Pages(p)) => p.site.is_some(),
        (SettingsChange::Pages(PagesChange::Delete { .. }), Settings::Pages(p)) => p.site.is_none(),
        (SettingsChange::Pages(PagesChange::Update { to, .. }), Settings::Pages(p)) => {
            p.site.as_ref().is_some_and(|s| same_site(&s.form, to))
        }
        (SettingsChange::SetProperties { values, .. }, Settings::Properties(p)) => {
            (values.iter()).all(|(name, v)| p.value(name) == v.as_ref())
        }
        _ => true,
    }
}

/// A site as GitHub shows it matches the form: a workflow's site has no
/// branch to compare.
fn same_site(shown: &PagesForm, form: &PagesForm) -> bool {
    let source = |f: &PagesForm| (!f.workflow).then(|| (f.branch.clone(), f.path.clone()));
    shown.workflow == form.workflow
        && source(shown) == source(form)
        && shown.domain.eq_ignore_ascii_case(&form.domain)
        && shown.https == form.https
}

pub fn said(c: &PagesChange) -> (&'static str, &'static str) {
    match c {
        PagesChange::Create { .. } => ("Publishing the site", "the new site"),
        PagesChange::Update { .. } => ("Saving the site", "the site's change"),
        PagesChange::Delete { .. } => ("Unpublishing the site", "unpublishing the site"),
    }
}

pub fn done(c: &PagesChange) -> String {
    match c {
        PagesChange::Create { repo, .. } => format!("Published {repo}'s site"),
        PagesChange::Update { repo, .. } => format!("Saved {repo}'s site"),
        PagesChange::Delete { repo } => format!("Unpublished {repo}'s site"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Pages form reads back as written and says what it can't take.
    #[test]
    fn a_pages_form_reads_back() {
        let form = PagesForm {
            path: "/docs".into(),
            domain: "docs.example.com".into(),
            ..PagesForm::default()
        };
        assert_eq!(
            parse_pages(&pages_text(&form), &PagesForm::default()),
            Ok(form.clone())
        );
        let err = |text| parse_pages(text, &form).unwrap_err();
        assert!(err("Folder: /src").contains("/ or /docs"));
        assert!(err("Source: ftp").contains("branch or actions"));
        assert!(err("Custom domain: https://x.com").contains("host name"));
        let to = parse_pages("Source: actions", &form).unwrap();
        assert!(to.workflow);
    }

    /// A property's value is one it allows, in its case; only what
    /// changed is sent, and an emptied one is unset.
    #[test]
    fn properties_are_checked_against_what_they_allow() {
        let prop = |name: &str, kind, allowed: &[&str]| Property {
            name: name.into(),
            kind,
            required: false,
            description: None,
            allowed: allowed.iter().map(|s| (*s).to_owned()).collect(),
            repo_may: true,
        };
        let schema = [
            prop("env", PropKind::Select, &["Prod", "Dev"]),
            prop("teams", PropKind::Multi, &["a", "b"]),
            prop("note", PropKind::Text, &[]),
        ];
        let values = vec![
            ("env".to_owned(), Some(PropValue::One("Dev".into()))),
            ("note".to_owned(), Some(PropValue::One("hi".into()))),
        ];
        let changed = parse_properties("env: prod\nteams: a, B\nnote:", &schema, &values);
        assert_eq!(
            changed,
            Ok(vec![
                ("env".into(), Some(PropValue::One("Prod".into()))),
                (
                    "teams".into(),
                    Some(PropValue::Many(vec!["a".into(), "b".into()]))
                ),
                ("note".into(), None),
            ])
        );
        let err = parse_properties("env: staging", &schema, &values).unwrap_err();
        assert!(err.contains("one of Prod, Dev"), "{err}");
    }
}
