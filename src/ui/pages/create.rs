//! New issues and pull requests: the template chooser, and the form.

use super::*;
use crate::ui::form::Row;

/// Something to start a new issue from: a template, a blank issue, or a
/// link the repository sends you to instead.
pub struct Starter {
    pub name: String,
    pub about: String,
    pub link: Link,
}

/// GitHub's template chooser.
pub fn template_chooser(page: &mut Page, repo: &RepoId, starters: &[Starter]) {
    page.line(vec![Seg::new(format!("New issue in {repo}"), Role::Title)]);
    page.line(vec![Seg::new("Choose how to start", Role::Meta)]);
    page.box_top(vec![Seg::new("Templates", Role::Strong)], Vec::new());
    for (i, s) in starters.iter().enumerate() {
        if i > 0 {
            page.box_rule();
        }
        let external = matches!(&s.link, Link::Url(_));
        item(page, s.link.clone(), |page, link| {
            let open = if external { "Open ↗" } else { "Get started" };
            let right = vec![Seg::new(format!(" {open} "), Role::Chip(Bg::ContainerHigh))];
            page.box_line(
                vec![Seg::linked(s.name.clone(), Role::Strong, link)],
                right,
                0,
            );
            if !s.about.trim().is_empty() {
                page.wrapped(vec![Seg::new(s.about.clone(), Role::Meta)], 0, Frame::Body);
            }
        });
    }
    page.box_bottom();
}

/// A row of the form: labels, assignees, a milestone.
pub struct Field {
    pub name: &'static str,
    /// Each with its label color, if it's a label.
    pub values: Vec<(String, Option<String>)>,
    /// What it says without values (or why they can't be set).
    pub none: String,
    pub link: Row,
}

/// How a line about the branches reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Fine,
    Waiting,
    Problem,
}

/// A new issue or pull request, as written so far.
pub struct FormView<'a> {
    pub repo: &'a RepoId,
    /// "New issue", "Open a pull request".
    pub heading: &'a str,
    /// The template it started from, when another can be chosen.
    pub template: Option<&'a str>,
    /// A pull request's base and head branches (empty: not chosen).
    pub branches: Option<(&'a str, &'a str)>,
    /// Lines about the branches; with `Push`, Enter on it pushes.
    pub status: Vec<(String, Mark, Option<Row>)>,
    /// The title on the first line, then the description.
    pub text: &'a str,
    /// The keys that write the text, and that make it.
    pub write: &'a str,
    pub create_key: &'a str,
    pub fields: Vec<Field>,
    /// The button's label, and why it can't be pressed yet.
    pub create: &'a str,
    pub why_not: Option<&'a str>,
    pub error: Option<&'a str>,
    /// A pull request's commits (short ID and subject), and how many more
    /// there are.
    pub commits: Option<(Vec<(String, String)>, u64)>,
}

pub fn new_form(page: &mut Page, v: &FormView<'_>) {
    page.line(vec![Seg::new(
        format!("{} in {}", v.heading, v.repo),
        Role::Title,
    )]);
    if let Some(name) = v.template {
        item(page, Link::Form(Row::Templates), |page, link| {
            page.line(vec![
                Seg::new("Template: ", Role::Meta),
                Seg::new(name.to_owned(), Role::Strong),
                Seg::linked("  · choose another", Role::Link, link),
            ]);
        });
    }
    if let Some((base, head)) = v.branches {
        page.blank();
        for (what, name, target) in [("base: ", base, Row::Base), ("compare: ", head, Row::Head)] {
            item(page, Link::Form(target), |page, link| {
                let name = if name.is_empty() {
                    Seg::linked("choose a branch ▾", Role::Link, link)
                } else {
                    Seg::linked(format!("{name} ▾"), Role::Strong, link)
                };
                page.line(vec![Seg::new(what, Role::Meta), name]);
            });
        }
        for (text, mark, target) in &v.status {
            let (sign, role) = match mark {
                Mark::Fine => ("✓ ", Role::Success),
                Mark::Waiting => ("… ", Role::Meta),
                Mark::Problem => ("! ", Role::Error),
            };
            let start = page.lines.len();
            let link = target.clone().map(|t| page.link(Link::Form(t)));
            let text = match link {
                Some(link) => Seg::linked(text.clone(), Role::Body, link),
                None => Seg::new(text.clone(), Role::Body),
            };
            page.wrapped(vec![Seg::new(sign, role), text], 0, Frame::None);
            if let Some(link) = link {
                page.item(start, link);
            }
        }
    }
    text_box(page, v);
    for f in &v.fields {
        item(page, Link::Form(f.link.clone()), |page, link| {
            let mut segs = vec![Seg::linked(format!("{:<11}", f.name), Role::Strong, link)];
            if f.values.is_empty() {
                segs.push(Seg::new(format!(" {}", f.none), Role::Meta));
            }
            for (value, color) in &f.values {
                segs.push(match color {
                    Some(color) => Seg::new(format!(" {value} "), Role::Label(color.clone())),
                    None => Seg::new(format!(" {value}"), Role::Body),
                });
                segs.push(space());
            }
            page.wrapped(segs, 0, Frame::None);
        });
    }
    page.blank();
    item(page, Link::Form(Row::Save), |page, link| {
        let button = format!(" {} ", v.create);
        let role = match v.why_not {
            Some(_) => Role::Chip(Bg::ContainerHigh),
            None => Role::Chip(Bg::SuccessContainer),
        };
        page.line(vec![
            Seg::linked(button, role, link),
            Seg::new(format!("  {}", v.create_key), Role::Meta),
        ]);
        if let Some(why) = v.why_not {
            page.wrapped(vec![Seg::new(why.to_owned(), Role::Meta)], 0, Frame::None);
        }
    });
    if let Some(error) = v.error {
        page.wrapped(
            vec![Seg::new(error.to_owned(), Role::Error)],
            0,
            Frame::None,
        );
    }
    if let Some((commits, more)) = &v.commits {
        let n = commits.len() as u64 + more;
        page.box_top(
            vec![Seg::new(plural(n, "commit"), Role::Strong)],
            Vec::new(),
        );
        if commits.is_empty() {
            empty_row(page, "No commits to merge");
        }
        for (oid, subject) in commits {
            body(
                page,
                vec![
                    Seg::new(format!("{oid} "), Role::Code),
                    Seg::new(subject.clone(), Role::Body),
                ],
            );
        }
        if *more > 0 {
            empty_row(page, &format!("and {}, older", plural(*more, "commit")));
        }
        page.box_bottom();
    }
}

/// The title and description, where the composer opens: the box GitHub
/// calls "Add a title" and "Add a description".
fn text_box(page: &mut Page, v: &FormView<'_>) {
    let (title, rest) = match v.text.split_once('\n') {
        Some((title, rest)) => (title.trim(), rest.trim()),
        None => (v.text.trim(), ""),
    };
    item(page, Link::Form(Row::Text(0)), |page, link| {
        let title = if title.is_empty() {
            Seg::linked("Add a title", Role::Meta, link)
        } else {
            Seg::linked(title.to_owned(), Role::Strong, link)
        };
        let right = vec![Seg::new(format!("{} writes", v.write), Role::Meta)];
        page.box_top(vec![title], right);
        if rest.is_empty() {
            empty_row(page, "No description yet.");
        } else {
            let base = LinkBase::new(v.repo, "HEAD", "");
            markdown::render(page, rest, Some(&base), Frame::Body);
        }
        page.box_gap();
        let hint =
            "The first line is the title; then the description, in Markdown (ctrl-e for $EDITOR)";
        body(page, vec![Seg::linked(hint, Role::Meta, link)]);
        page.box_bottom();
    });
    page.blank();
}
