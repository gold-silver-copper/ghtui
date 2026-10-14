//! Your gists: making one, changing one (its description and files),
//! and deleting one, as gist.github.com does.
//!
//! A new gist's form is a page ([`Route::NewGist`]), as is an edit's
//! ([`Route::EditGist`]): the description, then each file's name and
//! text, written in the composer. Making one asks whether it's secret or
//! public, as GitHub's two buttons do; GitHub's API can't change that
//! afterwards. Deleting asks first. Then the gist's page (or, once it's
//! deleted, your gists) follows GitHub until it shows the change.

use crate::api::GitHub;
use crate::api::browse::Gist;
use crate::api::gist::{FileChange, GistChange, MadeGist, NewGist};
use crate::route::app;
use crate::ui::overlays::PaletteItem;
use crate::ui::page::{Link, Page, WriteLink};
use crate::ui::pages::{FormRow, Mark, TextBox, WriteFormView};

use crate::data::DataKey;
use crate::keymap::Action;
use crate::nav::Doable;
use crate::review::ComposeTarget;
use crate::route::Route;
use crate::state::{Cmd, Overlay, State};
use crate::writes::{Answer, Chosen, Expected, Pick, Text, Work as Writes, item, row};

/// A gist being made or changed.
#[derive(Debug, Clone, Default)]
pub struct Form {
    /// What's changed, once it's loaded: its ID and owner.
    pub gist: Option<(String, String)>,
    /// Whether the gist changed is public.
    pub public: bool,
    pub description: String,
    pub files: Vec<File>,
    /// The files GitHub has that were removed, by name.
    pub removed: Vec<String>,
    /// Filled in (from the gist changed), and the composer opened once.
    pub started: bool,
    pub sending: bool,
    pub error: Option<String>,
}

/// One of a gist's files, as written.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct File {
    pub name: String,
    pub text: String,
    /// Its name and text on GitHub, for a file it has.
    pub was: Option<(String, String)>,
    /// Why it can't be changed here, if it can't.
    pub locked: Option<String>,
}

/// Work in the background.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Work {
    Create(Route, NewGist),
    Update(Route, GistChange),
    Delete { id: String, owner: String },
}

#[derive(Debug)]
pub enum Done {
    Created(Route, Result<MadeGist, String>),
    Updated(Route, GistChange, Result<(), String>),
    Deleted {
        id: String,
        owner: String,
        result: Result<(), String>,
    },
}

impl Done {
    pub fn wrote(&self) -> bool {
        match self {
            Done::Created(_, result) => result.is_ok(),
            Done::Updated(.., result) | Done::Deleted { result, .. } => result.is_ok(),
        }
    }
}

impl Work {
    pub fn failed(&self, why: String) -> Done {
        match self.clone() {
            Work::Create(route, _) => Done::Created(route, Err(why)),
            Work::Update(route, change) => Done::Updated(route, change, Err(why)),
            Work::Delete { id, owner } => Done::Deleted {
                id,
                owner,
                result: Err(why),
            },
        }
    }

    pub async fn run(self, gh: &GitHub) -> Done {
        let why = |err: crate::api::ApiError| crate::writes::reason(&err);
        match self {
            Work::Create(route, new) => {
                Done::Created(route, gh.create_gist(&new).await.map_err(why))
            }
            Work::Update(route, change) => {
                let result = gh.update_gist(&change).await.map_err(why);
                Done::Updated(route, change, result)
            }
            Work::Delete { id, owner } => {
                let result = gh.delete_gist(&id).await.map_err(why);
                Done::Deleted { id, owner, result }
            }
        }
    }
}

/// The gist form on screen (blank until it's started), and its page.
fn here(state: &State) -> Option<(Route, &Form)> {
    static BLANK: std::sync::LazyLock<Form> = std::sync::LazyLock::new(Form::default);
    let route = state
        .route()
        .filter(|r| matches!(r, Route::NewGist | Route::EditGist { .. }))?;
    let form = state.writing.gists.get(route).unwrap_or(&BLANK);
    Some((route.clone(), form))
}

fn form_mut<'a>(state: &'a mut State, route: &Route) -> &'a mut Form {
    state.writing.gists.entry(route.clone()).or_default()
}

/// The gist on screen, as loaded.
fn shown(state: &State) -> Option<&Gist> {
    let Some(Route::Gist { id, .. }) = state.route() else {
        return None;
    };
    state.picked::<Gist>(&DataKey::Gist(id.clone()))
}

/// Why the gist on screen isn't yours to change, if it isn't.
fn not_yours(state: &State, g: &Gist) -> Option<String> {
    match (&g.owner, &state.viewer) {
        (Some(owner), Some(me)) if owner.eq_ignore_ascii_case(me) => None,
        (Some(owner), _) => Some(format!("It's @{owner}'s gist: only they can change it")),
        (None, _) => Some("It has no owner GitHub names: no one can change it".into()),
    }
}

/// `,G`: a new gist's page, with what was written there before.
#[must_use]
pub fn open_new(state: &mut State) -> Vec<Cmd> {
    if state.route() == Some(&Route::NewGist) {
        return compose(state, 0);
    }
    let mut cmds = state.push(Route::NewGist);
    cmds.extend(follow(state));
    cmds
}

/// `,e` on your gist: its form, filled in from it.
#[must_use]
pub fn open_edit(state: &mut State) -> Vec<Cmd> {
    let Some(g) = shown(state) else {
        state.info("The gist hasn't loaded yet");
        return Vec::new();
    };
    if let Some(why) = not_yours(state, g) {
        state.info(why);
        return Vec::new();
    }
    let route = Route::EditGist {
        owner: g.owner.clone().unwrap_or_default(),
        id: g.id.clone(),
    };
    // Started again from what GitHub has now.
    state.writing.gists.remove(&route);
    let mut cmds = state.push(route);
    cmds.extend(follow(state));
    cmds
}

/// Brings the form on screen up to date: an edit's filled in from the
/// gist once it's loaded, then the composer opens on its first file.
#[must_use]
pub fn follow(state: &mut State) -> Vec<Cmd> {
    let Some((route, form)) = here(state) else {
        return Vec::new();
    };
    if form.started {
        return Vec::new();
    }
    match &route {
        Route::EditGist { id, .. } => {
            let Some(g) = state.picked::<Gist>(&DataKey::Gist(id.clone())).cloned() else {
                return Vec::new();
            };
            if not_yours(state, &g).is_some() {
                return Vec::new();
            }
            let form = form_mut(state, &route);
            *form = Form {
                gist: Some((g.id.clone(), g.owner.clone().unwrap_or_default())),
                public: g.public,
                description: g.description.clone(),
                files: (g.files.iter()).map(file_of).collect(),
                started: true,
                ..Form::default()
            };
            state.data_gen += 1;
            Vec::new()
        }
        _ => {
            let form = form_mut(state, &route);
            form.started = true;
            if form.files.is_empty() {
                form.files.push(File::default());
            }
            if state.overlay.is_none() {
                return compose(state, 0);
            }
            Vec::new()
        }
    }
}

/// A file of the gist, to change.
fn file_of(f: &crate::api::browse::GistFile) -> File {
    let locked = match &f.text {
        Some(_) if !f.truncated => None,
        _ => Some("Too big for GitHub's API to send whole: change it on github.com".to_owned()),
    };
    let text = f.text.clone().unwrap_or_default();
    File {
        name: f.name.clone(),
        was: Some((f.name.clone(), text.clone())),
        text,
        locked,
    }
}

/// The composer, on file `i` of the form on screen.
#[must_use]
pub fn compose(state: &mut State, i: usize) -> Vec<Cmd> {
    let Some((route, form)) = here(state) else {
        return Vec::new();
    };
    if form.sending {
        state.info("It's being saved");
        return Vec::new();
    }
    let Some(file) = form.files.get(i) else {
        return Vec::new();
    };
    if let Some(why) = &file.locked {
        state.info(why.clone());
        return Vec::new();
    }
    let text = file.text.clone();
    state.data_gen += 1;
    state.compose(ComposeTarget::Write(Text::GistFile(route, i)), &text)
}

pub fn keep_text(state: &mut State, route: &Route, i: usize, text: String) {
    let form = form_mut(state, route);
    if let Some(file) = form.files.get_mut(i) {
        file.text = text;
    }
    form.error = None;
}

/// The composer's title, for file `i`.
pub fn file_title(state: &State, route: &Route, i: usize) -> String {
    let name = file_name(state, route, i).filter(|n| !n.is_empty());
    match name {
        Some(name) => format!("Gist file {name}"),
        None => format!("Gist file {} (name it on the form)", i + 1),
    }
}

/// The gist's description, as written.
pub fn description(state: &State, route: &Route) -> Option<String> {
    let form = state.writing.gists.get(route)?;
    Some(form.description.clone())
}

/// File `i`'s name, as written.
pub fn file_name(state: &State, route: &Route, i: usize) -> Option<String> {
    let form = state.writing.gists.get(route)?;
    Some(form.files.get(i)?.name.clone())
}

/// Enter on one of the form's rows.
#[must_use]
pub fn follow_link(state: &mut State, link: &WriteLink) -> Vec<Cmd> {
    let Some((route, _)) = here(state) else {
        return Vec::new();
    };
    match link {
        WriteLink::Description => crate::writes::open_picker(state, Pick::Description(route)),
        WriteLink::File(i) => compose(state, *i),
        WriteLink::Rename(i) => crate::writes::open_picker(state, Pick::FileName(route, Some(*i))),
        WriteLink::AddFile => crate::writes::open_picker(state, Pick::FileName(route, None)),
        WriteLink::Remove(i) => {
            remove(state, &route, *i);
            Vec::new()
        }
        WriteLink::Save => submit(state),
        _ => Vec::new(),
    }
}

/// Takes file `i` out of the form.
fn remove(state: &mut State, route: &Route, i: usize) {
    let form = form_mut(state, route);
    if form.files.len() < 2 {
        state.info("A gist needs a file: add another before removing this one");
        return;
    }
    if i >= form.files.len() {
        return;
    }
    let file = form.files.remove(i);
    if let Some((was, _)) = file.was {
        form.removed.push(was);
    }
    state.data_gen += 1;
    let name = if file.name.is_empty() {
        "the file".to_owned()
    } else {
        file.name
    };
    state.info(format!("Removed {name}: saving takes it out of the gist"));
}

pub fn description_rows(route: &Route, q: &str) -> Vec<(PaletteItem, Option<Chosen>)> {
    let label = if q.is_empty() {
        "No description".to_owned()
    } else {
        format!("Describe it: “{q}”")
    };
    vec![item(
        label,
        "↵",
        Some(Chosen::Description(route.clone(), q.to_owned())),
    )]
}

/// Why `name` can't name file `i` of the form, if it can't.
fn name_problem(form: &Form, i: Option<usize>, name: &str) -> Option<String> {
    if name.is_empty() {
        return Some("Type the file's name".into());
    }
    if name.contains('/') {
        return Some("A gist's files can't be in folders: no /".into());
    }
    let taken = (form.files.iter().enumerate())
        .any(|(j, f)| Some(j) != i && f.name.eq_ignore_ascii_case(name));
    taken.then(|| format!("The gist has a file named {name} already"))
}

pub fn name_rows(
    state: &State,
    route: &Route,
    file: Option<usize>,
    q: &str,
) -> Vec<(PaletteItem, Option<Chosen>)> {
    let blank = Form::default();
    let form = state.writing.gists.get(route).unwrap_or(&blank);
    if let Some(why) = name_problem(form, file, q) {
        return vec![item(why, "", None)];
    }
    let label = match file {
        Some(_) => format!("Name it {q}"),
        None => format!("Add {q}"),
    };
    let chosen = Chosen::FileName(route.clone(), file, q.to_owned());
    vec![item(label, "↵", Some(chosen))]
}

#[must_use]
pub fn set_description(state: &mut State, route: &Route, text: String) -> Vec<Cmd> {
    form_mut(state, route).description = text;
    state.data_gen += 1;
    Vec::new()
}

#[must_use]
pub fn set_name(state: &mut State, route: &Route, file: Option<usize>, name: String) -> Vec<Cmd> {
    state.data_gen += 1;
    let form = form_mut(state, route);
    match file {
        Some(i) => {
            if let Some(f) = form.files.get_mut(i) {
                f.name = name;
            }
            Vec::new()
        }
        None => {
            form.files.push(File {
                name,
                ..File::default()
            });
            let i = form.files.len().saturating_sub(1);
            compose(state, i)
        }
    }
}

/// The files to send: each named (GitHub's way, for a file without a
/// name: `gistfile1.txt`), or why they can't be sent.
fn named(form: &Form) -> Result<Vec<(String, String)>, String> {
    if form.files.iter().all(|f| f.text.trim().is_empty()) {
        return Err("A gist needs a file with something in it".into());
    }
    let mut files: Vec<(String, String)> = Vec::new();
    for (i, f) in form.files.iter().enumerate() {
        let name = match f.name.trim() {
            "" => format!("gistfile{}.txt", i + 1),
            name => name.to_owned(),
        };
        if f.text.trim().is_empty() {
            return Err(format!(
                "{name} is empty: write something in it, or remove it"
            ));
        }
        if let Some(why) = name_problem(form, Some(i), &name) {
            return Err(why);
        }
        if files.iter().any(|(n, _)| n.eq_ignore_ascii_case(&name)) {
            return Err(format!("Two files are named {name}"));
        }
        files.push((name, f.text.clone()));
    }
    Ok(files)
}

/// The change an edit makes: only the files that changed.
fn change(form: &Form, id: &str, files: &[(String, String)]) -> GistChange {
    let mut changed: Vec<FileChange> = (form.removed.iter())
        .map(|was| FileChange {
            was: was.clone(),
            now: None,
        })
        .collect();
    let mut added = Vec::new();
    for (f, (name, text)) in form.files.iter().zip(files) {
        match &f.was {
            // The composer drops a last newline: that isn't a change.
            Some((was, before)) if was == name && before.trim_end() == text.trim_end() => {}
            Some((was, _)) => changed.push(FileChange {
                was: was.clone(),
                now: Some((name.clone(), text.clone())),
            }),
            None => added.push((name.clone(), text.clone())),
        }
    }
    GistChange {
        id: id.to_owned(),
        description: form.description.clone(),
        changed,
        added,
    }
}

/// `,s`, ctrl-s or its button: asks whether a new gist is secret or
/// public; saves an edit.
#[must_use]
pub fn submit(state: &mut State) -> Vec<Cmd> {
    let Some((route, form)) = here(state) else {
        return Vec::new();
    };
    if form.sending {
        crate::writes::say(state, "It's being saved".into());
        return Vec::new();
    }
    let files = match named(form) {
        Ok(files) => files,
        Err(why) => {
            crate::writes::say(state, why);
            return Vec::new();
        }
    };
    if let Route::EditGist { .. } = route {
        let Some((id, _)) = form.gist.clone() else {
            crate::writes::say(state, "The gist hasn't loaded yet".into());
            return Vec::new();
        };
        let change = change(form, &id, &files);
        form_mut(state, &route).sending = true;
        crate::writes::sending(state);
        state.info("Saving the gist…");
        return vec![Cmd::Write(Writes::Gist(Work::Update(route, change)))];
    }
    let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
    let mut facts = vec![(names.join(", "), false)];
    if !form.description.is_empty() {
        facts.push((format!("“{}”", form.description), false));
    }
    facts.push((
        "A secret gist is hidden from search, not private: anyone with its link can see it".into(),
        false,
    ));
    crate::writes::ask(
        state,
        route,
        "Create the gist?".into(),
        facts,
        vec![
            ("Create secret gist".into(), Answer::Gist { public: false }),
            ("Create public gist".into(), Answer::Gist { public: true }),
        ],
    );
    Vec::new()
}

/// Makes the new gist, secret or public.
#[must_use]
pub fn create(state: &mut State, route: &Route, public: bool) -> Vec<Cmd> {
    let Some(form) = state.writing.gists.get(route) else {
        return Vec::new();
    };
    let files = match named(form) {
        Ok(files) => files,
        Err(why) => {
            crate::writes::say(state, why);
            return Vec::new();
        }
    };
    let new = NewGist {
        description: form.description.clone(),
        public,
        files,
    };
    form_mut(state, route).sending = true;
    crate::writes::sending(state);
    vec![Cmd::Write(Writes::Gist(Work::Create(route.clone(), new)))]
}

/// `,D` on your gist: asks first, as GitHub does.
#[must_use]
pub fn ask_delete(state: &mut State) -> Vec<Cmd> {
    let Some(g) = shown(state) else {
        state.info("The gist hasn't loaded yet");
        return Vec::new();
    };
    if let Some(why) = not_yours(state, g) {
        state.info(why);
        return Vec::new();
    }
    let n = g.files.len();
    let files = format!("{n} {}", crate::ui::text::noun(n as u64, "file"));
    let what = if g.description.is_empty() {
        format!("Its {files}")
    } else {
        format!("“{}” and its {files}", g.description)
    };
    let Some(route) = state.route().cloned() else {
        return Vec::new();
    };
    crate::writes::ask(
        state,
        route,
        "Delete this gist?".into(),
        vec![(format!("{what}: it can't be brought back"), true)],
        vec![("Delete gist".into(), Answer::DeleteGist)],
    );
    Vec::new()
}

/// Deletes the gist on `route`'s page.
#[must_use]
pub fn delete(state: &mut State, route: &Route) -> Vec<Cmd> {
    let Route::Gist { id, .. } = route else {
        return Vec::new();
    };
    let Some(g) = state.picked::<Gist>(&DataKey::Gist(id.clone())) else {
        return Vec::new();
    };
    let owner = g.owner.clone().unwrap_or_default();
    crate::writes::sending(state);
    vec![Cmd::Write(Writes::Gist(Work::Delete {
        id: id.clone(),
        owner,
    }))]
}

/// What a gist shows once GitHub has it as sent.
fn expected(id: String, description: String, files: Vec<(String, String)>) -> Expected {
    Expected::Gist {
        id,
        description,
        files,
    }
}

/// Work's answers: the gist's page follows GitHub until it shows the
/// change; once it's deleted, your gists do.
#[must_use]
pub fn on_done(state: &mut State, done: Done) -> Vec<Cmd> {
    match done {
        Done::Created(route, Ok(made)) => {
            let form = state.writing.gists.remove(&route).unwrap_or_default();
            crate::writes::close_sender(state, &route);
            let owner = made.owner.map(|o| o.login).or_else(|| state.viewer.clone());
            state.info("Created the gist");
            if state.route() != Some(&route) {
                return Vec::new();
            }
            let files = named(&form).unwrap_or_default();
            let page = Route::Gist {
                owner,
                id: made.id.clone(),
            };
            crate::writes::expect(
                state,
                page.clone(),
                expected(made.id, form.description, files),
            );
            state.replace(page, true, None)
        }
        Done::Updated(route, change, Ok(())) => {
            let form = state.writing.gists.remove(&route).unwrap_or_default();
            crate::writes::close_sender(state, &route);
            state.info("Saved the gist");
            let Route::EditGist { owner, id } = &route else {
                return Vec::new();
            };
            if state.route() != Some(&route) {
                return Vec::new();
            }
            let page = Route::Gist {
                owner: Some(owner.clone()),
                id: id.clone(),
            };
            let files = named(&form).unwrap_or_default();
            crate::writes::expect(
                state,
                page.clone(),
                expected(change.id, change.description, files),
            );
            back_to(state, page)
        }
        Done::Deleted {
            id,
            owner,
            result: Ok(()),
        } => {
            state.overlay = None;
            state.info("Deleted the gist");
            let on_it =
                matches!(state.route(), Some(Route::Gist { id: shown, .. }) if *shown == id);
            if !on_it {
                return Vec::new();
            }
            let page = Route::Gists(owner.clone());
            crate::writes::expect(state, page.clone(), Expected::GistGone(owner, id));
            state.replace(page, true, None)
        }
        Done::Created(route, Err(why)) | Done::Updated(route, _, Err(why)) => {
            if let Some(form) = state.writing.gists.get_mut(&route) {
                form.sending = false;
                form.error = Some(why.clone());
            }
            crate::writes::say(state, format!("GitHub didn't save the gist: {why}"));
            Vec::new()
        }
        Done::Deleted {
            result: Err(why), ..
        } => {
            crate::writes::say(state, format!("GitHub didn't delete the gist: {why}"));
            Vec::new()
        }
    }
}

/// Back to `page` when it's the page before, else to it in place.
#[must_use]
fn back_to(state: &mut State, page: Route) -> Vec<Cmd> {
    let below = state.screens.below().and_then(|s| match s {
        crate::state::Screen::Page(p) => Some(&p.route),
        crate::state::Screen::Diff(_) => None,
    });
    if below == Some(&page) {
        let mut cmds = state.back();
        state.forward.clear();
        cmds.extend(state.ensure_route(&page, true));
        cmds
    } else {
        state.replace(page, true, None)
    }
}

/// The menu's rows: a new gist anywhere a gist or profile is; changing
/// and deleting your gist on its page; the form's own.
pub fn doables(state: &State, route: &Route, out: &mut Vec<Doable>) {
    if let Some((route, form)) = here(state) {
        let busy = form.sending.then(|| "it's being saved".to_owned());
        out.push(row(
            "This page",
            Action::Comment,
            "Write the selected file",
            "write",
            busy.clone(),
        ));
        let (label, short) = match route {
            Route::NewGist => ("Create the gist (secret or public)", "create"),
            _ => ("Save the gist", "save"),
        };
        let why = busy.or_else(|| named(form).err());
        out.push(row("This page", Action::Create, label, short, why));
        return;
    }
    if let Route::Gist { .. } = route {
        let why = match shown(state) {
            Some(g) => not_yours(state, g),
            None => Some("it hasn't loaded yet".into()),
        };
        out.push(row(
            "Edit",
            Action::EditComment,
            "Edit the gist",
            "edit",
            why.clone(),
        ));
        out.push(row(
            "Edit",
            Action::DeleteComment,
            "Delete the gist",
            "",
            why,
        ));
    }
    if matches!(
        route,
        Route::Gist { .. } | Route::Gists(_) | Route::User { .. } | Route::App(app::Home)
    ) {
        out.push(row("New", Action::NewGist, "New gist", "", None));
    }
}

/// The form's page.
pub fn build(state: &State, page: &mut Page, route: &Route) {
    let blank = Form::default();
    let form = state.writing.gists.get(route).unwrap_or(&blank);
    let composing = match &state.overlay {
        Some(Overlay::Compose(c)) => match &c.target {
            ComposeTarget::Write(Text::GistFile(r, i)) if r == route => Some(*i),
            _ => None,
        },
        _ => None,
    };
    let editing = matches!(route, Route::EditGist { .. });
    let mut rows = vec![FormRow {
        name: "Description",
        value: if form.description.is_empty() {
            "Gist description…".into()
        } else {
            form.description.clone()
        },
        unset: form.description.is_empty(),
        link: Some(WriteLink::Description),
    }];
    let mut status = Vec::new();
    if editing {
        let (visibility, why) = if form.public {
            ("Public", "GitHub doesn't let a public gist become secret")
        } else {
            (
                "Secret",
                "Making it public works on github.com: GitHub's API can't",
            )
        };
        rows.push(FormRow {
            name: "Visibility",
            value: format!("{visibility} · {why}"),
            unset: true,
            link: None,
        });
        if !form.removed.is_empty() {
            let text = format!("Saving removes {}", form.removed.join(", "));
            status.push((text, Mark::Waiting));
        }
    }
    let boxes: Vec<TextBox<'_>> = (form.files.iter().enumerate())
        .map(|(i, f)| {
            let mut actions = vec![("rename", WriteLink::Rename(i))];
            if form.files.len() > 1 {
                actions.push(("remove", WriteLink::Remove(i)));
            }
            let markdown = crate::ui::pages::is_markdown(&f.name);
            TextBox {
                title: f.name.clone(),
                untitled: "Filename including extension…",
                body: if composing == Some(i) { "" } else { &f.text },
                markdown,
                empty: "Nothing in it yet.",
                hint: f
                    .locked
                    .as_deref()
                    .unwrap_or("Enter writes it in the composer (ctrl-e for $EDITOR)"),
                link: Link::Write(WriteLink::File(i)),
                actions,
            }
        })
        .collect();
    let after = vec![FormRow {
        name: "",
        value: "+ Add file".into(),
        unset: false,
        link: Some(WriteLink::AddFile),
    }];
    let write = state.first_key(Action::Comment);
    let save_key = state.first_key(Action::Create);
    let view = WriteFormView {
        heading: if editing {
            "Editing gist".into()
        } else {
            "New gist".into()
        },
        base: None,
        rows,
        status,
        boxes,
        after,
        write: &write,
        button: if editing {
            "Update gist".into()
        } else {
            "Create gist…".into()
        },
        save_key: &save_key,
        why_not: named(form).err(),
        error: form.error.as_deref(),
    };
    crate::ui::pages::write_form(page, &view);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, text: &str, was: Option<(&str, &str)>) -> File {
        File {
            name: name.into(),
            text: text.into(),
            was: was.map(|(n, t)| (n.into(), t.into())),
            locked: None,
        }
    }

    /// An edit sends only what changed: renamed and rewritten files by
    /// the names GitHub has, new files, and removed ones as nothing.
    #[test]
    fn an_edit_sends_only_what_changed() {
        let form = Form {
            description: "Notes".into(),
            files: vec![
                file("a.md", "same", Some(("a.md", "same"))),
                file("b2.rs", "fn b() {}", Some(("b.rs", "fn b() {}"))),
                file("c.txt", "new text", Some(("c.txt", "old text"))),
                file("", "added", None),
            ],
            removed: vec!["gone.txt".into()],
            ..Form::default()
        };
        let files = named(&form).unwrap();
        assert_eq!(files[3].0, "gistfile4.txt");
        let change = change(&form, "abc", &files);
        let moved = |was: &str, name: &str, text: &str| FileChange {
            was: was.into(),
            now: Some((name.into(), text.into())),
        };
        assert_eq!(
            change.changed,
            vec![
                FileChange {
                    was: "gone.txt".into(),
                    now: None
                },
                moved("b.rs", "b2.rs", "fn b() {}"),
                moved("c.txt", "c.txt", "new text"),
            ]
        );
        assert_eq!(
            change.added,
            vec![("gistfile4.txt".to_owned(), "added".to_owned())]
        );
    }

    #[test]
    fn a_gist_needs_something_in_each_file_and_unique_names() {
        let form = |files| Form {
            files,
            ..Form::default()
        };
        named(&form(vec![file("a", " ", None)])).unwrap_err();
        let twice = form(vec![file("a.rs", "1", None), file("A.rs", "2", None)]);
        assert!(named(&twice).unwrap_err().contains("already"));
        let folder = form(vec![file("src/a.rs", "1", None)]);
        assert!(named(&folder).unwrap_err().contains("folders"));
    }
}
