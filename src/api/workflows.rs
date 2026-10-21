//! What GitHub Actions says beyond runs and jobs: why a check failed (its
//! annotations), what a run made (its artifacts), and running a workflow
//! by hand (its `workflow_dispatch` inputs).

use serde::{Deserialize, Serialize};

use crate::api::client::{ApiError, GitHub};
use crate::api::model::{Capped, RepoId};
use crate::api::queries::fragments;
use crate::schema::schema;

/// How serious an annotation is.
#[derive(cynic::Enum, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cynic(graphql_type = "CheckAnnotationLevel")]
pub enum Level {
    Failure,
    Warning,
    Notice,
}

impl Level {
    pub fn word(self) -> &'static str {
        match self {
            Level::Failure => "Error",
            Level::Warning => "Warning",
            Level::Notice => "Notice",
        }
    }
}

/// What a check run says about a file's lines: a failure, a warning or a
/// notice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Annotation {
    /// The file, or `None` for one about the run (GitHub names the
    /// workflow's folder, `.github`, for those).
    pub path: Option<String>,
    pub start_line: u32,
    pub end_line: u32,
    pub level: Level,
    /// Empty when it has none.
    pub title: String,
    pub message: String,
    /// The file at the commit checked, at its lines; `None` for one about
    /// the run.
    pub url: Option<String>,
}

/// An annotation as GitHub sends it.
struct Raw {
    path: String,
    start: i64,
    end: i64,
    level: Level,
    title: String,
    message: String,
    /// The file at the commit checked.
    blob: String,
}

impl Annotation {
    fn new(raw: Raw) -> Self {
        let line = |n: i64| u32::try_from(n).unwrap_or(0);
        let path = Some(raw.path).filter(|p| !p.is_empty() && p != ".github");
        let (start_line, end_line) = (line(raw.start), line(raw.end.max(raw.start)));
        let lines = match (start_line, end_line) {
            (0, _) => String::new(),
            (a, b) if a == b => format!("#L{a}"),
            (a, b) => format!("#L{a}-L{b}"),
        };
        Self {
            url: path.as_ref().map(|_| format!("{}{lines}", raw.blob)),
            path,
            start_line,
            end_line,
            level: raw.level,
            title: raw.title,
            message: raw.message,
        }
    }

    /// Where it is, as people say it: `src/a.rs:12`, or "the run".
    pub fn place(&self) -> String {
        match &self.path {
            Some(path) if self.start_line > 0 => format!("{path}:{}", self.start_line),
            Some(path) => path.clone(),
            None => "the run".to_owned(),
        }
    }
}

/// Annotations, worst first, then by file and line.
pub fn sorted(mut list: Vec<Annotation>) -> Vec<Annotation> {
    list.sort_by(|a, b| (a.level, &a.path, a.start_line).cmp(&(b.level, &b.path, b.start_line)));
    list
}

/// What a run made to keep: a file to download, for a while.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artifact {
    pub id: u64,
    pub name: String,
    /// Bytes, as stored (zipped).
    pub size: u64,
    /// ISO 8601.
    pub expires_at: Option<String>,
    /// Gone: it can't be downloaded.
    pub expired: bool,
}

/// A `workflow_dispatch` input, as its workflow file declares it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    pub name: String,
    pub description: String,
    pub kind: InputKind,
    /// As the file has it (`true`, a choice, text).
    pub default: Option<String>,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputKind {
    /// One of these.
    Choice(Vec<String>),
    Boolean,
    /// Text, a number, or an environment's name: typed.
    Text,
}

impl Input {
    /// What's offered for it before anything is chosen: its default, or a
    /// choice's first option, or a boolean's `false`.
    pub fn start(&self) -> String {
        match (&self.default, &self.kind) {
            (Some(d), _) => d.clone(),
            (None, InputKind::Choice(options)) => options.first().cloned().unwrap_or_default(),
            (None, InputKind::Boolean) => "false".into(),
            (None, InputKind::Text) => String::new(),
        }
    }
}

/// A workflow file's `workflow_dispatch` inputs; `None` if it can't be
/// run by hand.
pub fn dispatch_inputs(file: &str) -> Option<Vec<Input>> {
    use crate::yaml::{self, Value};
    let doc = yaml::parse(file);
    // `on` is a name, a list of names, or a map of them.
    let on = doc.get("on").or_else(|| doc.get("true"))?;
    let dispatch = match on {
        Value::Text(name) => return (name == "workflow_dispatch").then(Vec::new),
        Value::List(names) => {
            let named = names.iter().any(|n| n.text() == Some("workflow_dispatch"));
            return named.then(Vec::new);
        }
        Value::Map(_) => on.get("workflow_dispatch")?,
        Value::Null => return None,
    };
    let Some(Value::Map(inputs)) = dispatch.get("inputs") else {
        return Some(Vec::new());
    };
    let text = |v: &Value, key: &str| v.get(key).and_then(Value::text).map(str::to_owned);
    let inputs = inputs.iter().map(|(name, spec)| {
        let kind = match text(spec, "type").as_deref() {
            Some("choice") => {
                let options = spec.get("options").map(Value::names).unwrap_or_default();
                InputKind::Choice(options)
            }
            Some("boolean") => InputKind::Boolean,
            _ => InputKind::Text,
        };
        Input {
            name: name.clone(),
            description: text(spec, "description").unwrap_or_default(),
            default: text(spec, "default"),
            required: spec.get("required").and_then(Value::text) == Some("true"),
            kind,
        }
    });
    Some(inputs.collect())
}

// Annotations as GraphQL has them, on a check run.
fragments! { counted: WireAnnotations = "CheckAnnotationConnection" => WireAnnotation }

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "CheckAnnotation")]
pub struct WireAnnotation {
    pub path: String,
    pub blob_url: crate::api::queries::Uri,
    pub location: WireSpan,
    pub annotation_level: Option<Level>,
    pub title: Option<String>,
    pub message: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "CheckAnnotationSpan")]
pub struct WireSpan {
    pub start: WirePosition,
    pub end: WirePosition,
}

fragments! { line: i32; WirePosition = "CheckAnnotationPosition" }

impl From<WireAnnotation> for Annotation {
    fn from(a: WireAnnotation) -> Self {
        Annotation::new(Raw {
            path: a.path,
            start: a.location.start.line.into(),
            end: a.location.end.line.into(),
            level: a.annotation_level.unwrap_or(Level::Notice),
            title: a.title.unwrap_or_default(),
            message: a.message,
            blob: a.blob_url.0,
        })
    }
}

/// REST's shapes.
mod rest {
    use serde::Deserialize;

    #[derive(Deserialize)]
    pub struct Annotation {
        pub path: String,
        pub blob_href: String,
        pub start_line: i64,
        pub end_line: i64,
        pub annotation_level: String,
        pub title: Option<String>,
        pub message: String,
    }

    #[derive(Deserialize)]
    pub struct CheckRun {
        pub output: Output,
    }

    #[derive(Deserialize)]
    pub struct Output {
        pub annotations_count: u64,
    }

    #[derive(Deserialize)]
    pub struct Artifacts {
        pub total_count: u64,
        pub artifacts: Vec<Artifact>,
    }

    #[derive(Deserialize)]
    pub struct Artifact {
        pub id: u64,
        pub name: String,
        pub size_in_bytes: u64,
        pub expires_at: Option<String>,
        pub expired: bool,
    }

    #[derive(Deserialize)]
    pub struct Dispatched {
        pub workflow_run_id: Option<u64>,
    }
}

/// How many of a job's annotations are fetched.
const ANNOTATIONS: u64 = 50;
/// How many of a run's artifacts are fetched.
const ARTIFACTS: u64 = 100;

impl GitHub {
    /// A job's annotations (an Actions job is a check run), worst first.
    pub async fn job_annotations(
        &self,
        repo: &RepoId,
        job: u64,
    ) -> Result<Capped<Annotation>, ApiError> {
        let base = format!("/repos/{repo}/check-runs/{job}");
        let what = format!("{repo}'s check run {job}");
        let run: rest::CheckRun = serde_json::from_str(&self.rest_get(&base, &what).await?)?;
        let total = run.output.annotations_count;
        if total == 0 {
            return Ok(Capped::default());
        }
        let path = format!("{base}/annotations?per_page={ANNOTATIONS}");
        let wire: Vec<rest::Annotation> =
            serde_json::from_str(&self.rest_get(&path, &what).await?)?;
        let list = wire.into_iter().map(|a| {
            let level = match a.annotation_level.as_str() {
                "failure" => Level::Failure,
                "warning" => Level::Warning,
                _ => Level::Notice,
            };
            Annotation::new(Raw {
                path: a.path,
                start: a.start_line,
                end: a.end_line,
                level,
                title: a.title.unwrap_or_default(),
                message: a.message,
                blob: a.blob_href,
            })
        });
        Ok(Capped::new(sorted(list.collect()), total))
    }

    /// A run's artifacts, newest first.
    pub async fn artifacts(&self, repo: &RepoId, run: u64) -> Result<Capped<Artifact>, ApiError> {
        let path = format!("/repos/{repo}/actions/runs/{run}/artifacts?per_page={ARTIFACTS}");
        let wire: rest::Artifacts = serde_json::from_str(
            &self
                .rest_get(&path, format!("run {run}'s artifacts"))
                .await?,
        )?;
        let items = wire.artifacts.into_iter().map(|a| Artifact {
            id: a.id,
            name: a.name,
            size: a.size_in_bytes,
            expires_at: a.expires_at,
            expired: a.expired,
        });
        Ok(Capped::new(items.collect(), wire.total_count))
    }

    /// Saves artifact `id` (a zip) to `dest`, replacing what's there only
    /// if `replace`; its size.
    pub async fn download_artifact(
        &self,
        repo: &RepoId,
        id: u64,
        dest: &std::path::Path,
        replace: bool,
    ) -> Result<u64, ApiError> {
        let path = format!("/repos/{repo}/actions/artifacts/{id}/zip");
        self.download(&path, dest, replace).await
    }

    /// Runs workflow `file` by hand on `git_ref` with `inputs`: the new
    /// run's ID, where GitHub says it (github.com does; an older
    /// Enterprise Server doesn't).
    pub async fn dispatch(
        &self,
        repo: &RepoId,
        file: &str,
        git_ref: &str,
        inputs: &[(String, String)],
    ) -> Result<Option<u64>, ApiError> {
        let file = crate::ui::pages::url::encode_path(file);
        let path = format!("/repos/{repo}/actions/workflows/{file}/dispatches");
        let inputs: serde_json::Map<String, serde_json::Value> = (inputs.iter())
            .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
            .collect();
        let body = serde_json::json!({
            "ref": git_ref,
            "inputs": inputs,
            "return_run_details": true,
        });
        let answer = self
            .rest_write_answer(http::Method::POST, &path, Some(&body))
            .await?;
        if answer.trim().is_empty() {
            return Ok(None);
        }
        let dispatched: rest::Dispatched = serde_json::from_str(&answer)?;
        Ok(dispatched.workflow_run_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Inputs are read from the workflow file as GitHub reads them:
    /// choices with their options, booleans, text, and defaults.
    #[test]
    fn dispatch_inputs_come_from_the_workflow_file() {
        let file = "name: actions\non:\n  workflow_dispatch:\n    inputs:\n      outcome:\n        description: How the job ends\n        type: choice\n        options: [fail, pass]\n        default: fail\n      seconds:\n        type: string\n        default: \"60\"\n        required: true\n      summary:\n        type: boolean\n  push:\njobs: {}\n";
        let inputs = dispatch_inputs(file).unwrap();
        let names: Vec<&str> = inputs.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["outcome", "seconds", "summary"]);
        assert_eq!(
            inputs[0].kind,
            InputKind::Choice(vec!["fail".into(), "pass".into()])
        );
        assert_eq!(inputs[0].description, "How the job ends");
        assert_eq!(inputs[1].start(), "60");
        assert!(inputs[1].required);
        assert_eq!(
            (&inputs[2].kind, inputs[2].start().as_str()),
            (&InputKind::Boolean, "false")
        );
        assert_eq!(
            dispatch_inputs("on: [push, workflow_dispatch]\n"),
            Some(Vec::new())
        );
        assert_eq!(dispatch_inputs("on: push\n"), None);
    }
}
