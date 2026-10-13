//! A repository's settings, as its Settings tab shows them to admins:
//! branch protection rules and rulesets, collaborators and invitations,
//! webhooks and their deliveries, deploy keys, and whether it's archived.
//! Reading is [`GitHub::settings`]; every change is a [`SettingsChange`],
//! sent by [`GitHub::change_setting`].

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::api::client::{ApiError, GitHub};
use crate::api::model::{Capped, NodeId, RepoId};
use crate::api::raw;

/// One page of the Settings tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SettingsPage {
    /// The repository itself: archiving it, and where deleting it is.
    General,
    /// Branch protection rules and rulesets.
    Branches,
    /// Collaborators and invitations.
    Access,
    Hooks,
    /// One webhook and its recent deliveries.
    Hook(u64),
    /// Deploy keys.
    Keys,
}

impl SettingsPage {
    /// The pages of the tab's own menu, in GitHub's order.
    pub const MENU: [SettingsPage; 5] = [
        SettingsPage::General,
        SettingsPage::Access,
        SettingsPage::Branches,
        SettingsPage::Hooks,
        SettingsPage::Keys,
    ];

    /// What follows `/settings` in its URL.
    pub fn slug(self) -> String {
        match self {
            SettingsPage::General => String::new(),
            SettingsPage::Branches => "/branches".into(),
            SettingsPage::Access => "/access".into(),
            SettingsPage::Hooks => "/hooks".into(),
            SettingsPage::Hook(id) => format!("/hooks/{id}"),
            SettingsPage::Keys => "/keys".into(),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            SettingsPage::General => "General",
            SettingsPage::Branches => "Branches",
            SettingsPage::Access => "Collaborators",
            SettingsPage::Hooks => "Webhooks",
            SettingsPage::Hook(_) => "Webhook",
            SettingsPage::Keys => "Deploy keys",
        }
    }

    /// The page of the menu it's under.
    pub fn menu(self) -> SettingsPage {
        match self {
            SettingsPage::Hook(_) => SettingsPage::Hooks,
            page => page,
        }
    }
}

/// A REST list's first page: what GitHub listed, and whether it has more.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listed<T> {
    pub items: Vec<T>,
    /// GitHub has more than these.
    pub more: bool,
}

/// What a settings page shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Settings {
    General(General),
    Branches(Branches),
    Access(Access),
    Hooks(Listed<Hook>),
    Hook(Box<HookPage>),
    Keys(Listed<DeployKey>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct General {
    pub archived: bool,
    /// `public`, `private` or `internal`.
    pub visibility: String,
    pub default_branch: String,
    /// You administer it.
    pub admin: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Branches {
    /// The repository's node ID, for a new rule.
    pub repo_id: NodeId,
    pub rules: Capped<Rule>,
    /// Its rulesets, or why GitHub won't list them.
    pub rulesets: Result<Vec<Ruleset>, String>,
}

/// A branch protection rule: the branches it applies to, by pattern, and
/// what it asks of them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub id: NodeId,
    pub form: RuleForm,
    /// The status checks bound to an app, by name: kept so when the rule
    /// is edited.
    pub apps: Vec<(String, NodeId)>,
    /// The branches it applies to now.
    pub matching: Capped<String>,
}

/// What a branch protection rule asks: GitHub's form's fields.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleForm {
    pub pattern: String,
    /// A pull request before merging, with these approvals.
    pub reviews: Option<Reviews>,
    /// Status checks that must pass.
    pub checks: Option<Checks>,
    pub conversations: bool,
    pub signed: bool,
    pub linear: bool,
    pub lock: bool,
    /// Admins can't bypass it ("Do not allow bypassing").
    pub admins: bool,
    pub force_pushes: bool,
    pub deletions: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reviews {
    pub count: u8,
    pub dismiss_stale: bool,
    pub code_owners: bool,
    pub last_push: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checks {
    /// Branches must be up to date with the base first.
    pub strict: bool,
    pub contexts: Vec<String>,
}

/// A ruleset: GitHub's newer rules, turned on or off as a whole here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ruleset {
    pub id: u64,
    pub name: String,
    /// `branch`, `tag` or `push`.
    pub target: String,
    pub enforcement: Enforcement,
    /// Its organization's, when it isn't the repository's own.
    pub org: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Enforcement {
    Active,
    Disabled,
    /// Evaluate: it reports, without enforcing (Enterprise).
    Evaluate,
}

impl Enforcement {
    pub fn word(self) -> &'static str {
        match self {
            Enforcement::Active => "active",
            Enforcement::Disabled => "disabled",
            Enforcement::Evaluate => "evaluate",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Enforcement::Active => "Active",
            Enforcement::Disabled => "Disabled",
            Enforcement::Evaluate => "Evaluate",
        }
    }

    fn parse(word: &str) -> Self {
        match word {
            "active" => Enforcement::Active,
            "evaluate" => Enforcement::Evaluate,
            _ => Enforcement::Disabled,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Access {
    pub people: Listed<Collaborator>,
    pub invitations: Listed<Invitation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Collaborator {
    pub login: String,
    /// GitHub's role, or a custom role's name.
    pub role: Role,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    Is(Permission),
    /// An organization's custom role, by its name.
    Custom(String),
}

impl Role {
    pub fn name(&self) -> &str {
        match self {
            Role::Is(p) => p.name(),
            Role::Custom(name) => name,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invitation {
    pub id: u64,
    pub login: String,
    pub permission: Permission,
    /// ISO 8601.
    pub created_at: String,
    pub expired: bool,
}

/// A repository role GitHub gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Permission {
    Read,
    Triage,
    Write,
    Maintain,
    Admin,
}

impl Permission {
    pub const ALL: [Permission; 5] = [
        Permission::Read,
        Permission::Triage,
        Permission::Write,
        Permission::Maintain,
        Permission::Admin,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Permission::Read => "Read",
            Permission::Triage => "Triage",
            Permission::Write => "Write",
            Permission::Maintain => "Maintain",
            Permission::Admin => "Admin",
        }
    }

    /// What it lets someone do, in GitHub's words.
    pub fn about(self) -> &'static str {
        match self {
            Permission::Read => "read and clone; open and comment on issues and pull requests",
            Permission::Triage => "read, and manage issues and pull requests",
            Permission::Write => "read, clone and push",
            Permission::Maintain => "push, and manage the repository without its settings",
            Permission::Admin => "everything, settings and deleting it included",
        }
    }

    /// The word REST takes for it.
    fn wire(self) -> &'static str {
        match self {
            Permission::Read => "pull",
            Permission::Triage => "triage",
            Permission::Write => "push",
            Permission::Maintain => "maintain",
            Permission::Admin => "admin",
        }
    }

    /// From REST's words: a role's name (`write`) or a permission's
    /// (`push`).
    pub fn parse(word: &str) -> Option<Self> {
        Some(match word {
            "read" | "pull" => Permission::Read,
            "triage" => Permission::Triage,
            "write" | "push" => Permission::Write,
            "maintain" => Permission::Maintain,
            "admin" => Permission::Admin,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hook {
    pub id: u64,
    pub form: HookForm,
    /// It has a secret (GitHub never shows it).
    pub secret: bool,
    /// What its last delivery got back, if it has made one.
    pub last: Option<LastResponse>,
    /// ISO 8601.
    pub updated_at: String,
}

/// A webhook's form's fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookForm {
    pub url: String,
    /// `json` or `form`.
    pub content_type: String,
    /// A new secret; `None` keeps the one set (empty: none).
    pub secret: Option<String>,
    pub verify_ssl: bool,
    /// GitHub's event names; `*` is every event.
    pub events: Vec<String>,
    pub active: bool,
}

impl Default for HookForm {
    fn default() -> Self {
        Self {
            url: String::new(),
            content_type: "json".into(),
            secret: None,
            verify_ssl: true,
            events: vec!["push".into()],
            active: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastResponse {
    pub code: Option<u16>,
    /// GitHub's word: `active`, `unused`, `misconfigured`…
    pub status: String,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookPage {
    pub hook: Hook,
    pub deliveries: Listed<Delivery>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delivery {
    pub id: u64,
    /// Shared by a delivery and its redeliveries.
    pub guid: String,
    /// ISO 8601.
    pub delivered_at: String,
    pub redelivery: bool,
    /// `OK`, or what went wrong.
    pub status: String,
    pub code: u16,
    pub event: String,
    pub action: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployKey {
    pub id: u64,
    pub title: String,
    pub key: String,
    pub read_only: bool,
    /// ISO 8601.
    pub created_at: String,
    pub last_used: Option<String>,
    pub added_by: Option<String>,
}

/// A change to a repository's settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsChange {
    CreateRule {
        repo_id: NodeId,
        form: RuleForm,
    },
    /// Only what differs between `from` and `to` is sent: what the form
    /// doesn't show stays as it is.
    UpdateRule {
        id: NodeId,
        from: Box<RuleForm>,
        to: Box<RuleForm>,
        apps: Vec<(String, NodeId)>,
    },
    DeleteRule {
        id: NodeId,
        pattern: String,
    },
    SetRuleset {
        repo: RepoId,
        id: u64,
        name: String,
        enforcement: Enforcement,
    },
    /// Invite someone, or change a collaborator's role: GitHub's one
    /// request for both.
    SetCollaborator {
        repo: RepoId,
        login: String,
        permission: Permission,
        invite: bool,
    },
    RemoveCollaborator {
        repo: RepoId,
        login: String,
    },
    SetInvitation {
        repo: RepoId,
        id: u64,
        login: String,
        permission: Permission,
    },
    CancelInvitation {
        repo: RepoId,
        id: u64,
        login: String,
    },
    CreateHook {
        repo: RepoId,
        form: HookForm,
    },
    /// Only what differs is sent.
    UpdateHook {
        repo: RepoId,
        id: u64,
        from: Box<HookForm>,
        to: Box<HookForm>,
    },
    DeleteHook {
        repo: RepoId,
        id: u64,
        url: String,
    },
    Redeliver {
        repo: RepoId,
        hook: u64,
        delivery: u64,
        guid: String,
    },
    AddKey {
        repo: RepoId,
        title: String,
        key: String,
        write: bool,
    },
    DeleteKey {
        repo: RepoId,
        id: u64,
        title: String,
    },
    Archive {
        repo: RepoId,
        archived: bool,
    },
}

/// The cache's keys.
pub mod keys {
    use super::SettingsPage;
    use crate::api::model::RepoId;

    pub fn page(repo: &RepoId, page: SettingsPage) -> String {
        format!("settings:{repo}{}", page.slug())
    }
}

/// How many a REST list asks for.
const PER_PAGE: usize = 100;

/// GitHub's words for a REST refusal of a page only admins see, said as
/// what's missing: a scope, or admin access.
fn refused(status: u16, scopes: Option<&str>, what: &str, scope: &str) -> Option<String> {
    if !matches!(status, 403 | 404) {
        return None;
    }
    let lacks = scopes.is_some_and(|s| {
        let has = |name: &str| s.split(',').any(|x| x.trim() == name);
        !has("repo") && !has(scope)
    });
    Some(if lacks {
        format!(
            "{what} take the {scope} scope, which your token lacks: `gh auth refresh -s {scope}` adds it"
        )
    } else {
        format!("GitHub shows {what} only to the repository's admins")
    })
}

impl GitHub {
    /// A settings page's data.
    pub async fn settings(&self, repo: &RepoId, page: SettingsPage) -> Result<Settings, ApiError> {
        let settings = match page {
            SettingsPage::General => Settings::General(self.general(repo).await?),
            SettingsPage::Branches => Settings::Branches(self.protection_rules(repo).await?),
            SettingsPage::Access => Settings::Access(Access {
                people: self
                    .listed(
                        repo,
                        "collaborators?affiliation=direct&",
                        "collaborators",
                        wire::person,
                    )
                    .await?,
                invitations: self
                    .listed(repo, "invitations?", "invitations", wire::invitation)
                    .await?,
            }),
            SettingsPage::Hooks => {
                Settings::Hooks(self.listed(repo, "hooks?", "webhooks", wire::hook).await?)
            }
            SettingsPage::Hook(id) => {
                let path = format!("/repos/{repo}/hooks/{id}");
                let hook = wire::hook(self.admin_json(&path, "webhooks").await?)
                    .ok_or_else(|| ApiError::Decode(format!("{repo}'s webhook {id}")))?;
                let deliveries = self
                    .listed(
                        repo,
                        &format!("hooks/{id}/deliveries?"),
                        "webhooks",
                        wire::delivery,
                    )
                    .await?;
                Settings::Hook(Box::new(HookPage { hook, deliveries }))
            }
            SettingsPage::Keys => {
                Settings::Keys(self.listed(repo, "keys?", "deploy keys", wire::key).await?)
            }
        };
        self.remember(&keys::page(repo, page), settings.clone())
            .await;
        Ok(settings)
    }

    /// GitHub's answer for a path only admins may read, or why not.
    async fn admin_answer(
        &self,
        path: &str,
        what: &str,
    ) -> Result<crate::api::client::Answer, ApiError> {
        let answer = self.rest_answer(path).await?;
        if (200..300).contains(&answer.status()) {
            return Ok(answer);
        }
        let scope = if what == "webhooks" {
            "admin:repo_hook"
        } else {
            "repo"
        };
        let scopes = answer.header("x-oauth-scopes");
        if let Some(why) = refused(answer.status(), scopes, what, scope) {
            return Err(ApiError::Denied(why));
        }
        answer.checked(what)?;
        Err(ApiError::Decode(format!(
            "{what}: status {}",
            answer.status()
        )))
    }

    async fn admin_json(&self, path: &str, what: &str) -> Result<Value, ApiError> {
        let answer = self.admin_answer(path, what).await?;
        Ok(serde_json::from_str(answer.body())?)
    }

    /// The first page of a list of the repository's (`list` is its path
    /// and query, ending in `?` or `&`), each item read by `read`.
    async fn listed<T>(
        &self,
        repo: &RepoId,
        list: &str,
        what: &str,
        read: fn(Value) -> Option<T>,
    ) -> Result<Listed<T>, ApiError> {
        let path = format!("/repos/{repo}/{list}per_page={PER_PAGE}");
        let answer = self.admin_answer(&path, what).await?;
        let wire: Vec<Value> = serde_json::from_str(answer.body())?;
        Ok(Listed {
            more: answer.next_page().is_some(),
            items: wire.into_iter().filter_map(read).collect(),
        })
    }

    async fn general(&self, repo: &RepoId) -> Result<General, ApiError> {
        #[derive(Deserialize)]
        struct Wire {
            archived: bool,
            #[serde(default)]
            visibility: Option<String>,
            #[serde(default)]
            private: bool,
            #[serde(default)]
            default_branch: String,
            #[serde(default)]
            permissions: Option<Perms>,
        }
        #[derive(Deserialize)]
        struct Perms {
            #[serde(default)]
            admin: bool,
        }
        let path = format!("/repos/{repo}");
        let wire: Wire = serde_json::from_str(&self.rest_get(&path, repo).await?)?;
        Ok(General {
            archived: wire.archived,
            visibility: wire
                .visibility
                .unwrap_or_else(|| if wire.private { "private" } else { "public" }.into()),
            default_branch: wire.default_branch,
            admin: wire.permissions.is_some_and(|p| p.admin),
        })
    }

    async fn protection_rules(&self, repo: &RepoId) -> Result<Branches, ApiError> {
        let vars = json!({ "owner": repo.owner, "name": repo.name });
        let found: Value = self
            .graphql_json(raw::PROTECTION_RULES, vars, repo, "/repository")
            .await?;
        let repo_id = NodeId::new(found["id"].as_str().unwrap_or_default());
        let rules = &found["branchProtectionRules"];
        let nodes = rules["nodes"].as_array().cloned().unwrap_or_default();
        let total = rules["totalCount"].as_u64().unwrap_or_default();
        let rules = Capped::new(nodes.into_iter().filter_map(wire::rule).collect(), total);
        let rulesets = match self
            .admin_answer(
                &format!("/repos/{repo}/rulesets?per_page={PER_PAGE}"),
                "rulesets",
            )
            .await
        {
            Ok(answer) => {
                let wire: Vec<Value> = serde_json::from_str(answer.body())?;
                Ok(wire
                    .into_iter()
                    .filter_map(|r| wire::ruleset(r, repo))
                    .collect())
            }
            // An older Enterprise Server has none.
            Err(ApiError::NotFound(_)) => Err("GitHub doesn't have rulesets here".into()),
            Err(ApiError::Denied(why)) => Err(why),
            Err(err) => return Err(err),
        };
        Ok(Branches {
            repo_id,
            rules,
            rulesets,
        })
    }

    /// Makes a change to a repository's settings.
    pub(crate) async fn change_setting(&self, change: &SettingsChange) -> Result<(), ApiError> {
        use SettingsChange as C;
        use http::Method;
        match change {
            C::CreateRule { repo_id, form } => {
                let mut input = rule_input(&RuleForm::default(), form, &[], true);
                input.insert("repositoryId".into(), repo_id.as_str().into());
                let vars = json!({ "input": Value::Object(input) });
                self.mutate_json(raw::CREATE_PROTECTION_RULE, vars).await
            }
            C::UpdateRule { id, from, to, apps } => {
                let mut input = rule_input(from, to, apps, false);
                input.insert("branchProtectionRuleId".into(), id.as_str().into());
                let vars = json!({ "input": Value::Object(input) });
                self.mutate_json(raw::UPDATE_PROTECTION_RULE, vars).await
            }
            C::DeleteRule { id, .. } => {
                let vars = json!({ "id": id.as_str() });
                self.mutate_json(raw::DELETE_PROTECTION_RULE, vars).await
            }
            C::SetRuleset {
                repo,
                id,
                enforcement,
                ..
            } => {
                let body = json!({ "enforcement": enforcement.word() });
                let path = format!("/repos/{repo}/rulesets/{id}");
                self.rest_write(Method::PUT, &path, Some(&body)).await
            }
            C::SetCollaborator {
                repo,
                login,
                permission,
                ..
            } => {
                let body = json!({ "permission": permission.wire() });
                let path = format!("/repos/{repo}/collaborators/{login}");
                self.rest_write(Method::PUT, &path, Some(&body)).await
            }
            C::RemoveCollaborator { repo, login } => {
                let path = format!("/repos/{repo}/collaborators/{login}");
                self.rest_write(Method::DELETE, &path, None).await
            }
            C::SetInvitation {
                repo,
                id,
                permission,
                ..
            } => {
                // An invitation's roles are named as roles are (`write`).
                let body = json!({ "permissions": permission.name().to_lowercase() });
                let path = format!("/repos/{repo}/invitations/{id}");
                self.rest_write(Method::PATCH, &path, Some(&body)).await
            }
            C::CancelInvitation { repo, id, .. } => {
                let path = format!("/repos/{repo}/invitations/{id}");
                self.rest_write(Method::DELETE, &path, None).await
            }
            C::CreateHook { repo, form } => {
                let body = hook_body(None, form);
                let path = format!("/repos/{repo}/hooks");
                self.rest_write(Method::POST, &path, Some(&body)).await
            }
            C::UpdateHook { repo, id, from, to } => {
                let body = hook_body(Some(from), to);
                let path = format!("/repos/{repo}/hooks/{id}");
                self.rest_write(Method::PATCH, &path, Some(&body)).await
            }
            C::DeleteHook { repo, id, .. } => {
                let path = format!("/repos/{repo}/hooks/{id}");
                self.rest_write(Method::DELETE, &path, None).await
            }
            C::Redeliver {
                repo,
                hook,
                delivery,
                ..
            } => {
                let path = format!("/repos/{repo}/hooks/{hook}/deliveries/{delivery}/attempts");
                self.rest_write(Method::POST, &path, None).await
            }
            C::AddKey {
                repo,
                title,
                key,
                write,
            } => {
                let body = json!({ "title": title, "key": key, "read_only": !write });
                let path = format!("/repos/{repo}/keys");
                self.rest_write(Method::POST, &path, Some(&body)).await
            }
            C::DeleteKey { repo, id, .. } => {
                let path = format!("/repos/{repo}/keys/{id}");
                self.rest_write(Method::DELETE, &path, None).await
            }
            C::Archive { repo, archived } => {
                let body = json!({ "archived": archived });
                let path = format!("/repos/{repo}");
                self.rest_write(Method::PATCH, &path, Some(&body)).await
            }
        }
    }
}

/// A rule's input for GitHub's mutations: every field of `to` (`all`), or
/// those that differ from `from`. Status checks named as before keep the
/// app they're bound to.
fn rule_input(
    from: &RuleForm,
    to: &RuleForm,
    apps: &[(String, NodeId)],
    all: bool,
) -> Map<String, Value> {
    let mut input = Map::new();
    let mut put = |name: &str, old: Value, new: Value| {
        if all || old != new {
            input.insert(name.to_owned(), new);
        }
    };
    put(
        "pattern",
        from.pattern.clone().into(),
        to.pattern.clone().into(),
    );
    let reviews = |f: &RuleForm| f.reviews.clone();
    let (old, new) = (reviews(from), reviews(to));
    put(
        "requiresApprovingReviews",
        old.is_some().into(),
        new.is_some().into(),
    );
    let (old, new) = (old.unwrap_or_default(), new.unwrap_or_default());
    put(
        "requiredApprovingReviewCount",
        old.count.into(),
        new.count.into(),
    );
    put(
        "dismissesStaleReviews",
        old.dismiss_stale.into(),
        new.dismiss_stale.into(),
    );
    put(
        "requiresCodeOwnerReviews",
        old.code_owners.into(),
        new.code_owners.into(),
    );
    put(
        "requireLastPushApproval",
        old.last_push.into(),
        new.last_push.into(),
    );
    let (old, new) = (from.checks.clone(), to.checks.clone());
    put(
        "requiresStatusChecks",
        old.is_some().into(),
        new.is_some().into(),
    );
    let (old, new) = (old.unwrap_or_default(), new.unwrap_or_default());
    put(
        "requiresStrictStatusChecks",
        old.strict.into(),
        new.strict.into(),
    );
    let checks = |c: &Checks| -> Value {
        c.contexts
            .iter()
            .map(|context| {
                let app = apps.iter().find(|(name, _)| name == context);
                match app {
                    Some((_, id)) => json!({ "context": context, "appId": id.as_str() }),
                    None => json!({ "context": context }),
                }
            })
            .collect()
    };
    put("requiredStatusChecks", checks(&old), checks(&new));
    let flags: [(&str, fn(&RuleForm) -> bool); 7] = [
        ("requiresConversationResolution", |f| f.conversations),
        ("requiresCommitSignatures", |f| f.signed),
        ("requiresLinearHistory", |f| f.linear),
        ("lockBranch", |f| f.lock),
        ("isAdminEnforced", |f| f.admins),
        ("allowsForcePushes", |f| f.force_pushes),
        ("allowsDeletions", |f| f.deletions),
    ];
    for (name, flag) in flags {
        put(name, flag(from).into(), flag(to).into());
    }
    input
}

/// A webhook's body for REST: everything (`from: None`), or what differs.
fn hook_body(from: Option<&HookForm>, to: &HookForm) -> Value {
    let mut body = Map::new();
    let mut config = Map::new();
    let changed = |f: fn(&HookForm) -> Value| from.is_none_or(|from| f(from) != f(to));
    if changed(|h| h.url.clone().into()) {
        config.insert("url".into(), to.url.clone().into());
    }
    if changed(|h| h.content_type.clone().into()) {
        config.insert("content_type".into(), to.content_type.clone().into());
    }
    if changed(|h| h.verify_ssl.into()) {
        let insecure = if to.verify_ssl { "0" } else { "1" };
        config.insert("insecure_ssl".into(), insecure.into());
    }
    if let Some(secret) = &to.secret {
        config.insert("secret".into(), secret.clone().into());
    }
    if from.is_none() {
        body.insert("name".into(), "web".into());
    }
    // REST replaces the whole config: a change sends the rest as it was.
    if !config.is_empty() {
        if let Some(from) = from {
            config
                .entry("url")
                .or_insert_with(|| from.url.clone().into());
            config
                .entry("content_type")
                .or_insert_with(|| from.content_type.clone().into());
            let insecure = if from.verify_ssl { "0" } else { "1" };
            config
                .entry("insecure_ssl")
                .or_insert_with(|| insecure.into());
        }
        body.insert("config".into(), Value::Object(config));
    }
    if changed(|h| h.events.clone().into()) {
        body.insert("events".into(), to.events.clone().into());
    }
    if changed(|h| h.active.into()) {
        body.insert("active".into(), to.active.into());
    }
    Value::Object(body)
}

/// GitHub's settings as its APIs send them.
mod wire {
    use serde_json::Value;

    use super::*;

    fn text(v: &Value) -> String {
        v.as_str().unwrap_or_default().to_owned()
    }

    pub(super) fn rule(n: Value) -> Option<Rule> {
        let flag = |name: &str| n[name].as_bool().unwrap_or_default();
        let checks = n["requiredStatusChecks"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let apps = (checks.iter())
            .filter_map(|c| Some((text(&c["context"]), NodeId::new(c["app"]["id"].as_str()?))))
            .collect();
        let contexts = checks.iter().map(|c| text(&c["context"])).collect();
        let matching = &n["matchingRefs"];
        let names = (matching["nodes"].as_array().into_iter().flatten())
            .map(|r| text(&r["name"]))
            .collect();
        Some(Rule {
            id: NodeId::new(n["id"].as_str()?),
            form: RuleForm {
                pattern: text(&n["pattern"]),
                reviews: flag("requiresApprovingReviews").then(|| Reviews {
                    count: u8::try_from(
                        n["requiredApprovingReviewCount"]
                            .as_u64()
                            .unwrap_or_default(),
                    )
                    .unwrap_or(u8::MAX),
                    dismiss_stale: flag("dismissesStaleReviews"),
                    code_owners: flag("requiresCodeOwnerReviews"),
                    last_push: flag("requireLastPushApproval"),
                }),
                checks: flag("requiresStatusChecks").then(|| Checks {
                    strict: flag("requiresStrictStatusChecks"),
                    contexts,
                }),
                conversations: flag("requiresConversationResolution"),
                signed: flag("requiresCommitSignatures"),
                linear: flag("requiresLinearHistory"),
                lock: flag("lockBranch"),
                admins: flag("isAdminEnforced"),
                force_pushes: flag("allowsForcePushes"),
                deletions: flag("allowsDeletions"),
            },
            apps,
            matching: Capped::new(names, matching["totalCount"].as_u64().unwrap_or_default()),
        })
    }

    pub(super) fn ruleset(r: Value, repo: &RepoId) -> Option<Ruleset> {
        let source = text(&r["source"]);
        let org = (r["source_type"] == "Organization"
            || !source.eq_ignore_ascii_case(&repo.to_string()))
        .then_some(source)
        .filter(|s| !s.is_empty());
        Some(Ruleset {
            id: r["id"].as_u64()?,
            name: text(&r["name"]),
            target: r["target"].as_str().unwrap_or("branch").to_owned(),
            enforcement: Enforcement::parse(r["enforcement"].as_str().unwrap_or_default()),
            org,
        })
    }

    pub(super) fn person(p: Value) -> Option<Collaborator> {
        let role = text(&p["role_name"]);
        Some(Collaborator {
            login: p["login"].as_str()?.to_owned(),
            role: Permission::parse(&role).map_or(Role::Custom(role), Role::Is),
        })
    }

    pub(super) fn invitation(i: Value) -> Option<Invitation> {
        Some(Invitation {
            id: i["id"].as_u64()?,
            login: i["invitee"]["login"].as_str()?.to_owned(),
            permission: Permission::parse(i["permissions"].as_str()?)?,
            created_at: text(&i["created_at"]),
            expired: i["expired"].as_bool().unwrap_or_default(),
        })
    }

    pub(super) fn hook(h: Value) -> Option<Hook> {
        let config = &h["config"];
        let last = &h["last_response"];
        let status = text(&last["status"]);
        Some(Hook {
            id: h["id"].as_u64()?,
            form: HookForm {
                url: text(&config["url"]),
                content_type: config["content_type"].as_str().unwrap_or("form").to_owned(),
                secret: None,
                verify_ssl: config["insecure_ssl"].as_str().unwrap_or("0") == "0",
                events: (h["events"].as_array().into_iter().flatten())
                    .map(text)
                    .collect(),
                active: h["active"].as_bool().unwrap_or(true),
            },
            secret: config["secret"].as_str().is_some_and(|s| !s.is_empty()),
            last: (!status.is_empty() && status != "unused").then(|| LastResponse {
                code: last["code"].as_u64().and_then(|c| u16::try_from(c).ok()),
                status,
                message: last["message"]
                    .as_str()
                    .filter(|m| !m.is_empty())
                    .map(str::to_owned),
            }),
            updated_at: text(&h["updated_at"]),
        })
    }

    pub(super) fn delivery(d: Value) -> Option<Delivery> {
        Some(Delivery {
            id: d["id"].as_u64()?,
            guid: text(&d["guid"]),
            delivered_at: text(&d["delivered_at"]),
            redelivery: d["redelivery"].as_bool().unwrap_or_default(),
            status: text(&d["status"]),
            code: d["status_code"]
                .as_u64()
                .and_then(|c| u16::try_from(c).ok())
                .unwrap_or_default(),
            event: text(&d["event"]),
            action: d["action"].as_str().map(str::to_owned),
        })
    }

    pub(super) fn key(k: Value) -> Option<DeployKey> {
        Some(DeployKey {
            id: k["id"].as_u64()?,
            title: text(&k["title"]),
            key: text(&k["key"]),
            read_only: k["read_only"].as_bool().unwrap_or(true),
            created_at: text(&k["created_at"]),
            last_used: k["last_used"].as_str().map(str::to_owned),
            added_by: k["added_by"].as_str().map(str::to_owned),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A rule's edit sends only what changed, and keeps a check's app.
    #[test]
    fn an_edited_rule_sends_only_what_changed() {
        let from = RuleForm {
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
        };
        let mut to = from.clone();
        if let Some(r) = &mut to.reviews {
            r.count = 2;
        }
        if let Some(c) = &mut to.checks {
            c.contexts.push("lint".into());
        }
        let apps = [("check".to_owned(), NodeId::new("A_1"))];
        let input = Value::Object(rule_input(&from, &to, &apps, false));
        assert_eq!(
            input,
            json!({
                "requiredApprovingReviewCount": 2,
                "requiredStatusChecks": [{"context": "check", "appId": "A_1"}, {"context": "lint"}],
            })
        );
    }

    /// A webhook's edit sends what changed, its whole config when any of
    /// it did (REST replaces it), and a secret only when one is typed.
    #[test]
    fn an_edited_hook_sends_what_changed() {
        let from = HookForm {
            url: "https://example.com/a".into(),
            ..HookForm::default()
        };
        let to = HookForm {
            active: false,
            ..from.clone()
        };
        assert_eq!(hook_body(Some(&from), &to), json!({"active": false}));
        let to = HookForm {
            url: "https://example.com/b".into(),
            secret: Some("s3".into()),
            ..from.clone()
        };
        assert_eq!(
            hook_body(Some(&from), &to),
            json!({"config": {
                "url": "https://example.com/b", "secret": "s3",
                "content_type": "json", "insecure_ssl": "0",
            }})
        );
    }

    /// A refusal names the scope a classic token lacks, else admin
    /// access.
    #[test]
    fn a_refusal_names_what_is_missing() {
        let why = refused(404, Some("repo, gist"), "webhooks", "admin:repo_hook");
        assert_eq!(
            why.as_deref(),
            Some("GitHub shows webhooks only to the repository's admins")
        );
        let why = refused(404, Some("gist"), "webhooks", "admin:repo_hook").unwrap_or_default();
        assert!(
            why.contains("`gh auth refresh -s admin:repo_hook`"),
            "{why}"
        );
        assert_eq!(refused(500, None, "webhooks", "admin:repo_hook"), None);
    }
}
