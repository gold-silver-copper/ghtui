//! A repository's Settings tab, as GitHub shows it to admins: its own
//! menu, then the page's lists, each row a [`SettingLink`] that the keys
//! of [`crate::settings`] act on.

use super::*;
use crate::api::model::NodeId;
use crate::api::settings::{
    Access, Branches, DeployKey, Enforcement, General, Hook, HookPage, Listed, Role as Given, Rule,
    Settings, SettingsPage,
};

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
}

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
    let base = format!("{}/settings", url::repo(repo));
    let mut segs = Vec::new();
    for item in SettingsPage::MENU {
        if !segs.is_empty() {
            segs.push(meta(" · "));
        }
        let role = if item == on.menu() {
            Role::Strong
        } else {
            Role::Link
        };
        let target = format!("{base}{}", item.slug());
        segs.push(link_seg(page, item.name(), target, role));
    }
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
    settings_nav(page, repo, at);
    if !admin {
        say(
            page,
            format!("You don't administer {repo}: GitHub shows its settings only to admins."),
            Role::Meta,
        );
        page.blank();
    }
    let Some(data) = data.show(page, &at.name().to_lowercase()) else {
        return;
    };
    match data {
        Settings::General(g) => general(page, repo, g, keys),
        Settings::Branches(b) => branches(page, b, keys),
        Settings::Access(a) => access(page, a, keys, now),
        Settings::Hooks(h) => hooks(page, h, keys, now),
        Settings::Hook(h) => hook(page, h, keys, now),
        Settings::Keys(k) => deploy_keys(page, k, keys, now),
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

/// A list's first page in a box titled `name` and how many it shows.
fn listed<T>(
    page: &mut Page,
    name: &str,
    right: Vec<Seg>,
    list: &Listed<T>,
    empty: &str,
    row: impl FnMut(&mut Page, &T),
) {
    let title = counted(name, list);
    list_box(page, title, right, list, empty, row);
}

fn general(page: &mut Page, repo: &RepoId, g: &General, keys: SettingKeys<'_>) {
    page.box_top(vec![strong("General")], Vec::new());
    let mut visibility = g.visibility.clone();
    if let Some(first) = visibility.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    for (name, value) in [
        ("Repository", repo.to_string()),
        ("Visibility", visibility),
        ("Default branch", g.default_branch.clone()),
        (
            "Archived",
            if g.archived {
                "yes: it's read-only".into()
            } else {
                "no".into()
            },
        ),
    ] {
        body(
            page,
            vec![meta(format!("{name:<14}  ")), Seg::new(value, Role::Body)],
        );
    }
    page.box_bottom();
    page.blank();
    if !g.admin {
        say(
            page,
            "You don't administer it: GitHub shows its settings only to admins.",
            Role::Meta,
        );
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
    item(page, Link::Setting(SettingLink::Archive), |page, link| {
        page.box_line(
            vec![Seg::linked(label, Role::Strong, link)],
            vec![meta(keys.toggle.to_owned())],
            0,
        );
        body(page, vec![meta(about)]);
    });
    page.box_rule();
    item(
        page,
        Link::Setting(SettingLink::DeleteRepo),
        |page, link| {
            body(
                page,
                vec![Seg::linked("Delete this repository", Role::Strong, link)],
            );
            body(
                page,
                vec![meta("On github.com only: ghtui never deletes a repository")],
            );
        },
    );
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
    let title = counted("Branch protection rules", &b.rules);
    let right = add_button(page, "Add rule", keys);
    list_box(
        page,
        title,
        right,
        &b.rules,
        "No branch protection rules: any branch can be pushed to, force-pushed and deleted.",
        |page, r| {
            item(
                page,
                Link::Setting(SettingLink::Rule(r.id.clone())),
                |page, link| {
                    let n = r.matching.total();
                    let applies = match n {
                        0 => "applies to no branch yet".to_owned(),
                        1 => "applies to 1 branch".to_owned(),
                        n => format!("applies to {n} branches"),
                    };
                    page.box_line(
                        vec![Seg::linked(r.form.pattern.clone(), Role::Strong, link)],
                        vec![meta(applies)],
                        0,
                    );
                    body(page, vec![meta(rule_meta(r))]);
                },
            );
        },
    );
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
    list_box(
        page,
        title,
        Vec::new(),
        &Whole(rulesets),
        empty,
        |page, r| {
            item(
                page,
                Link::Setting(SettingLink::Ruleset(r.id)),
                |page, link| {
                    let bg = match r.enforcement {
                        Enforcement::Active => Bg::SuccessContainer,
                        Enforcement::Evaluate => Bg::TertiaryContainer,
                        Enforcement::Disabled => Bg::SecondaryContainer,
                    };
                    page.box_line(
                        vec![Seg::linked(r.name.clone(), Role::Strong, link)],
                        vec![chip(r.enforcement.label(), bg)],
                        0,
                    );
                    let mut subline = format!("{} ruleset", r.target);
                    if let Some(org) = &r.org {
                        subline.push_str(&format!(" · from {org}"));
                    } else {
                        subline.push_str(&format!(" · {} turns it on or off", keys.toggle));
                    }
                    body(page, vec![meta(subline)]);
                },
            );
        },
    );
}

fn access(page: &mut Page, a: &Access, keys: SettingKeys<'_>, now: u64) {
    let right = add_button(page, "Add people", keys);
    listed(
        page,
        "Collaborators",
        right,
        &a.people,
        "Nobody has direct access.",
        |page, p| {
            let target = Link::Setting(SettingLink::Person(p.login.clone()));
            item(page, target, |page, link| {
                let bg = match &p.role {
                    Given::Is(crate::api::settings::Permission::Admin) => Bg::PrimaryContainer,
                    _ => Bg::SecondaryContainer,
                };
                page.box_line(
                    vec![Seg::linked(format!("@{}", p.login), Role::Strong, link)],
                    vec![chip(p.role.name(), bg)],
                    0,
                );
            });
        },
    );
    page.blank();
    let (invitations, none) = (&a.invitations, "No invitations waiting.");
    listed(
        page,
        "Pending invitations",
        Vec::new(),
        invitations,
        none,
        |page, i| {
            let target = Link::Setting(SettingLink::Invitation(i.id, i.login.clone()));
            item(page, target, |page, link| {
                let when = if i.expired {
                    "expired".to_owned()
                } else {
                    format!("invited {}", time::ago_iso(&i.created_at, now))
                };
                page.box_line(
                    vec![Seg::linked(format!("@{}", i.login), Role::Strong, link)],
                    vec![
                        meta(format!("{when}  ")),
                        chip(i.permission.name(), Bg::SecondaryContainer),
                    ],
                    0,
                );
            });
        },
    );
    page.blank();
    say(
        page,
        format!(
            "{} changes the selected one's role, {} removes them",
            keys.edit, keys.delete
        ),
        Role::Meta,
    );
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
    listed(page, "Webhooks", right, list, "No webhooks.", |page, h| {
        item(
            page,
            Link::Setting(SettingLink::Hook(h.id)),
            |page, link| {
                page.box_line(
                    vec![Seg::linked(h.form.url.clone(), Role::Strong, link)],
                    vec![active_chip(h.form.active)],
                    0,
                );
                let subline = format!(
                    "{} · {} · {} · changed {}",
                    events(h),
                    h.form.content_type,
                    last_said(h),
                    time::ago_iso(&h.updated_at, now)
                );
                body(page, vec![meta(subline)]);
            },
        );
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
    page.wrapped(
        vec![active_chip(hook.form.active), meta(subline)],
        0,
        Frame::None,
    );
    say(
        page,
        format!(
            "{} edits it, {} turns it on or off, {} deletes it",
            keys.edit, keys.toggle, keys.delete
        ),
        Role::Meta,
    );
    page.blank();
    let right = vec![meta(format!(
        "{} redelivers the selected one",
        keys.redeliver
    ))];
    let none = "Nothing delivered yet.";
    listed(
        page,
        "Recent deliveries",
        right,
        &h.deliveries,
        none,
        |page, d| {
            item(
                page,
                Link::Setting(SettingLink::Delivery(d.id)),
                |page, link| {
                    let ok = (200..300).contains(&d.code);
                    let (mark, role) = if ok {
                        ("✓", Role::Success)
                    } else {
                        ("✗", Role::Error)
                    };
                    let event = match &d.action {
                        Some(action) => format!("{}.{action}", d.event),
                        None => d.event.clone(),
                    };
                    let mut right = vec![meta(format!("{} {}", d.code, d.status))];
                    if d.redelivery {
                        right.insert(0, meta("redelivery · "));
                    }
                    page.box_line(
                        vec![
                            Seg::new(format!("{mark} "), role),
                            Seg::linked(event, Role::Strong, link),
                            meta(format!(
                                "  {} · {}",
                                short(&d.guid),
                                time::ago_iso(&d.delivered_at, now)
                            )),
                        ],
                        right,
                        0,
                    );
                },
            );
        },
    );
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
    let tail: String = (body
        .chars()
        .rev()
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev())
    .collect();
    format!("{kind} …{tail}")
}

fn deploy_keys(page: &mut Page, list: &Listed<DeployKey>, keys: SettingKeys<'_>, now: u64) {
    let right = add_button(page, "Add deploy key", keys);
    listed(
        page,
        "Deploy keys",
        right,
        list,
        "No deploy keys.",
        |page, k| {
            item(page, Link::Setting(SettingLink::Key(k.id)), |page, link| {
                let access = if k.read_only {
                    chip("Read-only", Bg::SecondaryContainer)
                } else {
                    chip("Read/write", Bg::TertiaryContainer)
                };
                page.box_line(
                    vec![Seg::linked(k.title.clone(), Role::Strong, link)],
                    vec![access],
                    0,
                );
                let mut subline = format!(
                    "{} · added {}",
                    key_text(k),
                    time::ago_iso(&k.created_at, now)
                );
                if let Some(by) = &k.added_by {
                    subline.push_str(&format!(" by {by}"));
                }
                match &k.last_used {
                    Some(at) => {
                        subline.push_str(&format!(" · last used {}", time::ago_iso(at, now)))
                    }
                    None => subline.push_str(" · never used"),
                }
                body(page, vec![meta(subline)]);
            });
        },
    );
}
