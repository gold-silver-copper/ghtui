//! A repository's Settings tab, as GitHub shows it to admins: its own
//! menu, then the page's lists, each row a [`SettingLink`] that the keys
//! of [`crate::settings`] act on.

use super::*;
use crate::api::model::NodeId;
use crate::api::settings::{
    Access, Branches, DeployKey, Enforcement, General, Hook, HookPage, Listed, Role as Given, Rule,
    Settings, SettingsPage,
};
use std::fmt::Write as _;

/// What a row of the Settings tab is.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SettingLink {
    /// The page's "Add" button.
    Add,
    Rule(NodeId),
    Ruleset(u64),
    Person(String),
    Invitation(u64, String),
    Hook(u64),
    Delivery(u64),
    Key(u64),
    /// Archive or unarchive the repository.
    Archive,
    /// Deleting the repository, which is GitHub's to do.
    DeleteRepo,
    /// "New secret" (or variable) in an environment's box.
    AddIn(String),
    /// A secret, in an environment or (`None`) the repository.
    Secret(Option<String>, String),
    Variable(Option<String>, String),
    /// An organization's secret or variable, changed in its settings.
    OrgItem(String),
    Cache(u64),
    /// The Pages site's settings.
    Site,
    /// Unpublishing the Pages site.
    Unpublish,
    Property(String),
}

mod actions;
mod site;

pub(crate) use site::source as pages_source;

/// The keys the page names, as they're bound.
#[derive(Debug, Clone, Copy)]
pub struct SettingKeys<'a> {
    pub add: &'a str,
    pub edit: &'a str,
    pub delete: &'a str,
    pub toggle: &'a str,
    pub redeliver: &'a str,
}

/// The tab's own menu, as GitHub's sidebar; `on` is the page shown.
fn settings_nav(page: &mut Page, repo: &RepoId, on: SettingsPage) {
    let base = url::repo(repo);
    let tabs = SettingsPage::MENU.into_iter().map(|item| {
        (
            item.name(),
            format!("{base}{}", item.slug()),
            item == on.menu(),
        )
    });
    let segs = nav_strip(page, &meta(" · "), &Role::Link, tabs);
    page.wrapped(segs, 0, Frame::None);
    page.blank();
}

/// A page of the Settings tab, once its data is in.
pub fn settings_page(
    page: &mut Page,
    repo: &RepoId,
    at: SettingsPage,
    data: Fetched<'_, Settings>,
    keys: SettingKeys<'_>,
    admin: bool,
    now: u64,
) {
    if at.admins_only() {
        settings_nav(page, repo, at);
    }
    if !admin && at.admins_only() {
        let why = format!("You don't administer {repo}: GitHub shows its settings only to admins.");
        say(page, why, Role::Meta);
        page.blank();
    }
    let Some(data) = data.show(page, &at.name().to_lowercase()) else { return };
    match data {
        Settings::General(g) => general(page, repo, g, keys),
        Settings::Branches(b) => branches(page, b, keys),
        Settings::Access(a) => access(page, a, keys, now),
        Settings::Hooks(h) => hooks(page, h, keys, now),
        Settings::Hook(h) => hook(page, h, keys, now),
        Settings::Keys(k) => deploy_keys(page, k, keys, now),
        Settings::Secrets(s) => actions::store(page, repo, s, keys, now),
        Settings::Variables(v) => actions::store(page, repo, v, keys, now),
        Settings::Caches(c) => actions::caches(page, c, keys, now),
        Settings::Pages(p) => site::pages(page, p, keys, now),
        Settings::Properties(p) => site::properties(page, repo, p, keys),
    }
}

/// A box's `Add` button, with its key.
fn add_button(page: &mut Page, text: &str, keys: SettingKeys<'_>) -> Vec<Seg> {
    let link = Link::Setting(SettingLink::Add);
    vec![button(page, text, link), meta(format!(" {}", keys.add))]
}

impl<T> Rows<T> for Listed<T> {
    fn rows(&self) -> &[T] {
        &self.items
    }
    fn rest(&self) -> Rest {
        if self.more {
            Rest::Uncounted
        } else {
            Rest::All
        }
    }
}

/// A row that opens `to`: its strong `name`, `right` on its right, and
/// `about` under it.
fn setting_row(
    page: &mut Page,
    to: SettingLink,
    name: impl Into<String>,
    right: Vec<Seg>,
    about: Option<String>,
) {
    item(page, Link::Setting(to), |page, link| {
        page.box_line(vec![Seg::linked(name, Role::Strong, link)], right, 0);
        if let Some(about) = about {
            body(page, vec![meta(about)]);
        }
    });
}

fn general(page: &mut Page, repo: &RepoId, g: &General, keys: SettingKeys<'_>) {
    page.box_top(vec![strong("General")], Vec::new());
    let archived = if g.archived {
        "yes: it's read-only"
    } else {
        "no"
    };
    for (name, value) in [
        ("Repository", repo.to_string()),
        ("Visibility", capitalized(&g.visibility)),
        ("Default branch", g.default_branch.clone()),
        ("Archived", archived.to_owned()),
    ] {
        let name = meta(format!("{name:<14}  "));
        body(page, vec![name, Seg::new(value, Role::Body)]);
    }
    page.box_bottom();
    page.blank();
    if !g.admin {
        let why = "You don't administer it: GitHub shows its settings only to admins.";
        say(page, why, Role::Meta);
        return;
    }
    page.box_top(vec![strong("Danger zone")], Vec::new());
    let (label, about) = if g.archived {
        (
            "Unarchive this repository",
            "It becomes writable again: pushes, issues, pull requests and comments",
        )
    } else {
        (
            "Archive this repository",
            "It becomes read-only: no pushes, issues, pull requests or comments",
        )
    };
    let (toggle, about) = (vec![meta(keys.toggle.to_owned())], Some(about.into()));
    setting_row(page, SettingLink::Archive, label, toggle, about);
    page.box_rule();
    let about = Some("On github.com only: ghtui never deletes a repository".to_owned());
    let delete = "Delete this repository";
    setting_row(page, SettingLink::DeleteRepo, delete, Vec::new(), about);
    page.box_bottom();
}

/// What a rule asks, said briefly for its row.
fn rule_meta(r: &Rule) -> String {
    let f = &r.form;
    let mut asks = Vec::new();
    if let Some(reviews) = &f.reviews {
        asks.push(match reviews.count {
            0 => "a pull request".to_owned(),
            1 => "1 approval".to_owned(),
            n => format!("{n} approvals"),
        });
    }
    if let Some(checks) = &f.checks {
        let names = if checks.contexts.is_empty() {
            "status checks".to_owned()
        } else {
            format!("checks: {}", checks.contexts.join(", "))
        };
        let strict = if checks.strict { " (up to date)" } else { "" };
        asks.push(format!("{names}{strict}"));
    }
    for (on, what) in [
        (f.conversations, "conversations resolved"),
        (f.signed, "signed commits"),
        (f.linear, "linear history"),
        (f.lock, "locked"),
        (f.admins, "no bypassing"),
        (f.force_pushes, "force pushes allowed"),
        (f.deletions, "deletions allowed"),
    ] {
        if on {
            asks.push(what.to_owned());
        }
    }
    if asks.is_empty() {
        "Asks nothing yet".to_owned()
    } else {
        asks.join(" · ")
    }
}

fn branches(page: &mut Page, b: &Branches, keys: SettingKeys<'_>) {
    let right = add_button(page, "Add rule", keys);
    let none = "No branch protection rules: any branch can be pushed to, force-pushed and deleted.";
    let (rules, name) = (&b.rules, "Branch protection rules");
    counted_box(page, name, right, rules, none, |page, r| {
        let applies = match r.matching.total() {
            0 => "applies to no branch yet".to_owned(),
            1 => "applies to 1 branch".to_owned(),
            n => format!("applies to {n} branches"),
        };
        let to = SettingLink::Rule(r.id.clone());
        let right = vec![meta(applies)];
        setting_row(page, to, r.form.pattern.clone(), right, Some(rule_meta(r)));
    });
    page.blank();
    let rulesets = match &b.rulesets {
        Ok(r) => r,
        Err(why) => {
            empty_box(page, strong("Rulesets"), &format!("{why}."));
            return;
        }
    };
    let title = vec![strong(format!("Rulesets  {}", rulesets.len()))];
    let empty = "No rulesets. They're made on github.com; here they're turned on and off.";
    let rulesets = Whole(rulesets);
    list_box(page, title, Vec::new(), &rulesets, empty, |page, r| {
        let bg = match r.enforcement {
            Enforcement::Active => Bg::SuccessContainer,
            Enforcement::Evaluate => Bg::TertiaryContainer,
            Enforcement::Disabled => Bg::SecondaryContainer,
        };
        let about = match &r.org {
            Some(org) => format!("{} ruleset · from {org}", r.target),
            None => format!("{} ruleset · {} turns it on or off", r.target, keys.toggle),
        };
        let right = vec![chip(r.enforcement.label(), bg)];
        let to = SettingLink::Ruleset(r.id);
        setting_row(page, to, r.name.clone(), right, Some(about));
    });
}

fn access(page: &mut Page, a: &Access, keys: SettingKeys<'_>, now: u64) {
    let right = add_button(page, "Add people", keys);
    let none = "Nobody has direct access.";
    counted_box(page, "Collaborators", right, &a.people, none, |page, p| {
        let bg = match &p.role {
            Given::Is(crate::api::settings::Permission::Admin) => Bg::PrimaryContainer,
            _ => Bg::SecondaryContainer,
        };
        let to = SettingLink::Person(p.login.clone());
        let right = vec![chip(p.role.name(), bg)];
        setting_row(page, to, format!("@{}", p.login), right, None);
    });
    page.blank();
    let (invitations, none) = (&a.invitations, "No invitations waiting.");
    let name = "Pending invitations";
    counted_box(page, name, Vec::new(), invitations, none, |page, i| {
        let when = if i.expired {
            "expired".to_owned()
        } else {
            format!("invited {}", time::ago_iso(&i.created_at, now))
        };
        let right = vec![
            meta(format!("{when}  ")),
            chip(i.permission.name(), Bg::SecondaryContainer),
        ];
        let to = SettingLink::Invitation(i.id, i.login.clone());
        setting_row(page, to, format!("@{}", i.login), right, None);
    });
    page.blank();
    let (edit, delete) = (keys.edit, keys.delete);
    let how = format!("{edit} changes the selected one's role, {delete} removes them");
    say(page, how, Role::Meta);
}

/// What a webhook's last delivery got back.
fn last_said(h: &Hook) -> String {
    match &h.last {
        None => "never delivered".into(),
        Some(last) => {
            let code = last.code.map_or_else(String::new, |c| format!("{c} "));
            let said = last.message.as_deref().unwrap_or(&last.status);
            format!("last delivery: {code}{said}")
        }
    }
}

fn events(h: &Hook) -> String {
    match h.form.events.as_slice() {
        [all] if all == "*" => "every event".into(),
        events => events.join(", "),
    }
}

fn active_chip(active: bool) -> Seg {
    if active {
        chip("Active", Bg::SuccessContainer)
    } else {
        chip("Inactive", Bg::SecondaryContainer)
    }
}

fn hooks(page: &mut Page, list: &Listed<Hook>, keys: SettingKeys<'_>, now: u64) {
    let right = add_button(page, "Add webhook", keys);
    counted_box(page, "Webhooks", right, list, "No webhooks.", |page, h| {
        let about = format!(
            "{} · {} · {} · changed {}",
            events(h),
            h.form.content_type,
            last_said(h),
            time::ago_iso(&h.updated_at, now)
        );
        let (to, right) = (SettingLink::Hook(h.id), vec![active_chip(h.form.active)]);
        setting_row(page, to, h.form.url.clone(), right, Some(about));
    });
}

fn hook(page: &mut Page, h: &HookPage, keys: SettingKeys<'_>, now: u64) {
    let hook = &h.hook;
    say(page, hook.form.url.clone(), Role::Title);
    let ssl = if hook.form.verify_ssl {
        "SSL verified"
    } else {
        "SSL not verified"
    };
    let secret = if hook.secret {
        "a secret set"
    } else {
        "no secret"
    };
    let subline = format!(
        "  {} · {} · {ssl} · {secret} · {}",
        events(hook),
        hook.form.content_type,
        last_said(hook)
    );
    let active = active_chip(hook.form.active);
    page.wrapped(vec![active, meta(subline)], 0, Frame::None);
    let (edit, toggle, delete) = (keys.edit, keys.toggle, keys.delete);
    let how = format!("{edit} edits it, {toggle} turns it on or off, {delete} deletes it");
    say(page, how, Role::Meta);
    page.blank();
    let right = vec![meta(format!(
        "{} redelivers the selected one",
        keys.redeliver
    ))];
    let none = "Nothing delivered yet.";
    let (deliveries, name) = (&h.deliveries, "Recent deliveries");
    counted_box(page, name, right, deliveries, none, |page, d| {
        let (mark, role) = if (200..300).contains(&d.code) {
            ("✓", Role::Success)
        } else {
            ("✗", Role::Error)
        };
        let event = match &d.action {
            Some(action) => format!("{}.{action}", d.event),
            None => d.event.clone(),
        };
        let when = time::ago_iso(&d.delivered_at, now);
        let about = meta(format!("  {} · {when}", short(&d.guid)));
        let mut right = vec![meta(format!("{} {}", d.code, d.status))];
        if d.redelivery {
            right.insert(0, meta("redelivery · "));
        }
        let mark = Seg::new(format!("{mark} "), role);
        let to = Link::Setting(SettingLink::Delivery(d.id));
        item(page, to, |page, link| {
            let segs = vec![mark, Seg::linked(event, Role::Strong, link), about];
            page.box_line(segs, right, 0);
        });
    });
}

/// A GUID's first part, as GitHub shows deliveries.
fn short(guid: &str) -> &str {
    guid.split('-').next().unwrap_or(guid)
}

/// A public key, its type and the ends of its text.
fn key_text(k: &DeployKey) -> String {
    let mut parts = k.key.split_whitespace();
    let kind = parts.next().unwrap_or_default();
    let body = parts.next().unwrap_or_default();
    let tail =
        (body.char_indices().rev().nth(7)).map_or(body, |(i, _)| body.get(i..).unwrap_or(body));
    format!("{kind} …{tail}")
}

fn deploy_keys(page: &mut Page, list: &Listed<DeployKey>, keys: SettingKeys<'_>, now: u64) {
    let right = add_button(page, "Add deploy key", keys);
    let none = "No deploy keys.";
    counted_box(page, "Deploy keys", right, list, none, |page, k| {
        let access = if k.read_only {
            chip("Read-only", Bg::SecondaryContainer)
        } else {
            chip("Read/write", Bg::TertiaryContainer)
        };
        let added = time::ago_iso(&k.created_at, now);
        let mut about = format!("{} · added {added}", key_text(k));
        if let Some(by) = &k.added_by {
            let _ = write!(about, " by {by}");
        }
        match &k.last_used {
            Some(at) => _ = write!(about, " · last used {}", time::ago_iso(at, now)),
            None => about.push_str(" · never used"),
        }
        let (to, about) = (SettingLink::Key(k.id), Some(about));
        setting_row(page, to, k.title.clone(), vec![access], about);
    });
}
