//! A new issue's or pull request's page, from its form.

use super::*;
use crate::route::pulls;
use crate::ui::page::{Link, Page};
use crate::ui::pages::{self, FormRow, Mark, Starter, TextBox, WriteFormView};

/// The page for `route`, a form's, once what it can have has loaded.
pub fn build(state: &State, page: &mut Page, route: &Route, d: &NewForm) {
    let Some(repo) = form_repo(route) else { return };
    let form = state.forms.shown::<Form>(route);
    let form = form.as_ref();
    let pr = matches!(route, Route::Pulls(pulls::NewPr(_)));
    let composing = crate::forms::composing(state, route, 0);
    let chooser = form.chooser.as_ref().filter(|c| !pr && c.offers());
    if let (Some(chooser), None) = (chooser, &form.template) {
        let mut starters: Vec<Starter> = (chooser.templates.iter())
            .map(|t| Starter {
                name: t.name.clone(),
                about: t.about.clone(),
                link: Link::Form(Row::Template(t.file.clone())),
            })
            .collect();
        if chooser.blank {
            starters.push(Starter {
                name: "Blank issue".into(),
                about: "Start from nothing".into(),
                link: Link::Form(Row::Template(String::new())),
            });
        }
        starters.extend(chooser.links.iter().map(|l| Starter {
            name: l.name.clone(),
            about: l.about.clone(),
            link: Link::Url(l.url.clone()),
        }));
        pages::template_chooser(page, repo, &starters);
        return;
    }
    let template = chooser.and_then(|c| {
        let file = form.template.as_deref()?;
        let named = c.templates.iter().find(|t| t.file == file);
        Some(named.map_or("Blank issue", |t| t.name.as_str()))
    });
    let problem = if pr {
        head_problem(state, repo, form).err()
    } else {
        None
    };
    let mut status = Vec::new();
    let mut commits = None;
    if pr && form.head.is_some() {
        let head = form.head_state(state, repo);
        let pushing = matches!(
            head,
            Some(Ok(HeadState { local: Some(b), .. })) if !b.pushed()
        );
        match (&problem, head) {
            (Some(why), None) => {
                status.push((why.clone(), Mark::Waiting, None));
            }
            (Some(why), _) => {
                let push = pushing.then_some(Row::Push);
                status.push((why.clone(), Mark::Problem, push));
            }
            (None, _) => {}
        }
        if let Some(Ok(hs)) = head {
            if let Some(b) = hs.local.as_ref().filter(|b| b.pushed()) {
                let text = match b.behind {
                    Some(n) if n > 0 => format!(
                        "GitHub's {} has {} your clone's doesn't",
                        b.name,
                        count(n, "commit")
                    ),
                    _ => format!("{} is on GitHub as it is in your clone", b.name),
                };
                status.push((text, Mark::Fine, None));
            }
            let listed: Vec<_> = (hs.commits.iter().rev())
                .map(|c| {
                    (
                        crate::ui::text::short_sha(&c.oid).to_owned(),
                        c.subject().to_owned(),
                    )
                })
                .collect();
            commits = Some((listed, hs.commits.left_out()));
        }
    }
    let mut rows = Vec::new();
    if let Some(name) = template {
        rows.push(row("Template", name, Row::Templates));
    }
    if pr {
        rows.push(row(
            "Base",
            form.base.as_deref().unwrap_or_default(),
            Row::Base,
        ));
        rows.push(row(
            "Compare",
            form.head.as_deref().unwrap_or_default(),
            Row::Head,
        ));
    }
    let set = |name: &'static str,
               ids: &[NodeId],
               from: &Capped<crate::api::create::Choice>,
               none: &str,
               link| {
        let chosen = (from.iter()).filter(|c| ids.contains(&c.id));
        let (chips, names): (Vec<_>, Vec<_>) = chosen.partition(|c| c.color.is_some());
        let names: Vec<&str> = names.iter().map(|c| c.name.as_str()).collect();
        let unset = names.is_empty() && chips.is_empty();
        let none = if d.can_triage {
            none
        } else {
            "Setting it takes triage access"
        };
        FormRow {
            name,
            value: if unset {
                none.to_owned()
            } else {
                names.join(" ")
            },
            unset,
            chips: (chips.iter())
                .map(|c| (c.name.clone(), c.color.clone().unwrap_or_default()))
                .collect(),
            link: Some(link),
        }
    };
    let after = if pr {
        Vec::new()
    } else {
        vec![
            set("Labels", &form.labels, &d.labels, "None yet", Row::Labels),
            set(
                "Assignees",
                &form.assignees,
                &d.assignees,
                "No one",
                Row::Assignees,
            ),
            set(
                "Milestone",
                form.milestone.as_slice(),
                &d.milestones,
                "No milestone",
                Row::Milestone,
            ),
        ]
    };
    // What the branches lack shows beside them, once one is chosen.
    let problem = problem.filter(|_| form.head.is_none());
    let why_not = refused(state, repo, pr).or(problem).or_else(|| {
        let (title, _) = templates::split(&form.text);
        title
            .is_empty()
            .then(|| "It needs a title first".to_owned())
    });
    // While the composer has it, its box is only the place it opens in.
    let text = if composing { "" } else { form.text.as_str() };
    let (title, description) = match text.split_once('\n') {
        Some((title, rest)) => (title.trim(), rest.trim()),
        None => (text.trim(), ""),
    };
    let heading = if pr {
        "Open a pull request"
    } else {
        "New issue"
    };
    let view = WriteFormView {
        heading: format!("{heading} in {repo}"),
        base: Some(crate::ui::markdown::LinkBase::new(repo, "HEAD", "")),
        rows,
        status,
        boxes: vec![TextBox {
            title: title.to_owned(),
            untitled: "Add a title",
            body: description,
            markdown: true,
            empty: "No description yet.",
            hint: "The first line is the title; then the description, in Markdown (ctrl-e for $EDITOR)",
            link: Link::Form(Row::Text(0)),
            actions: Vec::new(),
        }],
        after,
        button: if pr {
            "Create pull request…".into()
        } else {
            "Create issue".into()
        },
        why_not,
        ..WriteFormView::default()
    };
    crate::forms::write_form(state, route, page, view);
    if let Some((commits, more)) = commits {
        pages::commits_box(page, &commits, more);
    }
}

/// A row naming what's chosen (a template, a branch), which Enter changes.
fn row(name: &'static str, chosen: &str, link: Row) -> FormRow {
    FormRow {
        name,
        value: if chosen.is_empty() {
            "choose a branch ▾".to_owned()
        } else {
            format!("{chosen} ▾")
        },
        unset: chosen.is_empty(),
        chips: Vec::new(),
        link: Some(link),
    }
}
