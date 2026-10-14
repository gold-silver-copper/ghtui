//! What ghtui does with GitHub Actions beyond showing it: saving a run's
//! artifacts where you say, and running a workflow by hand
//! (`workflow_dispatch`) from a form of its inputs, read from its file,
//! then following the run it started.

use crate::route::{Route, runs};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::api::actions::{Input, InputKind, dispatch_inputs};
use crate::api::browse::{Blob, Refs, Workflow, WorkflowRun};
use crate::api::model::RepoId;
use crate::api::{ApiError, GitHub};
use crate::data::DataKey;
use crate::keymap::Action;
use crate::nav::Doable;
use crate::state::{Api, Cmd, State};
use crate::ui::overlays::PaletteItem;
use crate::ui::page::{DispatchLink, Link, Page};
use crate::ui::pages::{self, DispatchView};

/// The forms that run a workflow by hand, by repository and workflow file.
#[derive(Debug, Default)]
pub struct Dispatching {
    forms: HashMap<(RepoId, String), Form>,
}

/// What's chosen on a form.
#[derive(Debug, Default, Clone)]
struct Form {
    /// The branch; the default branch until one is chosen.
    git_ref: Option<String>,
    values: HashMap<String, String>,
    sending: bool,
    error: Option<String>,
}

/// Work done away from the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Work {
    Save {
        repo: RepoId,
        id: u64,
        name: String,
        path: PathBuf,
        replace: bool,
    },
    Dispatch {
        repo: RepoId,
        file: String,
        git_ref: String,
        inputs: Vec<(String, String)>,
    },
}

/// What came of it.
#[derive(Debug)]
pub enum Done {
    Saved(String, PathBuf, Result<u64, ApiError>),
    Dispatched {
        repo: RepoId,
        file: String,
        result: Result<Option<u64>, ApiError>,
    },
}

impl Work {
    pub async fn run(self, gh: &GitHub) -> Done {
        match self {
            Work::Save {
                repo,
                id,
                name,
                path,
                replace,
            } => {
                let result = gh.download_artifact(&repo, id, &path, replace).await;
                Done::Saved(name, path, result)
            }
            Work::Dispatch {
                repo,
                file,
                git_ref,
                inputs,
            } => {
                let result = gh.dispatch(&repo, &file, &git_ref, &inputs).await;
                Done::Dispatched { repo, file, result }
            }
        }
    }

    /// What it would have come to, had it run: the runtime's answer to a
    /// command it couldn't start.
    pub fn failed(&self) -> Done {
        let bug = || ApiError::Internal("ghtui hit a bug".into());
        match self {
            Work::Save { name, path, .. } => Done::Saved(name.clone(), path.clone(), Err(bug())),
            Work::Dispatch { repo, file, .. } => Done::Dispatched {
                repo: repo.clone(),
                file: file.clone(),
                result: Err(bug()),
            },
        }
    }
}

// ---- artifacts --------------------------------------------------------------

/// A picker of this module's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// Where to save an artifact: the field is the path.
    Save { repo: RepoId, id: u64, name: String },
    /// The branch a workflow runs on.
    Ref,
    /// A workflow input's value.
    Input(String),
}

impl Pick {
    pub fn title(&self) -> &'static str {
        match self {
            Pick::Save { .. } => "Save the artifact",
            Pick::Ref => "Use workflow from",
            Pick::Input(_) => "Set the input",
        }
    }

    pub fn placeholder(&self) -> &'static str {
        match self {
            Pick::Save { .. } => "A file or folder (↵ saves in the current folder)",
            Pick::Ref => "Find a branch",
            Pick::Input(_) => "Type a value",
        }
    }
}

/// What choosing one of this module's picker rows does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Chosen {
    Save {
        repo: RepoId,
        id: u64,
        name: String,
        path: PathBuf,
        replace: bool,
    },
    Ref(String),
    Input(String, String),
}

/// Where an artifact `name` is saved for what's typed: in the current
/// folder by default, in a folder named, or as the file named.
fn save_path(typed: &str, name: &str) -> PathBuf {
    let file = format!("{name}.zip");
    let typed = typed.trim();
    if typed.is_empty() {
        return Path::new(".").join(file);
    }
    let path = PathBuf::from(typed);
    if typed.ends_with('/') || path.is_dir() {
        path.join(file)
    } else {
        path
    }
}

/// The picker's rows for `pick`, with `q` typed.
pub fn rows(state: &State, pick: &Pick, q: &str) -> Vec<(PaletteItem, Option<Chosen>)> {
    let item = |label: String, hint: &str| PaletteItem {
        label,
        hint: hint.to_owned(),
    };
    let note = |text: String| vec![(item(text, ""), None)];
    match pick {
        Pick::Save { repo, id, name } => {
            let path = save_path(q, name);
            let shown = path.display().to_string();
            let chosen = |replace| {
                Some(Chosen::Save {
                    repo: repo.clone(),
                    id: *id,
                    name: name.clone(),
                    path: path.clone(),
                    replace,
                })
            };
            // Never over a file without asking: this row is the asking.
            if path.exists() {
                vec![(item(format!("Replace {shown}"), "it exists"), chosen(true))]
            } else {
                vec![(item(format!("Save to {shown}"), "a zip"), chosen(false))]
            }
        }
        Pick::Ref => {
            let Some((repo, _)) = here(state) else {
                return note("The form isn't on screen".into());
            };
            let key = DataKey::Refs(repo);
            let Some(refs) = state.picked::<Refs>(&key) else {
                let failed = state.remote(&key).and_then(|r| r.error.clone());
                return note(failed.map_or_else(
                    || "Loading branches…".to_owned(),
                    |e| format!("Couldn't load the branches: {e}"),
                ));
            };
            let chosen = form_ref(state);
            let mut rows: Vec<_> = (refs.branches.iter())
                .filter(|b| crate::picker::fuzzy_score(q, b).is_some())
                .map(|b| {
                    let hint = if chosen.as_deref() == Some(b.as_str()) {
                        "chosen"
                    } else {
                        ""
                    };
                    (item(b.clone(), hint), Some(Chosen::Ref(b.clone())))
                })
                .collect();
            if refs.branches.left_out() > 0 {
                rows.extend(note(format!("{} more on GitHub", refs.branches.left_out())));
            }
            rows
        }
        Pick::Input(name) => {
            let Some(input) =
                inputs_here(state).and_then(|i| i.into_iter().find(|i| i.name == *name))
            else {
                return note("The form isn't on screen".into());
            };
            let set = |value: &str| Some(Chosen::Input(name.clone(), value.to_owned()));
            match &input.kind {
                InputKind::Choice(options) => (options.iter())
                    .filter(|o| crate::picker::fuzzy_score(q, o).is_some())
                    .map(|o| (item(o.clone(), ""), set(o)))
                    .collect(),
                InputKind::Boolean => ["true", "false"]
                    .into_iter()
                    .filter(|o| crate::picker::fuzzy_score(q, o).is_some())
                    .map(|o| (item(o.to_owned(), ""), set(o)))
                    .collect(),
                InputKind::Text => {
                    let label = if q.is_empty() {
                        "Set it empty".to_owned()
                    } else {
                        format!("Set it to “{q}”")
                    };
                    vec![(item(label, &input.description), set(q))]
                }
            }
        }
    }
}

/// Choosing one of this module's picker rows.
#[must_use]
pub fn choose(state: &mut State, chosen: Chosen) -> Vec<Cmd> {
    match chosen {
        Chosen::Save {
            repo,
            id,
            name,
            path,
            replace,
        } => {
            state.info(format!("Saving {name} to {}…", path.display()));
            vec![Cmd::Api(Api::Actions(Work::Save {
                repo,
                id,
                name,
                path,
                replace,
            }))]
        }
        Chosen::Ref(name) => {
            let Some(key) = here(state) else {
                return Vec::new();
            };
            let form = state.dispatching.forms.entry(key).or_default();
            form.git_ref = Some(name);
            form.error = None;
            state.shown_gen += 1;
            // Its inputs are read from the file on that branch.
            blob_need(state).map_or_else(Vec::new, |need| state.ensure(need, false))
        }
        Chosen::Input(name, value) => {
            let Some(key) = here(state) else {
                return Vec::new();
            };
            let form = state.dispatching.forms.entry(key).or_default();
            form.values.insert(name, value);
            form.error = None;
            state.shown_gen += 1;
            Vec::new()
        }
    }
}

/// Enter on one of this module's rows: an artifact, or the form's.
#[must_use]
pub fn follow_link(state: &mut State, link: &Link) -> Vec<Cmd> {
    match link {
        Link::Artifact { id, name } => save(state, *id, name),
        Link::Dispatch(DispatchLink::Ref) => {
            let repo = here(state).map(|(r, _)| r);
            let mut cmds = state.open_picker(crate::picker::Kind::Actions(Pick::Ref));
            if let Some(repo) = repo {
                cmds.extend(state.ensure(DataKey::Refs(repo), false));
            }
            cmds
        }
        Link::Dispatch(DispatchLink::Input(name)) => {
            state.open_picker(crate::picker::Kind::Actions(Pick::Input(name.clone())))
        }
        Link::Dispatch(DispatchLink::Run) => run(state),
        _ => Vec::new(),
    }
}

/// What Enter does on one of this module's rows, for the menu.
pub fn describe(link: &Link) -> &'static str {
    match link {
        Link::Artifact { .. } => "Save the artifact",
        Link::Dispatch(DispatchLink::Ref) => "Choose the branch it runs on",
        Link::Dispatch(DispatchLink::Input(_)) => "Set the input",
        Link::Dispatch(DispatchLink::Run) => "Run the workflow",
        _ => "",
    }
}

/// Asks where to save artifact `id` of the run on screen.
#[must_use]
fn save(state: &mut State, id: u64, name: &str) -> Vec<Cmd> {
    let Some(Route::Runs(runs::WorkflowRun { repo, run, attempt })) = state.route().cloned() else {
        return Vec::new();
    };
    let key = DataKey::Run(repo.clone(), run, attempt);
    let expired = (state.picked::<WorkflowRun>(&key))
        .and_then(|r| r.artifacts.iter().find(|a| a.id == id))
        .is_some_and(|a| a.expired);
    if expired {
        state.info(format!("{name} has expired: GitHub no longer keeps it"));
        return Vec::new();
    }
    let pick = Pick::Save {
        repo,
        id,
        name: name.to_owned(),
    };
    state.open_picker(crate::picker::Kind::Actions(pick))
}

// ---- running a workflow by hand -----------------------------------------------

/// The workflow file a page is about: a workflow's, a run's.
fn workflow_here(state: &State) -> Option<(RepoId, String)> {
    let file = |path: &str| path.rsplit('/').next().unwrap_or(path).to_owned();
    match state.route()? {
        Route::Runs(runs::Workflow { repo, file }) | Route::Runs(runs::Dispatch { repo, file }) => {
            Some((repo.clone(), file.clone()))
        }
        Route::Runs(runs::WorkflowRun { repo, run, attempt }) => {
            let key = DataKey::Run(repo.clone(), *run, *attempt);
            let r = state.picked::<WorkflowRun>(&key)?;
            Some((repo.clone(), file(&r.path))).filter(|(_, f)| !f.is_empty())
        }
        _ => None,
    }
}

/// The form on screen: its repository and workflow file.
fn here(state: &State) -> Option<(RepoId, String)> {
    match state.route()? {
        Route::Runs(runs::Dispatch { repo, file }) => Some((repo.clone(), file.clone())),
        _ => None,
    }
}

/// The workflow's path in its repository.
fn workflow_path(file: &str) -> String {
    format!(".github/workflows/{file}")
}

/// The branch the form on screen runs on: chosen, or the default.
fn form_ref(state: &State) -> Option<String> {
    let (repo, file) = here(state)?;
    let chosen = state.dispatching.forms.get(&(repo.clone(), file));
    chosen
        .and_then(|f| f.git_ref.clone())
        .or_else(|| state.overview(&repo)?.default_branch.clone())
}

/// The workflow file on the form's branch.
fn blob_need(state: &State) -> Option<DataKey> {
    let (repo, file) = here(state)?;
    let chosen = state.dispatching.forms.get(&(repo.clone(), file.clone()));
    let git_ref = chosen
        .and_then(|f| f.git_ref.clone())
        .unwrap_or_else(|| "HEAD".into());
    Some(DataKey::Blob(repo, git_ref, workflow_path(&file)))
}

/// The form's inputs, once the file is in; `Err` says why there are none.
fn inputs_of(state: &State) -> Result<Vec<Input>, String> {
    let Some(key) = blob_need(state) else {
        return Err("Loading…".into());
    };
    let Some(blob) = state.picked::<Blob>(&key) else {
        let failed = state.remote(&key).and_then(|r| r.error.clone());
        return Err(failed.map_or_else(
            || "Reading the workflow file…".to_owned(),
            |e| format!("Couldn't read the workflow file: {e}"),
        ));
    };
    let text = blob.text.as_deref().unwrap_or_default();
    dispatch_inputs(text).ok_or_else(|| {
        "This workflow can't be run by hand: its file has no workflow_dispatch trigger".into()
    })
}

fn inputs_here(state: &State) -> Option<Vec<Input>> {
    inputs_of(state).ok()
}

/// What the form page needs: the workflow (its name) and its file.
pub fn needs(repo: &RepoId, file: &str) -> Vec<DataKey> {
    vec![
        DataKey::Repo(repo.clone()),
        DataKey::Workflow(repo.clone(), file.to_owned()),
        DataKey::Blob(repo.clone(), "HEAD".into(), workflow_path(file)),
    ]
}

/// The form page.
pub fn build(state: &State, page: &mut Page, repo: &RepoId, file: &str) {
    let path = workflow_path(file);
    let workflow = state.picked::<Workflow>(&DataKey::Workflow(repo.clone(), file.to_owned()));
    let name = workflow.map_or(file, |w| w.name.as_str());
    let blank = Form::default();
    let form = (state
        .dispatching
        .forms
        .get(&(repo.clone(), file.to_owned())))
    .unwrap_or(&blank);
    let git_ref = form_ref(state).unwrap_or_else(|| "the default branch".into());
    let inputs = inputs_of(state);
    let run_key = state.first_key(Action::RunWorkflow);
    let view = DispatchView {
        name,
        path: &path,
        git_ref: &git_ref,
        inputs: inputs.as_deref().map_err(String::clone),
        values: &form.values,
        sending: form.sending,
        error: form.error.as_deref(),
        run_key: &run_key,
    };
    pages::dispatch_form(page, &view);
}

/// Why the workflow on screen can't be run by hand, if it can't.
fn refused(state: &State, repo: &RepoId, file: &str) -> Option<String> {
    if state.overview(repo).is_some_and(|o| !o.can_write) {
        return Some(format!("Running a workflow takes write access to {repo}"));
    }
    let workflow = state.picked::<Workflow>(&DataKey::Workflow(repo.clone(), file.to_owned()));
    let disabled = workflow.and_then(|w| w.disabled.as_ref());
    disabled.map(|why| format!("The workflow is {why}: enable it on GitHub first"))
}

/// `,E`: from a workflow's page or a run's, the form; on the form, runs it.
#[must_use]
pub fn action(state: &mut State) -> Vec<Cmd> {
    if here(state).is_some() {
        return run(state);
    }
    let Some((repo, file)) = workflow_here(state) else {
        state.info("Running a workflow by hand works on a workflow's page or one of its runs");
        return Vec::new();
    };
    if let Some(why) = refused(state, &repo, &file) {
        state.info(why);
        return Vec::new();
    }
    state.push(Route::Runs(runs::Dispatch { repo, file }))
}

/// Runs the form's workflow, if it can.
#[must_use]
fn run(state: &mut State) -> Vec<Cmd> {
    let Some((repo, file)) = here(state) else {
        return Vec::new();
    };
    let why = refused(state, &repo, &file).or_else(|| {
        let form = state.dispatching.forms.get(&(repo.clone(), file.clone()));
        form.filter(|f| f.sending)
            .map(|_| "It's being started".to_owned())
    });
    let inputs = inputs_of(state);
    let (why, inputs) = match (why, inputs) {
        (Some(why), _) | (None, Err(why)) => (Some(why), Vec::new()),
        (None, Ok(inputs)) => (None, inputs),
    };
    let Some(git_ref) = form_ref(state) else {
        state.info("Still loading the repository's default branch");
        return Vec::new();
    };
    if let Some(why) = why {
        state.info(why);
        return Vec::new();
    }
    let form = state
        .dispatching
        .forms
        .entry((repo.clone(), file.clone()))
        .or_default();
    let values: Vec<(String, String)> = (inputs.iter())
        .map(|i| {
            let value = form
                .values
                .get(&i.name)
                .cloned()
                .unwrap_or_else(|| i.start());
            (i.name.clone(), value)
        })
        .collect();
    if let Some(missing) = inputs
        .iter()
        .zip(&values)
        .find(|(i, (_, v))| i.required && v.is_empty())
    {
        form.error = Some(format!("{} is required", missing.0.name));
        state.shown_gen += 1;
        return Vec::new();
    }
    form.sending = true;
    form.error = None;
    state.shown_gen += 1;
    vec![Cmd::Api(Api::Actions(Work::Dispatch {
        repo,
        file,
        git_ref,
        inputs: values,
    }))]
}

/// Work's answers.
#[must_use]
pub fn on_done(state: &mut State, done: Done) -> Vec<Cmd> {
    match done {
        Done::Saved(name, path, Ok(size)) => {
            let size = crate::ui::text::size(size);
            state.info(format!("Saved {name} ({size}) to {}", path.display()));
            Vec::new()
        }
        Done::Saved(name, path, Err(err)) => {
            state.error(format!("Couldn't save {name} to {}: {err}", path.display()));
            Vec::new()
        }
        Done::Dispatched { repo, file, result } => {
            let key = (repo.clone(), file.clone());
            let form = state.dispatching.forms.entry(key.clone()).or_default();
            form.sending = false;
            state.shown_gen += 1;
            let on_form = here(state) == Some(key.clone());
            match result {
                Err(err) => {
                    let text = format!("Couldn't run {file}: {err}");
                    match state.dispatching.forms.get_mut(&key) {
                        Some(form) if on_form => form.error = Some(text),
                        _ => state.error(text),
                    }
                    Vec::new()
                }
                // It's a new run: the form starts over next time.
                Ok(run) => {
                    state.dispatching.forms.remove(&key);
                    if !on_form {
                        state.info(format!("Started {file}"));
                        return Vec::new();
                    }
                    let route = match run {
                        Some(run) => {
                            state.info(format!("Started {file}: following its run"));
                            Route::Runs(runs::WorkflowRun {
                                repo,
                                run,
                                attempt: None,
                            })
                        }
                        // GitHub didn't say which: its runs show it soon.
                        None => {
                            state.info(format!("Started {file}: it shows in its runs in a moment"));
                            Route::Runs(runs::Workflow { repo, file })
                        }
                    };
                    state.replace(route, true, None)
                }
            }
        }
    }
}

/// What the menu offers here: running the workflow by hand.
pub fn doables(state: &State, out: &mut Vec<Doable>) {
    let Some((repo, file)) = workflow_here(state) else {
        return;
    };
    let label = if here(state).is_some() {
        "Run the workflow".to_owned()
    } else {
        format!("Run {file} by hand…")
    };
    out.push(Doable {
        section: "Change",
        action: Action::RunWorkflow,
        label,
        short: "",
        unavailable: refused(state, &repo, &file),
    });
}

// ---- annotations in Files changed -----------------------------------------------

/// A pull request's checks came in: its diff shows their annotations.
pub fn checks_loaded(state: &mut State, pr: &crate::api::model::PrRef) {
    let key = DataKey::PrChecks(pr.clone());
    let Some(checks) = state.picked::<crate::api::browse::Checks>(&key) else {
        return;
    };
    let list: Vec<(String, crate::api::actions::Annotation)> = (checks.items.iter())
        .flat_map(|c| c.annotations.iter().map(|a| (c.name.clone(), a.clone())))
        .filter(|(_, a)| a.path.is_some())
        .collect();
    let of = crate::diff_screen::DiffOf::Pr(pr.clone());
    if let Some(diff) = state.diffs.get_mut(&of)
        && diff.inputs().checks != list
    {
        diff.edit(|i| i.checks = list);
    }
}

/// Check annotations as the diff shows them: on their lines, as GitHub
/// shows them in Files changed, each a one-comment thread from its check.
pub fn in_diff(
    checks: &[(String, crate::api::actions::Annotation)],
) -> Vec<crate::ui::annotations::Annotation> {
    use crate::api::actions::Level;
    use crate::ui::annotations::{Annotation, AnnotationComment, AnnotationKey, Place};
    (checks.iter().enumerate())
        .filter_map(|(n, (check, a))| {
            let path = a.path.clone()?;
            let mark = match a.level {
                Level::Failure => "✗",
                Level::Warning => "!",
                Level::Notice => "i",
            };
            let place = match a.start_line {
                0 => Place::File { outdated: false },
                line => Place::Line {
                    at: crate::diff::anchor::LinePos {
                        side: crate::diff::anchor::Side::Right,
                        line: a.end_line.max(line),
                    },
                    start: (a.end_line > line).then_some(line),
                },
            };
            let body = if a.title.is_empty() {
                a.message.clone()
            } else {
                format!("**{}**\n\n{}", a.title, a.message)
            };
            Some(Annotation {
                key: AnnotationKey::Check(n),
                path,
                place,
                resolved: false,
                comments: vec![AnnotationComment {
                    author: format!("{mark} {check} · {}", a.level.word()),
                    body,
                    created_at: String::new(),
                    pending: false,
                    new: false,
                    editable: None,
                }],
                left_out: 0,
                error: None,
                can_reply: false,
                can_resolve: false,
                can_unresolve: false,
                new: false,
            })
        })
        .collect()
}
