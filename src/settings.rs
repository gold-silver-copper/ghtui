//! A repository's Settings tab, for its admins: branch protection rules
//! and rulesets, collaborators, webhooks, deploy keys, and archiving.
//! These change a repository for everyone, so every change asks first
//! and names what it changes: a question ([`crate::forms::Ask`]) lists
//! it, line by line.
//! Rules, webhooks and deploy keys are written in the composer as a form
//! of `Name: value` lines ([`Form`]); a role is picked ([`Pick`]);
//! archiving asks twice, the second time for the repository's name, as
//! GitHub does. Deleting a repository is left to github.com. A change
//! goes to GitHub as a [`Change::Settings`], and the page follows it as
//! [`crate::act`] follows its own ([`shows`]).

use crate::api::change::Change;
use crate::api::model::{NodeId, RepoId};
use crate::api::settings::{
    Checks, Enforcement, HookForm, Permission, Reviews, Role as Given, RuleForm, Settings,
    SettingsChange, SettingsPage,
};
use crate::route::{Route, settings};
use crate::ui::overlays::PaletteItem;
use crate::ui::pages::{SettingKeys, SettingLink};

use crate::act::{Step, Subject};
use crate::data::DataKey;
use crate::keymap::Action;
use crate::nav::Doable;
use crate::state::{Cmds, State};
use crate::ui::page::{Link, Page};

/// The actions this module takes as its own.
pub const ACTIONS: [Action; 5] = [
    Action::AddSetting,
    Action::EditSetting,
    Action::DeleteSetting,
    Action::ToggleSetting,
    Action::Redeliver,
];

// ---- where you are ------------------------------------------------------------------

/// The settings page on screen.
fn here(state: &State) -> Option<(RepoId, SettingsPage)> {
    match state.route()? {
        Route::Settings(settings::Page { repo, page }) => Some((repo.clone(), *page)),
        _ => None,
    }
}

fn data<'a>(state: &'a State, repo: &RepoId, page: SettingsPage) -> Option<&'a Settings> {
    state.picked::<Settings>(&DataKey::Settings(repo.clone(), page))
}

/// The row selected on the page.
fn selected(state: &State) -> Option<SettingLink> {
    let p = state.screen().page()?;
    match p.selected_link()? {
        Link::Setting(s) => Some(s.clone()),
        _ => None,
    }
}

const LOADING: &str = "It's still loading";

// ---- what a key does --------------------------------------------------------------

/// A confirmation of one change.
fn ask(
    action: Action,
    at: (&RepoId, SettingsPage),
    title: String,
    facts: Vec<(String, bool)>,
    choice: (&str, SettingsChange),
) -> Step {
    let about = Subject::Settings(at.0.clone(), at.1);
    let change = Change::Settings(choice.1);
    crate::act::ask_one(action, about, title, facts, choice.0, change)
}

fn elsewhere(action: Action) -> String {
    match action {
        Action::Redeliver => "Redelivering works on a webhook's delivery, on its page".into(),
        _ => "That works in a repository's Settings tab".into(),
    }
}

fn plan(state: &State, action: Action) -> Result<Step, String> {
    let (repo, page) = here(state).ok_or_else(|| elsewhere(action))?;
    if state.overview(&repo).is_some_and(|o| !o.can_admin) {
        return Err(format!(
            "Changing {repo}'s settings takes admin access, which GitHub doesn't give you"
        ));
    }
    crate::act::settled(state)?;
    let settings = data(state, &repo, page).ok_or(LOADING)?;
    let at = (&repo, page);
    let row = selected(state);
    let select = |what: &str| {
        let next = state.first_key(Action::Down);
        format!("Select {what} first: {next} goes row by row")
    };
    match (action, settings) {
        (Action::AddSetting, Settings::Branches(b)) => {
            let form = Form::Rule {
                repo: repo.clone(),
                repo_id: b.repo_id.clone(),
                rule: None,
            };
            write(form, rule_text(&RuleForm::default()))
        }
        (Action::AddSetting, Settings::Access(_)) => pick(Pick::Invite { repo }),
        (Action::AddSetting, Settings::Hooks(_) | Settings::Hook(_)) => {
            let form = Form::Hook { repo, hook: None };
            write(form, hook_text(&HookForm::default()))
        }
        (Action::AddSetting, Settings::Keys(_)) => write(
            Form::Key { repo },
            key_text("", "", false),
        ),
        (Action::AddSetting, Settings::General(_)) => Err(
            "Adding works on Collaborators, Branches, Webhooks and Deploy keys: the menu above goes there"
                .into(),
        ),
        (Action::EditSetting | Action::DeleteSetting, Settings::General(_)) => Err(
            "Deleting a repository is done on github.com: ghtui never deletes one".into(),
        ),
        (Action::EditSetting | Action::DeleteSetting | Action::ToggleSetting, Settings::Branches(b)) => {
            match row {
                Some(SettingLink::Rule(id)) => {
                    let rule = (b.rules.iter())
                        .find(|r| r.id == id)
                        .ok_or("That rule is gone")?;
                    match action {
                        Action::EditSetting => {
                            let form = Form::Rule {
                                repo: repo.clone(),
                                repo_id: b.repo_id.clone(),
                                rule: Some(Box::new((
                                    rule.id.clone(),
                                    rule.form.clone(),
                                    rule.apps.clone(),
                                ))),
                            };
                            write(form, rule_text(&rule.form))
                        }
                        Action::DeleteSetting => {
                            let pattern = &rule.form.pattern;
                            let mut facts = vec![(
                                format!("Branches matching {pattern} are then unprotected: anyone with write access can push to them"),
                                true,
                            )];
                            if !rule.matching.is_empty() {
                                let mut names = rule.matching.join(", ");
                                if rule.matching.left_out() > 0 {
                                    names.push_str(&format!(" and {} more", rule.matching.left_out()));
                                }
                                facts.push((format!("It applies to {names}"), false));
                            }
                            Ok(ask(
                                action,
                                at,
                                format!("Delete the protection rule for {pattern}?"),
                                facts,
                                (
                                    "Delete rule",
                                    SettingsChange::DeleteRule {
                                        id: rule.id.clone(),
                                        pattern: pattern.clone(),
                                    },
                                ),
                            ))
                        }
                        _ => Err("A protection rule is on while it's there: edit it, or delete it".into()),
                    }
                }
                Some(SettingLink::Ruleset(id)) => {
                    let rulesets = b.rulesets.as_ref().map_err(Clone::clone)?;
                    let r = rulesets.iter().find(|r| r.id == id).ok_or("That ruleset is gone")?;
                    if let Some(org) = &r.org {
                        return Err(format!(
                            "It's {org}'s ruleset: it's changed in the organization's settings"
                        ));
                    }
                    if action != Action::ToggleSetting {
                        let toggle = state.first_key(Action::ToggleSetting);
                        return Err(format!(
                            "A ruleset's rules are edited on github.com; {toggle} turns it on or off"
                        ));
                    }
                    let targets = match r.target.as_str() {
                        "tag" => "tags",
                        "push" => "pushes",
                        _ => "branches",
                    };
                    let (to, verb, fact) = match r.enforcement {
                        Enforcement::Active | Enforcement::Evaluate => (
                            Enforcement::Disabled,
                            "Disable",
                            format!("Its rules stop applying to the {targets} it targets"),
                        ),
                        Enforcement::Disabled => (
                            Enforcement::Active,
                            "Enable",
                            format!("Its rules apply to the {targets} it targets again"),
                        ),
                    };
                    Ok(ask(
                        action,
                        at,
                        format!("{verb} ruleset “{}”?", r.name),
                        vec![(fact, to == Enforcement::Disabled)],
                        (
                            verb,
                            SettingsChange::SetRuleset {
                                repo: repo.clone(),
                                id: r.id,
                                name: r.name.clone(),
                                enforcement: to,
                            },
                        ),
                    ))
                }
                _ => Err(select("a rule or a ruleset")),
            }
        }
        (Action::EditSetting | Action::DeleteSetting, Settings::Access(a)) => {
            let (login, invitation) = match row {
                Some(SettingLink::Person(login)) => (login, None),
                Some(SettingLink::Invitation(id, login)) => (login, Some(id)),
                _ => return Err(select("someone")),
            };
            let person = a.people.items.iter().find(|p| p.login == login);
            if login.eq_ignore_ascii_case(&repo.owner) {
                return Err(format!("@{login} owns {repo}: their access can't change"));
            }
            let now = match (person, invitation) {
                (_, Some(id)) => (a.invitations.items.iter())
                    .find(|i| i.id == id)
                    .map(|i| Given::Is(i.permission)),
                (Some(p), None) => Some(p.role.clone()),
                (None, None) => None,
            }
            .ok_or("They're gone from the list")?;
            match (action, invitation) {
                (Action::EditSetting, _) => pick(Pick::Role {
                    repo,
                    who: match invitation {
                        Some(id) => Who::Invitation(id, login),
                        None => Who::Person(login),
                    },
                    now: Some(now),
                }),
                (_, Some(id)) => Ok(ask(
                    action,
                    at,
                    format!("Cancel the invitation to @{login}?"),
                    vec![(format!("They won't be able to join {repo} with it"), false)],
                    (
                        "Cancel invitation",
                        SettingsChange::CancelInvitation {
                            repo: repo.clone(),
                            id,
                            login: login.clone(),
                        },
                    ),
                )),
                (_, None) => Ok(ask(
                    action,
                    at,
                    format!("Remove @{login} from {repo}?"),
                    vec![(
                        format!("They lose their {} access to it", now.name()),
                        true,
                    )],
                    (
                        "Remove",
                        SettingsChange::RemoveCollaborator {
                            repo: repo.clone(),
                            login: login.clone(),
                        },
                    ),
                )),
            }
        }
        (Action::EditSetting | Action::DeleteSetting | Action::ToggleSetting, Settings::Hooks(_) | Settings::Hook(_)) => {
            let hook = match (settings, row) {
                (Settings::Hook(h), _) => &h.hook,
                (Settings::Hooks(list), Some(SettingLink::Hook(id))) => list
                    .items
                    .iter()
                    .find(|h| h.id == id)
                    .ok_or("That webhook is gone")?,
                _ => return Err(select("a webhook")),
            };
            let url = hook.form.url.clone();
            match action {
                Action::EditSetting => write(
                    Form::Hook {
                        repo,
                        hook: Some(Box::new((hook.id, hook.form.clone()))),
                    },
                    hook_text(&hook.form),
                ),
                Action::DeleteSetting => Ok(ask(
                    action,
                    // From its own page, the list follows it.
                    (&repo, SettingsPage::Hooks),
                    "Delete this webhook?".into(),
                    vec![
                        (url.clone(), false),
                        (
                            "GitHub stops sending it events, and its deliveries go with it".into(),
                            true,
                        ),
                    ],
                    (
                        "Delete webhook",
                        SettingsChange::DeleteHook {
                            repo: repo.clone(),
                            id: hook.id,
                            url,
                        },
                    ),
                )),
                _ => {
                    let active = !hook.form.active;
                    let (verb, fact) = if active {
                        ("Enable", "GitHub sends it events again")
                    } else {
                        ("Disable", "GitHub stops sending it events until it's enabled again")
                    };
                    let to = HookForm {
                        active,
                        secret: None,
                        ..hook.form.clone()
                    };
                    Ok(ask(
                        action,
                        at,
                        format!("{verb} this webhook?"),
                        vec![(url, false), (fact.into(), !active)],
                        (
                            verb,
                            SettingsChange::UpdateHook {
                                repo: repo.clone(),
                                id: hook.id,
                                from: Box::new(hook.form.clone()),
                                to: Box::new(to),
                            },
                        ),
                    ))
                }
            }
        }
        (Action::Redeliver, Settings::Hook(h)) => {
            let Some(SettingLink::Delivery(id)) = row else {
                return Err(select("a delivery"));
            };
            let d = (h.deliveries.items.iter())
                .find(|d| d.id == id)
                .ok_or("That delivery is gone")?;
            Ok(ask(
                action,
                at,
                format!("Redeliver the {} delivery?", d.event),
                vec![(
                    format!("GitHub sends it to {} again, as it was", h.hook.form.url),
                    false,
                )],
                (
                    "Redeliver",
                    SettingsChange::Redeliver {
                        repo: repo.clone(),
                        hook: h.hook.id,
                        delivery: d.id,
                        guid: d.guid.clone(),
                    },
                ),
            ))
        }
        (Action::EditSetting, Settings::Keys(_)) => Err(
            "GitHub doesn't change a deploy key: delete it and add it again".into(),
        ),
        (Action::DeleteSetting, Settings::Keys(list)) => {
            let Some(SettingLink::Key(id)) = row else {
                return Err(select("a deploy key"));
            };
            let k = list.items.iter().find(|k| k.id == id).ok_or("That key is gone")?;
            Ok(ask(
                action,
                at,
                format!("Delete deploy key “{}”?", k.title),
                vec![(
                    format!("Whatever uses it can no longer reach {repo}"),
                    true,
                )],
                (
                    "Delete key",
                    SettingsChange::DeleteKey {
                        repo: repo.clone(),
                        id: k.id,
                        title: k.title.clone(),
                    },
                ),
            ))
        }
        (Action::ToggleSetting, Settings::General(g)) if !g.admin => Err(format!(
            "Archiving {repo} takes admin access, which GitHub doesn't give you"
        )),
        (Action::ToggleSetting, Settings::General(g)) => pick(Pick::Archive {
            repo,
            archived: !g.archived,
        }),
        (Action::ToggleSetting, _) => Err(
            "That turns a ruleset or a webhook on or off, and on General, archives the repository"
                .into(),
        ),
        (Action::Redeliver, _) => Err(elsewhere(action)),
        _ => Err("Nothing here to change that way".into()),
    }
}

/// What a repository's settings add: their own actions.
pub const FEATURE: crate::features::Feature = crate::features::Feature {
    owns: &ACTIONS,
    act,
    offers: doables,
    ..crate::features::Feature::NONE
};

/// Runs one of [`ACTIONS`].
pub fn act(state: &mut State, action: Action) -> Cmds {
    crate::act::take(state, plan(state, action))
}

/// Choosing in one of this module's pickers.
#[expect(clippy::unnecessary_wraps, reason = "a plan's step")]
fn pick(pick: Pick) -> Result<Step, String> {
    Ok(Step::Pick(crate::picker::Kind::Settings(pick)))
}

/// Writing a form's text in the composer.
#[expect(clippy::unnecessary_wraps, reason = "a plan's step")]
fn write(form: Form, text: String) -> Result<Step, String> {
    Ok(Step::Compose(crate::compose::Target::Setting(form), text))
}

/// The menu's rows on a settings page.
pub fn doables(state: &State, out: &mut Vec<Doable>) {
    let Some((repo, page)) = here(state) else { return };
    let row = |action, label: &str| {
        // The status bar names the page's main ones.
        let short = match action {
            Action::AddSetting => "add",
            Action::EditSetting => "edit",
            Action::Redeliver => "redeliver",
            _ => "",
        };
        crate::nav::row("Change", action, label, short, plan(state, action).err())
    };
    let archived = matches!(data(state, &repo, page), Some(Settings::General(g)) if g.archived);
    let rows: &[(Action, &str)] = match page {
        SettingsPage::General => &[(
            Action::ToggleSetting,
            if archived {
                "Unarchive the repository"
            } else {
                "Archive the repository"
            },
        )],
        SettingsPage::Branches => &[
            (Action::AddSetting, "Add a branch protection rule"),
            (Action::EditSetting, "Edit the rule"),
            (Action::DeleteSetting, "Delete the rule"),
            (Action::ToggleSetting, "Turn the ruleset on or off"),
        ],
        SettingsPage::Access => &[
            (Action::AddSetting, "Invite a collaborator"),
            (Action::EditSetting, "Change their role"),
            (
                Action::DeleteSetting,
                "Remove them, or cancel the invitation",
            ),
        ],
        SettingsPage::Hooks => &[
            (Action::AddSetting, "Add a webhook"),
            (Action::EditSetting, "Edit the webhook"),
            (Action::ToggleSetting, "Turn the webhook on or off"),
            (Action::DeleteSetting, "Delete the webhook"),
        ],
        SettingsPage::Hook(_) => &[
            (Action::EditSetting, "Edit the webhook"),
            (Action::ToggleSetting, "Turn the webhook on or off"),
            (Action::DeleteSetting, "Delete the webhook"),
            (Action::Redeliver, "Redeliver the delivery"),
        ],
        SettingsPage::Keys => &[
            (Action::AddSetting, "Add a deploy key"),
            (Action::DeleteSetting, "Delete the deploy key"),
        ],
    };
    out.extend(rows.iter().map(|(action, label)| row(*action, label)));
}

/// Following one of the page's rows.
pub fn follow_link(state: &mut State, link: &SettingLink) -> Cmds {
    match link {
        SettingLink::Add => act(state, Action::AddSetting),
        SettingLink::Rule(_) => act(state, Action::EditSetting),
        SettingLink::Archive => act(state, Action::ToggleSetting),
        SettingLink::Person(login) | SettingLink::Invitation(_, login) => {
            state.push(Route::user(login))
        }
        SettingLink::Hook(id) => match here(state) {
            Some((repo, _)) => state.push(Route::Settings(settings::Page {
                repo,
                page: SettingsPage::Hook(*id),
            })),
            None => Cmds::new(),
        },
        SettingLink::Ruleset(_) | SettingLink::Delivery(_) | SettingLink::Key(_) => {
            let (action, what) = match link {
                SettingLink::Ruleset(_) => (
                    Action::ToggleSetting,
                    "turns the ruleset on or off; its rules are edited on github.com",
                ),
                SettingLink::Delivery(_) => (Action::Redeliver, "redelivers it"),
                _ => (Action::DeleteSetting, "deletes the key"),
            };
            let key = state.first_key(action);
            state.just_say(format!("{key} {what}"))
        }
        SettingLink::DeleteRepo => {
            let open = state.first_key(Action::OpenInBrowser);
            state.just_say(format!(
                "ghtui never deletes a repository: that's done on github.com ({open} opens this page there)"
            ))
        }
    }
}

/// What following a row does, for the menu.
pub fn describe(link: &SettingLink) -> &'static str {
    match link {
        SettingLink::Add => "Add one",
        SettingLink::Rule(_) => "Edit the rule",
        SettingLink::Archive => "Archive or unarchive the repository",
        SettingLink::Person(_) | SettingLink::Invitation(..) => "Open their profile",
        SettingLink::Hook(_) => "Open the webhook and its deliveries",
        SettingLink::Ruleset(_) => "Say how to turn the ruleset on or off",
        SettingLink::Delivery(_) => "Say how to redeliver it",
        SettingLink::Key(_) => "Say how to delete the key",
        SettingLink::DeleteRepo => "Say where a repository is deleted",
    }
}

/// Builds a settings page.
pub fn build(
    state: &State,
    page: &mut Page,
    repo: &RepoId,
    at: SettingsPage,
    data: crate::ui::Fetched<'_, Settings>,
    now: u64,
) {
    let keys = ACTIONS.map(|a| state.first_key(a));
    let [add, edit, delete, toggle, redeliver] = &keys;
    let keys = SettingKeys {
        add,
        edit,
        delete,
        toggle,
        redeliver,
    };
    let admin = state.overview(repo).is_none_or(|o| o.can_admin);
    crate::ui::pages::settings_page(page, repo, at, data, keys, admin, now);
}

// ---- forms in the composer ----------------------------------------------------------

/// A form written in the composer: a rule, a webhook or a deploy key, new
/// or (for a rule or a webhook) one being edited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Form {
    Rule {
        repo: RepoId,
        repo_id: NodeId,
        /// The rule edited: its ID, what it asks, its checks' apps.
        rule: Option<Box<Editing>>,
    },
    Hook {
        repo: RepoId,
        hook: Option<Box<(u64, HookForm)>>,
    },
    Key {
        repo: RepoId,
    },
}

/// A rule being edited: its ID, what it asks, its checks' apps.
pub type Editing = (NodeId, RuleForm, Vec<(String, NodeId)>);

impl Form {
    pub fn title(&self) -> String {
        match self {
            Form::Rule { rule: None, .. } => {
                "New branch protection rule: yes or no after each, checks by name".into()
            }
            Form::Rule { rule: Some(r), .. } => format!(
                "The protection rule for {}: yes or no after each, checks by name",
                r.1.pattern
            ),
            Form::Hook { hook: None, .. } => "New webhook: events by name, * for all".into(),
            Form::Hook { hook: Some(_), .. } => {
                "The webhook: events by name, * for all; an empty secret keeps it".into()
            }
            Form::Key { .. } => "New deploy key: its title, then the public key".into(),
        }
    }

    pub fn save(&self) -> &'static str {
        match self {
            Form::Rule { rule: None, .. } | Form::Hook { hook: None, .. } => "create…",
            Form::Key { .. } => "add…",
            _ => "save…",
        }
    }

    fn repo(&self) -> &RepoId {
        match self {
            Form::Rule { repo, .. } | Form::Hook { repo, .. } | Form::Key { repo } => repo,
        }
    }

    fn page(&self) -> SettingsPage {
        match self {
            Form::Rule { .. } => SettingsPage::Branches,
            Form::Hook { hook: Some(h), .. } => SettingsPage::Hook(h.0),
            Form::Hook { hook: None, .. } => SettingsPage::Hooks,
            Form::Key { .. } => SettingsPage::Keys,
        }
    }
}

fn yes(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

const PATTERN: &str = "Branch name pattern";
const PR: &str = "Require a pull request before merging";
const APPROVALS: &str = "Required approvals";
const STALE: &str = "Dismiss stale approvals when new commits are pushed";
const OWNERS: &str = "Require review from Code Owners";
const LAST_PUSH: &str = "Require approval of the most recent push";
const CHECKS: &str = "Require status checks to pass";
const STRICT: &str = "Require branches to be up to date before merging";
const CONTEXTS: &str = "Status checks";
const CONVERSATIONS: &str = "Require conversation resolution before merging";
const SIGNED: &str = "Require signed commits";
const LINEAR: &str = "Require linear history";
const LOCK: &str = "Lock branch";
const ADMINS: &str = "Do not allow bypassing the above settings";
const FORCE: &str = "Allow force pushes";
const DELETIONS: &str = "Allow deletions";

/// A rule as the form's lines.
fn rule_lines(f: &RuleForm) -> Vec<(&'static str, String)> {
    let reviews = f.reviews.clone().unwrap_or_default();
    let checks = f.checks.clone().unwrap_or_default();
    vec![
        (PATTERN, f.pattern.clone()),
        (PR, yes(f.reviews.is_some()).into()),
        (APPROVALS, reviews.count.to_string()),
        (STALE, yes(reviews.dismiss_stale).into()),
        (OWNERS, yes(reviews.code_owners).into()),
        (LAST_PUSH, yes(reviews.last_push).into()),
        (CHECKS, yes(f.checks.is_some()).into()),
        (STRICT, yes(checks.strict).into()),
        (CONTEXTS, checks.contexts.join(", ")),
        (CONVERSATIONS, yes(f.conversations).into()),
        (SIGNED, yes(f.signed).into()),
        (LINEAR, yes(f.linear).into()),
        (LOCK, yes(f.lock).into()),
        (ADMINS, yes(f.admins).into()),
        (FORCE, yes(f.force_pushes).into()),
        (DELETIONS, yes(f.deletions).into()),
    ]
}

fn text_of(lines: &[(&str, String)]) -> String {
    let lines: Vec<String> = lines
        .iter()
        .map(|(name, value)| format!("{name}: {value}"))
        .collect();
    lines.join("\n")
}

pub fn rule_text(f: &RuleForm) -> String {
    text_of(&rule_lines(f))
}

/// The form's `Name: value` lines, by name (as written in any case), each
/// once.
fn fields(text: &str, names: &[&'static str]) -> Result<Vec<(&'static str, String)>, String> {
    let mut found: Vec<(&'static str, String)> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(format!("Line {}: write it as “Name: value”", n + 1));
        };
        let name = name.trim();
        let Some(&known) = (names.iter()).find(|k| k.eq_ignore_ascii_case(name)) else {
            return Err(format!("“{name}” isn't one of the form's settings"));
        };
        if found.iter().any(|(k, _)| *k == known) {
            return Err(format!("“{known}” is there twice"));
        }
        found.push((known, value.trim().to_owned()));
    }
    Ok(found)
}

fn flag(name: &str, value: &str) -> Result<bool, String> {
    match value.to_ascii_lowercase().as_str() {
        "yes" | "y" | "true" | "on" => Ok(true),
        "no" | "n" | "false" | "off" | "" => Ok(false),
        _ => Err(format!("{name}: yes or no, not “{value}”")),
    }
}

/// The value given for `name`, else `default`.
fn given<'a>(found: &'a [(&str, String)], name: &str, default: &'a str) -> &'a str {
    (found.iter())
        .find(|(k, _)| *k == name)
        .map_or(default, |(_, v)| v.as_str())
}

/// A form's text read against its `lines`: each field's value, or what
/// it was in `lines` when it isn't written.
fn read(text: &str, lines: Vec<(&'static str, String)>) -> Result<impl Fn(&str) -> String, String> {
    let names: Vec<&'static str> = lines.iter().map(|(n, _)| *n).collect();
    let found = fields(text, &names)?;
    Ok(move |name: &str| {
        let default = (lines.iter())
            .find(|(n, _)| *n == name)
            .map_or("", |(_, v)| v.as_str());
        given(&found, name, default).to_owned()
    })
}

fn list(value: &str) -> Vec<String> {
    (value.split(','))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// A rule from the form's text; what isn't written stays as in `base`.
pub fn parse_rule(text: &str, base: &RuleForm) -> Result<RuleForm, String> {
    let get = read(text, rule_lines(base))?;
    let on = |name: &str| flag(name, &get(name));
    let pattern = get(PATTERN);
    if pattern.is_empty() {
        return Err(format!("{PATTERN}: which branches, as main or release/*"));
    }
    let count = get(APPROVALS);
    let count: u8 = count
        .parse()
        .ok()
        .filter(|n| *n <= 6)
        .ok_or_else(|| format!("{APPROVALS}: 0 to 6, not “{count}”"))?;
    Ok(RuleForm {
        pattern,
        reviews: on(PR)?
            .then(|| -> Result<Reviews, String> {
                Ok(Reviews {
                    count,
                    dismiss_stale: on(STALE)?,
                    code_owners: on(OWNERS)?,
                    last_push: on(LAST_PUSH)?,
                })
            })
            .transpose()?,
        checks: on(CHECKS)?
            .then(|| -> Result<Checks, String> {
                Ok(Checks {
                    strict: on(STRICT)?,
                    contexts: list(&get(CONTEXTS)),
                })
            })
            .transpose()?,
        conversations: on(CONVERSATIONS)?,
        signed: on(SIGNED)?,
        linear: on(LINEAR)?,
        lock: on(LOCK)?,
        admins: on(ADMINS)?,
        force_pushes: on(FORCE)?,
        deletions: on(DELETIONS)?,
    })
}

const URL: &str = "Payload URL";
const CONTENT: &str = "Content type";
const SECRET: &str = "Secret";
const SSL: &str = "SSL verification";
const EVENTS: &str = "Events";
const ACTIVE: &str = "Active";

fn hook_lines(h: &HookForm) -> Vec<(&'static str, String)> {
    vec![
        (URL, h.url.clone()),
        (CONTENT, h.content_type.clone()),
        (SECRET, h.secret.clone().unwrap_or_default()),
        (SSL, yes(h.verify_ssl).into()),
        (EVENTS, h.events.join(", ")),
        (ACTIVE, yes(h.active).into()),
    ]
}

/// A webhook's form; its secret, never shown, is left empty.
pub fn hook_text(h: &HookForm) -> String {
    text_of(&hook_lines(h))
}

pub fn parse_hook(text: &str, base: &HookForm) -> Result<HookForm, String> {
    let get = read(text, hook_lines(base))?;
    let url = get(URL);
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(format!(
            "{URL}: where GitHub sends events, an http(s):// address"
        ));
    }
    let content_type = get(CONTENT).to_ascii_lowercase();
    if !matches!(content_type.as_str(), "json" | "form") {
        return Err(format!("{CONTENT}: json or form, not “{content_type}”"));
    }
    let events = list(&get(EVENTS));
    if events.is_empty() {
        return Err(format!(
            "{EVENTS}: push, pull_request… or * for every event"
        ));
    }
    let secret = get(SECRET);
    Ok(HookForm {
        url,
        content_type,
        secret: (!secret.is_empty()).then_some(secret),
        verify_ssl: flag(SSL, &get(SSL))?,
        events,
        active: flag(ACTIVE, &get(ACTIVE))?,
    })
}

const TITLE: &str = "Title";
const KEY: &str = "Key";
const WRITE: &str = "Allow write access";

pub fn key_text(title: &str, key: &str, write: bool) -> String {
    text_of(&[
        (TITLE, title.to_owned()),
        (WRITE, yes(write).into()),
        (KEY, key.to_owned()),
    ])
}

/// A deploy key's title, its key and whether it may push.
pub fn parse_key(text: &str) -> Result<(String, String, bool), String> {
    let found = fields(text, &[TITLE, KEY, WRITE])?;
    let title = given(&found, TITLE, "").to_owned();
    let key = given(&found, KEY, "").to_owned();
    if title.is_empty() {
        return Err(format!("{TITLE}: what it's for"));
    }
    let kinds = ["ssh-", "ecdsa-", "sk-"];
    if !kinds.iter().any(|k| key.starts_with(k)) {
        return Err(format!(
            "{KEY}: a public key, as ssh-ed25519 AAAA… (the .pub file's line)"
        ));
    }
    let write = flag(WRITE, given(&found, WRITE, "no"))?;
    Ok((title, key, write))
}

/// The lines that differ: `Name: old → new`.
fn differences(old: &[(&str, String)], new: &[(&str, String)]) -> Vec<(String, bool)> {
    (old.iter().zip(new))
        .filter(|((_, a), (_, b))| a != b)
        .map(|((name, a), (_, b))| {
            let a = if a.is_empty() { "(none)" } else { a };
            let b = if b.is_empty() { "(none)" } else { b };
            (format!("{name}: {a} → {b}"), false)
        })
        .collect()
}

/// What a form asks GitHub: the confirmation naming what changes, or why
/// the form can't be sent.
fn confirm(form: &Form, text: &str) -> Result<crate::forms::Ask, String> {
    let repo = form.repo().clone();
    let (action, title, facts, choice, change) = match form {
        Form::Rule { repo_id, rule, .. } => match rule {
            None => {
                let to = parse_rule(text, &RuleForm::default())?;
                let facts = differences(&rule_lines(&RuleForm::default()), &rule_lines(&to))
                    .into_iter()
                    .filter(|(line, _)| !line.starts_with(PATTERN))
                    .collect();
                (
                    Action::AddSetting,
                    format!("Protect branches matching {}?", to.pattern),
                    facts,
                    "Create rule",
                    SettingsChange::CreateRule {
                        repo_id: repo_id.clone(),
                        form: to,
                    },
                )
            }
            Some(r) => {
                let (id, from, apps) = &**r;
                let to = parse_rule(text, from)?;
                let facts = differences(&rule_lines(from), &rule_lines(&to));
                if facts.is_empty() {
                    return Err("Nothing changed".into());
                }
                (
                    Action::EditSetting,
                    format!("Change the protection rule for {}?", from.pattern),
                    facts,
                    "Save rule",
                    SettingsChange::UpdateRule {
                        id: id.clone(),
                        from: Box::new(from.clone()),
                        to: Box::new(to),
                        apps: apps.clone(),
                    },
                )
            }
        },
        Form::Hook { hook, .. } => {
            let base = hook
                .as_ref()
                .map_or_else(HookForm::default, |h| h.1.clone());
            let to = parse_hook(text, &base)?;
            let shown = |h: &HookForm| {
                let mut lines = hook_lines(h);
                lines.retain(|(name, _)| *name != SECRET);
                lines
            };
            let mut facts = match hook {
                None => vec![
                    (format!("{URL}: {}", to.url), false),
                    (format!("{EVENTS}: {}", to.events.join(", ")), false),
                    (format!("{CONTENT}: {}", to.content_type), false),
                ],
                Some(_) => differences(&shown(&base), &shown(&to)),
            };
            if to.secret.is_some() {
                facts.push(("A new secret".into(), false));
            }
            if !to.verify_ssl {
                facts.push((
                    "Its certificate isn't checked: deliveries can be read on the way".into(),
                    true,
                ));
            }
            match hook {
                None => (
                    Action::AddSetting,
                    format!("Add a webhook to {repo}?"),
                    facts,
                    "Add webhook",
                    SettingsChange::CreateHook {
                        repo: repo.clone(),
                        form: to,
                    },
                ),
                Some(h) => {
                    if facts.is_empty() {
                        return Err("Nothing changed".into());
                    }
                    (
                        Action::EditSetting,
                        "Change this webhook?".into(),
                        facts,
                        "Save webhook",
                        SettingsChange::UpdateHook {
                            repo: repo.clone(),
                            id: h.0,
                            from: Box::new(base),
                            to: Box::new(to),
                        },
                    )
                }
            }
        }
        Form::Key { .. } => {
            let (title, key, write) = parse_key(text)?;
            let mut parts = key.split_whitespace();
            let kind = parts.next().unwrap_or_default();
            let mut facts = vec![(format!("{kind} key"), false)];
            if write {
                facts.push((format!("With write access: it can push to {repo}"), true));
            } else {
                facts.push(("Read-only: it can clone, not push".into(), false));
            }
            (
                Action::AddSetting,
                format!("Add deploy key “{title}” to {repo}?"),
                facts,
                "Add key",
                SettingsChange::AddKey {
                    repo: repo.clone(),
                    title,
                    key,
                    write,
                },
            )
        }
    };
    Ok(crate::act::confirm(
        action,
        Subject::Settings(repo, form.page()),
        title,
        facts,
        vec![(choice.to_owned(), Change::Settings(change))],
    ))
}

/// ctrl-s in the composer: asks about what the form changes.
pub fn submit(state: &mut State, form: &Form, text: &str) -> Cmds {
    match confirm(form, text) {
        Err(why) => crate::forms::say(state, why),
        Ok(mut confirm) => {
            let target = crate::compose::Target::Setting(form.clone());
            confirm.back = Some((target, text.to_owned()));
            crate::act::take(state, Ok(crate::act::Step::Ask(Box::new(confirm))))
        }
    }
}

/// After GitHub took a change: a webhook deleted from its own page goes
/// back to the list.
pub fn after(state: &mut State, change: &Change) -> Cmds {
    let Change::Settings(SettingsChange::DeleteHook { repo, id, .. }) = change else {
        return Cmds::new();
    };
    let on_it = Route::Settings(settings::Page {
        repo: repo.clone(),
        page: SettingsPage::Hook(*id),
    });
    if state.route() != Some(&on_it) {
        return Cmds::new();
    }
    let list = Route::Settings(settings::Page {
        repo: repo.clone(),
        page: SettingsPage::Hooks,
    });
    state.replace(list, true, None)
}

// ---- pickers --------------------------------------------------------------------

/// Whose role is chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Who {
    /// Someone to invite.
    New(String),
    Person(String),
    Invitation(u64, String),
}

impl Who {
    fn login(&self) -> &str {
        match self {
            Who::New(l) | Who::Person(l) | Who::Invitation(_, l) => l,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// Someone to invite, typed.
    Invite { repo: RepoId },
    /// A role for someone.
    Role {
        repo: RepoId,
        who: Who,
        now: Option<Given>,
    },
    /// The repository's name, typed to archive or unarchive it.
    Archive { repo: RepoId, archived: bool },
}

impl Pick {
    pub fn title(&self) -> &'static str {
        match self {
            Pick::Invite { .. } => "Invite a collaborator",
            Pick::Role {
                who: Who::New(_), ..
            } => "Invite them as",
            Pick::Role { .. } => "Change their role to",
            Pick::Archive { archived: true, .. } => "Archive the repository?",
            Pick::Archive { .. } => "Unarchive the repository?",
        }
    }

    pub fn placeholder(&self) -> &'static str {
        match self {
            Pick::Invite { .. } => "Their GitHub login",
            Pick::Role { .. } => "Choose a role",
            Pick::Archive { .. } => "Type its owner/name to confirm",
        }
    }
}

pub fn rows(pick: &Pick, q: &str) -> Vec<(PaletteItem, Option<Step>)> {
    match pick {
        Pick::Invite { repo } => {
            let login = q.trim().trim_start_matches('@');
            if login.is_empty() {
                return vec![(crate::picker::item("Type their GitHub login", ""), None)];
            }
            if !crate::route::valid_login(login) {
                return vec![(
                    crate::picker::item(format!("“{login}” isn't a GitHub login"), ""),
                    None,
                )];
            }
            let next = Pick::Role {
                repo: repo.clone(),
                who: Who::New(login.to_owned()),
                now: None,
            };
            vec![(
                crate::picker::item(format!("Invite @{login}"), "then choose their role"),
                Some(Step::Pick(crate::picker::Kind::Settings(next))),
            )]
        }
        Pick::Role { repo, who, now } => (Permission::ALL.into_iter())
            .filter(|p| crate::picker::fuzzy_score(q, p.name()).is_some())
            .map(|p| {
                let is_now = now.as_ref() == Some(&Given::Is(p));
                let label = if is_now {
                    format!("✓ {} (now)", p.name())
                } else {
                    p.name().to_owned()
                };
                let row = crate::picker::item(label, p.about());
                if is_now {
                    return (row, None);
                }
                let login = who.login().to_owned();
                let at = (repo, SettingsPage::Access);
                let (title, facts, choice, change) = match who {
                    Who::New(_) => (
                        format!("Invite @{login} to {repo} with {} access?", p.name()),
                        vec![(
                            format!(
                                "GitHub emails them; once they accept, they can {}",
                                p.about()
                            ),
                            p == Permission::Admin,
                        )],
                        "Invite",
                        SettingsChange::SetCollaborator {
                            repo: repo.clone(),
                            login,
                            permission: p,
                            invite: true,
                        },
                    ),
                    Who::Person(_) => {
                        let was = now.as_ref().map_or("their", Given::name);
                        (
                            format!("Give @{login} {} access to {repo}?", p.name()),
                            vec![(
                                format!("From {was} to {}: they can {}", p.name(), p.about()),
                                p == Permission::Admin,
                            )],
                            "Change role",
                            SettingsChange::SetCollaborator {
                                repo: repo.clone(),
                                login,
                                permission: p,
                                invite: false,
                            },
                        )
                    }
                    Who::Invitation(id, _) => (
                        format!("Invite @{login} with {} access instead?", p.name()),
                        vec![(format!("Once they accept, they can {}", p.about()), false)],
                        "Change invitation",
                        SettingsChange::SetInvitation {
                            repo: repo.clone(),
                            id: *id,
                            login,
                            permission: p,
                        },
                    ),
                };
                let confirm = ask(Action::EditSetting, at, title, facts, (choice, change));
                (row, Some(confirm))
            })
            .collect(),
        Pick::Archive { repo, archived } => {
            let name = repo.to_string();
            if !q.trim().eq_ignore_ascii_case(&name) {
                let verb = if *archived { "archive" } else { "unarchive" };
                return vec![(
                    crate::picker::item(format!("Type {name} to {verb} it"), ""),
                    None,
                )];
            }
            let at = (repo, SettingsPage::General);
            let (verb, facts) = if *archived {
                (
                    "Archive",
                    vec![
                        ("The last ask: it's done on ↵".to_owned(), false),
                        (
                            "It becomes read-only: no pushes, issues, pull requests or comments"
                                .to_owned(),
                            true,
                        ),
                        ("Unarchiving makes it writable again".to_owned(), false),
                    ],
                )
            } else {
                (
                    "Unarchive",
                    vec![
                        ("The last ask: it's done on ↵".to_owned(), false),
                        ("It becomes writable again".to_owned(), false),
                    ],
                )
            };
            let change = SettingsChange::Archive {
                repo: repo.clone(),
                archived: *archived,
            };
            let confirm = ask(
                Action::ToggleSetting,
                at,
                format!("{verb} {name}?"),
                facts,
                (&format!("{verb} repository"), change),
            );
            vec![(
                crate::picker::item(format!("{verb} {name}"), "asks once more"),
                Some(confirm),
            )]
        }
    }
}

// ---- following GitHub ------------------------------------------------------------

/// Whether the page `at`, as loaded, shows `c`.
pub fn shows(state: &State, c: &SettingsChange, repo: &RepoId, at: SettingsPage) -> bool {
    use SettingsChange as C;
    let Some(s) = data(state, repo, at) else { return false };
    match (c, s) {
        (C::CreateRule { form, .. }, Settings::Branches(b)) => {
            b.rules.iter().any(|r| r.form.pattern == form.pattern)
        }
        (C::UpdateRule { id, to, .. }, Settings::Branches(b)) => b
            .rules
            .iter()
            .any(|r| r.id == *id && same_rule(&r.form, to)),
        (C::DeleteRule { id, .. }, Settings::Branches(b)) => b.rules.iter().all(|r| r.id != *id),
        (
            C::SetRuleset {
                id, enforcement, ..
            },
            Settings::Branches(b),
        ) => b.rulesets.as_ref().is_ok_and(|r| {
            r.iter()
                .any(|r| r.id == *id && r.enforcement == *enforcement)
        }),
        (
            C::SetCollaborator {
                login, permission, ..
            },
            Settings::Access(a),
        ) => {
            let invited = (a.invitations.items.iter())
                .any(|i| i.login.eq_ignore_ascii_case(login) && i.permission == *permission);
            let given = (a.people.items.iter())
                .any(|p| p.login.eq_ignore_ascii_case(login) && p.role == Given::Is(*permission));
            invited || given
        }
        (C::RemoveCollaborator { login, .. }, Settings::Access(a)) => !a
            .people
            .items
            .iter()
            .any(|p| p.login.eq_ignore_ascii_case(login)),
        (C::SetInvitation { id, permission, .. }, Settings::Access(a)) => {
            (a.invitations.items.iter()).any(|i| i.id == *id && i.permission == *permission)
        }
        (C::CancelInvitation { id, .. }, Settings::Access(a)) => {
            a.invitations.items.iter().all(|i| i.id != *id)
        }
        (C::CreateHook { form, .. }, Settings::Hooks(h)) => {
            h.items.iter().any(|h| h.form.url == form.url)
        }
        (C::UpdateHook { id, to, .. }, Settings::Hooks(h)) => {
            (h.items.iter()).any(|h| h.id == *id && same_hook(&h.form, to))
        }
        (C::UpdateHook { id, to, .. }, Settings::Hook(h)) => {
            h.hook.id == *id && same_hook(&h.hook.form, to)
        }
        (C::DeleteHook { id, .. }, Settings::Hooks(h)) => h.items.iter().all(|h| h.id != *id),
        (C::Redeliver { guid, delivery, .. }, Settings::Hook(h)) => (h.deliveries.items.iter())
            .any(|d| d.guid == *guid && d.redelivery && d.id != *delivery),
        (C::AddKey { key, .. }, Settings::Keys(k)) => {
            // GitHub keeps a key's type and text, not its comment.
            let text: Vec<&str> = key.split_whitespace().take(2).collect();
            (k.items.iter()).any(|k| k.key.split_whitespace().take(2).eq(text.iter().copied()))
        }
        (C::DeleteKey { id, .. }, Settings::Keys(k)) => k.items.iter().all(|k| k.id != *id),
        (C::Archive { archived, .. }, Settings::General(g)) => g.archived == *archived,
        // Another page: nothing there to wait for.
        _ => true,
    }
}

/// A webhook as GitHub shows it matches the form (its secret is never
/// shown).
fn same_hook(shown: &HookForm, form: &HookForm) -> bool {
    let without = |h: &HookForm| {
        let mut events = h.events.clone();
        events.sort_unstable();
        HookForm {
            secret: None,
            events,
            ..h.clone()
        }
    };
    without(shown) == without(form)
}

/// A rule as GitHub shows it matches the form, whatever order it lists
/// the checks in.
fn same_rule(shown: &RuleForm, form: &RuleForm) -> bool {
    let sorted = |f: &RuleForm| {
        let mut f = f.clone();
        if let Some(c) = &mut f.checks {
            c.contexts.sort_unstable();
        }
        f
    };
    sorted(shown) == sorted(form)
}

// ---- what changes say ------------------------------------------------------------

/// What it's doing while GitHub answers, and what it is, for "GitHub refused …".
pub fn said(c: &SettingsChange) -> (&'static str, &'static str) {
    use SettingsChange as C;
    match c {
        C::CreateRule { .. } => ("Creating the rule", "the new rule"),
        C::UpdateRule { .. } => ("Saving the rule", "the rule's change"),
        C::DeleteRule { .. } => ("Deleting the rule", "deleting the rule"),
        C::SetRuleset { .. } => ("Changing the ruleset", "changing the ruleset"),
        C::SetCollaborator { invite: true, .. } => ("Inviting them", "the invitation"),
        C::SetCollaborator { .. } | C::SetInvitation { .. } => {
            ("Changing their role", "the new role")
        }
        C::RemoveCollaborator { .. } => ("Removing them", "removing them"),
        C::CancelInvitation { .. } => ("Cancelling the invitation", "cancelling the invitation"),
        C::CreateHook { .. } => ("Adding the webhook", "the new webhook"),
        C::UpdateHook { .. } => ("Saving the webhook", "the webhook's change"),
        C::DeleteHook { .. } => ("Deleting the webhook", "deleting the webhook"),
        C::Redeliver { .. } => ("Redelivering", "the redelivery"),
        C::AddKey { .. } => ("Adding the key", "the new key"),
        C::DeleteKey { .. } => ("Deleting the key", "deleting the key"),
        C::Archive { archived: true, .. } => ("Archiving the repository", "archiving it"),
        C::Archive { .. } => ("Unarchiving the repository", "unarchiving it"),
    }
}

pub fn done(c: &SettingsChange) -> String {
    use SettingsChange as C;
    match c {
        C::CreateRule { form, .. } => format!("Protected {}", form.pattern),
        C::UpdateRule { to, .. } => format!("Saved the rule for {}", to.pattern),
        C::DeleteRule { pattern, .. } => format!("Deleted the rule for {pattern}"),
        C::SetRuleset {
            name, enforcement, ..
        } => format!("Ruleset “{name}”: {}", enforcement.word()),
        C::SetCollaborator {
            login,
            permission,
            invite: true,
            ..
        } => format!("Invited @{login} with {} access", permission.name()),
        C::SetCollaborator {
            login, permission, ..
        }
        | C::SetInvitation {
            login, permission, ..
        } => format!("@{login}: {}", permission.name()),
        C::RemoveCollaborator { login, .. } => format!("Removed @{login}"),
        C::CancelInvitation { login, .. } => format!("Cancelled the invitation to @{login}"),
        C::CreateHook { form, .. } => format!("Added the webhook to {}", form.url),
        C::UpdateHook { to, .. } => format!("Saved the webhook to {}", to.url),
        C::DeleteHook { url, .. } => format!("Deleted the webhook to {url}"),
        C::Redeliver { .. } => "Redelivered".into(),
        C::AddKey { title, .. } => format!("Added deploy key “{title}”"),
        C::DeleteKey { title, .. } => format!("Deleted deploy key “{title}”"),
        C::Archive {
            archived: true,
            repo,
        } => format!("Archived {repo}: it's read-only"),
        C::Archive { repo, .. } => format!("Unarchived {repo}"),
    }
}

/// The permission a fine-grained token needs.
pub fn needs(c: &SettingsChange) -> &'static str {
    use SettingsChange as C;
    match c {
        C::CreateHook { .. }
        | C::UpdateHook { .. }
        | C::DeleteHook { .. }
        | C::Redeliver { .. } => "Webhooks: write",
        _ => "Administration: write",
    }
}

/// Settings as GitHub might send them, for tests.
#[cfg(test)]
pub(crate) mod fixtures {
    use crate::api::model::{Capped, NodeId};
    use crate::api::settings::{
        Access, Branches, Checks, Collaborator, Delivery, DeployKey, Enforcement, General, Hook,
        HookForm, HookPage, Invitation, LastResponse, Listed, Permission, Reviews, Role, Rule,
        RuleForm, Ruleset, Settings, SettingsPage,
    };

    /// `main`'s rule, as the test repository's: a review and a check,
    /// for admins too.
    pub fn rule() -> Rule {
        Rule {
            id: NodeId::new("BPR_1"),
            form: RuleForm {
                pattern: "main".into(),
                reviews: Some(Reviews {
                    count: 1,
                    ..Reviews::default()
                }),
                checks: Some(Checks {
                    strict: true,
                    contexts: vec!["check".into()],
                }),
                admins: true,
                ..RuleForm::default()
            },
            apps: vec![("check".into(), NodeId::new("A_15368"))],
            matching: Capped::new(vec!["main".into()], 1),
        }
    }

    pub fn hook(id: u64, active: bool) -> Hook {
        Hook {
            id,
            form: HookForm {
                url: format!("https://example.com/hook/{id}"),
                events: vec!["push".into(), "pull_request".into()],
                active,
                ..HookForm::default()
            },
            secret: true,
            last: Some(LastResponse {
                code: Some(200),
                status: "active".into(),
                message: Some("OK".into()),
            }),
            updated_at: "2026-10-01T10:00:00Z".into(),
        }
    }

    pub fn page(page: SettingsPage) -> Settings {
        match page {
            SettingsPage::General => Settings::General(General {
                archived: false,
                visibility: "public".into(),
                default_branch: "main".into(),
                admin: true,
            }),
            SettingsPage::Branches => Settings::Branches(Branches {
                repo_id: NodeId::new("R_ghtui"),
                rules: vec![rule()].into(),
                rulesets: Ok(vec![
                    Ruleset {
                        id: 7,
                        name: "Tags are kept".into(),
                        target: "tag".into(),
                        enforcement: Enforcement::Active,
                        org: None,
                    },
                    Ruleset {
                        id: 8,
                        name: "Signed".into(),
                        target: "branch".into(),
                        enforcement: Enforcement::Disabled,
                        org: Some("octo-org".into()),
                    },
                ]),
            }),
            SettingsPage::Access => Settings::Access(Access {
                people: Listed {
                    items: vec![
                        Collaborator {
                            login: "gold-silver-copper".into(),
                            role: Role::Is(Permission::Admin),
                        },
                        Collaborator {
                            login: "octocat".into(),
                            role: Role::Is(Permission::Write),
                        },
                    ],
                    more: false,
                },
                invitations: Listed {
                    items: vec![Invitation {
                        id: 31,
                        login: "hubot".into(),
                        permission: Permission::Triage,
                        created_at: "2026-10-01T10:00:00Z".into(),
                        expired: false,
                    }],
                    more: false,
                },
            }),
            SettingsPage::Hooks => Settings::Hooks(Listed {
                items: vec![hook(11, true), hook(12, false)],
                more: false,
            }),
            SettingsPage::Hook(id) => Settings::Hook(Box::new(HookPage {
                hook: hook(id, true),
                deliveries: Listed {
                    items: vec![
                        Delivery {
                            id: 901,
                            guid: "0b989ba4-242f-11e5-81e1-c7b6966d2516".into(),
                            delivered_at: "2026-10-01T10:00:00Z".into(),
                            redelivery: false,
                            status: "OK".into(),
                            code: 200,
                            event: "push".into(),
                            action: None,
                        },
                        Delivery {
                            id: 902,
                            guid: "1c989ba4-242f-11e5-81e1-c7b6966d2516".into(),
                            delivered_at: "2026-10-02T10:00:00Z".into(),
                            redelivery: false,
                            status: "Invalid HTTP Response: 500".into(),
                            code: 500,
                            event: "pull_request".into(),
                            action: Some("opened".into()),
                        },
                    ],
                    more: false,
                },
            })),
            SettingsPage::Keys => Settings::Keys(Listed {
                items: vec![DeployKey {
                    id: 41,
                    title: "CI".into(),
                    key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl"
                        .into(),
                    read_only: true,
                    created_at: "2026-09-01T10:00:00Z".into(),
                    last_used: None,
                    added_by: Some("gold-silver-copper".into()),
                }],
                more: false,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A rule's form reads back as it was written, and says what's wrong
    /// with a line it can't read.
    #[test]
    fn a_rule_form_reads_back_and_names_its_mistakes() {
        let rule = RuleForm {
            pattern: "release/*".into(),
            reviews: Some(Reviews {
                count: 2,
                code_owners: true,
                ..Reviews::default()
            }),
            checks: Some(Checks {
                strict: true,
                contexts: vec!["ci".into(), "lint".into()],
            }),
            admins: true,
            ..RuleForm::default()
        };
        assert_eq!(
            parse_rule(&rule_text(&rule), &RuleForm::default()),
            Ok(rule.clone())
        );
        let fewer = "branch name pattern: main\nRequired approvals: 1";
        let read = parse_rule(fewer, &rule).unwrap_or_default();
        assert_eq!(read.pattern, "main");
        assert_eq!(read.reviews.map(|r| r.count), Some(1));
        assert!(read.admins, "what isn't written stays");
        let err = |text: &str| parse_rule(text, &rule).err().unwrap_or_default();
        assert!(err("Allow deletions: maybe").contains("yes or no"));
        assert!(err("Colour: blue").contains("isn't one of the form's settings"));
        assert!(err("Required approvals: 9").contains("0 to 6"));
        assert!(err("just words").contains("Line 1"));
    }

    /// A webhook's form wants an address and events; a deploy key, a
    /// title and a public key.
    #[test]
    fn hook_and_key_forms_check_what_github_needs() {
        let hook = parse_hook(
            "Payload URL: https://example.com/hook\nEvents: push, issues\nSecret: s3",
            &HookForm::default(),
        );
        assert_eq!(
            hook.map(|h| (h.url, h.events, h.secret)),
            Ok((
                "https://example.com/hook".into(),
                vec!["push".into(), "issues".into()],
                Some("s3".into())
            ))
        );
        parse_hook("Payload URL: example.com", &HookForm::default()).unwrap_err();
        let key = parse_key("Title: ci\nAllow write access: yes\nKey: ssh-ed25519 AAAA me@x");
        assert_eq!(key, Ok(("ci".into(), "ssh-ed25519 AAAA me@x".into(), true)));
        parse_key("Title: ci\nKey: hello").unwrap_err();
    }
}
