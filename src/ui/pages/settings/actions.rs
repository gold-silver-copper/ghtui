//! Actions' secrets, variables and caches, as their pages show them.

use super::*;
use crate::api::settings::actions::{Cache, Caches, DEFAULT_LIMIT, Kept, Store};
use crate::api::settings::{Secret, Variable};

/// How a secret's or a variable's row reads.
pub(super) trait Row: Kept {
    /// What it's called on the page: `secret` or `variable`.
    const NOUN: &'static str;
    fn link(&self, env: Option<&str>) -> SettingLink;
    /// What goes under its name.
    fn about(&self) -> Option<String>;
}

impl Row for Secret {
    const NOUN: &'static str = "secret";
    fn link(&self, env: Option<&str>) -> SettingLink {
        SettingLink::Secret(env.map(str::to_owned), self.name.clone())
    }
    fn about(&self) -> Option<String> {
        None
    }
}

impl Row for Variable {
    const NOUN: &'static str = "variable";
    fn link(&self, env: Option<&str>) -> SettingLink {
        SettingLink::Variable(env.map(str::to_owned), self.name.clone())
    }
    fn about(&self) -> Option<String> {
        let mut lines = self.value.lines();
        let first = lines.next().unwrap_or_default();
        Some(match lines.next() {
            Some(_) => format!("{first} …"),
            None if first.is_empty() => "(empty)".into(),
            None => first.to_owned(),
        })
    }
}

/// Who may use an organization's secret or variable.
fn visibility(v: Option<&str>) -> &'static str {
    match v {
        Some("all") => "all repositories",
        Some("private") => "private repositories",
        _ => "selected repositories",
    }
}

/// The Secrets or Variables page: the repository's, each environment's,
/// then the organization's (read here, changed in its settings).
pub(super) fn store<T: Row>(
    page: &mut Page,
    repo: &RepoId,
    s: &Store<T>,
    keys: SettingKeys<'_>,
    now: u64,
) {
    let (noun, kind) = (T::NOUN, T::KIND);
    let updated = |t: &T| {
        vec![meta(format!(
            "updated {}",
            time::ago_iso(t.updated_at(), now)
        ))]
    };
    let right = add_button(page, &format!("New repository {noun}"), keys);
    let none = format!("No repository {kind}.");
    let name = format!("Repository {kind}");
    counted_box(page, &name, right, &s.repo, &none, |page, t| {
        setting_row(page, t.link(None), t.name(), updated(t), t.about());
    });
    for env in &s.environments {
        page.blank();
        let link = Link::Setting(SettingLink::AddIn(env.name.clone()));
        let right = vec![button(page, format!("New {noun}"), link)];
        let title = format!("Environment {kind} · {}", env.name);
        let none = format!("No {kind} in {}.", env.name);
        counted_box(page, &title, right, &env.items, &none, |page, t| {
            setting_row(
                page,
                t.link(Some(&env.name)),
                t.name(),
                updated(t),
                t.about(),
            );
        });
    }
    if s.more_environments {
        let n = s.environments.len();
        let why = format!("Only the first {n} environments are listed: the rest are on GitHub.");
        say(page, why, Role::Meta);
    }
    let title = format!("Organization {kind}");
    match &s.org {
        None => {}
        Some(Err(why)) => {
            page.blank();
            empty_box(page, strong(title), &format!("{why}."));
        }
        Some(Ok(org)) => {
            page.blank();
            let none = format!("{} shares no {kind} with it.", repo.owner);
            counted_box(page, &title, Vec::new(), org, &none, |page, t| {
                let link = SettingLink::OrgItem(t.name().to_owned());
                let mut right = updated(t);
                right.insert(0, meta(format!("{} · ", visibility(t.visibility()))));
                setting_row(page, link, t.name(), right, t.about());
            });
        }
    }
    page.blank();
    let (edit, delete) = (keys.edit, keys.delete);
    let org = match s.org {
        Some(_) => " The organization's are changed in its settings.",
        None => "",
    };
    let how = match kind {
        "secrets" => format!(
            "GitHub never shows a secret's value, nor does ghtui: {edit} sets a new one, {delete} deletes it.{org}"
        ),
        _ => format!("{edit} edits the selected one, {delete} deletes it.{org}"),
    };
    say(page, how, Role::Meta);
}

pub(super) fn caches(page: &mut Page, c: &Caches, keys: SettingKeys<'_>, now: u64) {
    let (limit, whose) = match c.limit {
        Some(limit) => (limit, ""),
        None => (DEFAULT_LIMIT, " (GitHub's usual limit)"),
    };
    let used = format!(
        "{} {} · {} of {}{whose}",
        c.count,
        if c.count == 1 { "cache" } else { "caches" },
        size(c.used),
        size(limit),
    );
    say(page, used, Role::Meta);
    if c.used >= limit {
        let full = "It's at its limit: GitHub deletes the least recently used caches first.";
        say(page, full, Role::Error);
    }
    page.blank();
    let none = "No caches. A workflow makes one with actions/cache.";
    counted_box(page, "Caches", Vec::new(), &c.items, none, |page, cache| {
        cache_row(page, cache, now);
    });
    page.blank();
    let how = format!(
        "{} deletes the selected cache, or every cache with its key, after asking.",
        keys.delete
    );
    say(page, how, Role::Meta);
}

fn cache_row(page: &mut Page, cache: &Cache, now: u64) {
    let right = vec![meta(size(cache.size))];
    let about = format!(
        "{} · last used {} · made {}",
        cache.git_ref,
        time::ago_iso(&cache.last_used, now),
        time::ago_iso(&cache.created_at, now)
    );
    let link = SettingLink::Cache(cache.id);
    setting_row(page, link, cache.key.clone(), right, Some(about));
}
