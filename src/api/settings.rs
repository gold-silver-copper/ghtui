//! A repository's settings, as its Settings tab shows them to admins:
//! branch protection rules and rulesets, collaborators and invitations,
//! webhooks and their deliveries, deploy keys, and whether it's archived;
//! Actions secrets, variables and caches ([`actions`]), its Pages site
//! ([`pages`]) and custom properties ([`properties`]).
//! Reading is [`GitHub::settings`]; every change is a [`SettingsChange`],
//! sent by [`GitHub::change_setting`].

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::api::client::{ApiError, GitHub};
use crate::api::model::{Capped, NodeId, RepoId};
use crate::api::{has_scope, raw, scope_missing};

pub mod actions;
pub mod pages;
pub mod properties;

pub use actions::{ActionsChange, Caches, Secret, Store, Variable};
pub use pages::{Pages, PagesChange, PagesForm};
pub use properties::{PropValue, Properties, PropertyValues};

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
    /// Actions secrets: the repository's, its environments' and its
    /// organization's.
    Secrets,
    /// Actions variables, as secrets are kept.
    Variables,
    /// Its GitHub Pages site.
    Pages,
    /// Actions caches, under the Actions tab.
    Caches,
    /// Its custom properties, shown to anyone who can read it.
    Properties,
}

impl SettingsPage {
    /// The pages of the tab's own menu, in GitHub's order.
    pub const MENU: [SettingsPage; 8] = [
        SettingsPage::General,
        SettingsPage::Access,
        SettingsPage::Branches,
        SettingsPage::Hooks,
        SettingsPage::Pages,
        SettingsPage::Keys,
        SettingsPage::Secrets,
        SettingsPage::Variables,
    ];

    /// What follows the repository's own path in its URL.
    pub fn slug(self) -> String {
        match self {
            SettingsPage::General => "/settings".into(),
            SettingsPage::Branches => "/settings/branches".into(),
            SettingsPage::Access => "/settings/access".into(),
            SettingsPage::Hooks => "/settings/hooks".into(),
            SettingsPage::Hook(id) => format!("/settings/hooks/{id}"),
            SettingsPage::Keys => "/settings/keys".into(),
            SettingsPage::Secrets => "/settings/secrets/actions".into(),
            SettingsPage::Variables => "/settings/variables/actions".into(),
            SettingsPage::Pages => "/settings/pages".into(),
            SettingsPage::Caches => "/actions/caches".into(),
            SettingsPage::Properties => "/custom-properties".into(),
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
            SettingsPage::Secrets => "Secrets",
            SettingsPage::Variables => "Variables",
            SettingsPage::Pages => "Pages",
            SettingsPage::Caches => "Caches",
            SettingsPage::Properties => "Custom properties",
        }
    }

    /// GitHub shows it only to admins (caches and custom properties are
    /// for anyone who can read the repository).
    pub fn admins_only(self) -> bool {
        !matches!(self, SettingsPage::Caches | SettingsPage::Properties)
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
    Secrets(Box<Store<Secret>>),
    Variables(Box<Store<Variable>>),
    Pages(Box<Pages>),
    Caches(Caches),
    Properties(Properties),
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

words! {
    #[derive(Serialize, Deserialize)]
    pub enum Enforcement { pub fn word -> fn from_word, pub fn label }
    Active   "active"   "Active";
    Disabled "disabled" "Disabled";
    /// Evaluate: it reports, without enforcing (Enterprise).
    Evaluate "evaluate" "Evaluate";
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

words! {
    /// A repository role GitHub gives.
    #[derive(Hash, Serialize, Deserialize)]
    pub enum Permission {
        pub fn name,
        /// What it lets someone do, in GitHub's words.
        pub fn about,
        /// The word REST takes for it.
        fn wire,
    }
    Read     "Read"     "read and clone; open and comment on issues and pull requests" "pull";
    Triage   "Triage"   "read, and manage issues and pull requests"                    "triage";
    Write    "Write"    "read, clone and push"                                         "push";
    Maintain "Maintain" "push, and manage the repository without its settings"         "maintain";
    Admin    "Admin"    "everything, settings and deleting it included"                "admin";
}

impl Permission {
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

/// A webhook's delivery, read straight from GitHub's REST answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delivery {
    pub id: u64,
    /// Shared by a delivery and its redeliveries.
    #[serde(default)]
    pub guid: String,
    /// ISO 8601.
    #[serde(default)]
    pub delivered_at: String,
    #[serde(default)]
    pub redelivery: bool,
    /// `OK`, or what went wrong.
    #[serde(default)]
    pub status: String,
    #[serde(default, alias = "status_code")]
    pub code: u16,
    #[serde(default)]
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
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
    /// Secrets, variables and caches.
    Actions(ActionsChange),
    Pages(PagesChange),
    SetProperties {
        repo: RepoId,
        values: PropertyValues,
    },
}

/// How many a REST list asks for.
const PER_PAGE: usize = 100;

/// GitHub's words for a REST refusal of a page only admins see, said as
/// what's missing: a scope, or admin access.
fn refused(status: u16, scopes: Option<&str>, what: &str, scope: &str) -> Option<String> {
    if !matches!(status, 403 | 404) {
        return None;
    }
    let lacks = scopes.is_some_and(|s| !has_scope(s, "repo") && !has_scope(s, scope));
    Some(if lacks {
        scope_missing(what, scope)
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
            SettingsPage::Access => {
                let collaborators = "collaborators?affiliation=direct&";
                let (people, invitations) = tokio::try_join!(
                    self.listed(repo, collaborators, "collaborators", wire::person),
                    self.listed(repo, "invitations?", "invitations", wire::invitation),
                )?;
                Settings::Access(Access {
                    people,
                    invitations,
                })
            }
            SettingsPage::Hooks => {
                Settings::Hooks(self.listed(repo, "hooks?", "webhooks", wire::hook).await?)
            }
            SettingsPage::Hook(id) => {
                let path = format!("/repos/{repo}/hooks/{id}");
                let deliveries = format!("hooks/{id}/deliveries?");
                let (answer, deliveries) = tokio::try_join!(
                    self.admin_answer(&path, "webhooks"),
                    self.listed(repo, &deliveries, "webhooks", Some),
                )?;
                let hook = wire::hook(serde_json::from_str(answer.body())?)
                    .ok_or_else(|| ApiError::Decode(format!("{repo}'s webhook {id}")))?;
                Settings::Hook(Box::new(HookPage { hook, deliveries }))
            }
            SettingsPage::Keys => {
                Settings::Keys(self.listed(repo, "keys?", "deploy keys", wire::key).await?)
            }
            SettingsPage::Secrets => Settings::Secrets(Box::new(self.kept(repo).await?)),
            SettingsPage::Variables => Settings::Variables(Box::new(self.kept(repo).await?)),
            SettingsPage::Pages => Settings::Pages(Box::new(self.pages(repo).await?)),
            SettingsPage::Caches => Settings::Caches(self.caches(repo).await?),
            SettingsPage::Properties => Settings::Properties(self.properties(repo).await?),
        };
        Ok(settings)
    }

    /// GitHub's answer for a path only admins may read, or why not.
    async fn admin_answer(
        &self,
        path: &str,
        what: &str,
    ) -> Result<crate::api::client::Answer, ApiError> {
        let scope = if what == "webhooks" {
            "admin:repo_hook"
        } else {
            "repo"
        };
        let answer = self.rest_refusable(path, what, |status, _, answer| {
            refused(status, answer.header("x-oauth-scopes"), what, scope)
        });
        answer.await?.map_err(ApiError::Denied)
    }

    /// The first page of a list of the repository's (`list` is its path
    /// and query, ending in `?` or `&`), each item read by `read`.
    async fn listed<W: serde::de::DeserializeOwned, T>(
        &self,
        repo: &RepoId,
        list: &str,
        what: &str,
        read: fn(W) -> Option<T>,
    ) -> Result<Listed<T>, ApiError> {
        let path = format!("/repos/{repo}/{list}per_page={PER_PAGE}");
        let answer = self.admin_answer(&path, what).await?;
        let wire: Vec<W> = serde_json::from_str(answer.body())?;
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
        let wire: Wire = self.rest_json(&path, repo).await?;
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
        let vars = json!(repo);
        let rulesets = format!("/repos/{repo}/rulesets?per_page={PER_PAGE}");
        let (found, rulesets) = tokio::join!(
            self.graphql_json(raw::PROTECTION_RULES, vars, repo, "/repository"),
            self.admin_answer(&rulesets, "rulesets"),
        );
        let found: wire::Rules = found?;
        let repo_id = NodeId::new(found.id);
        let rules = (found.branch_protection_rules).into_capped(|n| Some(wire::rule(n)));
        let rulesets = match rulesets {
            Ok(answer) => {
                let wire: Vec<wire::RulesetWire> = serde_json::from_str(answer.body())?;
                Ok(wire.into_iter().map(|r| wire::ruleset(r, repo)).collect())
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
                let vars = json!({ "id": id });
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
            C::Actions(change) => self.change_actions(change).await,
            C::Pages(change) => self.change_pages(change).await,
            C::SetProperties { repo, values } => self.set_properties(repo, values).await,
        }
    }
}

/// One of a rule's settings, as GitHub's input has it, read from its form.
type Field = fn(&RuleForm) -> Value;

/// A rule's reviews, the defaults when it asks for none.
fn reviews(f: &RuleForm) -> Reviews {
    f.reviews.clone().unwrap_or_default()
}

/// A rule's status checks, the defaults when it asks for none.
fn checks(f: &RuleForm) -> Checks {
    f.checks.clone().unwrap_or_default()
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
    let fields: [(&str, Field); 15] = [
        ("pattern", |f| f.pattern.clone().into()),
        ("requiresApprovingReviews", |f| f.reviews.is_some().into()),
        ("requiredApprovingReviewCount", |f| reviews(f).count.into()),
        ("dismissesStaleReviews", |f| reviews(f).dismiss_stale.into()),
        ("requiresCodeOwnerReviews", |f| {
            reviews(f).code_owners.into()
        }),
        ("requireLastPushApproval", |f| reviews(f).last_push.into()),
        ("requiresStatusChecks", |f| f.checks.is_some().into()),
        ("requiresStrictStatusChecks", |f| checks(f).strict.into()),
        ("requiresConversationResolution", |f| f.conversations.into()),
        ("requiresCommitSignatures", |f| f.signed.into()),
        ("requiresLinearHistory", |f| f.linear.into()),
        ("lockBranch", |f| f.lock.into()),
        ("isAdminEnforced", |f| f.admins.into()),
        ("allowsForcePushes", |f| f.force_pushes.into()),
        ("allowsDeletions", |f| f.deletions.into()),
    ];
    // The one that isn't a [`Field`]: it reads `apps`.
    let contexts = |f: &RuleForm| -> Value {
        checks(f)
            .contexts
            .iter()
            .map(
                |context| match apps.iter().find(|(name, _)| name == context) {
                    Some((_, id)) => json!({ "context": context, "appId": id }),
                    None => json!({ "context": context }),
                },
            )
            .collect()
    };
    (fields.into_iter())
        .map(|(name, field)| (name, field(from), field(to)))
        .chain([("requiredStatusChecks", contexts(from), contexts(to))])
        .filter(|(_, old, new)| all || old != new)
        .map(|(name, _, new)| (name.to_owned(), new))
        .collect()
}

/// A webhook's body for REST: everything (`from: None`), or what differs.
fn hook_body(from: Option<&HookForm>, to: &HookForm) -> Value {
    let mut body = Map::new();
    let changed = |f: fn(&HookForm) -> Value| from.is_none_or(|from| f(from) != f(to));
    if from.is_none() {
        body.insert("name".into(), "web".into());
    }
    let config = |h: &HookForm| {
        let insecure = if h.verify_ssl { "0" } else { "1" };
        let mut config =
            json!({ "url": h.url, "content_type": h.content_type, "insecure_ssl": insecure });
        if let (Some(secret), Value::Object(config)) = (&h.secret, &mut config) {
            config.insert("secret".into(), secret.as_str().into());
        }
        config
    };
    // REST replaces the whole config, so a change sends all of it: what
    // didn't change is the same in `to`. A hook's form holds a secret only
    // when it's new.
    if changed(config) {
        body.insert("config".into(), config(to));
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
    use serde::Deserialize;

    use super::*;
    use crate::api::browse::wire::{Counted, Name};

    /// A branch protection rule, as GraphQL sends it.
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub(super) struct Rules {
        pub id: String,
        pub branch_protection_rules: Counted<RuleNode>,
    }

    #[derive(Deserialize, Default)]
    #[serde(rename_all = "camelCase", default)]
    pub(super) struct RuleNode {
        id: String,
        pattern: String,
        requires_approving_reviews: bool,
        required_approving_review_count: Option<u64>,
        dismisses_stale_reviews: bool,
        requires_code_owner_reviews: bool,
        require_last_push_approval: bool,
        requires_status_checks: bool,
        requires_strict_status_checks: bool,
        required_status_checks: Option<Vec<Check>>,
        requires_conversation_resolution: bool,
        requires_commit_signatures: bool,
        requires_linear_history: bool,
        lock_branch: bool,
        is_admin_enforced: bool,
        allows_force_pushes: bool,
        allows_deletions: bool,
        matching_refs: Option<Counted<Name>>,
    }

    #[derive(Deserialize, Default)]
    struct Check {
        context: String,
        app: Option<App>,
    }

    #[derive(Deserialize, Default)]
    struct App {
        id: String,
    }

    pub(super) fn rule(n: RuleNode) -> Rule {
        let checks = n.required_status_checks.unwrap_or_default();
        let apps = (checks.iter())
            .filter_map(|c| Some((c.context.clone(), NodeId::new(&c.app.as_ref()?.id))))
            .collect();
        let contexts = checks.into_iter().map(|c| c.context).collect();
        let matching =
            (n.matching_refs).map_or_else(Capped::default, |m| m.into_capped(|r| Some(r.name)));
        Rule {
            id: NodeId::new(n.id),
            form: RuleForm {
                pattern: n.pattern,
                reviews: n.requires_approving_reviews.then(|| Reviews {
                    count: u8::try_from(n.required_approving_review_count.unwrap_or_default())
                        .unwrap_or(u8::MAX),
                    dismiss_stale: n.dismisses_stale_reviews,
                    code_owners: n.requires_code_owner_reviews,
                    last_push: n.require_last_push_approval,
                }),
                checks: n.requires_status_checks.then_some(Checks {
                    strict: n.requires_strict_status_checks,
                    contexts,
                }),
                conversations: n.requires_conversation_resolution,
                signed: n.requires_commit_signatures,
                linear: n.requires_linear_history,
                lock: n.lock_branch,
                admins: n.is_admin_enforced,
                force_pushes: n.allows_force_pushes,
                deletions: n.allows_deletions,
            },
            apps,
            matching,
        }
    }

    #[derive(Deserialize)]
    pub(super) struct RulesetWire {
        id: u64,
        #[serde(default)]
        name: String,
        target: Option<String>,
        #[serde(default)]
        enforcement: String,
        #[serde(default)]
        source: String,
        #[serde(default)]
        source_type: String,
    }

    pub(super) fn ruleset(r: RulesetWire, repo: &RepoId) -> Ruleset {
        let theirs =
            r.source_type == "Organization" || !r.source.eq_ignore_ascii_case(&repo.to_string());
        Ruleset {
            id: r.id,
            name: r.name,
            target: r.target.unwrap_or_else(|| "branch".into()),
            enforcement: Enforcement::from_word(&r.enforcement).unwrap_or(Enforcement::Disabled),
            org: Some(r.source).filter(|s| theirs && !s.is_empty()),
        }
    }

    #[derive(Deserialize)]
    pub(super) struct Person {
        login: String,
        #[serde(default)]
        role_name: String,
    }

    #[expect(clippy::unnecessary_wraps, reason = "read as every list's items are")]
    pub(super) fn person(p: Person) -> Option<Collaborator> {
        Some(Collaborator {
            login: p.login,
            role: Permission::parse(&p.role_name).map_or(Role::Custom(p.role_name), Role::Is),
        })
    }

    #[derive(Deserialize)]
    pub(super) struct InvitationWire {
        id: u64,
        invitee: Option<crate::api::browse::UserLogin>,
        #[serde(default)]
        permissions: String,
        #[serde(default)]
        created_at: String,
        #[serde(default)]
        expired: bool,
    }

    pub(super) fn invitation(i: InvitationWire) -> Option<Invitation> {
        Some(Invitation {
            id: i.id,
            login: i.invitee?.login,
            permission: Permission::parse(&i.permissions)?,
            created_at: i.created_at,
            expired: i.expired,
        })
    }

    #[derive(Deserialize)]
    pub(super) struct HookWire {
        id: u64,
        #[serde(default)]
        config: Config,
        #[serde(default)]
        events: Vec<String>,
        active: Option<bool>,
        last_response: Option<Last>,
        #[serde(default)]
        updated_at: String,
    }

    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Config {
        url: String,
        content_type: Option<String>,
        insecure_ssl: Option<String>,
        secret: Option<String>,
    }

    #[derive(Deserialize)]
    struct Last {
        code: Option<u16>,
        #[serde(default)]
        status: String,
        message: Option<String>,
    }

    #[expect(clippy::unnecessary_wraps, reason = "read as every list's items are")]
    pub(super) fn hook(h: HookWire) -> Option<Hook> {
        let c = h.config;
        Some(Hook {
            id: h.id,
            form: HookForm {
                url: c.url,
                content_type: c.content_type.unwrap_or_else(|| "form".into()),
                secret: None,
                verify_ssl: c.insecure_ssl.as_deref().unwrap_or("0") == "0",
                events: h.events,
                active: h.active.unwrap_or(true),
            },
            secret: c.secret.is_some_and(|s| !s.is_empty()),
            last: (h.last_response)
                .filter(|l| !l.status.is_empty() && l.status != "unused")
                .map(|l| LastResponse {
                    code: l.code,
                    status: l.status,
                    message: l.message.filter(|m| !m.is_empty()),
                }),
            updated_at: h.updated_at,
        })
    }

    #[derive(Deserialize)]
    pub(super) struct KeyWire {
        id: u64,
        #[serde(default)]
        title: String,
        #[serde(default)]
        key: String,
        read_only: Option<bool>,
        #[serde(default)]
        created_at: String,
        last_used: Option<String>,
        added_by: Option<String>,
    }

    #[expect(clippy::unnecessary_wraps, reason = "read as every list's items are")]
    pub(super) fn key(k: KeyWire) -> Option<DeployKey> {
        Some(DeployKey {
            id: k.id,
            title: k.title,
            key: k.key,
            read_only: k.read_only.unwrap_or(true),
            created_at: k.created_at,
            last_used: k.last_used,
            added_by: k.added_by,
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

    /// Every field a rule's input names is one GitHub's mutations take
    /// (the input is JSON, which the query checks don't see).
    #[test]
    fn a_rules_input_is_githubs() {
        let sdl = include_str!(env!("GITHUB_SDL"));
        let input = rule_input(&RuleForm::default(), &RuleForm::default(), &[], true);
        for kind in [
            "CreateBranchProtectionRuleInput",
            "UpdateBranchProtectionRuleInput",
        ] {
            let (_, after) = sdl
                .split_once(&format!("input {kind} "))
                .unwrap_or_default();
            let (fields, _) = after.split_once("\n}").unwrap_or_default();
            for name in input.keys() {
                assert!(
                    fields.contains(&format!("  {name}:")),
                    "{kind} has no {name}"
                );
            }
        }
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
