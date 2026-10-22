//! A repository's security advisory, drafted from its Security tab with
//! the fields of GitHub's form: a title, the affected package (its
//! ecosystem, name, affected and patched versions), the severity (a level
//! or a CVSS vector), the weaknesses (CWEs), a CVE ID if it has one, and
//! the description. GitHub doesn't ask before making a draft, nor does
//! this; the advisory's page opens once GitHub has it.

use crate::api::advisory::{NewAdvisory, Severity};
use crate::api::model::RepoId;
use crate::route::{Route, security};
use crate::ui::page::{Link, Page};
use crate::ui::pages::{Mark, TextBox, WriteFormView};

use super::{Field, Form, FormOf, form, form_mut, form_row, item, setting};
use crate::forms::say;
use crate::state::{Cmds, State};
use crate::work::{self, Cx, Failure, Job};

/// The ecosystems GitHub's advisories know: its word, and its name.
const ECOSYSTEMS: &[(&str, &str)] = &[
    ("actions", "GitHub Actions"),
    ("composer", "Composer"),
    ("erlang", "Erlang"),
    ("go", "Go"),
    ("maven", "Maven"),
    ("npm", "npm"),
    ("nuget", "NuGet"),
    ("other", "Other"),
    ("pip", "pip"),
    ("pub", "Pub"),
    ("rubygems", "RubyGems"),
    ("rust", "Rust"),
    ("swift", "Swift"),
];

/// GitHub's severities, as its form lists them.
const LEVELS: &[(&str, &str)] = &[
    ("critical", "Critical"),
    ("high", "High"),
    ("medium", "Medium"),
    ("low", "Low"),
];

/// Why you can't draft one in `repo`, from what's known.
pub(super) fn refused(state: &State, repo: &RepoId) -> Option<String> {
    let o = state.overview(repo)?;
    (!o.can_admin).then(|| {
        format!(
            "Drafting a security advisory takes admin access to {repo}, which GitHub doesn't say you have"
        )
    })
}

/// `:new here` on the Security tab: the form.
pub(super) fn open(state: &mut State, repo: &RepoId) -> Cmds {
    if let Some(why) = refused(state, repo) {
        return state.just_say(why);
    }
    super::open(state, FormOf::Advisory(repo.clone()))
}

/// A new form starts with its title.
pub(super) fn follow(state: &mut State, route: &Route) -> Cmds {
    if form(state, route).started {
        return Cmds::new();
    }
    form_mut(state, route).started = true;
    if state.overlay.is_none() {
        return super::open_field(state, route, Field::Title);
    }
    Cmds::new()
}

/// The ecosystem picker's rows.
pub(super) fn ecosystem_rows(route: &Route, q: &str) -> crate::picker::Rows {
    (ECOSYSTEMS.iter())
        .filter(|(word, name)| {
            crate::picker::fuzzy_score(q, name).is_some() || word.starts_with(&q.to_lowercase())
        })
        .map(|(word, name)| item(*name, *word, setting(route, Field::Ecosystem, word)))
        .collect()
}

/// The severity picker's rows: a CVSS vector typed, or GitHub's levels.
pub(super) fn severity_rows(route: &Route, q: &str) -> crate::picker::Rows {
    let mut rows = Vec::new();
    if q.to_ascii_uppercase().starts_with("CVSS:") {
        rows.push(match cvss(q) {
            Some(v) => item(
                format!("Scored from {v}"),
                "↵",
                setting(route, Field::Severity, &v),
            ),
            None => item(
                "A CVSS vector is CVSS:3.1/AV:…/AC:… (or CVSS:4.0/…)",
                "",
                None,
            ),
        });
        return rows;
    }
    for (word, name) in LEVELS {
        if crate::picker::fuzzy_score(q, name).is_some() {
            rows.push(item(*name, "", setting(route, Field::Severity, word)));
        }
    }
    rows.push(item("Or type a CVSS vector: CVSS:3.1/AV:N/…", "", None));
    rows
}

/// A CVSS vector as GitHub takes it, if `text` is one.
fn cvss(text: &str) -> Option<String> {
    let text = text.trim();
    let (head, rest) = text.split_once('/')?;
    let head = head.to_ascii_uppercase();
    let known = matches!(head.as_str(), "CVSS:3.0" | "CVSS:3.1" | "CVSS:4.0");
    let metrics = rest.split('/').all(|m| {
        m.split_once(':').is_some_and(|(k, v)| {
            !k.is_empty() && !v.is_empty() && k.chars().all(|c| c.is_ascii_alphabetic())
        })
    });
    (known && metrics).then(|| format!("{head}/{rest}"))
}

/// Why `value` can't be the form's `field`, if it can't.
pub(super) fn check(field: Field, value: &str) -> Option<String> {
    let value = value.trim();
    match field {
        Field::Cve if !value.is_empty() && !valid_cve(value) => {
            Some("A CVE ID is CVE-YYYY-NNNN".into())
        }
        Field::Cwes if cwes(value).is_none() => {
            Some("Weaknesses are CWE IDs, comma-separated: CWE-79, CWE-89".into())
        }
        Field::Title if value.chars().count() > 1024 => {
            Some("A title is at most 1024 characters".into())
        }
        _ => None,
    }
}

fn valid_cve(s: &str) -> bool {
    let mut parts = s.splitn(3, '-');
    let (cve, year, n) = (parts.next(), parts.next(), parts.next());
    cve.is_some_and(|c| c.eq_ignore_ascii_case("CVE"))
        && year.is_some_and(|y| y.len() == 4 && y.chars().all(|c| c.is_ascii_digit()))
        && n.is_some_and(|n| n.len() >= 4 && n.chars().all(|c| c.is_ascii_digit()))
}

/// CWE IDs, comma-separated (`79` reads as `CWE-79`), if they're that.
fn cwes(text: &str) -> Option<Vec<String>> {
    (text.split(',').map(str::trim).filter(|s| !s.is_empty()))
        .map(|s| {
            let n = s
                .strip_prefix("CWE-")
                .or_else(|| s.strip_prefix("cwe-"))
                .unwrap_or(s);
            (!n.is_empty() && n.chars().all(|c| c.is_ascii_digit())).then(|| format!("CWE-{n}"))
        })
        .collect()
}

/// The advisory as the form has it, or what it still needs.
pub(super) fn draft(f: &Form) -> Result<NewAdvisory, String> {
    let get = |field| f.get(field).trim().to_owned();
    let summary = get(Field::Title);
    if summary.is_empty() {
        return Err("An advisory needs a title".into());
    }
    let (ecosystem, package) = (get(Field::Ecosystem), get(Field::Package));
    if ecosystem.is_empty() || package.is_empty() {
        return Err("Name the affected package and its ecosystem".into());
    }
    let severity = get(Field::Severity);
    let severity = if severity.is_empty() {
        return Err("Choose its severity, or type a CVSS vector".into());
    } else if severity.starts_with("CVSS:") {
        Severity::Cvss(severity)
    } else {
        Severity::Level(severity)
    };
    if f.text.trim().is_empty() {
        return Err(
            "Describe the vulnerability: what it is, what it affects, how it's fixed".into(),
        );
    }
    for field in [Field::Cve, Field::Cwes] {
        if let Some(why) = check(field, f.get(field)) {
            return Err(why);
        }
    }
    let cve = Some(get(Field::Cve).to_uppercase()).filter(|c| !c.is_empty());
    Ok(NewAdvisory {
        summary,
        description: f.text.clone(),
        ecosystem,
        package,
        affected: get(Field::Affected),
        patched: get(Field::Patched),
        severity,
        cve,
        cwes: cwes(f.get(Field::Cwes)).unwrap_or_default(),
    })
}

/// Drafts it: GitHub doesn't ask.
pub(super) fn submit(state: &mut State, route: &Route, repo: &RepoId) -> Cmds {
    if let Some(why) = refused(state, repo) {
        return say(state, why);
    }
    let advisory = or_return!(draft(&form(state, route)), |why| say(state, why));
    crate::forms::form_sending(state, route);
    state.info("Drafting the advisory…");
    work::start(Draft {
        repo: repo.clone(),
        advisory,
    })
}

/// A draft advisory GitHub is asked to make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub repo: RepoId,
    pub advisory: NewAdvisory,
}

impl Job for Draft {
    type Out = String;

    async fn run(&self, cx: &Cx) -> Result<String, Failure> {
        Ok(cx.gh.draft_advisory(&self.repo, &self.advisory).await?)
    }

    /// GitHub's answer: the draft's page, in the form's place.
    fn done(self, state: &mut State, result: Result<String, Failure>) -> Cmds {
        let form = Route::Housekeeping(FormOf::Advisory(self.repo.clone()));
        let ghsa = match result {
            Ok(ghsa) => ghsa,
            Err(why) => {
                let why = format!("GitHub refused the advisory: {why}");
                return crate::forms::failed(state, &form, why);
            }
        };
        crate::forms::done::<Form>(state, &form);
        state.info(format!(
            "Drafted {ghsa}: only {}'s admins see it",
            self.repo
        ));
        let page = Route::Security(security::Advisory {
            repo: Some(self.repo),
            ghsa,
        });
        if state.route() == Some(&form) {
            state.forward.clear();
            return state.replace(page, true, None);
        }
        Cmds::new()
    }
}

/// The form's page.
pub(super) fn build(state: &State, page: &mut Page, route: &Route) {
    let Route::Housekeeping(of @ FormOf::Advisory(repo)) = route else { return };
    let f = form(state, route);
    let composing = crate::forms::composing(state, route, 0);
    let body = if composing { "" } else { f.text.as_str() };
    let ecosystem = ECOSYSTEMS
        .iter()
        .find(|(word, _)| *word == f.get(Field::Ecosystem))
        .map(|(_, name)| *name);
    let mut ecosystem_row = form_row(&f, Field::Ecosystem, "Choose one");
    if let Some(name) = ecosystem {
        ecosystem_row.value = name.to_owned();
    }
    let mut severity_row = form_row(&f, Field::Severity, "Choose one, or type a CVSS vector");
    if let Some((_, name)) = LEVELS.iter().find(|(w, _)| *w == f.get(Field::Severity)) {
        severity_row.value = (*name).to_owned();
    }
    let status = vec![(
        format!("A draft: only {repo}'s admins (and those they add) see it until it's published"),
        Mark::Fine,
        None,
    )];
    let view = WriteFormView {
        heading: format!("New draft security advisory in {repo}"),
        base: Some(crate::ui::markdown::LinkBase::new(repo, "HEAD", "")),
        rows: vec![
            form_row(&f, Field::Title, "Title"),
            ecosystem_row,
            form_row(&f, Field::Package, "The package's name"),
            form_row(&f, Field::Affected, "e.g. < 1.2.3"),
            form_row(&f, Field::Patched, "e.g. 1.2.3 (none yet if empty)"),
            severity_row,
            form_row(&f, Field::Cwes, "None (e.g. CWE-79, CWE-89)"),
            form_row(&f, Field::Cve, "None: GitHub can ask for one later"),
        ],
        status,
        boxes: vec![TextBox {
            title: "Description".into(),
            untitled: "Description",
            body,
            markdown: true,
            empty: "No description yet: what the vulnerability is, its impact, its fix.",
            hint: "In Markdown (ctrl-e for $EDITOR)",
            link: Link::Form(crate::ui::form::Row::Text(0)),
            actions: Vec::new(),
        }],
        button: super::button(of, &f),
        why_not: super::problem(state, route, of),
        ..WriteFormView::default()
    };
    crate::forms::write_form(state, route, page, view);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_draft_reads_githubs_fields_and_names_what_it_lacks() {
        let mut f = Form::default();
        assert_eq!(draft(&f).unwrap_err(), "An advisory needs a title");
        for (field, value) in [
            (Field::Title, "Path traversal in the archive reader"),
            (Field::Ecosystem, "rust"),
            (Field::Package, "ghtui"),
            (Field::Affected, "< 0.3.0"),
            (
                Field::Severity,
                "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:N/A:N",
            ),
            (Field::Cwes, "22, CWE-73"),
        ] {
            f.values.insert(field, value.into());
        }
        assert!(draft(&f).unwrap_err().starts_with("Describe"));
        f.text = "Archives can write outside the folder.".into();
        let made = draft(&f).unwrap();
        assert_eq!(made.cwes, ["CWE-22", "CWE-73"]);
        assert!(matches!(made.severity, Severity::Cvss(_)));
        let body = made.body();
        assert_eq!(body["vulnerabilities"][0]["package"]["ecosystem"], "rust");
        assert_eq!(
            body["vulnerabilities"][0]["patched_versions"],
            serde_json::Value::Null
        );
        assert!(body.get("severity").is_none());
        f.values.insert(Field::Cve, "CVE-26-1".into());
        assert_eq!(draft(&f).unwrap_err(), "A CVE ID is CVE-YYYY-NNNN");
        assert_eq!(cvss("cvss:4.0/AV:N"), Some("CVSS:4.0/AV:N".into()));
        assert_eq!(cvss("CVSS:2/AV:N"), None);
    }
}
