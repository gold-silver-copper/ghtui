//! The Security tab's alerts: Dependabot's (vulnerable dependencies),
//! code scanning's and secret scanning's. All REST, but a Dependabot
//! alert's pull request, which only GraphQL knows.
//!
//! Each needs what tokens often lack: a classic token's `security_events`
//! (or `repo`) scope, a fine-grained token's permission for that kind of
//! alert, and the account's own access (write, or admin for secrets). A
//! refusal says which, and how to add a scope ([`refusal`]).

use serde::{Deserialize, Serialize};

use crate::api::browse::{Results, Severity};
use crate::api::client::{Answer, ApiError, GitHub};
use crate::api::model::RepoId;

/// A kind of alert, as the Security tab lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AlertKind {
    Dependabot,
    CodeScanning,
    SecretScanning,
}

impl AlertKind {
    pub const ALL: [AlertKind; 3] = [
        AlertKind::Dependabot,
        AlertKind::CodeScanning,
        AlertKind::SecretScanning,
    ];

    /// Its part of GitHub's paths, on the website and in the API.
    pub fn slug(self) -> &'static str {
        match self {
            AlertKind::Dependabot => "dependabot",
            AlertKind::CodeScanning => "code-scanning",
            AlertKind::SecretScanning => "secret-scanning",
        }
    }

    pub fn from_slug(slug: &str) -> Option<AlertKind> {
        Self::ALL.into_iter().find(|k| k.slug() == slug)
    }

    /// An alert of this kind, mid-sentence: `code scanning alert #3`.
    pub fn alert(self, number: u64) -> String {
        let kind = match self {
            AlertKind::Dependabot => "Dependabot",
            AlertKind::CodeScanning => "code scanning",
            AlertKind::SecretScanning => "secret scanning",
        };
        format!("{kind} alert #{number}")
    }

    /// `Dependabot`, as the tab's menu names it.
    pub fn name(self) -> &'static str {
        match self {
            AlertKind::Dependabot => "Dependabot",
            AlertKind::CodeScanning => "Code scanning",
            AlertKind::SecretScanning => "Secret scanning",
        }
    }

    /// `Dependabot alerts`.
    pub fn alerts(self) -> &'static str {
        match self {
            AlertKind::Dependabot => "Dependabot alerts",
            AlertKind::CodeScanning => "Code scanning alerts",
            AlertKind::SecretScanning => "Secret scanning alerts",
        }
    }

    /// The query parameter its website's list keeps its filter in.
    pub fn query_param(self) -> &'static str {
        match self {
            AlertKind::Dependabot => "q",
            AlertKind::CodeScanning | AlertKind::SecretScanning => "query",
        }
    }

    /// The fine-grained token permission it needs, as GitHub names it.
    pub fn permission(self) -> &'static str {
        // Code scanning's is called "Code scanning alerts".
        self.alerts()
    }

    /// Whole filters its list takes, each with what it finds: the search
    /// box suggests them.
    pub fn filters(self) -> &'static [(&'static str, &'static str)] {
        match self {
            AlertKind::Dependabot => &[
                ("is:open", "open"),
                ("is:closed", "dismissed or fixed"),
                ("severity:critical", "critical"),
                ("severity:high", "high severity"),
                ("scope:runtime", "in runtime dependencies"),
                ("scope:development", "in development dependencies"),
            ],
            AlertKind::CodeScanning => &[
                ("is:open", "open"),
                ("is:closed", "dismissed or fixed"),
                ("severity:critical", "critical"),
                ("severity:high", "high severity"),
                ("severity:error", "errors"),
                ("severity:warning", "warnings"),
            ],
            AlertKind::SecretScanning => &[
                ("is:open", "open"),
                ("is:closed", "resolved"),
                ("validity:active", "still working"),
                ("validity:inactive", "no longer working"),
                ("resolution:revoked", "resolved as revoked"),
            ],
        }
    }

    /// The reasons to dismiss (for secrets, resolve) an alert of this
    /// kind: GitHub's word for each, a label, and what it means.
    pub fn reasons(self) -> &'static [(&'static str, &'static str, &'static str)] {
        match self {
            AlertKind::Dependabot => &[
                (
                    "fix_started",
                    "A fix has already been started",
                    "fix started",
                ),
                ("no_bandwidth", "No bandwidth to fix this", "no bandwidth"),
                (
                    "tolerable_risk",
                    "Risk is tolerable to this project",
                    "tolerable risk",
                ),
                (
                    "inaccurate",
                    "This alert is inaccurate or incorrect",
                    "inaccurate",
                ),
                (
                    "not_used",
                    "Vulnerable code is not actually used",
                    "not used",
                ),
            ],
            AlertKind::CodeScanning => &[
                ("false positive", "False positive", "the alert isn't valid"),
                ("won't fix", "Won't fix", "the alert isn't relevant"),
                (
                    "used in tests",
                    "Used in tests",
                    "the code is only in tests",
                ),
            ],
            AlertKind::SecretScanning => &[
                ("revoked", "Revoked", "the secret no longer works"),
                ("false_positive", "False positive", "it isn't a secret"),
                ("used_in_tests", "Used in tests", "it's only in tests"),
                ("wont_fix", "Won't fix", "it's not worth fixing"),
            ],
        }
    }
}

/// One alert of a repository.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AlertRef {
    pub repo: RepoId,
    pub kind: AlertKind,
    pub number: u64,
}

/// Where an alert stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlertState {
    Open,
    Dismissed,
    /// Dependabot dismissed it itself (a development dependency, say).
    AutoDismissed,
    Fixed,
    /// A secret's: someone said what became of it.
    Resolved,
}

impl AlertState {
    fn parse(s: &str) -> AlertState {
        match s {
            "dismissed" => AlertState::Dismissed,
            "auto_dismissed" => AlertState::AutoDismissed,
            "fixed" => AlertState::Fixed,
            "resolved" => AlertState::Resolved,
            _ => AlertState::Open,
        }
    }

    pub fn is_open(self) -> bool {
        self == AlertState::Open
    }

    pub fn label(self) -> &'static str {
        match self {
            AlertState::Open => "Open",
            AlertState::Dismissed => "Dismissed",
            AlertState::AutoDismissed => "Auto-dismissed",
            AlertState::Fixed => "Fixed",
            AlertState::Resolved => "Closed",
        }
    }
}

/// Who closed an alert, when, why and what they said.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Closing {
    pub by: Option<String>,
    pub at: Option<String>,
    /// GitHub's word for why (`tolerable_risk`, `won't fix`, `revoked`).
    pub reason: Option<String>,
    pub comment: Option<String>,
}

/// An alert, as lists and its own page show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alert {
    pub number: u64,
    pub state: AlertState,
    /// What it's about: the advisory's summary, the rule, the kind of
    /// secret.
    pub title: String,
    /// How severe, where GitHub rates it.
    pub severity: Option<Severity>,
    /// A code scanning rule's level (`error`, `warning`, `note`), shown
    /// where no security severity is.
    pub level: Option<String>,
    pub created_at: String,
    pub updated_at: Option<String>,
    pub fixed_at: Option<String>,
    pub closing: Option<Closing>,
    pub detail: Detail,
}

impl Alert {
    pub fn kind(&self) -> AlertKind {
        match self.detail {
            Detail::Dependency(_) => AlertKind::Dependabot,
            Detail::Code(_) => AlertKind::CodeScanning,
            Detail::Secret(_) => AlertKind::SecretScanning,
        }
    }

    /// Why it was closed, as people say it.
    pub fn reason_label(&self) -> Option<&str> {
        let reason = self.closing.as_ref()?.reason.as_deref()?;
        let reasons = self.kind().reasons();
        Some(
            reasons
                .iter()
                .find(|(word, ..)| *word == reason)
                .map_or(reason, |(_, label, _)| label),
        )
    }
}

/// What's particular to each kind of alert.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Detail {
    Dependency(Box<Dependency>),
    Code(Box<CodeAlert>),
    Secret(Box<SecretAlert>),
}

/// A vulnerable dependency and the advisory about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dependency {
    pub package: String,
    pub ecosystem: String,
    /// The lockfile or manifest it's in.
    pub manifest: String,
    /// `runtime` or `development`.
    pub scope: Option<String>,
    /// `direct`, `transitive` (or GitHub doesn't know).
    pub relationship: Option<String>,
    /// The versions affected, and the first that isn't.
    pub vulnerable: Option<String>,
    pub patched: Option<String>,
    pub ghsa: String,
    pub cve: Option<String>,
    pub description: String,
    pub cwes: Vec<String>,
    pub references: Vec<String>,
    /// Dependabot's pull request for it, or why it couldn't make one: on
    /// the alert's own page only.
    pub update: Option<Update>,
}

/// What Dependabot did about an alert.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Update {
    Pr {
        number: u64,
        title: String,
        /// `OPEN`, `CLOSED`, `MERGED`.
        state: String,
    },
    Failed {
        title: String,
        body: String,
    },
}

/// A code scanning tool's finding, where it last saw it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeAlert {
    pub rule: String,
    pub rule_description: String,
    pub help: String,
    /// `CodeQL 2.19.0`.
    pub tool: String,
    pub message: String,
    pub path: Option<String>,
    pub lines: Option<(u32, u32)>,
    /// The commit the tool looked at: its lines are that commit's.
    pub commit: Option<String>,
    /// `refs/heads/main`.
    pub git_ref: Option<String>,
    pub tags: Vec<String>,
}

/// A secret found in the repository. The secret itself isn't kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretAlert {
    /// `GitHub Personal Access Token`.
    pub secret_type: String,
    /// `active`, `inactive`, `unknown`.
    pub validity: Option<String>,
    pub publicly_leaked: bool,
    /// Who pushed it past push protection.
    pub bypassed_by: Option<String>,
    /// Where it is: on the alert's page, all of them; in a list, the
    /// first found.
    pub locations: Vec<SecretLocation>,
}

/// Where a secret is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SecretLocation {
    /// In a file, at a commit.
    File {
        path: String,
        lines: (u32, u32),
        commit: String,
    },
    /// Elsewhere: an issue's title, a comment… (GitHub's word for it).
    Other(String),
}

/// A page of a repository's alerts of one kind, or why there are none to
/// list: GitHub's word that the feature is off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertList {
    pub results: Results<Alert>,
    pub off: Option<String>,
}

impl crate::api::browse::Page for AlertList {
    fn counts(&self) -> (usize, u64) {
        self.results.counts()
    }
}

/// Dismissing (for a secret, resolving) an alert, or reopening it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertChange {
    pub alert: AlertRef,
    /// GitHub's word for why, and a comment (may be empty); `None`
    /// reopens it.
    pub dismiss: Option<(String, String)>,
}

/// A list's filter, as the REST API's parameters, and the words it
/// doesn't take.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Filter {
    pub params: Vec<(&'static str, String)>,
    pub ignored: Vec<String>,
}

/// `query` (as GitHub's website writes it: `is:open severity:high`) as
/// the REST API's parameters for `kind`'s list.
pub fn filter(kind: AlertKind, query: &str) -> Filter {
    use AlertKind as K;
    let mut f = Filter::default();
    for word in query.split_whitespace() {
        let Some((key, value)) = word.split_once(':') else {
            f.ignored.push(word.to_owned());
            continue;
        };
        let param = match (kind, key, value) {
            (_, "is" | "state", "open") => Some(("state", "open".to_owned())),
            (K::Dependabot, "is" | "state", "closed") => {
                Some(("state", "dismissed,fixed,auto_dismissed".to_owned()))
            }
            (K::CodeScanning, "is" | "state", "closed") => Some(("state", "closed".to_owned())),
            (K::SecretScanning, "is" | "state", "closed") => Some(("state", "resolved".to_owned())),
            (K::Dependabot, "is" | "state", "dismissed" | "fixed" | "auto_dismissed")
            | (K::CodeScanning, "is" | "state", "dismissed" | "fixed") => {
                Some(("state", value.to_owned()))
            }
            (K::Dependabot | K::CodeScanning, "severity", _) => {
                Some(("severity", value.to_owned()))
            }
            (K::Dependabot, "ecosystem", _) => Some(("ecosystem", value.to_owned())),
            (K::Dependabot, "package", _) => Some(("package", value.to_owned())),
            (K::Dependabot, "manifest", _) => Some(("manifest", value.to_owned())),
            (K::Dependabot, "scope", "runtime" | "development") => {
                Some(("scope", value.to_owned()))
            }
            (K::CodeScanning, "tool", _) => Some(("tool_name", value.to_owned())),
            (K::CodeScanning, "branch", _) => Some(("ref", format!("refs/heads/{value}"))),
            (K::CodeScanning, "ref", _) => Some(("ref", value.to_owned())),
            (K::SecretScanning, "secret-type", _) => Some(("secret_type", value.to_owned())),
            (K::SecretScanning, "resolution", _) => Some(("resolution", value.to_owned())),
            (K::SecretScanning, "validity", _) => Some(("validity", value.to_owned())),
            _ => None,
        };
        match param {
            Some(p) if !value.is_empty() => f.params.push(p),
            _ => f.ignored.push(word.to_owned()),
        }
    }
    f
}

/// How many alerts a page of a list has.
const PAGE: usize = 30;

/// Why GitHub answered an alerts request with an error, read from its
/// answer: the feature is off, the token lacks a scope or permission,
/// or the account lacks access. `None`: an ordinary error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The feature is off (or never ran): there's nothing to list.
    Off(String),
    Denied(String),
}

/// What an error answer means. `scopes` is GitHub's `X-OAuth-Scopes`
/// (a classic token's), `permissions` its
/// `X-Accepted-GitHub-Permissions` (what a fine-grained one needs).
pub fn refusal(
    kind: AlertKind,
    status: u16,
    message: &str,
    scopes: Option<&str>,
    permissions: Option<&str>,
) -> Option<Refusal> {
    if !matches!(status, 403 | 404 | 422) {
        return None;
    }
    let lower = message.to_lowercase();
    let off = [
        "disabled",
        "not enabled",
        "no analysis found",
        "advanced security must be enabled",
    ];
    if off.iter().any(|w| lower.contains(w)) {
        let message = match lower.as_str() {
            "no analysis found" => {
                "Code scanning isn't set up: no tool has sent GitHub an analysis".to_owned()
            }
            _ => message.trim_end_matches('.').to_owned(),
        };
        return Some(Refusal::Off(message));
    }
    // A classic token (or gh's own) says its scopes.
    let held = |s: &str| scopes.is_some_and(|all| crate::api::has_scope(all, s));
    if scopes.is_some() && !held("repo") && !held("security_events") {
        let why = crate::api::scope_missing(kind.alerts(), "security_events");
        return Some(Refusal::Denied(why));
    }
    if lower.contains("not accessible by") {
        let named = permissions
            .and_then(|p| p.split(',').next())
            .and_then(|p| p.split_once('='))
            .map_or_else(|| "read".to_owned(), |(_, level)| level.trim().to_owned());
        return Some(Refusal::Denied(format!(
            "{} take the {}: {named} permission, which your token lacks: give it that permission on GitHub, under Settings › Developer settings",
            kind.alerts(),
            kind.permission()
        )));
    }
    // GitHub hides secrets from everyone but admins as if they weren't
    // there.
    if status == 403 || (status == 404 && kind == AlertKind::SecretScanning) {
        let who = match kind {
            AlertKind::SecretScanning => "admins",
            AlertKind::Dependabot | AlertKind::CodeScanning => "people with write access",
        };
        return Some(Refusal::Denied(format!(
            "GitHub shows {} only to the repository's {who} and security managers",
            kind.alerts().to_lowercase()
        )));
    }
    None
}

impl GitHub {
    /// GitHub's answer for an alerts path, or what its refusal means.
    async fn alerts_answer(
        &self,
        kind: AlertKind,
        path: &str,
        what: &str,
    ) -> Result<Result<Answer, String>, ApiError> {
        let answer = self.rest_refusable(path, what, |status, message, answer| {
            let scopes = answer.header("x-oauth-scopes");
            let permissions = answer.header("x-accepted-github-permissions");
            refusal(kind, status, message, scopes, permissions)
        });
        match answer.await? {
            Ok(answer) => Ok(Ok(answer)),
            Err(Refusal::Off(why)) => Ok(Err(why)),
            Err(Refusal::Denied(why)) => Err(ApiError::Denied(why)),
        }
    }

    /// A page of a repository's alerts of one kind, newest first, filtered
    /// by `query` (the website's words); from `after`, a cursor this gave.
    pub async fn alerts(
        &self,
        repo: &RepoId,
        kind: AlertKind,
        query: &str,
        after: Option<String>,
    ) -> Result<AlertList, ApiError> {
        let mut path = format!("/repos/{repo}/{}/alerts?per_page={PAGE}", kind.slug());
        for (name, value) in filter(kind, query).params {
            path.push_str(&format!(
                "&{name}={}",
                url::form_urlencoded::byte_serialize(value.as_bytes()).collect::<String>()
            ));
        }
        if let Some(after) = &after {
            path.push('&');
            path.push_str(after);
        }
        let what = format!("{repo}'s {}", kind.alerts().to_lowercase());
        let list = match self.alerts_answer(kind, &path, &what).await? {
            Err(off) => AlertList {
                results: Results::default(),
                off: Some(off),
            },
            Ok(answer) => {
                let items = wire::alerts(kind, answer.body())?;
                // A full page may have more after it.
                let next = answer.next_page().filter(|_| items.len() >= PAGE);
                AlertList {
                    results: Results {
                        total: items.len() as u64,
                        items,
                        next,
                    },
                    off: None,
                }
            }
        };
        Ok(self.checked(list, format!("{repo}'s alerts")))
    }

    /// One alert, with what only its page shows: a Dependabot alert's
    /// pull request, all of a secret's locations.
    pub async fn alert(&self, a: &AlertRef) -> Result<Alert, ApiError> {
        let path = format!("/repos/{}/{}/alerts/{}", a.repo, a.kind.slug(), a.number);
        let what = format!("{}'s {}", a.repo, a.kind.alert(a.number));
        let mut alert = match self.alerts_answer(a.kind, &path, &what).await? {
            Ok(answer) => wire::alert(a.kind, answer.body())?,
            Err(off) => return Err(ApiError::Denied(off)),
        };
        match &mut alert.detail {
            Detail::Dependency(d) => d.update = self.dependabot_update(a).await,
            Detail::Secret(s) => {
                let path = format!("{path}/locations?per_page=100");
                match self.rest_answer(&path).await {
                    Ok(answer) if (200..300).contains(&answer.status()) => {
                        if let Ok(found) = wire::locations(answer.body()) {
                            s.locations = found;
                        }
                    }
                    Ok(answer) => {
                        tracing::debug!(
                            status = answer.status(),
                            "couldn't list a secret's locations"
                        );
                    }
                    Err(err) => tracing::debug!(%err, "couldn't list a secret's locations"),
                }
            }
            Detail::Code(_) => {}
        }
        Ok(alert)
    }

    /// Dependabot's pull request for an alert, or why it couldn't make
    /// one; nothing if GraphQL won't say.
    async fn dependabot_update(&self, a: &AlertRef) -> Option<Update> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Wire {
            pull_request: Option<Pr>,
            error: Option<Failed>,
        }
        #[derive(Deserialize)]
        struct Pr {
            number: u64,
            title: String,
            state: String,
        }
        #[derive(Deserialize)]
        struct Failed {
            title: String,
            body: String,
        }
        let vars = serde_json::json!({
            "owner": a.repo.owner, "name": a.repo.name, "number": a.number,
        });
        let at = "/repository/vulnerabilityAlert/dependabotUpdate";
        let found: Result<Wire, _> = self
            .graphql_json(crate::api::raw::DEPENDABOT_UPDATE, vars, &a.number, at)
            .await;
        match found {
            Ok(Wire {
                pull_request: Some(pr),
                ..
            }) => Some(Update::Pr {
                number: pr.number,
                title: pr.title,
                state: pr.state,
            }),
            Ok(Wire { error: Some(e), .. }) => Some(Update::Failed {
                title: e.title,
                body: e.body,
            }),
            Ok(_) => None,
            Err(err) => {
                tracing::debug!(%err, "no Dependabot update for the alert");
                None
            }
        }
    }

    /// Dismisses (resolves) an alert, or reopens it.
    pub(crate) async fn set_alert(&self, c: &AlertChange) -> Result<(), ApiError> {
        let a = &c.alert;
        let path = format!("/repos/{}/{}/alerts/{}", a.repo, a.kind.slug(), a.number);
        let mut body = serde_json::Map::new();
        let mut put = |k: &str, v: &str| {
            body.insert(k.to_owned(), v.into());
        };
        match (&c.dismiss, a.kind) {
            (None, _) => put("state", "open"),
            (Some((reason, comment)), AlertKind::SecretScanning) => {
                put("state", "resolved");
                put("resolution", reason);
                if !comment.is_empty() {
                    put("resolution_comment", comment);
                }
            }
            (Some((reason, comment)), _) => {
                put("state", "dismissed");
                put("dismissed_reason", reason);
                if !comment.is_empty() {
                    put("dismissed_comment", comment);
                }
            }
        }
        let body = serde_json::Value::Object(body);
        self.rest_write(http::Method::PATCH, &path, Some(&body))
            .await
    }
}

/// GitHub's alerts as its REST API sends them.
mod wire {
    use serde::Deserialize;

    use super::{
        Alert, AlertKind, AlertState, Closing, CodeAlert, Detail, SecretAlert, SecretLocation,
    };
    use crate::api::ApiError;
    use crate::api::browse::Severity;
    use crate::api::browse::UserLogin as Login;
    use crate::api::browse::rest_advisories::{Cwe, Package};

    #[derive(Deserialize)]
    struct Dependency {
        package: Option<Package>,
        manifest_path: Option<String>,
        scope: Option<String>,
        relationship: Option<String>,
    }

    #[derive(Deserialize)]
    struct Version {
        identifier: String,
    }

    #[derive(Deserialize)]
    struct Vulnerability {
        severity: Option<Severity>,
        vulnerable_version_range: Option<String>,
        first_patched_version: Option<Version>,
    }

    #[derive(Deserialize)]
    struct Reference {
        url: String,
    }

    #[derive(Deserialize)]
    struct Advisory {
        ghsa_id: String,
        cve_id: Option<String>,
        summary: String,
        description: Option<String>,
        severity: Option<Severity>,
        #[serde(default)]
        cwes: Vec<Cwe>,
        #[serde(default)]
        references: Vec<Reference>,
    }

    #[derive(Deserialize)]
    struct Rule {
        id: Option<String>,
        name: Option<String>,
        severity: Option<String>,
        security_severity_level: Option<Severity>,
        description: Option<String>,
        full_description: Option<String>,
        help: Option<String>,
        #[serde(default)]
        tags: Vec<String>,
    }

    #[derive(Deserialize)]
    struct Tool {
        name: Option<String>,
        version: Option<String>,
    }

    #[derive(Deserialize)]
    struct Message {
        text: Option<String>,
    }

    #[derive(Deserialize)]
    struct Location {
        path: Option<String>,
        start_line: Option<u32>,
        end_line: Option<u32>,
    }

    #[derive(Deserialize)]
    struct Instance {
        #[serde(rename = "ref")]
        git_ref: Option<String>,
        commit_sha: Option<String>,
        message: Option<Message>,
        location: Option<Location>,
    }

    #[derive(Deserialize)]
    struct FileAt {
        path: Option<String>,
        start_line: Option<u32>,
        end_line: Option<u32>,
        commit_sha: Option<String>,
    }

    /// Every kind's fields in one: each reads its own. The secret
    /// itself (`secret`) isn't read.
    #[derive(Deserialize)]
    struct Wire {
        number: u64,
        state: String,
        created_at: String,
        updated_at: Option<String>,
        fixed_at: Option<String>,
        // Dismissed (Dependabot, code scanning).
        dismissed_at: Option<String>,
        dismissed_by: Option<Login>,
        dismissed_reason: Option<String>,
        dismissed_comment: Option<String>,
        auto_dismissed_at: Option<String>,
        // Resolved (secret scanning).
        resolved_at: Option<String>,
        resolved_by: Option<Login>,
        resolution: Option<String>,
        resolution_comment: Option<String>,
        // Dependabot.
        dependency: Option<Dependency>,
        security_advisory: Option<Advisory>,
        security_vulnerability: Option<Vulnerability>,
        // Code scanning.
        rule: Option<Rule>,
        tool: Option<Tool>,
        most_recent_instance: Option<Instance>,
        // Secret scanning.
        secret_type: Option<String>,
        secret_type_display_name: Option<String>,
        validity: Option<String>,
        #[serde(default)]
        publicly_leaked: Option<bool>,
        push_protection_bypassed_by: Option<Login>,
        first_location_detected: Option<FileAt>,
    }

    pub(super) fn alerts(kind: AlertKind, body: &str) -> Result<Vec<Alert>, ApiError> {
        let wire: Vec<Wire> = serde_json::from_str(body)?;
        Ok(wire.into_iter().map(|w| w.into_alert(kind)).collect())
    }

    pub(super) fn alert(kind: AlertKind, body: &str) -> Result<Alert, ApiError> {
        let wire: Wire = serde_json::from_str(body)?;
        Ok(wire.into_alert(kind))
    }

    #[derive(Deserialize)]
    struct WireLocation {
        #[serde(rename = "type")]
        kind: String,
        details: Option<FileAt>,
    }

    pub(super) fn locations(body: &str) -> Result<Vec<SecretLocation>, ApiError> {
        let wire: Vec<WireLocation> = serde_json::from_str(body)?;
        Ok(wire
            .into_iter()
            .map(|l| match (l.kind.as_str(), l.details) {
                ("commit" | "wiki_commit", Some(d)) => {
                    file(d).unwrap_or_else(|| SecretLocation::Other(l.kind.replace('_', " ")))
                }
                (kind, _) => SecretLocation::Other(kind.replace('_', " ")),
            })
            .collect())
    }

    fn file(d: FileAt) -> Option<SecretLocation> {
        let start = d.start_line?;
        Some(SecretLocation::File {
            path: d.path?,
            lines: (start, d.end_line.unwrap_or(start).max(start)),
            commit: d.commit_sha?,
        })
    }

    fn nonempty(s: Option<String>) -> Option<String> {
        s.filter(|s| !s.trim().is_empty())
    }

    impl Wire {
        fn into_alert(self, kind: AlertKind) -> Alert {
            let state = AlertState::parse(&self.state);
            let closing = match state {
                AlertState::Dismissed | AlertState::AutoDismissed => Some(Closing {
                    by: self.dismissed_by.map(|u| u.login),
                    at: self.dismissed_at.or(self.auto_dismissed_at),
                    reason: self.dismissed_reason,
                    comment: nonempty(self.dismissed_comment),
                }),
                AlertState::Resolved => Some(Closing {
                    by: self.resolved_by.map(|u| u.login),
                    at: self.resolved_at,
                    reason: self.resolution,
                    comment: nonempty(self.resolution_comment),
                }),
                AlertState::Open | AlertState::Fixed => None,
            };
            let (title, severity, level, detail) = match kind {
                AlertKind::Dependabot => {
                    let advisory = self.security_advisory;
                    let vulnerability = self.security_vulnerability;
                    let dependency = self.dependency;
                    let package = dependency.as_ref().and_then(|d| d.package.as_ref());
                    let severity = vulnerability
                        .as_ref()
                        .and_then(|v| v.severity)
                        .or(advisory.as_ref().and_then(|a| a.severity));
                    let title = advisory
                        .as_ref()
                        .map(|a| a.summary.clone())
                        .unwrap_or_default();
                    let detail = super::Dependency {
                        package: package.and_then(|p| p.name.clone()).unwrap_or_default(),
                        ecosystem: package
                            .and_then(|p| p.ecosystem.clone())
                            .unwrap_or_default(),
                        manifest: dependency
                            .as_ref()
                            .and_then(|d| d.manifest_path.clone())
                            .unwrap_or_default(),
                        scope: dependency.as_ref().and_then(|d| d.scope.clone()),
                        relationship: dependency
                            .and_then(|d| d.relationship)
                            .filter(|r| r != "unknown"),
                        vulnerable: vulnerability
                            .as_ref()
                            .and_then(|v| v.vulnerable_version_range.clone()),
                        patched: vulnerability
                            .and_then(|v| v.first_patched_version)
                            .map(|v| v.identifier),
                        ghsa: advisory
                            .as_ref()
                            .map(|a| a.ghsa_id.clone())
                            .unwrap_or_default(),
                        cve: advisory.as_ref().and_then(|a| a.cve_id.clone()),
                        description: advisory
                            .as_ref()
                            .and_then(|a| a.description.clone())
                            .unwrap_or_default(),
                        cwes: advisory
                            .as_ref()
                            .map(|a| {
                                a.cwes
                                    .iter()
                                    .map(|c| format!("{} {}", c.cwe_id, c.name))
                                    .collect()
                            })
                            .unwrap_or_default(),
                        references: advisory
                            .map(|a| a.references.into_iter().map(|r| r.url).collect())
                            .unwrap_or_default(),
                        update: None,
                    };
                    (title, severity, None, Detail::Dependency(Box::new(detail)))
                }
                AlertKind::CodeScanning => {
                    let rule = self.rule;
                    let instance = self.most_recent_instance;
                    let location = instance.as_ref().and_then(|i| i.location.as_ref());
                    let lines = location.and_then(|l| {
                        let start = l.start_line?;
                        Some((start, l.end_line.unwrap_or(start).max(start)))
                    });
                    let tool = self.tool.map(|t| {
                        let name = t.name.unwrap_or_default();
                        match t.version {
                            Some(v) if !v.is_empty() => format!("{name} {v}"),
                            _ => name,
                        }
                    });
                    let title = rule
                        .as_ref()
                        .and_then(|r| r.description.clone().or(r.name.clone()))
                        .unwrap_or_default();
                    let severity = rule.as_ref().and_then(|r| r.security_severity_level);
                    let level = rule.as_ref().and_then(|r| r.severity.clone());
                    let detail = CodeAlert {
                        rule: rule.as_ref().and_then(|r| r.id.clone()).unwrap_or_default(),
                        rule_description: rule
                            .as_ref()
                            .and_then(|r| nonempty(r.full_description.clone()))
                            .unwrap_or_default(),
                        help: rule
                            .as_ref()
                            .and_then(|r| r.help.clone())
                            .unwrap_or_default(),
                        tool: tool.unwrap_or_default(),
                        message: instance
                            .as_ref()
                            .and_then(|i| i.message.as_ref())
                            .and_then(|m| m.text.clone())
                            .unwrap_or_default(),
                        path: location.and_then(|l| l.path.clone()),
                        lines,
                        commit: instance.as_ref().and_then(|i| i.commit_sha.clone()),
                        git_ref: instance.and_then(|i| i.git_ref),
                        tags: rule.map(|r| r.tags).unwrap_or_default(),
                    };
                    (title, severity, level, Detail::Code(Box::new(detail)))
                }
                AlertKind::SecretScanning => {
                    let secret_type = self
                        .secret_type_display_name
                        .or(self.secret_type)
                        .unwrap_or_default();
                    let detail = SecretAlert {
                        secret_type: secret_type.clone(),
                        validity: self.validity,
                        publicly_leaked: self.publicly_leaked.unwrap_or(false),
                        bypassed_by: self.push_protection_bypassed_by.map(|u| u.login),
                        locations: self
                            .first_location_detected
                            .and_then(file)
                            .into_iter()
                            .collect(),
                    };
                    (secret_type, None, None, Detail::Secret(Box::new(detail)))
                }
            };
            Alert {
                number: self.number,
                state,
                title,
                severity,
                level,
                created_at: self.created_at,
                updated_at: self.updated_at,
                fixed_at: self.fixed_at,
                closing,
                detail,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_become_each_kinds_parameters() {
        let f = filter(AlertKind::Dependabot, "is:closed severity:high nonsense");
        assert_eq!(
            f.params,
            [
                ("state", "dismissed,fixed,auto_dismissed".to_owned()),
                ("severity", "high".to_owned())
            ]
        );
        assert_eq!(f.ignored, ["nonsense"]);
        let f = filter(AlertKind::CodeScanning, "is:open branch:main");
        assert_eq!(
            f.params,
            [
                ("state", "open".to_owned()),
                ("ref", "refs/heads/main".to_owned())
            ]
        );
        let f = filter(AlertKind::SecretScanning, "is:closed severity:high");
        assert_eq!(f.params, [("state", "resolved".to_owned())]);
        assert_eq!(f.ignored, ["severity:high"]);
    }

    /// A refusal says what's missing: the feature, a classic token's
    /// scope (with the command that adds it), a fine-grained token's
    /// permission, or the account's access.
    #[test]
    fn a_refusal_says_what_is_missing() {
        let off = refusal(
            AlertKind::Dependabot,
            403,
            "Dependabot alerts are disabled for this repository.",
            Some("repo"),
            None,
        );
        assert_eq!(
            off,
            Some(Refusal::Off(
                "Dependabot alerts are disabled for this repository".into()
            ))
        );
        let scope = refusal(
            AlertKind::CodeScanning,
            403,
            "Resource not accessible by integration",
            Some("gist, read:org"),
            None,
        );
        let Some(Refusal::Denied(why)) = scope else {
            panic!("{scope:?}")
        };
        assert!(
            why.contains("`gh auth refresh -s security_events`"),
            "{why}"
        );
        let fine = refusal(
            AlertKind::SecretScanning,
            403,
            "Resource not accessible by personal access token",
            None,
            Some("secret_scanning_alerts=read"),
        );
        let Some(Refusal::Denied(why)) = fine else {
            panic!("{fine:?}")
        };
        assert!(
            why.contains("Secret scanning alerts: read permission"),
            "{why}"
        );
        let access = refusal(
            AlertKind::Dependabot,
            403,
            "You are not authorized to perform this operation.",
            Some("repo, workflow"),
            None,
        );
        assert!(matches!(access, Some(Refusal::Denied(w)) if w.contains("write access")));
        assert_eq!(refusal(AlertKind::Dependabot, 500, "x", None, None), None);
    }
}
