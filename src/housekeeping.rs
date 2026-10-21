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
//! GitHub until it shows the change ([`crate::follow`]).

use crate::route::{Family, Route, Target, Url, code, issues};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::api::browse::Results;
use crate::api::housekeeping::{LabelChange, LabelInfo, MilestoneChange, RefKind};
use crate::api::model::RepoId;
use crate::git::local::Clone;
use crate::picker::entry as item;
pub(crate) use crate::ui::form::Field;
use crate::ui::form::Row;
use crate::ui::page::{Link, Page};

use crate::browse::{Build, Width};
use crate::chrome::Section;
use crate::data::DataKey;
use crate::follow::{Expectation, Shown, expect};
use crate::keymap::Action;
use crate::nav::{Doable, row};
use crate::state::{Cmds, Screen, State};

pub mod files;
pub mod git;
pub mod labels;
pub mod milestones;
pub mod refs;
pub mod repos;

/// What housekeeping adds: a fork, a new repository and what's new on a
/// page, and editing, deleting and closing on its pages; and a branch's
/// deletion asked about once GitHub says whether it's protected.
pub const FEATURE: crate::features::Feature = crate::features::Feature {
    owns: &[Action::NewHere, Action::Fork, Action::NewRepo],
    takes,
    act,
    offers: |state, out| out.extend(change_doables(state)),
    follow: refs::follow,
};

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

    fn form(&self) -> Option<&'static crate::forms::Kind> {
        Some(&FORM)
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
            FormOf::Fork(repo) => Route::code(repo.clone()),
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

/// A form as written so far.
#[derive(Debug, Clone, Default)]
pub struct Form {
    /// Each row's value; a yes-or-no row's is `yes` or empty.
    pub values: BTreeMap<Field, String>,
    /// The composer's text: a milestone's description, a file's text.
    pub text: String,
    /// Filled in from what GitHub has, once.
    pub started: bool,
}

impl crate::forms::Fields for Form {
    fn text(&mut self, _: usize) -> Option<&mut String> {
        Some(&mut self.text)
    }

    fn title(&self, route: &Route, _: usize) -> String {
        match route {
            Route::Housekeeping(FormOf::Milestone { .. }) => "The milestone's description".into(),
            Route::Housekeeping(FormOf::File { path, change, .. }) => match change {
                FileChange::New => "The new file's text".into(),
                _ => format!("Editing {path}"),
            },
            _ => String::new(),
        }
    }

    fn save(&self, route: &Route) -> &'static str {
        match route {
            Route::Housekeeping(FormOf::File { .. }) => "commit…",
            Route::Housekeeping(FormOf::Milestone { number: None, .. }) => "create",
            _ => "save",
        }
    }
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
    /// The branch to ask about deleting once GitHub says whether its
    /// rules protect it.
    pub deleting: Option<(RepoId, String)>,
}

/// What a deletion, or a new branch or tag, is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subject {
    Label(RepoId, String),
    Milestone(RepoId, u64),
    /// A branch or tag.
    Ref(RepoId, RefKind, String),
}

/// The form on screen: its page and what it's of.
fn here(state: &State) -> Option<(Route, FormOf)> {
    match state.route()? {
        route @ Route::Housekeeping(of) => Some((route.clone(), of.clone())),
        _ => None,
    }
}

pub(crate) fn form<'a>(state: &'a State, route: &Route) -> Cow<'a, Form> {
    state.forms.shown(route)
}

pub(crate) fn form_mut<'a>(state: &'a mut State, route: &Route) -> &'a mut Form {
    state.forms.edit(route)
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
fn open(state: &mut State, of: FormOf) -> Cmds {
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
    state.forms.take::<Form>(&route);
    let mut cmds = file.map_or_else(Cmds::new, |file| state.ensure(file, true));
    cmds.extend(state.push(route));
    cmds.extend(follow(state));
    cmds
}

/// Your clone of `repo`, once known: `Err` while it's being looked for.
fn clone_here(state: &mut State, repo: &RepoId) -> Result<Option<Clone>, Cmds> {
    if let Some(clone) = state.clone_of(repo) {
        return Ok(clone.cloned());
    }
    state.info("Looking for your clone of the repository: a moment");
    Err(state.ensure(DataKey::Clone(repo.clone()), false))
}

// ---- the actions ----------------------------------------------------------------------

/// Whether `action` is housekeeping's here: its own actions anywhere, and
/// editing, deleting, closing and saving on its pages.
fn takes(state: &State, action: Action) -> bool {
    let Some(route) = state.route() else {
        return false;
    };
    let milestones = matches!(
        route,
        Route::Issues(issues::Milestones { .. } | issues::Milestone { .. })
    );
    let lists = milestones || matches!(route, Route::Issues(issues::Labels(_)));
    let blob = matches!(route, Route::Code(code::Blob { .. }));
    match action {
        Action::EditComment => lists || blob,
        Action::DeleteComment => {
            lists || blob || matches!(route, Route::Code(code::Branches(_) | code::Tags(_)))
        }
        Action::Close => milestones,
        _ => false,
    }
}

/// Runs one of housekeeping's actions.
fn act(state: &mut State, action: Action) -> Cmds {
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
            Action::EditComment | Action::DeleteComment | Action::Close,
            Some(Route::Issues(issues::Milestones { repo, .. } | issues::Milestone { repo, .. })),
        ) => match action {
            Action::EditComment => milestones::edit(state, &repo),
            Action::DeleteComment => milestones::ask_delete(state, &repo),
            _ => milestones::toggle(state, &repo),
        },
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
        _ => state.just_say(action.not_here()),
    }
}

/// `,o`: a new label, milestone, branch, tag or file, by the page.
fn new_here(state: &mut State, route: Route) -> Cmds {
    match route {
        Route::Issues(issues::Labels(repo)) => labels::open(state, &repo, None),
        Route::Issues(issues::Milestones { repo, .. } | issues::Milestone { repo, .. }) => {
            milestones::open(state, &repo, None)
        }
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
                return state.just_say("The repository hasn't loaded yet");
            };
            files::open(state, repo, branch, String::new(), FileChange::New)
        }
        _ => {
            state.just_say(
                "New works on a repository's labels, milestones, branches, tags and files (a folder for a new file)",
            )
        }
    }
}

/// What housekeeping's forms do.
pub const FORM: crate::forms::Kind = crate::forms::Kind {
    compose: |state, _| compose(state),
    row: |state, row| match (row, state.route().cloned()) {
        (Row::Field(field), Some(route)) => open_field(state, &route, *field),
        _ => Cmds::new(),
    },
    fill,
    submit,
    follow,
    offers: |state, out| {
        if let Some((route, of)) = here(state) {
            form_doables(state, &route, &of, out);
        }
    },
};

/// `,s`, ctrl-s or a form's button.
fn submit(state: &mut State) -> Cmds {
    let Some((route, of)) = here(state) else {
        return Cmds::new();
    };
    match &of {
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
    // count. The row at its place is the one, unless the page has others.
    let at = p.selected.and_then(|i| items.get(i));
    at.filter(|item| url(item) == *link)
        .or_else(|| items.iter().find(|item| url(item) == *link))
}

// ---- the menu --------------------------------------------------------------------------

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
        Route::Issues(issues::Milestones { .. } | issues::Milestone { .. }) => {
            milestones::doables(state, out);
        }
        Route::Code(code::Branches(_) | code::Tags(_)) => refs::doables(state, route, out),
        Route::Code(code::Tree { .. } | code::Blob { .. } | code::Repo(_)) => {
            files::doables(state, route, out);
        }
        _ => {}
    }
}

/// A form's rows in the menu: write its text, and make or save it.
fn form_doables(state: &State, route: &Route, of: &FormOf, out: &mut Vec<Doable>) {
    let written = matches!(
        of,
        FormOf::Milestone { .. }
            | FormOf::File {
                change: FileChange::Edit | FileChange::New,
                ..
            }
    );
    let write = written.then_some("Write the text");
    let button = button(of, &form(state, route));
    crate::forms::offer(
        state,
        write,
        (&button, "save"),
        problem(state, route, of),
        out,
    );
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
            refused(state, repo, "Changing labels").or_else(|| labels::change(&f).err())
        }
        FormOf::Milestone { repo, .. } => {
            refused(state, repo, "Changing milestones").or_else(|| milestones::change(&f).err())
        }
        FormOf::Fork(repo) => repos::fork_problem(state, repo, &f),
        FormOf::NewRepo => repos::repo_problem(&f),
        FormOf::File { .. } => files::problem(state, route, of),
    }
}

// ---- following GitHub ----------------------------------------------------------------------

/// Brings housekeeping's form on screen up to date with what's arrived:
/// filled in from GitHub's data once.
fn follow(state: &mut State) -> Cmds {
    let Some((route, of)) = here(state) else {
        return Cmds::new();
    };
    match &of {
        FormOf::Label { repo, name } => labels::follow(state, &route, repo, name.as_ref()),
        FormOf::Milestone { repo, number } => milestones::follow(state, &route, repo, *number),
        FormOf::Fork(repo) => repos::follow(state, &route, Some(repo)),
        FormOf::NewRepo => repos::follow(state, &route, None),
        FormOf::File { .. } => files::follow(state, &route, &of),
    }
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

impl Expectation for Expected {
    fn waiting(&self) -> &'static str {
        match self {
            Expected::Label { .. } => "Waiting for GitHub to show the label",
            Expected::Milestone { .. } => "Waiting for GitHub to show the milestone",
            Expected::Ref(_, RefKind::Branch, ..) => "Waiting for GitHub to show the branch",
            Expected::Ref(_, RefKind::Tag, ..) => "Waiting for GitHub to show the tag",
            Expected::Repo(_) => "Waiting for GitHub to make the repository",
            Expected::File { .. } => "Waiting for GitHub to show the file",
        }
    }

    fn shown(&self, state: &State, _: &Route) -> Shown {
        use crate::api::browse::{
            Blob, BranchInfo, MilestoneDetail, MilestoneList, TagInfo, TreeEntry,
        };
        let shown = match self {
            Expected::Label { repo, gone, shown } => {
                let Some(list) = labels::list(state, repo) else {
                    return Shown::No;
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
                let found = match kind {
                    RefKind::Branch => (state
                        .picked::<Results<BranchInfo>>(&DataKey::Branches(repo.clone())))
                    .map(|l| {
                        let later = l.next.is_some()
                            && l.items.last().is_some_and(|last| last.name < *name);
                        (l.items.iter().any(|b| b.name == *name), later)
                    }),
                    RefKind::Tag => (state
                        .picked::<Results<TagInfo>>(&DataKey::Tags(repo.clone())))
                    .map(|l| (l.items.iter().any(|t| t.name == *name), false)),
                };
                let Some((listed, later)) = found else {
                    return Shown::No;
                };
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
        };
        shown.into()
    }
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
    crate::ui::pages::labels_page(page, repo, list, &keys);
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
fn compose(state: &mut State) -> Cmds {
    let Some((route, of)) = here(state) else {
        return Cmds::new();
    };
    match of {
        FormOf::Milestone { .. } => {}
        FormOf::File { change, .. } if change != FileChange::Delete => {
            if !form(state, &route).started {
                return state.just_say("The file hasn't loaded yet");
            }
        }
        _ => {
            // A form of rows: Enter sets the one selected.
            return Cmds::new();
        }
    }
    let start = [ratatui_textarea::CursorMove::Jump(0, 0)];
    crate::forms::compose::<Form>(state, &route, 0, &start)
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
            // Where it starts is asked about in your clone.
            Pick::RefName(repo, _) => Some(DataKey::Clone(repo.clone())),
            Pick::RefFrom(repo, ..) => Some(DataKey::Branches(repo.clone())),
            Pick::Field(..) => None,
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

/// The picker's row that sets the form at `route`'s `field` to `value`.
fn setting(route: &Route, field: Field, value: &str) -> Option<crate::picker::Choice> {
    Some(crate::forms::filling(route, Row::Field(field), value))
}

/// A picker's rows: what each says, and what choosing it does.
pub fn rows(state: &State, pick: &Pick, q: &str) -> crate::picker::Rows {
    match pick {
        Pick::Field(route, field) => field_rows(state, route, *field, q),
        Pick::RefName(repo, kind) => refs::name_rows(repo, *kind, q),
        Pick::RefFrom(repo, kind, name) => refs::from_rows(state, repo, *kind, name, q),
    }
}

fn field_rows(state: &State, route: &Route, field: Field, q: &str) -> crate::picker::Rows {
    let set = |value: &str| setting(route, field, value);
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
pub fn open_field(state: &mut State, route: &Route, field: Field) -> Cmds {
    if field.toggles() {
        form_mut(state, route).toggle(field);
        return Cmds::new();
    }
    let pick = crate::picker::Kind::Housekeeping(Pick::Field(route.clone(), field));
    state.open_picker(pick)
}

/// What was chosen for a form's row.
fn fill(state: &mut State, route: &Route, row: &Row, value: String) -> Cmds {
    let Row::Field(field) = *row else {
        return Cmds::new();
    };
    let f = form_mut(state, route);
    f.set(field, value);
    let blank = f.text.is_empty();
    // A new file, once named, is written, as on GitHub.
    let new_file = matches!(
        route,
        Route::Housekeeping(FormOf::File {
            change: FileChange::New,
            ..
        })
    );
    if field == Field::Path && new_file && blank && state.route() == Some(route) {
        return compose(state);
    }
    Cmds::new()
}

// ---- questions ----------------------------------------------------------------------

/// The answers to housekeeping's questions, with what they're about (a
/// file's commit: the form on screen).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Delete(Subject),
    /// Make a new branch or tag where it starts, with git in your clone.
    Push(RepoId, RefKind, String, String),
    /// Commit a file's form.
    Commit,
    /// Clone a new repository into `dir`.
    Clone {
        url: String,
        dir: PathBuf,
    },
    /// Propose a new branch a file was committed on as a pull request,
    /// into the branch it was made from (none: the default branch).
    Propose(RepoId, String, Option<String>),
}

impl Answer {
    pub fn busy(&self) -> &'static str {
        match self {
            Answer::Delete(_) => "Deleting…",
            Answer::Push(..) => "Pushing…",
            Answer::Commit => "Committing…",
            Answer::Clone { .. } => "Cloning…",
            Answer::Propose(..) => "Opening a pull request…",
        }
    }
}

/// The question `title` about the form or list at `route`.
fn question(
    route: Route,
    title: String,
    facts: Vec<(String, bool)>,
    choices: Vec<(String, Answer)>,
) -> Box<crate::forms::Ask> {
    let choices = (choices.into_iter())
        .map(|(label, a)| (label, crate::forms::Answer::Housekeeping(a)))
        .collect();
    let about = crate::forms::About::Form(route);
    Box::new(crate::forms::Ask::new(about, title, facts, choices))
}

/// Asks `title` about what's on screen.
fn ask(
    state: &mut State,
    title: String,
    facts: Vec<(String, bool)>,
    choices: Vec<(String, Answer)>,
) -> Cmds {
    if let Some(route) = state.route().cloned() {
        let ask = question(route, title, facts, choices);
        state.overlay = Some(crate::state::Overlay::Ask(ask));
    }
    Cmds::new()
}

/// The question about `route` was answered.
pub fn answer(state: &mut State, route: &Route, answer: Answer) -> Cmds {
    match answer {
        Answer::Commit => files::commit(state, route),
        Answer::Delete(Subject::Label(repo, name)) => labels::delete(state, repo, name),
        Answer::Delete(Subject::Milestone(repo, number)) => milestones::delete(state, repo, number),
        Answer::Delete(Subject::Ref(repo, kind, name)) => refs::send(state, repo, kind, name, None),
        Answer::Push(repo, kind, name, from) => refs::send(state, repo, kind, name, Some(from)),
        Answer::Clone { url, dir } => {
            crate::forms::sending(state);
            crate::work::start(repos::CloneTo { url, dir })
        }
        Answer::Propose(repo, branch, base) => {
            state.overlay = None;
            files::propose(state, repo, branch, base)
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
        chips: Vec::new(),
        link: Some(Row::Field(field)),
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
