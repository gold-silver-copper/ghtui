//! A new issue's or pull request's page, from its form.

use super::*;
use crate::route::pulls;
use crate::ui::page::{Link, Page};
use crate::ui::pages::{self, Field, FormView, Mark, Starter};

/// The page for `route`, a form's, once what it can have has loaded.
pub fn build(state: &State, page: &mut Page, route: &Route, d: &NewForm) {
    let Some(repo) = form_repo(route) else {
        return;
    };
    let blank = Form::default();
    let form = state.creating.forms.get(route).unwrap_or(&blank);
    let pr = matches!(route, Route::Pulls(pulls::NewPr(_)));
    let composing = matches!(
        &state.overlay,
        Some(Overlay::Compose(c)) if c.target == ComposeTarget::Form(route.clone())
    );
    let chooser = form.chooser.as_ref().filter(|c| !pr && c.offers());
    if let (Some(chooser), None) = (chooser, &form.template) {
        let mut starters: Vec<Starter> = (chooser.templates.iter())
            .map(|t| Starter {
                name: t.name.clone(),
                about: t.about.clone(),
                link: Link::Form(FormLink::Template(t.file.clone())),
            })
            .collect();
        if chooser.blank {
            starters.push(Starter {
                name: "Blank issue".into(),
                about: "Start from nothing".into(),
                link: Link::Form(FormLink::Template(String::new())),
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
        head_problem(state, form).err()
    } else {
        None
    };
    let mut status = Vec::new();
    let mut commits = None;
    if pr && form.head.is_some() {
        let pushing = matches!(
            form.head_state(),
            Some(Look::Found(HeadState { local: Some(b), .. })) if !b.pushed()
        );
        match (&problem, form.head_state()) {
            (Some(why), Some(Look::Asking) | None) => {
                status.push((why.clone(), Mark::Waiting, None));
            }
            (Some(why), _) => {
                let push = pushing.then_some(FormLink::Push);
                status.push((why.clone(), Mark::Problem, push));
            }
            (None, _) => {}
        }
        if let Some(Look::Found(hs)) = form.head_state() {
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
            let listed = (hs.commits.iter().rev())
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
    let names = |ids: &[NodeId], from: &Capped<crate::api::create::Choice>, labels: bool| {
        (from.iter())
            .filter(|c| ids.contains(&c.id))
            .map(|c| (c.name.clone(), c.color.clone().filter(|_| labels)))
            .collect()
    };
    let none = |text: &str| {
        if d.can_triage {
            text.to_owned()
        } else {
            "Setting it takes triage access".to_owned()
        }
    };
    let fields = if pr {
        Vec::new()
    } else {
        vec![
            Field {
                name: "Labels",
                values: names(&form.labels, &d.labels, true),
                none: none("None yet"),
                link: FormLink::Labels,
            },
            Field {
                name: "Assignees",
                values: names(&form.assignees, &d.assignees, false),
                none: none("No one"),
                link: FormLink::Assignees,
            },
            Field {
                name: "Milestone",
                values: names(form.milestone.as_slice(), &d.milestones, false),
                none: none("No milestone"),
                link: FormLink::Milestone,
            },
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
    let (write, create_key) = (
        state.first_key(Action::Comment),
        state.first_key(Action::Create),
    );
    let view = FormView {
        repo,
        heading: if pr {
            "Open a pull request"
        } else {
            "New issue"
        },
        template,
        branches: pr.then(|| {
            let (base, head) = (form.base.as_deref(), form.head.as_deref());
            (base.unwrap_or_default(), head.unwrap_or_default())
        }),
        status,
        // While the composer has it, its box is only the place it opens in.
        text: if composing { "" } else { &form.text },
        write: &write,
        create_key: &create_key,
        fields,
        create: if pr {
            "Create pull request…"
        } else {
            "Create issue"
        },
        why_not: why_not.as_deref(),
        error: form.error.as_deref(),
        commits,
    };
    pages::new_form(page, &view);
}
