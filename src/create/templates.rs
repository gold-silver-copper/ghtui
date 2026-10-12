//! What a new issue or pull request starts with: a repository's issue
//! templates (Markdown ones and YAML forms) and its pull request
//! template, and a pull request's title and description from its commits,
//! as GitHub fills them in.

use super::yaml::{self, Value};
use crate::git::local::CommitMessage;

/// An issue template, ready to fill in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueTemplate {
    /// Its file in `.github/ISSUE_TEMPLATE`.
    pub file: String,
    pub name: String,
    pub about: String,
    pub title: String,
    /// Markdown; for a form, a heading per field to answer under.
    pub body: String,
    pub labels: Vec<String>,
    pub assignees: Vec<String>,
}

/// A link the template chooser offers instead of an issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactLink {
    pub name: String,
    pub url: String,
    pub about: String,
}

/// GitHub's template chooser: the templates, whether a blank issue is
/// allowed, and the links `config.yml` adds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Chooser {
    pub templates: Vec<IssueTemplate>,
    pub blank: bool,
    pub links: Vec<ContactLink>,
}

impl Chooser {
    /// Whether there's anything to choose: without templates or links, a
    /// new issue is a blank one.
    pub fn offers(&self) -> bool {
        !self.templates.is_empty() || !self.links.is_empty()
    }
}

/// The chooser for the files in `.github/ISSUE_TEMPLATE` (name and text),
/// in GitHub's order: by file name.
pub fn chooser(files: &[(String, String)]) -> Chooser {
    let mut files: Vec<&(String, String)> = files.iter().collect();
    files.sort_by_key(|(name, _)| name.to_lowercase());
    let mut chooser = Chooser {
        blank: true,
        ..Chooser::default()
    };
    for (file, text) in files {
        let lower = file.to_lowercase();
        let yml = lower.ends_with(".yml") || lower.ends_with(".yaml");
        if yml && lower.starts_with("config.") {
            config(&mut chooser, &yaml::parse(text));
        } else if yml {
            chooser.templates.push(form(file, &yaml::parse(text)));
        } else if lower.ends_with(".md") {
            chooser.templates.push(markdown(file, text));
        }
    }
    chooser
}

fn config(chooser: &mut Chooser, config: &Value) {
    chooser.blank = !config
        .get("blank_issues_enabled")
        .is_some_and(Value::is_false);
    let links = config
        .get("contact_links")
        .map(Value::list)
        .unwrap_or_default();
    chooser.links = (links.iter())
        .filter_map(|link| {
            let text = |key| link.get(key).and_then(Value::text).map(str::to_owned);
            Some(ContactLink {
                name: text("name")?,
                url: text("url")?,
                about: text("about").unwrap_or_default(),
            })
        })
        .collect();
}

/// The text at `key`, or nothing.
fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::text)
        .unwrap_or_default()
        .to_owned()
}

/// A file's name without its extension: a template's name when it has none.
fn stem(file: &str) -> String {
    file.rsplit_once('.')
        .map_or(file, |(stem, _)| stem)
        .to_owned()
}

/// A Markdown template: its front matter, then its body.
fn markdown(file: &str, source: &str) -> IssueTemplate {
    let source = source.replace("\r\n", "\n");
    let (front, body) = match source.strip_prefix("---\n").and_then(|rest| {
        let end = rest.find("\n---")?;
        let body = rest.get(end + 4..)?;
        Some((
            rest.get(..end)?.to_owned(),
            body.trim_start_matches('\n').to_owned(),
        ))
    }) {
        Some((front, body)) => (yaml::parse(&front), body),
        None => (Value::Null, source.clone()),
    };
    let name = text_or(&front, "name", stem(file));
    IssueTemplate {
        file: file.to_owned(),
        name,
        about: text(&front, "about"),
        title: text(&front, "title"),
        body,
        labels: front.get("labels").map(Value::names).unwrap_or_default(),
        assignees: front.get("assignees").map(Value::names).unwrap_or_default(),
    }
}

fn text_or(value: &Value, key: &str, or: String) -> String {
    Some(text(value, key))
        .filter(|t| !t.trim().is_empty())
        .unwrap_or(or)
}

/// An issue form: each field becomes a heading to answer under, as GitHub
/// writes the issue it makes. Its Markdown blocks are instructions, not
/// part of the issue, so they're left out.
fn form(file: &str, form: &Value) -> IssueTemplate {
    let mut sections = Vec::new();
    let items = form.get("body").map(Value::list).unwrap_or_default();
    for item in items {
        let kind = text(item, "type");
        let attributes = item.get("attributes").unwrap_or(&Value::Null);
        let label = text(attributes, "label");
        if kind == "markdown" || label.is_empty() {
            continue;
        }
        let value = text(attributes, "value");
        let options = attributes
            .get("options")
            .map(Value::list)
            .unwrap_or_default();
        let answer = match kind.as_str() {
            "checkboxes" => (options.iter())
                .map(|option| {
                    let option = option
                        .text()
                        .map_or_else(|| text(option, "label"), str::to_owned);
                    format!("- [ ] {option}")
                })
                .collect::<Vec<_>>()
                .join("\n"),
            "dropdown" => {
                let names: Vec<&str> = options.iter().filter_map(Value::text).collect();
                format!("<!-- One of: {} -->", names.join(", "))
            }
            _ => match attributes.get("render").and_then(Value::text) {
                Some(lang) => format!("```{lang}\n{}```", with_newline(&value)),
                None => value.trim_end().to_owned(),
            },
        };
        let section = format!("### {label}\n\n{answer}");
        sections.push(section.trim_end().to_owned());
    }
    IssueTemplate {
        file: file.to_owned(),
        name: text_or(form, "name", stem(file)),
        about: text(form, "description"),
        title: text(form, "title"),
        body: sections.join("\n\n"),
        labels: form.get("labels").map(Value::names).unwrap_or_default(),
        assignees: form.get("assignees").map(Value::names).unwrap_or_default(),
    }
}

fn with_newline(text: &str) -> String {
    match text.trim_end() {
        "" => String::new(),
        text => format!("{text}\n"),
    }
}

/// A new pull request's text, as GitHub fills it in: one commit's subject
/// and the rest of its message; for more, the branch's name and a list of
/// the commits. The repository's template follows.
pub fn pr_text(branch: &str, commits: &[CommitMessage], template: Option<&str>) -> String {
    let (title, body) = match commits {
        [one] => {
            let rest = one
                .message
                .trim()
                .lines()
                .skip(1)
                .collect::<Vec<_>>()
                .join("\n");
            (one.subject().to_owned(), rest.trim().to_owned())
        }
        _ => {
            let list: Vec<String> = commits
                .iter()
                .map(|c| format!("- {}", c.subject()))
                .collect();
            (humanize(branch), list.join("\n"))
        }
    };
    let template = template.map(str::trim).filter(|t| !t.is_empty());
    let body = match (body.is_empty(), template) {
        (_, None) => body,
        (true, Some(template)) => template.to_owned(),
        (false, Some(template)) => format!("{body}\n\n{template}"),
    };
    join(&title, &body)
}

/// A branch's name as a title: `fix/the-thing` reads "Fix/the thing".
fn humanize(branch: &str) -> String {
    let words = branch.replace(['-', '_'], " ");
    let mut chars = words.trim().chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The composer's text: the title on the first line, then the body.
pub fn join(title: &str, body: &str) -> String {
    match body.trim() {
        "" => title.to_owned(),
        body => format!("{title}\n\n{body}"),
    }
}

/// The composer's text as a title (its first line) and a body (the rest).
pub fn split(text: &str) -> (String, String) {
    let mut lines = text.lines();
    let title = lines.next().unwrap_or_default().trim().to_owned();
    let body = lines.collect::<Vec<_>>().join("\n");
    (title, body.trim_matches('\n').trim_end().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, text: &str) -> (String, String) {
        (name.to_owned(), text.to_owned())
    }

    /// Markdown templates and forms are offered by file name, a form's
    /// fields become headings to answer under, and `config.yml` turns blank
    /// issues off and adds its links.
    #[test]
    fn templates_are_found_and_prefill_the_issue() {
        let chooser = chooser(&[
            file(
                "feature.md",
                "---\nname: Feature request\nabout: Suggest an idea\ntitle: \"[Idea] \"\nlabels: enhancement\nassignees: octocat\n---\n\n## What\n\nDescribe it.\n",
            ),
            file(
                "bug.yml",
                "name: Bug report\ndescription: Something broke\ntitle: \"[Bug]: \"\nlabels: [bug]\nbody:\n  - type: markdown\n    attributes:\n      value: Thanks!\n  - type: input\n    attributes:\n      label: Version\n      value: \"1.0\"\n  - type: textarea\n    attributes:\n      label: Logs\n      render: shell\n  - type: checkboxes\n    attributes:\n      label: Checks\n      options:\n        - label: I searched first\n  - type: dropdown\n    attributes:\n      label: OS\n      options: [Linux, macOS]\n",
            ),
            file(
                "config.yml",
                "blank_issues_enabled: false\ncontact_links:\n  - name: Questions\n    url: https://example.com/forum\n    about: Ask there\n",
            ),
        ]);
        assert!(!chooser.blank);
        assert_eq!(chooser.links[0].url, "https://example.com/forum");
        let names: Vec<&str> = chooser.templates.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["Bug report", "Feature request"]);
        let bug = &chooser.templates[0];
        assert_eq!(bug.title, "[Bug]: ");
        assert_eq!(bug.labels, ["bug"]);
        assert_eq!(
            bug.body,
            "### Version\n\n1.0\n\n### Logs\n\n```shell\n```\n\n### Checks\n\n- [ ] I searched first\n\n### OS\n\n<!-- One of: Linux, macOS -->"
        );
        let feature = &chooser.templates[1];
        assert_eq!(feature.about, "Suggest an idea");
        assert_eq!(feature.title, "[Idea] ");
        assert_eq!(feature.body, "## What\n\nDescribe it.\n");
        assert_eq!(
            (feature.labels.as_slice(), feature.assignees.as_slice()),
            (&["enhancement".to_owned()][..], &["octocat".to_owned()][..])
        );
    }

    /// One commit gives its subject and message; more give the branch's
    /// name and a list of them; the template comes after.
    #[test]
    fn a_pull_request_is_filled_in_from_its_commits() {
        let commit = |oid: &str, message: &str| CommitMessage {
            oid: oid.into(),
            message: message.into(),
        };
        let one = [commit("a", "Fix the parser\n\nIt dropped the last line.\n")];
        assert_eq!(
            pr_text("fix-parser", &one, None),
            "Fix the parser\n\nIt dropped the last line."
        );
        let two = [
            commit("a", "Fix the parser"),
            commit("b", "Test it\n\nMore."),
        ];
        assert_eq!(
            pr_text("fix_the-parser", &two, Some("## Checklist\n")),
            "Fix the parser\n\n- Fix the parser\n- Test it\n\n## Checklist"
        );
        assert_eq!(
            pr_text("x", &[commit("a", "Only a subject")], Some("T")),
            "Only a subject\n\nT"
        );
        assert_eq!(
            split("Title\n\nBody\nmore\n"),
            ("Title".into(), "Body\nmore".into())
        );
        assert_eq!(split("Title only"), ("Title only".into(), String::new()));
    }
}
