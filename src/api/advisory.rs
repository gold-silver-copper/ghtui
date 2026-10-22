//! A repository's security advisory, drafted with the fields of GitHub's
//! form: the website's "New draft security advisory" on the Security
//! tab, which GitHub's API makes as [`GitHub::draft_advisory`].

use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::client::{ApiError, GitHub};
use crate::api::model::RepoId;

/// How severe, as GitHub's form asks: a level, or a CVSS vector it
/// scores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Severity {
    /// `critical`, `high`, `medium` or `low`.
    Level(String),
    /// `CVSS:3.1/AV:N/…` or `CVSS:4.0/…`.
    Cvss(String),
}

/// A new draft advisory: what's vulnerable (one package), how severe,
/// and what it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAdvisory {
    pub summary: String,
    /// Markdown.
    pub description: String,
    /// GitHub's word for it: `npm`, `pip`, `rust`…
    pub ecosystem: String,
    pub package: String,
    /// `< 1.2.3`; empty: GitHub's default.
    pub affected: String,
    /// `1.2.3`; empty: none yet.
    pub patched: String,
    pub severity: Severity,
    pub cve: Option<String>,
    /// `CWE-79`…
    pub cwes: Vec<String>,
}

impl NewAdvisory {
    /// What GitHub's API takes.
    pub fn body(&self) -> Value {
        let some = |s: &str| (!s.trim().is_empty()).then(|| s.trim().to_owned());
        let mut body = json!({
            "summary": self.summary,
            "description": self.description,
            "vulnerabilities": [{
                "package": { "ecosystem": self.ecosystem, "name": self.package },
                "vulnerable_version_range": some(&self.affected),
                "patched_versions": some(&self.patched),
            }],
            "cve_id": self.cve,
            "cwe_ids": self.cwes,
        });
        if let Some(body) = body.as_object_mut() {
            let (name, value) = match &self.severity {
                Severity::Level(level) => ("severity", level),
                Severity::Cvss(vector) => ("cvss_vector_string", vector),
            };
            body.insert(name.into(), value.as_str().into());
        }
        body
    }
}

impl GitHub {
    /// Drafts `advisory` in `repo`: its GHSA ID.
    pub async fn draft_advisory(
        &self,
        repo: &RepoId,
        advisory: &NewAdvisory,
    ) -> Result<String, ApiError> {
        #[derive(Deserialize)]
        struct Made {
            ghsa_id: String,
        }
        let path = format!("/repos/{repo}/security-advisories");
        let body = advisory.body();
        let made: Made = (self.rest_write_json(http::Method::POST, &path, Some(&body))).await?;
        Ok(made.ghsa_id)
    }
}
