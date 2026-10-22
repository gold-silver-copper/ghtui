//! Dismissing and reopening the Security tab's alerts. `,x` (close or
//! reopen) plans it with the other changes ([`crate::act`]): reopening
//! asks first; dismissing lists GitHub's reasons in a picker, whose field
//! is the comment to leave. The page then follows GitHub until it shows
//! the alert's new state ([`shows`]).

use crate::api::change::Change;
use crate::api::security::{Alert, AlertChange, AlertKind, AlertRef};

use crate::act::{Plan, Step, Subject};
use crate::data::DataKey;
use crate::state::State;

/// An alert whose dismissal is being chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dismissing(pub AlertRef);

impl Dismissing {
    pub fn title(&self) -> &'static str {
        match self.0.kind {
            AlertKind::SecretScanning => "Close the alert as",
            AlertKind::Dependabot | AlertKind::CodeScanning => "Dismiss the alert",
        }
    }

    pub fn placeholder(&self) -> &'static str {
        "A comment (optional), then choose why"
    }
}

/// What `,x` does on alert `a`, or why it can't.
pub(crate) fn plan(state: &State, a: &AlertRef) -> Result<Plan, String> {
    let alert = match state.answer::<Alert>(&DataKey::Alert(a.clone())) {
        None => return Ok(Plan::Load),
        Some(Err(err)) => return Err(format!("Couldn't load the alert: {err}")),
        Some(Ok(alert)) => alert,
    };
    if state.overview(&a.repo).is_some_and(|o| !o.can_write) {
        return Err("GitHub lets only people with write access close its alerts".into());
    }
    let name = a.kind.alert(a.number);
    match alert.state {
        s if s.is_open() => Ok(Plan::Dismiss(Dismissing(a.clone()))),
        crate::api::security::AlertState::Fixed => Err(
            "It's fixed: GitHub closed it when the code changed, and opens it again if the problem comes back"
                .into(),
        ),
        _ => {
            let mut facts = Vec::new();
            if let Some(reason) = alert.reason_label() {
                let by = (alert.closing.as_ref())
                    .and_then(|c| c.by.as_deref())
                    .map_or_else(String::new, |by| format!(" by @{by}"));
                facts.push((format!("Closed{by}: {reason}"), false));
            }
            let change = Change::Alert(AlertChange {
                alert: a.clone(),
                dismiss: None,
            });
            Ok(Plan::Ask(
                format!("Reopen {name}?"),
                facts,
                vec![("Reopen".into(), change)],
            ))
        }
    }
}

/// The picker's rows: GitHub's reasons, each dismissing the alert with
/// what's typed as its comment.
pub fn rows(d: &Dismissing, q: &str) -> crate::picker::Rows {
    d.0.kind
        .reasons()
        .iter()
        .map(|(word, label, hint)| {
            let item = crate::picker::item(*label, *hint);
            let change = Change::Alert(AlertChange {
                alert: d.0.clone(),
                dismiss: Some(((*word).to_owned(), q.to_owned())),
            });
            let send = Step::Send(change, Subject::Alert(d.0.clone()));
            (item, Some(crate::picker::Choice::Step(send)))
        })
        .collect()
}

/// Whether the alert, as loaded, shows `c`.
pub fn shows(state: &State, c: &AlertChange) -> bool {
    let key = DataKey::Alert(c.alert.clone());
    state
        .picked::<Alert>(&key)
        .is_some_and(|a| a.state.is_open() == c.dismiss.is_none())
}

/// What it's doing while GitHub answers, and what it is, for "GitHub
/// refused …".
pub fn said(c: &AlertChange) -> (&'static str, &'static str) {
    match (&c.dismiss, c.alert.kind) {
        (None, _) => ("Reopening the alert", "reopening the alert"),
        (Some(_), AlertKind::SecretScanning) => ("Closing the alert", "closing the alert"),
        (Some(_), _) => ("Dismissing the alert", "closing the alert"),
    }
}

/// What it did.
pub fn done(c: &AlertChange) -> String {
    let reason = c.dismiss.as_ref().map(|(word, _)| {
        (c.alert.kind.reasons().iter())
            .find(|(w, ..)| w == word)
            .map_or(word.as_str(), |(_, label, _)| label)
    });
    match (reason, c.alert.kind) {
        (None, _) => "Alert reopened".into(),
        (Some(why), AlertKind::SecretScanning) => format!("Alert closed: {why}"),
        (Some(why), _) => format!("Alert dismissed: {why}"),
    }
}

/// The permission a fine-grained token needs for it.
pub fn needs(c: &AlertChange) -> &'static str {
    match c.alert.kind {
        AlertKind::Dependabot => "Dependabot alerts: write",
        AlertKind::CodeScanning => "Code scanning alerts: write",
        AlertKind::SecretScanning => "Secret scanning alerts: write",
    }
}

/// Alerts as GitHub might send them, for tests.
#[cfg(test)]
pub(crate) mod fixtures {
    use crate::api::browse::{Results, Severity};
    use crate::api::security::{
        Alert, AlertKind, AlertList, AlertState, Closing, CodeAlert, Dependency, Detail,
        SecretAlert, SecretLocation, Update,
    };

    /// An open alert of `kind`, numbered `number`.
    pub fn alert(kind: AlertKind, number: u64) -> Alert {
        let detail = match kind {
            AlertKind::Dependabot => Detail::Dependency(Box::new(Dependency {
                package: "rand".into(),
                ecosystem: "rust".into(),
                manifest: "Cargo.lock".into(),
                scope: Some("runtime".into()),
                relationship: Some("transitive".into()),
                vulnerable: Some(">= 0.7.0, < 0.8.6".into()),
                patched: Some("0.8.6".into()),
                ghsa: "GHSA-cq8v-f236-94qc".into(),
                cve: None,
                description: "Rand is unsound with a custom logger.".into(),
                cwes: vec!["CWE-20 Improper Input Validation".into()],
                references: vec!["https://rustsec.org/advisories/RUSTSEC-2026-0097.html".into()],
                update: Some(Update::Pr {
                    number: 12,
                    title: "Bump rand from 0.8.5 to 0.8.6".into(),
                    state: "OPEN".into(),
                }),
            })),
            AlertKind::CodeScanning => Detail::Code(Box::new(CodeAlert {
                rule: "rust/hard-coded-credentials".into(),
                rule_description: "Credentials written into the source can leak.".into(),
                help: "Read them from **configuration** instead.".into(),
                tool: "CodeQL 2.19.0".into(),
                message: "This hard-codes a password.".into(),
                path: Some("src/main.rs".into()),
                lines: Some((10, 12)),
                commit: Some("75c61df02bc009d61a80ad467606772ba0a92c22".into()),
                git_ref: Some("refs/heads/main".into()),
                tags: vec!["security".into()],
            })),
            AlertKind::SecretScanning => Detail::Secret(Box::new(SecretAlert {
                secret_type: "GitHub Personal Access Token".into(),
                validity: Some("active".into()),
                publicly_leaked: false,
                bypassed_by: None,
                locations: vec![
                    SecretLocation::File {
                        path: "config/dev.env".into(),
                        lines: (3, 3),
                        commit: "75c61df02bc009d61a80ad467606772ba0a92c22".into(),
                    },
                    SecretLocation::Other("issue comment".into()),
                ],
            })),
        };
        Alert {
            number,
            state: AlertState::Open,
            title: match kind {
                AlertKind::Dependabot => "Rand is unsound with a custom logger".into(),
                AlertKind::CodeScanning => "Hard-coded credentials".into(),
                AlertKind::SecretScanning => "GitHub Personal Access Token".into(),
            },
            severity: (kind != AlertKind::SecretScanning).then_some(Severity::High),
            level: (kind == AlertKind::CodeScanning).then(|| "error".into()),
            created_at: "2026-10-01T10:00:00Z".into(),
            updated_at: None,
            fixed_at: None,
            closing: None,
            detail,
        }
    }

    /// The same, dismissed.
    pub fn dismissed(kind: AlertKind, number: u64) -> Alert {
        let reason = kind.reasons().first().map(|(word, ..)| (*word).to_owned());
        Alert {
            state: match kind {
                AlertKind::SecretScanning => AlertState::Resolved,
                _ => AlertState::Dismissed,
            },
            closing: Some(Closing {
                by: Some("octocat".into()),
                at: Some("2026-10-02T10:00:00Z".into()),
                reason,
                comment: Some("Tracked in #40.".into()),
            }),
            ..alert(kind, number)
        }
    }

    /// A list with an open alert and a dismissed one.
    pub fn alerts(kind: AlertKind) -> AlertList {
        AlertList {
            results: Results {
                total: 2,
                items: vec![alert(kind, 2), dismissed(kind, 1)],
                next: None,
            },
            off: None,
        }
    }
}
