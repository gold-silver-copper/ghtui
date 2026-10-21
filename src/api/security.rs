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

words! {
    /// A kind of alert, as the Security tab lists them.
    #[derive(Hash, Serialize, Deserialize)]
    pub enum AlertKind {
        /// Its part of GitHub's paths, on the website and in the API.
        pub fn slug -> pub fn from_slug,
        /// `Dependabot`, as the tab's menu names it.
        pub fn name,
        /// `Dependabot alerts`; also the fine-grained token permission it
        /// needs, as GitHub names it.
        pub fn alerts,
        /// The query parameter its website's list keeps its filter in.
        pub fn query_param,
        /// Mid-sentence.
        fn said,
    }
    Dependabot     "dependabot"      "Dependabot"      "Dependabot alerts"      "q"     "Dependabot";
    CodeScanning   "code-scanning"   "Code scanning"   "Code scanning alerts"   "query" "code scanning";
    SecretScanning "secret-scanning" "Secret scanning" "Secret scanning alerts" "query" "secret scanning";
}

impl AlertKind {
    /// An alert of this kind, mid-sentence: `code scanning alert #3`.
    pub fn alert(self, number: u64) -> String {
        format!("{} alert #{number}", self.said())
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
    /// kind: GitHub's word for each, a label, and what it means. One row
    /// each, so rustfmt is kept off.
    #[rustfmt::skip]
    pub fn reasons(self) -> &'static [(&'static str, &'static str, &'static str)] {
        match self {
            AlertKind::Dependabot => &[
                ("fix_started", "A fix has already been started", "fix started"),
                ("no_bandwidth", "No bandwidth to fix this", "no bandwidth"),
                ("tolerable_risk", "Risk is tolerable to this project", "tolerable risk"),
                ("inaccurate", "This alert is inaccurate or incorrect", "inaccurate"),
                ("not_used", "Vulnerable code is not actually used", "not used"),
            ],
            AlertKind::CodeScanning => &[
                ("false positive", "False positive", "the alert isn't valid"),
                ("won't fix", "Won't fix", "the alert isn't relevant"),
                ("used in tests", "Used in tests", "the code is only in tests"),
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

words! {
    /// Where an alert stands.
    #[derive(Serialize, Deserialize)]
    pub enum AlertState { fn wire -> fn from_wire, pub fn label }
    Open          "open"           "Open";
    Dismissed     "dismissed"      "Dismissed";
    /// Dependabot dismissed it itself (a development dependency, say).
    AutoDismissed "auto_dismissed" "Auto-dismissed";
    Fixed         "fixed"          "Fixed";
    /// A secret's: someone said what became of it.
    Resolved      "resolved"       "Closed";
}

impl AlertState {
    pub fn is_open(self) -> bool {
        self == AlertState::Open
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
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
        let to = |name, value: &str| Some((name, value.to_owned()));
        let param = match (kind, key, value) {
            (K::Dependabot, "is" | "state", "closed") => {
                to("state", "dismissed,fixed,auto_dismissed")
            }
            (K::CodeScanning, "is" | "state", "closed") => to("state", "closed"),
            (K::SecretScanning, "is" | "state", "closed") => to("state", "resolved"),
            (_, "is" | "state", "open")
            | (K::Dependabot, "is" | "state", "dismissed" | "fixed" | "auto_dismissed")
            | (K::CodeScanning, "is" | "state", "dismissed" | "fixed") => to("state", value),
            (K::Dependabot | K::CodeScanning, "severity", _) => to("severity", value),
            (K::Dependabot, "ecosystem", _) => to("ecosystem", value),
            (K::Dependabot, "package", _) => to("package", value),
            (K::Dependabot, "manifest", _) => to("manifest", value),
            (K::Dependabot, "scope", "runtime" | "development") => to("scope", value),
            (K::CodeScanning, "tool", _) => to("tool_name", value),
            (K::CodeScanning, "branch", _) => to("ref", &format!("refs/heads/{value}")),
            (K::CodeScanning, "ref", _) => to("ref", value),
            (K::SecretScanning, "secret-type", _) => to("secret_type", value),
            (K::SecretScanning, "resolution", _) => to("resolution", value),
            (K::SecretScanning, "validity", _) => to("validity", value),
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
        // The permission is named as the alerts are.
        return Some(Refusal::Denied(format!(
            "{0} take the {0}: {named} permission, which your token lacks: give it that permission on GitHub, under Settings › Developer settings",
            kind.alerts()
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
            path.extend(["&", name, "="]);
            path.extend(url::form_urlencoded::byte_serialize(value.as_bytes()));
        }
        if let Some(after) = &after {
            path.extend(["&", after]);
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
        // What only the page shows needs only `a`, so it's asked alongside.
        let update = async {
            match a.kind {
                AlertKind::Dependabot => self.dependabot_update(a).await,
                _ => None,
            }
        };
        let locations = async {
            match a.kind {
                AlertKind::SecretScanning => self.secret_locations(&path).await,
                _ => None,
            }
        };
        let (answer, update, locations) =
            tokio::join!(self.alerts_answer(a.kind, &path, &what), update, locations);
        let mut alert = match answer? {
            Ok(answer) => wire::alert(a.kind, answer.body())?,
            Err(off) => return Err(ApiError::Denied(off)),
        };
        match &mut alert.detail {
            Detail::Dependency(d) => d.update = update,
            Detail::Secret(s) => {
                if let Some(found) = locations {
                    s.locations = found;
                }
            }
            Detail::Code(_) => {}
        }
        Ok(alert)
    }

    /// All of a secret's locations, from its alert's `path`; nothing if
    /// GitHub won't list them.
    async fn secret_locations(&self, path: &str) -> Option<Vec<SecretLocation>> {
        match self
            .rest_answer(&format!("{path}/locations?per_page=100"))
            .await
        {
            Ok(answer) if (200..300).contains(&answer.status()) => {
                wire::locations(answer.body()).ok()
            }
            Ok(answer) => {
                let status = answer.status();
                tracing::debug!(status, "couldn't list a secret's locations");
                None
            }
            Err(err) => {
                tracing::debug!(%err, "couldn't list a secret's locations");
                None
            }
        }
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

    #[derive(Deserialize, Default)]
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

    #[derive(Deserialize, Default)]
    struct Vulnerability {
        severity: Option<Severity>,
        vulnerable_version_range: Option<String>,
        first_patched_version: Option<Version>,
    }

    #[derive(Deserialize)]
    struct Reference {
        url: String,
    }

    #[derive(Deserialize, Default)]
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

    #[derive(Deserialize, Default)]
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

    #[derive(Deserialize, Default)]
    struct Location {
        path: Option<String>,
        start_line: Option<u32>,
        end_line: Option<u32>,
    }

    #[derive(Deserialize, Default)]
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
        Some(SecretLocation::File {
            path: d.path?,
            lines: span(d.start_line?, d.end_line),
            commit: d.commit_sha?,
        })
    }

    /// Lines `start` to `end`, one line when GitHub gives no end (or one
    /// before the start).
    fn span(start: u32, end: Option<u32>) -> (u32, u32) {
        (start, end.unwrap_or(start).max(start))
    }

    fn nonempty(s: Option<String>) -> Option<String> {
        s.filter(|s| !s.trim().is_empty())
    }

    impl Wire {
        fn into_alert(self, kind: AlertKind) -> Alert {
            let state = AlertState::from_wire(&self.state).unwrap_or(AlertState::Open);
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
                    let a = self.security_advisory.unwrap_or_default();
                    let v = self.security_vulnerability.unwrap_or_default();
                    let d = self.dependency.unwrap_or_default();
                    let (name, ecosystem) =
                        (d.package).map_or((None, None), |p| (p.name, p.ecosystem));
                    let detail = super::Dependency {
                        package: name.unwrap_or_default(),
                        ecosystem: ecosystem.unwrap_or_default(),
                        manifest: d.manifest_path.unwrap_or_default(),
                        scope: d.scope,
                        relationship: d.relationship.filter(|r| r != "unknown"),
                        vulnerable: v.vulnerable_version_range,
                        patched: v.first_patched_version.map(|v| v.identifier),
                        ghsa: a.ghsa_id,
                        cve: a.cve_id,
                        description: a.description.unwrap_or_default(),
                        cwes: (a.cwes.into_iter())
                            .map(|c| format!("{} {}", c.cwe_id, c.name))
                            .collect(),
                        references: a.references.into_iter().map(|r| r.url).collect(),
                        update: None,
                    };
                    let severity = v.severity.or(a.severity);
                    (
                        a.summary,
                        severity,
                        None,
                        Detail::Dependency(Box::new(detail)),
                    )
                }
                AlertKind::CodeScanning => {
                    let rule = self.rule.unwrap_or_default();
                    let at = self.most_recent_instance.unwrap_or_default();
                    let place = at.location.unwrap_or_default();
                    let tool = self.tool.map(|t| {
                        let name = t.name.unwrap_or_default();
                        match t.version {
                            Some(v) if !v.is_empty() => format!("{name} {v}"),
                            _ => name,
                        }
                    });
                    let detail = CodeAlert {
                        rule: rule.id.unwrap_or_default(),
                        rule_description: nonempty(rule.full_description).unwrap_or_default(),
                        help: rule.help.unwrap_or_default(),
                        tool: tool.unwrap_or_default(),
                        message: at.message.and_then(|m| m.text).unwrap_or_default(),
                        path: place.path,
                        lines: (place.start_line).map(|start| span(start, place.end_line)),
                        commit: at.commit_sha,
                        git_ref: at.git_ref,
                        tags: rule.tags,
                    };
                    let title = rule.description.or(rule.name).unwrap_or_default();
                    let detail = Detail::Code(Box::new(detail));
                    (title, rule.security_severity_level, rule.severity, detail)
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
        let f = filter(AlertKind::SecretScanning, "is:closed severity:high");
        assert_eq!(f.params, [("state", "resolved".to_owned())]);
        assert_eq!(f.ignored, ["severity:high"]);
    }

    /// A refusal says what's missing: a classic token's scope (with the
    /// command that adds it), a fine-grained token's permission, or the
    /// account's access.
    #[test]
    fn a_refusal_says_what_is_missing() {
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
