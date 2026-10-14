//! Forking a repository, and making a new one, as GitHub's two forms do;
//! then the new repository's page, which follows GitHub until it's
//! there, and (for a new one) an offer to clone it.

use crate::route::{Route, code};
use std::path::PathBuf;

use crate::api::housekeeping::{MadeRepo, NewFork, NewRepo, Owners, Templates};
use crate::api::model::RepoId;
use crate::picker::fuzzy_score;
use crate::ui::page::Page;
use crate::ui::pages::{Mark, WriteFormView};

use super::{
    Chosen, Expected, Field, Form, FormOf, Subject, ask, expect, form, form_mut, form_row, item,
};
use crate::data::DataKey;
use crate::keymap::Action;
use crate::state::{Cmd, State};
use crate::work::{self, Cx, Failure, Job};

/// Forking a repository; the form's page asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fork {
    pub route: Route,
    pub new: NewFork,
}

/// Making a new repository; the form's page asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Make {
    pub route: Route,
    pub new: NewRepo,
}

/// Cloning a new repository into `dir`, with your own git.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloneTo {
    pub url: String,
    pub dir: PathBuf,
}

fn owners(state: &State) -> Option<&Owners> {
    state.picked(&DataKey::Owners)
}

/// `,F`: the repository's fork form.
#[must_use]
pub(super) fn open_fork(state: &mut State) -> Vec<Cmd> {
    let Some(repo) = state.route().and_then(Route::repo).cloned() else {
        return state.just_say("Open a repository first: a fork is of one");
    };
    super::open(state, FormOf::Fork(repo))
}

/// Fills the fork's form once GitHub says who you are: your account, the
/// repository's name, its default branch only.
#[must_use]
pub(super) fn follow_fork(state: &mut State, route: &Route, repo: &RepoId) -> Vec<Cmd> {
    if form(state, route).started {
        return Vec::new();
    }
    let Some(viewer) = owners(state).map(|o| o.viewer.clone()) else {
        return Vec::new();
    };
    let f = form_mut(state, route);
    f.started = true;
    f.set(Field::Owner, viewer);
    f.set(Field::Name, repo.name.clone());
    Vec::new()
}

/// Fills the new repository's form once GitHub says who you are; then
/// its name is asked for.
#[must_use]
pub(super) fn follow_new(state: &mut State, route: &Route) -> Vec<Cmd> {
    if form(state, route).started {
        return Vec::new();
    }
    let Some(viewer) = owners(state).map(|o| o.viewer.clone()) else {
        return Vec::new();
    };
    let f = form_mut(state, route);
    f.started = true;
    f.set(Field::Owner, viewer);
    if state.overlay.is_none() {
        return super::open_field(state, route, Field::Name);
    }
    Vec::new()
}

/// Why `name` can't name a repository, if it can't: GitHub takes
/// letters, digits, `-`, `_` and `.`, and turns anything else into `-`.
pub(super) fn name_problem(name: &str) -> Option<String> {
    let ok = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
    if name.is_empty() {
        Some("A repository needs a name".into())
    } else if !name.chars().all(ok) {
        Some("A repository's name is letters, digits, -, _ and .".into())
    } else if name == "." || name == ".." || name.len() > 100 {
        Some("That can't be a repository's name".into())
    } else {
        None
    }
}

/// The owner, as GitHub's API takes it: none for your account.
fn org(state: &State, f: &Form) -> Option<String> {
    let viewer = owners(state).map(|o| o.viewer.as_str());
    let owner = f.get(Field::Owner);
    (Some(owner) != viewer && !owner.is_empty()).then(|| owner.to_owned())
}

/// Why the form can't be sent yet: who you are unknown, a bad name.
pub(super) fn repo_problem(f: &Form) -> Option<String> {
    if f.get(Field::Owner).is_empty() {
        return Some("Still asking GitHub who you are".into());
    }
    name_problem(f.get(Field::Name).trim())
}

/// [`repo_problem`], and a fork to where the repository is already.
pub(super) fn fork_problem(state: &State, repo: &RepoId, f: &Form) -> Option<String> {
    let owner = f.get(Field::Owner);
    if owner.eq_ignore_ascii_case(&repo.owner) {
        let parent = state.overview(repo).and_then(|o| o.parent.clone());
        return Some(match parent {
            Some(parent) => {
                format!("{repo} is {owner}'s fork of {parent} already: choose another owner")
            }
            None => format!("{owner} owns {repo}: fork it to another owner"),
        });
    }
    repo_problem(f)
}

/// The owner picker's rows: you, then your organizations.
pub(super) fn owner_rows(
    state: &State,
    route: &Route,
    q: &str,
) -> Vec<(
    crate::ui::overlays::PaletteItem,
    Option<crate::picker::Choice>,
)> {
    let Some(o) = owners(state) else {
        return vec![item("Asking GitHub who you are…", "", None)];
    };
    let set = |login: &str| Some(Chosen::Field(route.clone(), Field::Owner, login.to_owned()));
    let mut rows = Vec::new();
    if fuzzy_score(q, &o.viewer).is_some() {
        rows.push(item(o.viewer.clone(), "your account", set(&o.viewer)));
    }
    for (login, can) in &o.orgs {
        if fuzzy_score(q, login).is_none() {
            continue;
        }
        rows.push(if *can {
            item(login.clone(), "organization", set(login))
        } else {
            item(
                login.clone(),
                "the organization doesn't let you make repositories",
                None,
            )
        });
    }
    if o.more > 0 {
        rows.push(item(
            format!("{} more organizations GitHub didn't list", o.more),
            "",
            None,
        ));
    }
    rows
}

/// The `.gitignore` or licence picker's rows: none, then GitHub's.
/// The licence row, by the licence's name (`MIT License`), not the key
/// GitHub's API takes (`mit`).
fn licence_row(state: &State, f: &Form) -> crate::ui::pages::FormRow {
    let mut row = form_row(f, Field::License, "None");
    let templates = state.picked::<Templates>(&DataKey::Templates);
    let named = templates.and_then(|t| {
        (t.licenses.iter())
            .find(|(key, _)| *key == row.value)
            .map(|(_, name)| name.clone())
    });
    if let Some(name) = named {
        row.value = name;
    }
    row
}

pub(super) fn template_rows(
    state: &State,
    route: &Route,
    field: Field,
    q: &str,
) -> Vec<(
    crate::ui::overlays::PaletteItem,
    Option<crate::picker::Choice>,
)> {
    let set = |value: &str| Some(Chosen::Field(route.clone(), field, value.to_owned()));
    let Some(t) = state.picked::<Templates>(&DataKey::Templates) else {
        return vec![item("Loading GitHub's templates…", "", None)];
    };
    let mut rows = Vec::new();
    if fuzzy_score(q, "none").is_some() {
        rows.push(item("None", "", set("")));
    }
    let mut hits: Vec<(usize, String, String)> = match field {
        Field::Gitignore => (t.gitignores.iter())
            .filter_map(|g| Some((fuzzy_score(q, g)?, g.clone(), g.clone())))
            .collect(),
        _ => (t.licenses.iter())
            .filter_map(|(key, name)| Some((fuzzy_score(q, name)?, name.clone(), key.clone())))
            .collect(),
    };
    hits.sort_by_key(|(score, ..)| *score);
    for (_, label, value) in hits {
        rows.push(item(label, "", set(&value)));
    }
    rows
}

/// The fork's form sent.
#[must_use]
pub(super) fn fork(state: &mut State, route: &Route, repo: &RepoId) -> Vec<Cmd> {
    let f = form(state, route);
    if let Some(why) = fork_problem(state, repo, &f) {
        return crate::forms::say(state, why);
    }
    let new = NewFork {
        repo: repo.clone(),
        org: org(state, &f),
        name: f.get(Field::Name).trim().to_owned(),
        default_branch_only: !f.on(Field::AllBranches),
    };
    crate::forms::form_sending(state, route);
    state.info(format!("Forking {repo}…"));
    work::start(Fork {
        route: route.clone(),
        new,
    })
}

/// The new repository's form sent.
#[must_use]
pub(super) fn create(state: &mut State, route: &Route) -> Vec<Cmd> {
    let f = form(state, route);
    if let Some(why) = repo_problem(&f) {
        return crate::forms::say(state, why);
    }
    let template = |field| Some(f.get(field).to_owned()).filter(|v| !v.is_empty());
    let new = NewRepo {
        org: org(state, &f),
        name: f.get(Field::Name).trim().to_owned(),
        description: f.get(Field::Description).trim().to_owned(),
        private: f.on(Field::Private),
        readme: f.on(Field::Readme),
        gitignore: template(Field::Gitignore),
        license: template(Field::License),
    };
    crate::forms::form_sending(state, route);
    state.info(format!("Creating {}…", new.name));
    work::start(Make {
        route: route.clone(),
        new,
    })
}

/// Made: the form gives way to the repository's page, which follows
/// GitHub until it's there.
#[must_use]
fn opened(state: &mut State, route: &Route, made: &MadeRepo) -> Option<Vec<Cmd>> {
    crate::forms::done::<Form>(state, route);
    let repo = made.repo()?;
    if state.route() != Some(route) {
        return Some(Vec::new());
    }
    let page = Route::Code(code::Repo(repo.clone()));
    let cmds = state.replace(page.clone(), true, None);
    expect(state, page, Expected::Repo(repo));
    Some(cmds)
}

impl Job for Fork {
    type Out = MadeRepo;

    async fn run(&self, cx: &Cx) -> Result<MadeRepo, Failure> {
        Ok(cx.gh.fork(&self.new).await?)
    }

    fn done(self, state: &mut State, result: Result<MadeRepo, Failure>) -> Vec<Cmd> {
        let Fork { route, .. } = &self;
        match result {
            Ok(made) => {
                state.info(format!(
                    "Forked as {}: GitHub may take a moment to copy it",
                    made.full_name
                ));
                opened(state, route, &made).unwrap_or_default()
            }
            Err(why) => crate::forms::failed(state, route, why.to_string()),
        }
    }
}

impl Job for Make {
    type Out = MadeRepo;

    async fn run(&self, cx: &Cx) -> Result<MadeRepo, Failure> {
        Ok(cx.gh.create_repo(&self.new).await?)
    }

    fn done(self, state: &mut State, result: Result<MadeRepo, Failure>) -> Vec<Cmd> {
        let Make { route, .. } = &self;
        let made = match result {
            Ok(made) => made,
            Err(why) => {
                return crate::forms::failed(state, route, why.to_string());
            }
        };
        state.info(format!("Created {}", made.full_name));
        let on_form = state.route() == Some(route);
        let cmds = opened(state, route, &made).unwrap_or_default();
        // Offered as GitHub's page offers it: its clone command, run here.
        let Some(repo) = made.repo().filter(|_| on_form) else {
            return cmds;
        };
        let dir = PathBuf::from(&repo.name);
        ask(
            state,
            Some(Subject::Clone {
                url: made.clone_url.clone(),
                dir: dir.clone(),
            }),
            format!("Clone {repo}?"),
            vec![(
                format!(
                    "Into ./{} here, with your own git: git clone {}",
                    dir.display(),
                    made.clone_url
                ),
                false,
            )],
            vec![("Clone it".into(), super::Answer::Clone)],
        );
        cmds
    }
}

impl Job for CloneTo {
    type Out = ();
    const WRITES: bool = false;

    async fn run(&self, _: &Cx) -> Result<(), Failure> {
        super::git::clone(&self.url, &self.dir)
            .await
            .map_err(Failure::msg)
    }

    fn done(self, state: &mut State, result: Result<(), Failure>) -> Vec<Cmd> {
        let CloneTo { dir, .. } = &self;
        match result {
            Ok(()) => {
                state.overlay = None;
                state.just_say(format!("Cloned into ./{}", dir.display()))
            }
            Err(why) => crate::forms::say(state, why.to_string()),
        }
    }
}

/// The fork's or the new repository's page.
pub(super) fn build(state: &State, page: &mut Page, route: &Route, of: &FormOf) {
    let f = form(state, route);
    let (heading, rows, mut status) = match of {
        FormOf::Fork(repo) => (
            format!("Create a new fork of {repo}"),
            vec![
                form_row(&f, Field::Owner, "Asking GitHub who you are…"),
                form_row(&f, Field::Name, "Repository name"),
                form_row(&f, Field::AllBranches, ""),
            ],
            vec![(
                "A fork is a copy of the repository, under the owner you choose".to_owned(),
                Mark::Fine,
            )],
        ),
        _ => (
            "Create a new repository".to_owned(),
            vec![
                form_row(&f, Field::Owner, "Asking GitHub who you are…"),
                form_row(&f, Field::Name, "Repository name"),
                form_row(&f, Field::Description, "Description (optional)"),
                form_row(&f, Field::Private, ""),
                form_row(&f, Field::Readme, ""),
                form_row(&f, Field::Gitignore, "None"),
                licence_row(state, &f),
            ],
            Vec::new(),
        ),
    };
    let owner = f.get(Field::Owner);
    let name = f.get(Field::Name).trim();
    if !owner.is_empty() && !name.is_empty() {
        status.push((format!("It will be {owner}/{name}"), Mark::Fine));
    }
    let save_key = state.first_key(Action::Create);
    let view = WriteFormView {
        heading,
        base: None,
        rows,
        status,
        boxes: Vec::new(),
        after: Vec::new(),
        write: "",
        button: super::button(of, &f),
        save_key: &save_key,
        why_not: super::problem(state, route, of),
        error: state.forms.sent(route).error.as_deref(),
    };
    crate::ui::pages::write_form(page, &view);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_names_are_githubs() {
        assert_eq!(name_problem("ghtui-live-1"), None);
        assert_eq!(name_problem("a.b_c"), None);
        assert!(name_problem("has space").is_some());
        assert!(name_problem("..").is_some());
        assert!(name_problem("").is_some());
    }
}
