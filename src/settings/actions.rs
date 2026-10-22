//! Actions' secrets, variables and caches: a secret or a variable is
//! written in the composer (`Name:`, then `Value:` to the end, lines and
//! all), a cache only deleted. A secret's value is sealed when it's
//! confirmed ([`seal`]): what's sent, and kept until GitHub answers, is
//! never the value itself.

use super::{Asks, Form, ask, write};
use crate::act::{Step, Subject};
use crate::api::change::Change;
use crate::api::model::RepoId;
use crate::api::settings::actions::{ActionsChange, Kept, SealKey, Store, seal};
use crate::api::settings::{Settings, SettingsChange, SettingsPage};
use crate::keymap::Action;
use crate::state::State;
use crate::ui::pages::SettingLink;

/// Where a secret or variable is kept, as a question says it.
fn scope(repo: &RepoId, env: Option<&str>) -> String {
    match env {
        Some(env) => format!("the {env} environment"),
        None => repo.to_string(),
    }
}

/// GitHub's rules for a secret's or a variable's name, which it keeps in
/// capitals.
pub fn valid_name(name: &str) -> Result<String, String> {
    let name = name.trim().to_ascii_uppercase();
    if name.is_empty() {
        return Err("Name: what workflows call it, as API_TOKEN".into());
    }
    if !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err(format!(
            "Name: letters, digits and underscores only, not “{name}”"
        ));
    }
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        return Err("Name: it can't start with a digit".into());
    }
    if name.starts_with("GITHUB_") {
        return Err("Name: GITHUB_ names are GitHub's own".into());
    }
    Ok(name)
}

/// A secret's or variable's form: `Name: …` (when `named`), then
/// `Value:` and everything after it.
pub fn parse_named(text: &str, named: bool) -> Result<(Option<String>, String), String> {
    let mut rest = text.trim_start_matches('\n');
    let mut name = None;
    if named {
        let (line, after) = rest.split_once('\n').unwrap_or((rest, ""));
        let Some(n) = strip_label(line, "Name") else {
            return Err("Line 1: write it as “Name: API_TOKEN”".into());
        };
        name = Some(valid_name(n)?);
        rest = after;
    }
    let Some(value) = strip_label(rest, "Value") else {
        return Err("Write the value after “Value:”".into());
    };
    let value = value
        .strip_prefix(' ')
        .unwrap_or(value)
        .trim_end_matches('\n');
    if value.trim().is_empty() {
        return Err("Value: what it's set to".into());
    }
    Ok((name, value.to_owned()))
}

/// What follows `label:` at the start of `text`, in any case.
fn strip_label<'a>(text: &'a str, label: &str) -> Option<&'a str> {
    let (head, rest) = text.split_once(':')?;
    head.trim().eq_ignore_ascii_case(label).then_some(rest)
}

fn form_text(name: Option<&str>, value: &str) -> String {
    match name {
        Some(name) => format!("Name: {name}\nValue: {value}"),
        None => format!("Value: {value}"),
    }
}

/// The environment the selected row is in, if any.
fn env_of(row: Option<&SettingLink>) -> Option<String> {
    match row? {
        SettingLink::AddIn(env)
        | SettingLink::Secret(Some(env), _)
        | SettingLink::Variable(Some(env), _) => Some(env.clone()),
        _ => None,
    }
}

/// The names (and when each was set) already in `env`'s list.
fn known<T: Kept>(store: &Store<T>, env: Option<&str>) -> Vec<(String, String)> {
    (store.list(env).into_iter())
        .flat_map(|l| &l.items)
        .map(|t| (t.name().to_owned(), t.updated_at().to_owned()))
        .collect()
}

/// A secret or a variable on the page: the row's, else a hint.
fn chosen(row: Option<&SettingLink>, what: &str) -> Result<(Option<String>, String), String> {
    match row {
        Some(SettingLink::Secret(env, name) | SettingLink::Variable(env, name)) => {
            Ok((env.clone(), name.clone()))
        }
        Some(SettingLink::OrgItem(_)) => Err(format!(
            "It's the organization's {what}: it's changed in the organization's settings"
        )),
        _ => Err(format!("Select a {what} first")),
    }
}

/// What a key does on the Secrets, Variables or Caches page, or `None`
/// on another.
pub fn plan(
    state: &State,
    action: Action,
    repo: &RepoId,
    settings: &Settings,
    row: Option<&SettingLink>,
) -> Option<Result<Step, String>> {
    Some(match settings {
        Settings::Secrets(s) => secrets(action, repo, s, row),
        Settings::Variables(v) => variables(action, repo, v, row),
        Settings::Caches(c) => caches(state, action, repo, c, row),
        _ => return None,
    })
}

fn secrets(
    action: Action,
    repo: &RepoId,
    s: &Store<crate::api::settings::Secret>,
    row: Option<&SettingLink>,
) -> Result<Step, String> {
    let at = (repo, SettingsPage::Secrets);
    let key = |env: Option<&str>| -> Result<SealKey, String> {
        (s.key(env).cloned()).ok_or_else(|| "GitHub didn't give its key for secrets".to_owned())
    };
    match action {
        Action::AddSetting => {
            let env = env_of(row);
            let form = Form::Secret {
                repo: repo.clone(),
                key: key(env.as_deref())?,
                known: known(s, env.as_deref()),
                env,
                name: None,
            };
            write(form, form_text(Some(""), ""))
        }
        Action::EditSetting => {
            let (env, name) = chosen(row, "secret")?;
            let form = Form::Secret {
                repo: repo.clone(),
                key: key(env.as_deref())?,
                known: known(s, env.as_deref()),
                env,
                name: Some(name),
            };
            write(form, form_text(None, ""))
        }
        Action::DeleteSetting => {
            let (env, name) = chosen(row, "secret")?;
            let facts = vec![
                (
                    format!("Workflows that read secrets.{name} get an empty string"),
                    true,
                ),
                ("GitHub can't bring it back".into(), false),
            ];
            let title = format!("Delete secret {name} from {}?", scope(repo, env.as_deref()));
            let change = ActionsChange::DeleteSecret {
                repo: repo.clone(),
                env,
                name,
            };
            Ok(ask(
                action,
                at,
                title,
                facts,
                ("Delete secret", SettingsChange::Actions(change)),
            ))
        }
        _ => Err("A secret is set or deleted, not turned off".into()),
    }
}

fn variables(
    action: Action,
    repo: &RepoId,
    v: &Store<crate::api::settings::Variable>,
    row: Option<&SettingLink>,
) -> Result<Step, String> {
    let at = (repo, SettingsPage::Variables);
    match action {
        Action::AddSetting => {
            let env = env_of(row);
            let form = Form::Variable {
                repo: repo.clone(),
                known: known(v, env.as_deref()),
                env,
                from: None,
            };
            write(form, form_text(Some(""), ""))
        }
        Action::EditSetting | Action::DeleteSetting => {
            let (env, name) = chosen(row, "variable")?;
            let list = v.list(env.as_deref()).ok_or("That environment is gone")?;
            let found = (list.items.iter())
                .find(|t| t.name == name)
                .ok_or("That variable is gone")?;
            if action == Action::EditSetting {
                let text = form_text(Some(&found.name), &found.value);
                let form = Form::Variable {
                    repo: repo.clone(),
                    known: known(v, env.as_deref()),
                    env,
                    from: Some(Box::new((found.name.clone(), found.value.clone()))),
                };
                return write(form, text);
            }
            let facts = vec![(
                format!("Workflows that read vars.{name} get an empty string"),
                true,
            )];
            let title = format!(
                "Delete variable {name} from {}?",
                scope(repo, env.as_deref())
            );
            let change = ActionsChange::DeleteVariable {
                repo: repo.clone(),
                env,
                name,
            };
            let choice = ("Delete variable", SettingsChange::Actions(change));
            Ok(ask(action, at, title, facts, choice))
        }
        _ => Err("A variable is edited or deleted, not turned off".into()),
    }
}

fn caches(
    state: &State,
    action: Action,
    repo: &RepoId,
    c: &crate::api::settings::Caches,
    row: Option<&SettingLink>,
) -> Result<Step, String> {
    if action != Action::DeleteSetting {
        let delete = state.first_key(Action::DeleteSetting);
        return Err(format!(
            "Caches are made by workflows; here {delete} deletes one"
        ));
    }
    let Some(SettingLink::Cache(id)) = row else {
        return Err("Select a cache first".into());
    };
    let cache = (c.items.items.iter())
        .find(|cache| cache.id == *id)
        .ok_or("That cache is gone")?;
    let same = (c.items.items.iter())
        .filter(|other| other.key == cache.key)
        .count();
    let facts = vec![
        (
            format!("{} · {}", cache.git_ref, crate::ui::text::size(cache.size)),
            false,
        ),
        (
            "The next run that asks for it makes it again, more slowly".into(),
            false,
        ),
    ];
    let one = ActionsChange::DeleteCache {
        repo: repo.clone(),
        id: cache.id,
        key: cache.key.clone(),
    };
    let mut choices = vec![("Delete this cache".to_owned(), one)];
    if same > 1 {
        let all = ActionsChange::DeleteCaches {
            repo: repo.clone(),
            key: cache.key.clone(),
            count: same,
        };
        choices.push((format!("Delete all {same} with this key"), all));
    }
    let choices = (choices.into_iter())
        .map(|(label, c)| (label, Change::Settings(SettingsChange::Actions(c))))
        .collect();
    let about = Subject::Settings(repo.clone(), SettingsPage::Caches);
    let title = format!("Delete cache {}?", cache.key);
    Ok(Step::Ask(Box::new(crate::act::confirm(
        action, about, title, facts, choices,
    ))))
}

/// What a secret's or a variable's form asks GitHub, or why it can't.
pub(super) fn confirm(form: &Form, text: &str) -> Result<Asks, String> {
    match form {
        Form::Secret { .. } => secret(form, text),
        _ => variable(form, text),
    }
}

fn secret(form: &Form, text: &str) -> Result<Asks, String> {
    let Form::Secret {
        repo,
        env,
        name,
        key,
        known,
    } = form
    else {
        return Err("Not a secret".into());
    };
    let (typed, value) = parse_named(text, name.is_none())?;
    let name = typed.or_else(|| name.clone()).unwrap_or_default();
    let was = (known.iter())
        .find(|(n, _)| *n == name)
        .map(|(_, at)| at.clone());
    let mut facts = vec![
        (format!("Workflows read it as secrets.{name}"), false),
        (
            "It's sealed with GitHub's key before it's sent: nobody can read it back, ghtui neither"
                .into(),
            false,
        ),
    ];
    let action = if was.is_some() {
        let replaces = "It replaces its value: the old one is gone for good";
        facts.insert(0, (replaces.into(), true));
        Action::EditSetting
    } else {
        Action::AddSetting
    };
    let title = format!("Set secret {name} in {}?", scope(repo, env.as_deref()));
    let change = ActionsChange::SetSecret {
        repo: repo.clone(),
        env: env.clone(),
        sealed: seal(key, &value)?,
        key_id: key.key_id.clone(),
        was,
        name,
    };
    Ok((
        action,
        title,
        facts,
        "Set secret",
        SettingsChange::Actions(change),
    ))
}

fn variable(form: &Form, text: &str) -> Result<Asks, String> {
    let Form::Variable {
        repo,
        env,
        from,
        known,
    } = form
    else {
        return Err("Not a variable".into());
    };
    let (name, value) = parse_named(text, true)?;
    let name = name.unwrap_or_default();
    let renamed = from.as_ref().is_none_or(|f| f.0 != name);
    let at = scope(repo, env.as_deref());
    if renamed && known.iter().any(|(n, _)| *n == name) {
        return Err(format!("{at} already has a variable {name}"));
    }
    let first = |v: &str| v.lines().next().unwrap_or_default().to_owned();
    let (action, title, facts) = match from.as_deref() {
        None => (
            Action::AddSetting,
            format!("Create variable {name} in {at}?"),
            vec![(format!("{name} = {}", first(&value)), false)],
        ),
        Some((was, old)) => {
            let mut facts = Vec::new();
            if *was != name {
                facts.push((format!("Name: {was} → {name}"), true));
            }
            if *old != value {
                let change = format!("Value: {} → {}", first(old), first(&value));
                facts.push((change, false));
            }
            if facts.is_empty() {
                return Err("Nothing changed".into());
            }
            (
                Action::EditSetting,
                format!("Change variable {was}?"),
                facts,
            )
        }
    };
    let change = ActionsChange::SetVariable {
        repo: repo.clone(),
        env: env.clone(),
        from: from.as_ref().map(|f| f.0.clone()),
        name,
        value,
    };
    Ok((
        action,
        title,
        facts,
        "Save variable",
        SettingsChange::Actions(change),
    ))
}

/// Whether the page shows a change to secrets, variables or caches.
pub fn shows(c: &ActionsChange, s: &Settings) -> bool {
    use ActionsChange as C;
    match (c, s) {
        (C::SetSecret { env, name, was, .. }, Settings::Secrets(s)) => {
            (s.list(env.as_deref()).into_iter().flat_map(|l| &l.items))
                .any(|t| t.name == *name && Some(&t.updated_at) != was.as_ref())
        }
        (C::DeleteSecret { env, name, .. }, Settings::Secrets(s)) => {
            !(s.list(env.as_deref()).into_iter().flat_map(|l| &l.items)).any(|t| t.name == *name)
        }
        (
            C::SetVariable {
                env,
                from,
                name,
                value,
                ..
            },
            Settings::Variables(v),
        ) => {
            let list: Vec<_> = (v.list(env.as_deref()).into_iter())
                .flat_map(|l| &l.items)
                .collect();
            let there = list.iter().any(|t| t.name == *name && t.value == *value);
            let old_gone = from
                .as_ref()
                .is_none_or(|f| f == name || list.iter().all(|t| t.name != *f));
            there && old_gone
        }
        (C::DeleteVariable { env, name, .. }, Settings::Variables(v)) => {
            !(v.list(env.as_deref()).into_iter().flat_map(|l| &l.items)).any(|t| t.name == *name)
        }
        (C::DeleteCache { id, .. }, Settings::Caches(c)) => {
            c.items.items.iter().all(|c| c.id != *id)
        }
        (C::DeleteCaches { key, .. }, Settings::Caches(c)) => {
            c.items.items.iter().all(|c| c.key != *key)
        }
        _ => true,
    }
}

pub fn said(c: &ActionsChange) -> (&'static str, &'static str) {
    use ActionsChange as C;
    match c {
        C::SetSecret { .. } => ("Setting the secret", "the secret"),
        C::DeleteSecret { .. } => ("Deleting the secret", "deleting the secret"),
        C::SetVariable { from: None, .. } => ("Creating the variable", "the new variable"),
        C::SetVariable { .. } => ("Saving the variable", "the variable's change"),
        C::DeleteVariable { .. } => ("Deleting the variable", "deleting the variable"),
        C::DeleteCache { .. } => ("Deleting the cache", "deleting the cache"),
        C::DeleteCaches { .. } => ("Deleting the caches", "deleting the caches"),
    }
}

pub fn done(c: &ActionsChange) -> String {
    use ActionsChange as C;
    match c {
        C::SetSecret { name, .. } => format!("Set secret {name}"),
        C::DeleteSecret { name, .. } => format!("Deleted secret {name}"),
        C::SetVariable { name, .. } => format!("Saved variable {name}"),
        C::DeleteVariable { name, .. } => format!("Deleted variable {name}"),
        C::DeleteCache { key, .. } => format!("Deleted cache {key}"),
        C::DeleteCaches { key, count, .. } => format!("Deleted {count} caches with key {key}"),
    }
}

pub fn needs(c: &ActionsChange) -> &'static str {
    use ActionsChange as C;
    match c {
        C::SetSecret { .. } | C::DeleteSecret { .. } => "Secrets: write",
        C::SetVariable { .. } | C::DeleteVariable { .. } => "Variables: write",
        C::DeleteCache { .. } | C::DeleteCaches { .. } => "Actions: write",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A form's name is GitHub's (capitals, no GITHUB_), and its value
    /// runs to the end, lines and all.
    #[test]
    fn a_secrets_form_reads_its_name_and_value() {
        let read = parse_named("Name: api_token\nValue: -----BEGIN\nabc\n-----END\n", true);
        assert_eq!(
            read,
            Ok((Some("API_TOKEN".into()), "-----BEGIN\nabc\n-----END".into()))
        );
        assert_eq!(parse_named("value: x", false), Ok((None, "x".into())));
        let err = |text| parse_named(text, true).unwrap_err();
        assert!(err("Name: GITHUB_X\nValue: 1").contains("GitHub's own"));
        assert!(err("Name: 1X\nValue: 1").contains("digit"));
        assert!(err("Name: A-B\nValue: 1").contains("letters, digits"));
        assert!(err("Name: A\nValue: ").contains("what it's set to"));
    }
}
