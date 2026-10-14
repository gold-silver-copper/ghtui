//! Repository housekeeping, as GitHub's website does it: a repository's
//! labels (a page of their own, from its Issues tab), its milestones, its
//! branches and tags, forking it, making a new repository, and editing its
//! files (the website's pencil).
//!
//! Each form is a page ([`Route::Housekeeping`], one of [`FormOf`]): its
//! rows are set in pickers, its text in the composer (or `$EDITOR`), and
//! `,s` makes or saves it. A list's row is changed where it is: `,e` edits
//! the selected label or milestone, `,D` deletes it (asking first), `,x`
//! closes or reopens a milestone. What git can do in your clone (a branch
//! or a tag, a file's commit), it does there, after asking, with your own
//! git ([`git`]); elsewhere GitHub's API does it. Then the page follows
//! GitHub until it shows the change, as [`crate::writes`] does.

use crate::route::{Family, Route, Target, Url, code, issues};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use crate::api::browse::Results;
use crate::api::housekeeping::{LabelChange, LabelInfo, MilestoneChange, RefKind};
use crate::api::model::RepoId;
use crate::git::local::Clone;
use crate::ui::overlays::PaletteItem;
use crate::ui::page::{Link, Page, WriteLink};

use crate::browse::{Build, Width};
use crate::chrome::Section;
use crate::data::DataKey;
use crate::keymap::Action;
use crate::nav::Doable;
use crate::state::{Cmd, Screen, State};
use crate::writes::{Expected as Expects, row};

pub mod files;
pub mod git;
pub mod labels;
pub(crate) use labels::label_url;
pub mod milestones;
pub mod refs;
pub mod repos;

/// The actions housekeeping adds.
pub const ACTIONS: [Action; 3] = [Action::NewHere, Action::Fork, Action::NewRepo];

// ---- forms ------------------------------------------------------------------------------------

/// One of housekeeping's forms.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FormOf {
    /// A new label (without `name`), or the one named `name`.
    Label {
        repo: RepoId,
        name: Option<String>,
    },
    /// A new milestone (without `number`), or that one.
    Milestone {
        repo: RepoId,
        number: Option<u64>,
    },
    Fork(RepoId),
    NewRepo,
    /// A file at a revision: edited, deleted, or (with `change` New, `path`
    /// its folder) a new one.
    File {
        repo: RepoId,
        rev: String,
        path: String,
        change: FileChange,
    },
}

/// What a file's form does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileChange {
    Edit,
    New,
    Delete,
}

impl Family for FormOf {
    fn url(&self) -> String {
        use crate::ui::pages::url as links;
        match self {
            FormOf::Label { repo, .. } => format!("{}/labels", links::repo(repo)),
            FormOf::Milestone { repo, number } => match number {
                Some(n) => format!("{}/milestones/{n}/edit", links::repo(repo)),
                None => format!("{}/milestones/new", links::repo(repo)),
            },
            FormOf::Fork(repo) => format!("{}/fork", links::repo(repo)),
            FormOf::NewRepo => format!("{}/new", links::base()),
            FormOf::File {
                repo,
                rev,
                path,
                change,
            } => {
                let verb = match change {
                    FileChange::Edit => "edit",
                    FileChange::New => "new",
                    FileChange::Delete => "delete",
                };
                let url = format!("{}/{verb}/{}", links::repo(repo), links::encode_path(rev));
                if path.is_empty() {
                    url
                } else {
                    format!("{url}/{}", links::encode_path(path))
                }
            }
        }
    }

    fn title(&self) -> String {
        match self {
            FormOf::Label { repo, name: None } => format!("{repo} · New label"),
            FormOf::Label {
                repo,
                name: Some(name),
            } => format!("{repo} · Label {name}"),
            FormOf::Milestone { repo, number: None } => format!("{repo} · New milestone"),
            FormOf::Milestone {
                repo,
                number: Some(n),
            } => format!("{repo} · Editing milestone {n}"),
            FormOf::Fork(repo) => format!("{repo} · Fork"),
            FormOf::NewRepo => "New repository".to_owned(),
            FormOf::File {
                repo, path, change, ..
            } => match change {
                FileChange::Edit => format!("{}/{path} · Editing", repo.name),
                FileChange::Delete => format!("{}/{path} · Deleting", repo.name),
                FileChange::New if path.is_empty() => format!("{} · New file", repo.name),
                FileChange::New => format!("{}/{path} · New file", repo.name),
            },
        }
    }

    fn repo(&self) -> Option<&RepoId> {
        match self {
            FormOf::Label { repo, .. }
            | FormOf::Milestone { repo, .. }
            | FormOf::Fork(repo)
            | FormOf::File { repo, .. } => Some(repo),
            FormOf::NewRepo => None,
        }
    }

    /// What the form is of.
    fn up(&self, _: &State) -> Option<Route> {
        Some(match self {
            FormOf::Label { repo, .. } => Route::Issues(issues::Labels(repo.clone())),
            FormOf::Milestone { repo, .. } => Route::Issues(issues::Milestones {
                repo: repo.clone(),
                closed: false,
            }),
            FormOf::Fork(repo) => Route::Code(code::Repo(repo.clone())),
            FormOf::NewRepo => return None,
            FormOf::File {
                repo,
                rev,
                path,
                change: FileChange::New,
            } => Route::Code(code::Tree {
                repo: repo.clone(),
                rev: rev.clone(),
                path: path.clone(),
            }),
            FormOf::File {
                repo, rev, path, ..
            } => Route::blob(repo.clone(), rev.clone(), path.clone()),
        })
    }

    fn needs(&self) -> Vec<DataKey> {
        let header = self.repo().map(|repo| DataKey::Repo(repo.clone()));
        let own = match self {
            FormOf::Label { repo, .. } => vec![DataKey::Labels(repo.clone())],
            FormOf::Milestone {
                repo,
                number: Some(n),
            } => vec![DataKey::Milestone(repo.clone(), *n)],
            FormOf::Fork(_) => vec![DataKey::Owners],
            FormOf::NewRepo => vec![DataKey::Owners, DataKey::Templates],
            // Whether ghtui runs in a clone says how it's committed.
            FormOf::File {
                repo, rev, path, ..
            } => vec![
                DataKey::Clone(repo.clone()),
                DataKey::FileLook(repo.clone(), rev.clone(), path.clone()),
            ],
            FormOf::Milestone { number: None, .. } => Vec::new(),
        };
        header.into_iter().chain(own).collect()
    }

    fn width(&self) -> Width {
        Width::Reading
    }

    fn section(&self) -> Section {
        match self {
            FormOf::Label { .. } | FormOf::Milestone { .. } => Section::Issues,
            _ => Section::Code,
        }
    }

    fn build(&self, b: Build<'_>, page: &mut Page) {
        let route = Route::Housekeeping(self.clone());
        // What the form is filled from loads first, or says why not.
        if !loaded(b, page, self) {
            return;
        }
        match self {
            FormOf::Label { .. } => labels::build(b.state, page, &route),
            FormOf::Milestone { .. } => milestones::build(b.state, page, &route),
            FormOf::Fork(_) | FormOf::NewRepo => repos::build(b.state, page, &route, self),
            FormOf::File { .. } => files::build(b.state, page, &route, self),
        }
    }
}

impl FormOf {
    /// The form at a URL's path after `/<owner>/<repo>` (`fork`,
    /// `milestones/new`, `edit/main/README.md`), if it's one.
    fn parse(repo: &RepoId, rest: &[&str]) -> Option<FormOf> {
        let repo = repo.clone();
        Some(match rest {
            ["fork"] => FormOf::Fork(repo),
            ["milestones", "new"] => FormOf::Milestone { repo, number: None },
            ["milestones", n, "edit"] => FormOf::Milestone {
                repo,
                number: Some(n.parse().ok()?),
            },
            [verb @ ("edit" | "new" | "delete"), rev, path @ ..]
                if !rev.is_empty() && !rev.contains(char::is_control) =>
            {
                let change = match *verb {
                    "edit" => FileChange::Edit,
                    "delete" => FileChange::Delete,
                    _ => FileChange::New,
                };
                let path = path.join("/");
                if path.is_empty() && change != FileChange::New {
                    return None;
                }
                FormOf::File {
                    repo,
                    rev: (*rev).to_owned(),
                    path,
                    change,
                }
            }
            _ => return None,
        })
    }
}

/// A new repository's form, at the top of GitHub's paths.
pub(crate) fn parse(url: &Url<'_>) -> Option<Target> {
    (url.segments == ["new"]).then_some(Target::Page(Route::Housekeeping(FormOf::NewRepo)))
}

/// A repository's form.
pub(crate) fn parse_repo(repo: &RepoId, rest: &[&str], _: &Url<'_>) -> Option<Target> {
    FormOf::parse(repo, rest).map(|of| Target::Page(Route::Housekeeping(of)))
}

/// A row of a form, set in a picker (or, for yes-or-no, toggled).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Field {
    Name,
    Colour,
    Description,
    Title,
    Due,
    Owner,
    Private,
    Readme,
    Gitignore,
    License,
    AllBranches,
    Path,
    /// A file's commit's branch: empty for the one it's on, else a new one.
    Branch,
    Message,
}

impl Field {
    /// Its row's name.
    pub fn name(self) -> &'static str {
        match self {
            Field::Name => "Name",
            Field::Colour => "Colour",
            Field::Description => "Description",
            Field::Title => "Title",
            Field::Due => "Due date",
            Field::Owner => "Owner",
            Field::Private => "Visibility",
            Field::Readme => "README",
            Field::Gitignore => ".gitignore",
            Field::License => "Licence",
            Field::AllBranches => "Branches",
            Field::Path => "Path",
            Field::Branch => "Commit to",
            Field::Message => "Message",
        }
    }

    /// Set by Enter alone: yes or no.
    fn toggles(self) -> bool {
        matches!(self, Field::Private | Field::Readme | Field::AllBranches)
    }
}

/// What Enter on one of housekeeping's rows does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Row {
    Field(Field),
    /// The labels page's "New label".
    New,
}

/// A form as written so far.
#[derive(Debug, Clone, Default)]
pub struct Form {
    /// Each row's value; a yes-or-no row's is `yes` or empty.
    pub values: BTreeMap<Field, String>,
    /// The composer's text: a milestone's description, a file's text.
    pub text: String,
    /// Filled in from what GitHub has, once.
    pub started: bool,
    pub sending: bool,
    pub error: Option<String>,
}

impl Form {
    pub fn get(&self, field: Field) -> &str {
        self.values.get(&field).map_or("", String::as_str)
    }

    pub fn on(&self, field: Field) -> bool {
        !self.get(field).is_empty()
    }

    fn set(&mut self, field: Field, value: impl Into<String>) {
        self.values.insert(field, value.into());
    }

    fn toggle(&mut self, field: Field) {
        let on = self.on(field);
        self.set(field, if on { "" } else { "yes" });
    }
}

/// What housekeeping keeps between keys.
#[derive(Debug, Default)]
pub struct Housekeeping {
    /// The forms being written, by their page: leaving one keeps it.
    pub forms: HashMap<Route, Form>,
    /// What the question open is about.
    pub asked: Option<Subject>,
    /// The branch to ask about deleting once GitHub says whether its
    /// rules protect it.
    pub deleting: Option<(RepoId, String)>,
}

/// What a question, or a change of a list's row, is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subject {
    Label(RepoId, String),
    Milestone(RepoId, u64),
    /// A branch or tag; for a new one, where it starts.
    Ref {
        repo: RepoId,
        kind: RefKind,
        name: String,
        from: Option<String>,
    },
    /// A new repository, to clone into `dir`.
    Clone {
        url: String,
        dir: PathBuf,
    },
    /// A new branch a file was committed on, to propose into the branch
    /// it was made from (none: the default branch).
    Propose(RepoId, String, Option<String>),
}

fn blank() -> &'static Form {
    static BLANK: std::sync::LazyLock<Form> = std::sync::LazyLock::new(Form::default);
    &BLANK
}

/// The form on screen: its page and what it's of.
fn here(state: &State) -> Option<(Route, FormOf)> {
    match state.route()? {
        route @ Route::Housekeeping(of) => Some((route.clone(), of.clone())),
        _ => None,
    }
}

pub(crate) fn form<'a>(state: &'a State, route: &Route) -> &'a Form {
    (state.writing.housekeeping.forms.get(route)).unwrap_or_else(|| blank())
}

pub(crate) fn form_mut<'a>(state: &'a mut State, route: &Route) -> &'a mut Form {
    state
        .writing
        .housekeeping
        .forms
        .entry(route.clone())
        .or_default()
}

/// Why you can't change `repo`'s labels, milestones, branches, tags or
/// files, from what's known.
fn refused(state: &State, repo: &RepoId, what: &str) -> Option<String> {
    let o = state.overview(repo)?;
    if o.summary.archived {
        return Some(format!("{repo} is archived: it can't be changed"));
    }
    (!o.can_write).then(|| format!("{what} takes write access to {repo}"))
}

/// Opens a form's page, started again from what GitHub has.
#[must_use]
fn open(state: &mut State, of: FormOf) -> Vec<Cmd> {
    let file = match &of {
        FormOf::File {
            repo, rev, path, ..
        } => Some(DataKey::FileLook(repo.clone(), rev.clone(), path.clone())),
        _ => None,
    };
    let route = Route::Housekeeping(of);
    if state.route() == Some(&route) {
        return compose(state);
    }
    state.writing.housekeeping.forms.remove(&route);
    let mut cmds = file.map_or_else(Vec::new, |file| state.ensure(file, true));
    cmds.extend(state.push(route));
    cmds.extend(follow(state));
    cmds
}

/// Your clone of `repo`, once known: `Err` while it's being looked for.
fn clone_here(state: &mut State, repo: &RepoId) -> Result<Option<Clone>, Vec<Cmd>> {
    if let Some(clone) = state.clone_of(repo) {
        return Ok(clone.cloned());
    }
    state.info("Looking for your clone of the repository: a moment");
    Err(state.ensure(DataKey::Clone(repo.clone()), false))
}

// ---- the actions ----------------------------------------------------------------------

/// Whether `action` is housekeeping's here: its own actions anywhere, and
/// editing, deleting, closing and saving on its pages.
pub fn claims(state: &State, action: Action) -> bool {
    if ACTIONS.contains(&action) {
        return true;
    }
    let Some(route) = state.route() else {
        return false;
    };
    let lists = matches!(
        route,
        Route::Issues(issues::Labels(_))
            | Route::Issues(issues::Milestones { .. })
            | Route::Issues(issues::Milestone { .. })
    );
    match action {
        Action::EditComment => lists || matches!(route, Route::Code(code::Blob { .. })),
        Action::DeleteComment => {
            lists
                || matches!(
                    route,
                    Route::Code(code::Blob { .. })
                        | Route::Code(code::Branches(_))
                        | Route::Code(code::Tags(_))
                )
        }
        Action::Close => matches!(
            route,
            Route::Issues(issues::Milestones { .. }) | Route::Issues(issues::Milestone { .. })
        ),
        Action::Create => matches!(route, Route::Housekeeping(_)),
        _ => false,
    }
}

/// Runs one of the actions [`claims`] says are housekeeping's.
#[must_use]
pub fn act(state: &mut State, action: Action) -> Vec<Cmd> {
    let route = state.route().cloned();
    match (action, route) {
        (Action::NewHere, Some(route)) => new_here(state, route),
        (Action::Fork, _) => repos::open_fork(state),
        (Action::NewRepo, _) => open(state, FormOf::NewRepo),
        (Action::EditComment, Some(Route::Issues(issues::Labels(repo)))) => {
            labels::edit(state, &repo)
        }
        (Action::DeleteComment, Some(Route::Issues(issues::Labels(repo)))) => {
            labels::ask_delete(state, &repo)
        }
        (
            Action::EditComment,
            Some(
                Route::Issues(issues::Milestones { repo, .. })
                | Route::Issues(issues::Milestone { repo, .. }),
            ),
        ) => milestones::edit(state, &repo),
        (
            Action::DeleteComment,
            Some(
                Route::Issues(issues::Milestones { repo, .. })
                | Route::Issues(issues::Milestone { repo, .. }),
            ),
        ) => milestones::ask_delete(state, &repo),
        (
            Action::Close,
            Some(
                Route::Issues(issues::Milestones { repo, .. })
                | Route::Issues(issues::Milestone { repo, .. }),
            ),
        ) => milestones::toggle(state, &repo),
        (Action::DeleteComment, Some(Route::Code(code::Branches(repo)))) => {
            refs::ask_delete(state, &repo, RefKind::Branch)
        }
        (Action::DeleteComment, Some(Route::Code(code::Tags(repo)))) => {
            refs::ask_delete(state, &repo, RefKind::Tag)
        }
        (
            Action::EditComment | Action::DeleteComment,
            Some(Route::Code(code::Blob {
                repo, rev, path, ..
            })),
        ) => {
            let change = if action == Action::EditComment {
                FileChange::Edit
            } else {
                FileChange::Delete
            };
            files::open(state, repo, rev, path, change)
        }
        (Action::Create, Some(Route::Housekeeping(of))) => submit(state, &of),
        _ => {
            state.info(action.not_here());
            Vec::new()
        }
    }
}

/// `,o`: a new label, milestone, branch, tag or file, by the page.
#[must_use]
fn new_here(state: &mut State, route: Route) -> Vec<Cmd> {
    match route {
        Route::Issues(issues::Labels(repo)) => labels::open(state, &repo, None),
        Route::Issues(issues::Milestones { repo, .. })
        | Route::Issues(issues::Milestone { repo, .. }) => milestones::open(state, &repo, None),
        Route::Code(code::Branches(repo)) => refs::start(state, &repo, RefKind::Branch),
        Route::Code(code::Tags(repo)) => refs::start(state, &repo, RefKind::Tag),
        Route::Code(code::Tree { repo, rev, path }) => {
            files::open(state, repo, rev, path, FileChange::New)
        }
        Route::Code(code::Blob {
            repo, rev, path, ..
        }) => {
            let folder = path.rsplit_once('/').map_or("", |(dir, _)| dir).to_owned();
            files::open(state, repo, rev, folder, FileChange::New)
        }
        Route::Code(code::Repo(repo)) => {
            let Some(branch) = state.overview(&repo).and_then(|o| o.default_branch.clone()) else {
                state.info("The repository hasn't loaded yet");
                return Vec::new();
            };
            files::open(state, repo, branch, String::new(), FileChange::New)
        }
        _ => {
            state.info(
                "New works on a repository's labels, milestones, branches, tags and files (a folder for a new file)",
            );
            Vec::new()
        }
    }
}

/// `,s`, ctrl-s or a form's button.
#[must_use]
fn submit(state: &mut State, of: &FormOf) -> Vec<Cmd> {
    let route = Route::Housekeeping(of.clone());
    if form(state, &route).sending {
        crate::writes::say(state, "It's being sent".into());
        return Vec::new();
    }
    match of {
        FormOf::Label { repo, name } => labels::submit(state, &route, repo, name.as_deref()),
        FormOf::Milestone { repo, number } => milestones::submit(state, &route, repo, *number),
        FormOf::Fork(repo) => repos::fork(state, &route, repo),
        FormOf::NewRepo => repos::create(state, &route),
        FormOf::File { .. } => files::ask_commit(state, &route),
    }
}

/// The row selected on a list page, by its place among `items` (the
/// page's rows are its items, in order) and checked by its link.
fn selected<'a, T>(state: &State, items: &'a [T], url: impl Fn(&T) -> String) -> Option<&'a T> {
    let Screen::Page(p) = state.screen() else {
        return None;
    };
    let Some(Link::Url(link)) = p.selected_link() else {
        return None;
    };
    // Rows come in order; links in them (a milestone's pull request) don't
    // count.
    items.iter().find(|item| url(item) == *link)
}

// ---- the menu --------------------------------------------------------------------------

/// The menu's rows for the page on screen: what it edits, then what's
/// new here, then a fork and a new repository, wherever you are.
pub fn doables(state: &State, route: &Route, out: &mut Vec<Doable>) {
    if let Route::Housekeeping(of) = route {
        form_doables(state, route, of, out);
    }
}

/// The menu's rows for what changes GitHub here: the page's labels,
/// milestones, branches, tags or files edited, then what's new here, then
/// a fork and a new repository, wherever you are.
pub fn change_doables(state: &State) -> Vec<Doable> {
    let Some(route) = state.route() else {
        return Vec::new();
    };
    let mut mine = Vec::new();
    page_doables(state, route, &mut mine);
    if let Some(repo) = route.repo() {
        let why = state
            .overview(repo)
            .is_none()
            .then(|| "the repository hasn't loaded yet".to_owned());
        mine.push(row("New", Action::Fork, format!("Fork {repo}"), "", why));
    }
    mine.push(row("New", Action::NewRepo, "New repository", "", None));
    // One "New" section, after the page's own.
    mine.sort_by_key(|d| d.section == "New");
    mine
}

fn page_doables(state: &State, route: &Route, out: &mut Vec<Doable>) {
    let repo = route.repo();
    let can = |what: &str| repo.and_then(|r| refused(state, r, what));
    match route {
        Route::Issues(issues::Labels(_)) => {
            let why = can("Changing labels");
            out.push(row("New", Action::NewHere, "New label", "new", why.clone()));
            let picked = labels::picked(state).map(|l| l.name.clone());
            let none = || "no label is selected".to_owned();
            let row_why = why.or_else(|| picked.is_none().then(none));
            let name = picked.unwrap_or_else(|| "the label".into());
            out.push(row(
                "Edit",
                Action::EditComment,
                format!("Edit {name}"),
                "edit",
                row_why.clone(),
            ));
            out.push(row(
                "Edit",
                Action::DeleteComment,
                format!("Delete {name}"),
                "",
                row_why,
            ));
        }
        Route::Issues(issues::Milestones { .. }) | Route::Issues(issues::Milestone { .. }) => {
            milestones::doables(state, out);
        }
        Route::Code(code::Branches(_)) | Route::Code(code::Tags(_)) => {
            refs::doables(state, route, out);
        }
        Route::Code(code::Tree { .. })
        | Route::Code(code::Blob { .. })
        | Route::Code(code::Repo(_)) => {
            files::doables(state, route, out);
        }
        _ => {}
    }
}

/// A form's rows in the menu: write its text, and make or save it.
fn form_doables(state: &State, route: &Route, of: &FormOf, out: &mut Vec<Doable>) {
    let f = form(state, route);
    let busy = f.sending.then(|| "it's being sent".to_owned());
    if matches!(
        of,
        FormOf::Milestone { .. }
            | FormOf::File {
                change: FileChange::Edit | FileChange::New,
                ..
            }
    ) {
        out.push(row(
            "This page",
            Action::Comment,
            "Write the text",
            "write",
            busy.clone(),
        ));
    }
    let why = busy.or_else(|| problem(state, route, of));
    out.push(row("This page", Action::Create, button(of, f), "save", why));
}

/// What the form's button says.
fn button(of: &FormOf, f: &Form) -> String {
    match of {
        FormOf::Label { name: None, .. } => "Create label".into(),
        FormOf::Label { .. }
        | FormOf::Milestone {
            number: Some(_), ..
        } => "Save changes".into(),
        FormOf::Milestone { .. } => "Create milestone".into(),
        FormOf::Fork(_) => "Create fork".into(),
        FormOf::NewRepo => "Create repository".into(),
        FormOf::File { change, .. } => {
            let verb = if *change == FileChange::Delete {
                "Delete the file"
            } else {
                "Commit changes"
            };
            if f.get(Field::Branch).is_empty() {
                format!("{verb}…")
            } else {
                format!("{verb} and propose…")
            }
        }
    }
}

/// Why the form can't be sent yet, if it can't.
fn problem(state: &State, route: &Route, of: &FormOf) -> Option<String> {
    let f = form(state, route);
    match of {
        FormOf::Label { repo, .. } => {
            refused(state, repo, "Changing labels").or_else(|| labels::change(f).err())
        }
        FormOf::Milestone { repo, .. } => {
            refused(state, repo, "Changing milestones").or_else(|| milestones::change(f).err())
        }
        FormOf::Fork(repo) => repos::fork_problem(state, repo, f),
        FormOf::NewRepo => repos::repo_problem(f),
        FormOf::File { .. } => files::problem(state, route, of),
    }
}

// ---- following GitHub ----------------------------------------------------------------------

/// Brings housekeeping's pages up to date with what's arrived: a form
/// filled in from GitHub's data once, and a branch's deletion asked about
/// once GitHub says whether it's protected.
#[must_use]
pub fn follow(state: &mut State) -> Vec<Cmd> {
    let mut cmds = refs::follow(state);
    let Some(route @ Route::Housekeeping(_)) = state.route().cloned() else {
        return cmds;
    };
    let Route::Housekeeping(of) = &route else {
        return cmds;
    };
    cmds.extend(match of {
        FormOf::Label { repo, name } => labels::follow(state, &route, repo, name.as_ref()),
        FormOf::Milestone { repo, number } => milestones::follow(state, &route, repo, *number),
        FormOf::Fork(repo) => repos::follow_fork(state, &route, repo),
        FormOf::NewRepo => repos::follow_new(state, &route),
        FormOf::File { .. } => files::follow(state, &route, of),
    });
    cmds
}

/// What a page shows once GitHub has a change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expected {
    /// A label as it now is, and one (deleted, or its old name) gone.
    Label {
        repo: RepoId,
        gone: Option<String>,
        shown: Option<LabelChange>,
    },
    /// A milestone open or closed (`None`: deleted), with this title if
    /// one's given: on the list of open or closed ones (`list`), or on
    /// its own page.
    Milestone {
        repo: RepoId,
        number: u64,
        list: Option<bool>,
        closed: Option<bool>,
        title: Option<String>,
    },
    /// A branch or tag there, or gone.
    Ref(RepoId, RefKind, String, bool),
    /// A repository's page.
    Repo(RepoId),
    /// A file with this text, or (`None`) gone from its folder.
    File {
        repo: RepoId,
        rev: String,
        path: String,
        text: Option<String>,
    },
}

impl Expected {
    pub fn waiting(&self) -> &'static str {
        match self {
            Expected::Label { .. } => "Waiting for GitHub to show the label",
            Expected::Milestone { .. } => "Waiting for GitHub to show the milestone",
            Expected::Ref(_, RefKind::Branch, ..) => "Waiting for GitHub to show the branch",
            Expected::Ref(_, RefKind::Tag, ..) => "Waiting for GitHub to show the tag",
            Expected::Repo(_) => "Waiting for GitHub to make the repository",
            Expected::File { .. } => "Waiting for GitHub to show the file",
        }
    }

    pub fn shown(&self, state: &State) -> bool {
        use crate::api::browse::{
            Blob, BranchInfo, MilestoneDetail, MilestoneList, TagInfo, TreeEntry,
        };
        match self {
            Expected::Label { repo, gone, shown } => {
                let Some(list) = labels::list(state, repo) else {
                    return false;
                };
                let gone = gone
                    .as_ref()
                    .is_none_or(|name| list.items.iter().all(|l| l.name != *name));
                let shown = shown.as_ref().is_none_or(|c| {
                    list.items.iter().any(|l| {
                        l.name == c.name
                            && l.color.eq_ignore_ascii_case(&c.color)
                            && l.description == c.description
                    })
                });
                gone && shown
            }
            Expected::Milestone {
                repo,
                number,
                list,
                closed,
                title,
            } => {
                let titled = |t: &str| title.as_deref().is_none_or(|want| want == t);
                match list {
                    Some(list) => {
                        let key = DataKey::Milestones(repo.clone(), *list);
                        state.picked::<MilestoneList>(&key).is_some_and(|l| {
                            let found = l.results.items.iter().find(|m| m.number == *number);
                            match found {
                                Some(m) => *closed == Some(*list) && titled(&m.title),
                                None => *closed != Some(*list),
                            }
                        })
                    }
                    None => {
                        let one = DataKey::Milestone(repo.clone(), *number);
                        state.picked::<MilestoneDetail>(&one).is_some_and(|m| {
                            Some(m.info.closed) == *closed && titled(&m.info.title)
                        })
                    }
                }
            }
            Expected::Ref(repo, kind, name, there) => {
                // A list by name: one past its last row is on a later page.
                let (names, more) = match kind {
                    RefKind::Branch => {
                        let Some(l) =
                            state.picked::<Results<BranchInfo>>(&DataKey::Branches(repo.clone()))
                        else {
                            return false;
                        };
                        let names: Vec<&str> = l.items.iter().map(|b| b.name.as_str()).collect();
                        (names, l.next.is_some())
                    }
                    RefKind::Tag => {
                        let Some(l) =
                            state.picked::<Results<TagInfo>>(&DataKey::Tags(repo.clone()))
                        else {
                            return false;
                        };
                        let names: Vec<&str> = l.items.iter().map(|t| t.name.as_str()).collect();
                        (names, false)
                    }
                };
                let listed = names.contains(&name.as_str());
                let later = more
                    && *kind == RefKind::Branch
                    && names.last().is_some_and(|last| *last < name.as_str());
                if *there { listed || later } else { !listed }
            }
            Expected::Repo(repo) => state.overview(repo).is_some(),
            Expected::File {
                repo,
                rev,
                path,
                text: Some(text),
            } => state
                .picked::<Blob>(&DataKey::Blob(repo.clone(), rev.clone(), path.clone()))
                .is_some_and(|b| b.text.as_deref().map(str::trim_end) == Some(text.trim_end())),
            Expected::File {
                repo,
                rev,
                path,
                text: None,
            } => {
                let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
                let key = DataKey::Tree(repo.clone(), rev.clone(), dir.to_owned());
                state
                    .picked::<[TreeEntry]>(&key)
                    .is_some_and(|entries| entries.iter().all(|e| e.name != name))
            }
        }
    }
}

/// Waits, on `route`, for GitHub to show `what`.
fn expect(state: &mut State, route: Route, what: Expected) {
    crate::writes::expect(state, route, Expects::Housekeeping(what));
}

// ---- data -----------------------------------------------------------------------------------

/// Builds the labels page.
pub fn build(b: Build<'_>, page: &mut Page, repo: &RepoId) {
    let list = b.get::<Results<LabelInfo>>(&DataKey::Labels(repo.clone()));
    let keys = crate::ui::pages::LabelKeys {
        new: b.state.first_key(Action::NewHere),
        edit: b.state.first_key(Action::EditComment),
        delete: b.state.first_key(Action::DeleteComment),
    };
    crate::ui::pages::labels_page(page, repo, list, &keys, b.cx.now);
}

/// Whether a form's data (but its repository's header) is in; else the
/// page says it's loading, or why it couldn't.
fn loaded(b: Build<'_>, page: &mut Page, of: &FormOf) -> bool {
    for key in of.needs() {
        let what = match &key {
            DataKey::Labels(_) => "the labels",
            DataKey::Owners => "your account",
            DataKey::Templates => "GitHub's templates",
            DataKey::Milestone(..) => "the milestone",
            _ => continue,
        };
        let fetched = crate::state::Remote::fetched(b.state.remote(&key), b.retry);
        if fetched.show(page, what).is_none() {
            return false;
        }
    }
    true
}

// ---- the composer --------------------------------------------------------------------------

/// `c`, or Enter on a form's text: the composer on it.
#[must_use]
pub fn compose(state: &mut State) -> Vec<Cmd> {
    let Some((route, of)) = here(state) else {
        return Vec::new();
    };
    let f = form(state, &route);
    if f.sending {
        state.info("It's being sent");
        return Vec::new();
    }
    match of {
        FormOf::Milestone { .. } => {}
        FormOf::File { change, .. } if change != FileChange::Delete => {
            if !f.started {
                state.info("The file hasn't loaded yet");
                return Vec::new();
            }
        }
        _ => {
            // A form of rows: Enter sets the one selected.
            return Vec::new();
        }
    }
    let text = f.text.clone();
    state.shown_gen += 1;
    let target = crate::review::ComposeTarget::Write(crate::writes::Text::Housekeeping(route));
    let cmds = state.compose(target, &text);
    if let Some(crate::state::Overlay::Compose(c)) = &mut state.overlay {
        c.input
            .move_cursor(ratatui_textarea::CursorMove::Jump(0, 0));
    }
    cmds
}

pub fn keep_text(state: &mut State, route: &Route, text: String) {
    let f = form_mut(state, route);
    f.text = text;
    f.error = None;
}

/// The composer's title.
pub fn compose_title(route: &Route) -> String {
    match route {
        Route::Housekeeping(FormOf::Milestone { .. }) => "The milestone's description".into(),
        Route::Housekeeping(FormOf::File { path, change, .. }) => match change {
            FileChange::New => "The new file's text".into(),
            _ => format!("Editing {path}"),
        },
        _ => String::new(),
    }
}

/// What ctrl-s does, for the composer's hint.
pub fn compose_save(route: &Route) -> &'static str {
    match route {
        Route::Housekeeping(FormOf::File { .. }) => "commit…",
        Route::Housekeeping(FormOf::Milestone { number: None, .. }) => "create",
        _ => "save",
    }
}

// ---- pickers ------------------------------------------------------------------------

/// What one of housekeeping's pickers lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// A form's row.
    Field(Route, Field),
    /// A new branch's or tag's name.
    RefName(RepoId, RefKind),
    /// Where a new branch or tag starts.
    RefFrom(RepoId, RefKind, String),
}

/// What choosing a row does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Chosen {
    Field(Route, Field, String),
    RefName(RepoId, RefKind, String),
    /// A new branch or tag, and the branch, tag or commit it starts at.
    RefFrom(RepoId, RefKind, String, String),
}

impl Pick {
    pub fn title(&self) -> &'static str {
        match self {
            Pick::Field(_, field) => field.name(),
            Pick::RefName(_, RefKind::Branch) => "New branch: its name",
            Pick::RefName(_, RefKind::Tag) => "New tag: its name",
            Pick::RefFrom(..) => "Where it starts",
        }
    }

    pub fn placeholder(&self) -> &'static str {
        match self {
            Pick::Field(_, Field::Colour) => "Type a colour (#rrggbb), or choose one",
            Pick::Field(_, Field::Due) => "Type the date: YYYY-MM-DD",
            Pick::Field(_, Field::Owner | Field::Gitignore | Field::License) => "Type to filter",
            Pick::Field(_, Field::Branch) => "Type a new branch's name, or choose",
            Pick::Field(..) => "Type it",
            Pick::RefName(..) => "Type the name",
            Pick::RefFrom(..) => "Find a branch or tag, or type a commit's SHA",
        }
    }

    pub fn need(&self) -> Option<DataKey> {
        match self {
            Pick::Field(_, Field::Owner) => Some(DataKey::Owners),
            Pick::Field(_, Field::Gitignore | Field::License) => Some(DataKey::Templates),
            Pick::RefFrom(repo, ..) => Some(DataKey::Branches(repo.clone())),
            Pick::Field(..) | Pick::RefName(..) => None,
        }
    }

    /// The text the field starts with.
    pub fn start(&self, state: &State) -> Option<String> {
        match self {
            Pick::Field(route, field) if !field_lists(*field) => {
                Some(form(state, route).get(*field).to_owned()).filter(|v| !v.is_empty())
            }
            _ => None,
        }
    }
}

/// A row chosen from a list, not typed.
fn field_lists(field: Field) -> bool {
    matches!(
        field,
        Field::Owner | Field::Gitignore | Field::License | Field::Branch
    )
}

/// A picker's row.
fn item(
    label: impl Into<String>,
    hint: impl Into<String>,
    chosen: Option<Chosen>,
) -> (PaletteItem, Option<crate::writes::Chosen>) {
    crate::writes::item(label, hint, chosen.map(crate::writes::Chosen::Housekeeping))
}

/// A picker's rows: what each says, and what choosing it does.
pub fn rows(
    state: &State,
    pick: &Pick,
    q: &str,
) -> Vec<(PaletteItem, Option<crate::writes::Chosen>)> {
    match pick {
        Pick::Field(route, field) => field_rows(state, route, *field, q),
        Pick::RefName(repo, kind) => refs::name_rows(repo, *kind, q),
        Pick::RefFrom(repo, kind, name) => refs::from_rows(state, repo, *kind, name, q),
    }
}

fn field_rows(
    state: &State,
    route: &Route,
    field: Field,
    q: &str,
) -> Vec<(PaletteItem, Option<crate::writes::Chosen>)> {
    let set = |value: &str| Some(Chosen::Field(route.clone(), field, value.to_owned()));
    let q = q.trim();
    match field {
        Field::Colour => labels::colour_rows(route, q),
        Field::Owner => repos::owner_rows(state, route, q),
        Field::Gitignore | Field::License => repos::template_rows(state, route, field, q),
        Field::Branch => files::branch_rows(state, route, q),
        Field::Due => milestones::due_rows(route, q),
        Field::Name | Field::Title | Field::Path | Field::Message if q.is_empty() => {
            vec![item(
                format!("Type the {}", field.name().to_lowercase()),
                "",
                None,
            )]
        }
        _ => match check(route, field, q) {
            Some(why) => vec![item(why, "", None)],
            None if q.is_empty() => vec![item(
                format!("No {}", field.name().to_lowercase()),
                "↵",
                set(""),
            )],
            None => vec![item(format!("{}: {q}", field.name()), "↵", set(q))],
        },
    }
}

/// Why `value` can't be `field`'s, if it can't.
fn check(route: &Route, field: Field, value: &str) -> Option<String> {
    match (route, field) {
        (Route::Housekeeping(FormOf::Label { .. }), Field::Name) if value.chars().count() > 50 => {
            Some("A label's name is at most 50 characters".into())
        }
        (Route::Housekeeping(FormOf::Label { .. }), Field::Description)
            if value.chars().count() > 100 =>
        {
            Some("A label's description is at most 100 characters".into())
        }
        (Route::Housekeeping(FormOf::Fork(_) | FormOf::NewRepo), Field::Name) => {
            repos::name_problem(value)
        }
        (Route::Housekeeping(FormOf::File { .. }), Field::Path) => files::path_problem(value),
        _ => None,
    }
}

/// Opens a picker on a form's row (a yes-or-no row toggles instead).
#[must_use]
pub fn open_field(state: &mut State, route: &Route, field: Field) -> Vec<Cmd> {
    if field.toggles() {
        form_mut(state, route).toggle(field);
        state.shown_gen += 1;
        return Vec::new();
    }
    crate::writes::open_picker(
        state,
        crate::writes::Pick::Housekeeping(Pick::Field(route.clone(), field)),
    )
}

#[must_use]
pub fn choose(state: &mut State, chosen: Chosen) -> Vec<Cmd> {
    match chosen {
        Chosen::Field(route, field, value) => {
            let f = form_mut(state, &route);
            f.set(field, value);
            f.error = None;
            let blank = f.text.is_empty();
            state.shown_gen += 1;
            // A new file, once named, is written, as on GitHub.
            let new_file = matches!(
                &route,
                Route::Housekeeping(FormOf::File {
                    change: FileChange::New,
                    ..
                })
            );
            if field == Field::Path && new_file && blank && state.route() == Some(&route) {
                return compose(state);
            }
            Vec::new()
        }
        Chosen::RefName(repo, kind, name) => refs::named(state, repo, kind, name),
        Chosen::RefFrom(repo, kind, name, from) => refs::from_chosen(state, repo, kind, name, from),
    }
}

/// Enter on one of housekeeping's rows.
#[must_use]
pub fn follow_link(state: &mut State, row: Row) -> Vec<Cmd> {
    match (row, state.route().cloned()) {
        (Row::New, _) => act(state, Action::NewHere),
        (Row::Field(field), Some(route @ Route::Housekeeping(_))) => {
            open_field(state, &route, field)
        }
        _ => Vec::new(),
    }
}

/// What Enter on one of housekeeping's rows does, for the menu.
pub fn describe(row: Row) -> &'static str {
    match row {
        Row::New => "New label",
        Row::Field(Field::Private) => "Make it public or private",
        Row::Field(Field::Readme) => "Add a README, or not",
        Row::Field(Field::AllBranches) => "Copy all branches, or the default one",
        Row::Field(Field::Owner) => "Choose the owner",
        Row::Field(Field::Colour) => "Choose the colour",
        Row::Field(Field::Due) => "Set the due date",
        Row::Field(Field::Gitignore) => "Choose a .gitignore template",
        Row::Field(Field::License) => "Choose a licence",
        Row::Field(Field::Path) => "Name the file, or move it",
        Row::Field(Field::Branch) => "Choose the branch to commit to",
        Row::Field(Field::Message) => "Write the commit message",
        Row::Field(Field::Name | Field::Title) => "Name it",
        Row::Field(Field::Description) => "Describe it",
    }
}

// ---- questions ----------------------------------------------------------------------

/// The answers to housekeeping's questions; what they're about is
/// [`Housekeeping::asked`] (or the form on screen).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Delete,
    /// Make a new branch or tag, with git in your clone.
    Push,
    /// Commit a file's form.
    Commit,
    /// Clone a new repository.
    Clone,
    /// Propose a new branch's commit as a pull request.
    Propose,
}

impl Answer {
    pub fn busy(self) -> &'static str {
        match self {
            Answer::Delete => "Deleting…",
            Answer::Push => "Pushing…",
            Answer::Commit => "Committing…",
            Answer::Clone => "Cloning…",
            Answer::Propose => "Opening a pull request…",
        }
    }
}

/// Asks `title`, about `subject`.
fn ask(
    state: &mut State,
    subject: Option<Subject>,
    title: String,
    facts: Vec<(String, bool)>,
    choices: Vec<(String, Answer)>,
) {
    let Some(route) = state.route().cloned() else {
        return;
    };
    state.writing.housekeeping.asked = subject;
    let choices = (choices.into_iter())
        .map(|(label, a)| (label, crate::writes::Answer::Housekeeping(a)))
        .collect();
    crate::writes::ask(state, route, title, facts, choices);
}

/// The question about `route` was answered.
#[must_use]
pub fn answer(state: &mut State, route: &Route, answer: Answer) -> Vec<Cmd> {
    let subject = state.writing.housekeeping.asked.clone();
    match (answer, subject) {
        (Answer::Commit, _) => files::commit(state, route),
        (Answer::Delete, Some(Subject::Label(repo, name))) => labels::delete(state, repo, name),
        (Answer::Delete, Some(Subject::Milestone(repo, number))) => {
            milestones::delete(state, repo, number)
        }
        (
            Answer::Delete | Answer::Push,
            Some(Subject::Ref {
                repo,
                kind,
                name,
                from,
            }),
        ) => refs::send(state, repo, kind, name, from),
        (Answer::Clone, Some(Subject::Clone { url, dir })) => {
            crate::writes::sending(state);
            crate::work::start(repos::CloneTo { url, dir })
        }
        (Answer::Propose, Some(Subject::Propose(repo, branch, base))) => {
            state.overlay = None;
            files::propose(state, repo, branch, base)
        }
        _ => {
            state.overlay = None;
            Vec::new()
        }
    }
}

// ---- writes ------------------------------------------------------------

/// A milestone's change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MilestoneWrite {
    Save(MilestoneChange),
    Close(bool),
    Delete,
}

/// A branch's or tag's change: made at a commit, or deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefWrite {
    /// At the branch, tag or commit named.
    Create(String),
    Delete,
}

/// A file's commit, by GitHub's API or (with `clone`) with git in your
/// clone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileWrite {
    pub repo: RepoId,
    /// The branch the form was opened on, if it was one: a new branch's
    /// pull request goes into it.
    pub base: Option<String>,
    pub commit: git::Commit,
    /// The tip GitHub's API makes it on (a new branch's start).
    pub head: String,
    pub clone: Option<Clone>,
}

/// Says `why` where it'll be seen, and marks the form unsent.
fn failed(state: &mut State, route: &Route, why: &impl std::fmt::Display) {
    let why = why.to_string();
    if let Some(f) = state.writing.housekeeping.forms.get_mut(route) {
        f.sending = false;
        f.error = Some(why.clone());
    }
    crate::writes::say(state, why);
}

/// The form's rows for the page, each set in a picker.
fn form_row(f: &Form, field: Field, unset: &str) -> crate::ui::pages::FormRow {
    let colour = (field == Field::Colour && !f.get(field).is_empty())
        .then(|| format!("#{}", f.get(field).trim_start_matches('#')));
    let value = colour.as_deref().unwrap_or(f.get(field));
    let (value, unset) = if field.toggles() {
        (toggle_text(field, f.on(field)).to_owned(), false)
    } else if value.is_empty() {
        (unset.to_owned(), true)
    } else {
        (value.to_owned(), false)
    };
    crate::ui::pages::FormRow {
        name: field.name(),
        value,
        unset,
        link: Some(WriteLink::Housekeeping(Row::Field(field))),
    }
}

/// What a yes-or-no row says.
fn toggle_text(field: Field, on: bool) -> &'static str {
    match (field, on) {
        (Field::Private, true) => "Private: you choose who can see it",
        (Field::Private, false) => "Public: anyone can see it",
        (Field::Readme, true) => "Add a README file",
        (Field::Readme, false) => "No README",
        (Field::AllBranches, true) => "Copy all branches",
        (Field::AllBranches, false) => "Copy the default branch only",
        _ => "",
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    use crate::api::housekeeping::{Owners, Templates};

    pub fn labels() -> Results<LabelInfo> {
        let label = |name: &str, color: &str, description: &str, open| LabelInfo {
            name: name.into(),
            color: color.into(),
            description: description.into(),
            open,
        };
        Results {
            total: 3,
            items: vec![
                label("bug", "d73a4a", "Something isn't working", 4),
                label("documentation", "0075ca", "", 0),
                label("good first issue", "7057ff", "Good for newcomers", 12),
            ],
            next: None,
        }
    }

    pub fn owners() -> Owners {
        Owners {
            viewer: "octocat".into(),
            orgs: vec![("ghtui-org".into(), true), ("readers".into(), false)],
            more: 0,
        }
    }

    pub fn templates() -> Templates {
        Templates {
            gitignores: vec!["Go".into(), "Rust".into()],
            licenses: vec![
                ("mit".into(), "MIT License".into()),
                ("apache-2.0".into(), "Apache License 2.0".into()),
            ],
        }
    }
}
