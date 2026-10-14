//! Your gists: making one, changing one (its description and files),
//! and deleting one, as gist.github.com does.
//!
//! A new gist's form is a page (`gists::NewGist`), as is an edit's
//! (`gists::EditGist`): the description, then each file's name and
//! text, written in the composer. Making one asks whether it's secret or
//! public, as GitHub's two buttons do; GitHub's API can't change that
//! afterwards. Deleting asks first. Then the gist's page (or, once it's
//! deleted, your gists) follows GitHub until it shows the change.

use std::borrow::Cow;

use crate::api::browse::Gist;
use crate::api::gist::{FileChange, GistChange, MadeGist, NewGist};
use crate::route::{Route, app, gists, people};
use crate::ui::form::Row;
use crate::ui::overlays::PaletteItem;
use crate::ui::page::{Link, Page};
use crate::ui::pages::{FormRow, Mark, TextBox, WriteFormView};

use crate::data::DataKey;
use crate::follow::{Expectation, Shown};
use crate::forms::Answer;
use crate::forms::{Fields, say};
use crate::keymap::Action;
use crate::nav::{Doable, row};
use crate::picker::{Choice, entry};
use crate::state::{Cmd, State};
use crate::work::{self, Cx, Failure, Job};

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
}

impl Fields for Form {
    fn text(&mut self, part: usize) -> Option<&mut String> {
        Some(&mut self.files.get_mut(part)?.text)
    }

    fn title(&self, _: &Route, part: usize) -> String {
        let name = self.files.get(part).map(|f| f.name.as_str());
        match name.filter(|n| !n.is_empty()) {
            Some(name) => format!("Gist file {name}"),
            None => format!("Gist file {} (name it on the form)", part + 1),
        }
    }

    fn save(&self, route: &Route) -> &'static str {
        match route {
            Route::Gists(gists::NewGist) => "create gist…",
            _ => "save gist",
        }
    }
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

/// Making the gist written on the form at `route`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Create {
    pub route: Route,
    pub new: NewGist,
}

/// Changing a gist as its form at `route` has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub route: Route,
    pub change: GistChange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delete {
    pub id: String,
    pub owner: String,
}

/// The gist form on screen (blank until it's started), and its page.
fn here(state: &State) -> Option<(Route, Cow<'_, Form>)> {
    let route = state.route().filter(|r| {
        matches!(
            r,
            Route::Gists(gists::NewGist) | Route::Gists(gists::EditGist { .. })
        )
    })?;
    Some((route.clone(), state.forms.shown(route)))
}

/// What gists add: making one, and changing and deleting yours on its
/// page.
pub const FEATURE: crate::features::Feature = crate::features::Feature {
    owns: &[Action::NewGist],
    takes: |state, action| {
        matches!(action, Action::EditComment | Action::DeleteComment)
            && matches!(state.route(), Some(Route::Gists(gists::Gist { .. })))
    },
    act: |state, action| match action {
        Action::NewGist => open_new(state),
        Action::EditComment => open_edit(state),
        _ => ask_delete(state),
    },
    offers: doables,
    ..crate::features::Feature::NONE
};

/// The gist on screen, as loaded.
fn shown(state: &State) -> Option<&Gist> {
    let Some(Route::Gists(gists::Gist { id, .. })) = state.route() else {
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
    if state.route() == Some(&Route::Gists(gists::NewGist)) {
        return compose(state, 0);
    }
    let mut cmds = state.push(Route::Gists(gists::NewGist));
    cmds.extend(follow(state));
    cmds
}

/// `,e` on your gist: its form, filled in from it.
#[must_use]
pub fn open_edit(state: &mut State) -> Vec<Cmd> {
    let Some(g) = shown(state) else {
        return state.just_say("The gist hasn't loaded yet");
    };
    if let Some(why) = not_yours(state, g) {
        return state.just_say(why);
    }
    let route = Route::Gists(gists::EditGist {
        owner: g.owner.clone().unwrap_or_default(),
        id: g.id.clone(),
    });
    // Started again from what GitHub has now.
    state.forms.take::<Form>(&route);
    let mut cmds = state.push(route);
    cmds.extend(follow(state));
    cmds
}

/// Brings the form on screen up to date: an edit's filled in from the
/// gist once it's loaded, then the composer opens on its first file.
#[must_use]
fn follow(state: &mut State) -> Vec<Cmd> {
    let Some((route, form)) = here(state) else {
        return Vec::new();
    };
    if form.started {
        return Vec::new();
    }
    match &route {
        Route::Gists(gists::EditGist { id, .. }) => {
            let Some(g) = state.picked::<Gist>(&DataKey::Gist(id.clone())).cloned() else {
                return Vec::new();
            };
            if not_yours(state, &g).is_some() {
                return Vec::new();
            }
            *state.forms.edit(&route) = Form {
                gist: Some((g.id.clone(), g.owner.clone().unwrap_or_default())),
                public: g.public,
                description: g.description.clone(),
                files: (g.files.iter()).map(file_of).collect(),
                started: true,
                ..Form::default()
            };
            Vec::new()
        }
        _ => {
            let form = state.forms.edit::<Form>(&route);
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
fn compose(state: &mut State, i: usize) -> Vec<Cmd> {
    let Some((route, form)) = here(state) else {
        return Vec::new();
    };
    if let Some(why) = form.files.get(i).and_then(|f| f.locked.clone()) {
        return state.just_say(why);
    }
    crate::forms::compose::<Form>(state, &route, i, &[])
}

/// The gist's description, as written.
pub fn description(state: &State, route: &Route) -> Option<String> {
    let form = state.forms.get::<Form>(route)?;
    Some(form.description.clone())
}

/// File `i`'s name, as written.
pub fn file_name(state: &State, route: &Route, i: usize) -> Option<String> {
    let form = state.forms.get::<Form>(route)?;
    Some(form.files.get(i)?.name.clone())
}

/// What a gist's form does.
pub const FORM: crate::forms::Kind = crate::forms::Kind {
    compose,
    row: on_row,
    submit,
    follow,
    offers,
};

/// Enter on one of the form's rows.
#[must_use]
fn on_row(state: &mut State, link: &Row) -> Vec<Cmd> {
    let Some((route, _)) = here(state) else {
        return Vec::new();
    };
    match link {
        Row::Description => state.open_picker(crate::picker::Kind::Description(route)),
        Row::Rename(i) => state.open_picker(crate::picker::Kind::FileName(route, Some(*i))),
        Row::AddFile => state.open_picker(crate::picker::Kind::FileName(route, None)),
        Row::Remove(i) => {
            remove(state, &route, *i);
            Vec::new()
        }
        _ => Vec::new(),
    }
}

/// Takes file `i` out of the form.
fn remove(state: &mut State, route: &Route, i: usize) {
    let form = state.forms.edit::<Form>(route);
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
    let name = if file.name.is_empty() {
        "the file".to_owned()
    } else {
        file.name
    };
    state.info(format!("Removed {name}: saving takes it out of the gist"));
}

pub fn description_rows(route: &Route, q: &str) -> Vec<(PaletteItem, Option<Choice>)> {
    let label = if q.is_empty() {
        "No description".to_owned()
    } else {
        format!("Describe it: “{q}”")
    };
    vec![entry(
        label,
        "↵",
        Some(Choice::Description(route.clone(), q.to_owned())),
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
) -> Vec<(PaletteItem, Option<Choice>)> {
    let form = state.forms.shown::<Form>(route);
    if let Some(why) = name_problem(&form, file, q) {
        return vec![entry(why, "", None)];
    }
    let label = match file {
        Some(_) => format!("Name it {q}"),
        None => format!("Add {q}"),
    };
    let chosen = Choice::FileName(route.clone(), file, q.to_owned());
    vec![entry(label, "↵", Some(chosen))]
}

#[must_use]
pub fn set_description(state: &mut State, route: &Route, text: String) -> Vec<Cmd> {
    state.forms.edit::<Form>(route).description = text;
    Vec::new()
}

#[must_use]
pub fn set_name(state: &mut State, route: &Route, file: Option<usize>, name: String) -> Vec<Cmd> {
    let form = state.forms.edit::<Form>(route);
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
fn submit(state: &mut State) -> Vec<Cmd> {
    let Some((route, form)) = here(state) else {
        return Vec::new();
    };
    let files = match named(&form) {
        Ok(files) => files,
        Err(why) => {
            say(state, why);
            return Vec::new();
        }
    };
    if let Route::Gists(gists::EditGist { .. }) = route {
        let Some((id, _)) = form.gist.clone() else {
            say(state, "The gist hasn't loaded yet".into());
            return Vec::new();
        };
        let change = change(&form, &id, &files);
        crate::forms::form_sending(state, &route);
        state.info("Saving the gist…");
        return work::start(Update { route, change });
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
    crate::forms::ask(
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
    let Some(form) = state.forms.get::<Form>(route) else {
        return Vec::new();
    };
    let files = match named(form) {
        Ok(files) => files,
        Err(why) => {
            say(state, why);
            return Vec::new();
        }
    };
    let new = NewGist {
        description: form.description.clone(),
        public,
        files,
    };
    crate::forms::form_sending(state, route);
    work::start(Create {
        route: route.clone(),
        new,
    })
}

/// `,D` on your gist: asks first, as GitHub does.
#[must_use]
pub fn ask_delete(state: &mut State) -> Vec<Cmd> {
    let Some(g) = shown(state) else {
        return state.just_say("The gist hasn't loaded yet");
    };
    if let Some(why) = not_yours(state, g) {
        return state.just_say(why);
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
    crate::forms::ask(
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
    let Route::Gists(gists::Gist { id, .. }) = route else {
        return Vec::new();
    };
    let Some(g) = state.picked::<Gist>(&DataKey::Gist(id.clone())) else {
        return Vec::new();
    };
    let owner = g.owner.clone().unwrap_or_default();
    crate::forms::sending(state);
    work::start(Delete {
        id: id.clone(),
        owner,
    })
}

/// What a gist, or its owner's gists, show once GitHub has a change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expected {
    /// A gist with this description and these files' names and text.
    Gist {
        id: String,
        description: String,
        files: Vec<(String, String)>,
    },
    /// A gist no longer among its owner's.
    GistGone(String, String),
}

impl Expectation for Expected {
    fn waiting(&self) -> &'static str {
        match self {
            Expected::Gist { .. } => "Waiting for GitHub to show the gist",
            Expected::GistGone(..) => "Waiting for GitHub to show the gist gone",
        }
    }

    fn shown(&self, state: &State, _: &Route) -> Shown {
        use crate::api::browse::{GistSummary, Results};
        match self {
            Expected::Gist {
                id,
                description,
                files,
            } => state
                .picked::<Gist>(&DataKey::Gist(id.clone()))
                .is_some_and(|g| {
                    let shown: Vec<(&str, Option<&str>)> = (g.files.iter())
                        .map(|f| (f.name.as_str(), f.text.as_deref().filter(|_| !f.truncated)))
                        .collect();
                    g.description == *description
                        && shown.len() == files.len()
                        && files.iter().all(|(name, text)| {
                            shown
                                .iter()
                                .any(|(n, t)| n == name && t.is_none_or(|t| t == text))
                        })
                }),
            Expected::GistGone(owner, id) => state
                .picked::<Results<GistSummary>>(&DataKey::Gists(owner.to_lowercase()))
                .is_some_and(|r| r.items.iter().all(|g| g.id != *id)),
        }
        .into()
    }
}

/// What a gist shows once GitHub has it as sent.
fn expected(id: String, description: String, files: Vec<(String, String)>) -> Expected {
    Expected::Gist {
        id,
        description,
        files,
    }
}

/// The gist's page follows GitHub until it shows it.
impl Job for Create {
    type Out = MadeGist;

    async fn run(&self, cx: &Cx) -> Result<MadeGist, Failure> {
        Ok(cx.gh.create_gist(&self.new).await?)
    }

    fn done(self, state: &mut State, result: Result<MadeGist, Failure>) -> Vec<Cmd> {
        let route = self.route;
        let made = match result {
            Ok(made) => made,
            Err(why) => return unsaved(state, &route, &why),
        };
        let form: Form = crate::forms::done(state, &route);
        let owner = made.owner.map(|o| o.login).or_else(|| state.viewer.clone());
        state.info("Created the gist");
        if state.route() != Some(&route) {
            return Vec::new();
        }
        let files = named(&form).unwrap_or_default();
        let page = Route::Gists(gists::Gist {
            owner,
            id: made.id.clone(),
        });
        crate::follow::expect(
            state,
            page.clone(),
            expected(made.id, form.description, files),
        );
        state.replace(page, true, None)
    }
}

/// The gist's page follows GitHub until it shows the change.
impl Job for Update {
    type Out = ();

    async fn run(&self, cx: &Cx) -> Result<(), Failure> {
        Ok(cx.gh.update_gist(&self.change).await?)
    }

    fn done(self, state: &mut State, result: Result<(), Failure>) -> Vec<Cmd> {
        let Update { route, change } = self;
        if let Err(why) = result {
            return unsaved(state, &route, &why);
        }
        let form: Form = crate::forms::done(state, &route);
        state.info("Saved the gist");
        let Route::Gists(gists::EditGist { owner, id }) = &route else {
            return Vec::new();
        };
        if state.route() != Some(&route) {
            return Vec::new();
        }
        let page = Route::Gists(gists::Gist {
            owner: Some(owner.clone()),
            id: id.clone(),
        });
        let files = named(&form).unwrap_or_default();
        crate::follow::expect(
            state,
            page.clone(),
            expected(change.id, change.description, files),
        );
        back_to(state, page)
    }
}

/// Once it's deleted, your gists follow GitHub until it's gone.
impl Job for Delete {
    type Out = ();

    async fn run(&self, cx: &Cx) -> Result<(), Failure> {
        Ok(cx.gh.delete_gist(&self.id).await?)
    }

    fn done(self, state: &mut State, result: Result<(), Failure>) -> Vec<Cmd> {
        let Delete { id, owner } = self;
        if let Err(why) = result {
            say(state, format!("GitHub didn't delete the gist: {why}"));
            return Vec::new();
        }
        state.overlay = None;
        state.info("Deleted the gist");
        let on_it = matches!(state.route(), Some(Route::Gists(gists::Gist { id: shown, .. })) if *shown == id);
        if !on_it {
            return Vec::new();
        }
        let page = Route::Gists(gists::Gists(owner.clone()));
        crate::follow::expect(state, page.clone(), Expected::GistGone(owner, id));
        state.replace(page, true, None)
    }
}

/// GitHub didn't save the gist on the form at `route`, and why.
fn unsaved(state: &mut State, route: &Route, why: &Failure) -> Vec<Cmd> {
    crate::forms::failed(state, route, format!("GitHub didn't save the gist: {why}"));
    Vec::new()
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

/// The form's rows in the menu: writing it, and saving it.
fn offers(state: &State, out: &mut Vec<Doable>) {
    if let Some((route, form)) = here(state) {
        let save = match route {
            Route::Gists(gists::NewGist) => ("Create the gist (secret or public)", "create"),
            _ => ("Save the gist", "save"),
        };
        let write = Some("Write the selected file");
        crate::forms::offer(state, write, save, named(&form).err(), out);
    }
}

/// The menu's rows: a new gist anywhere a gist or profile is; changing
/// and deleting your gist on its page.
fn doables(state: &State, out: &mut Vec<Doable>) {
    let Some(route) = state.route().filter(|_| here(state).is_none()) else {
        return;
    };
    if let Route::Gists(gists::Gist { .. }) = route {
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
        Route::Gists(gists::Gist { .. })
            | Route::Gists(gists::Gists(_))
            | Route::People(people::User { .. })
            | Route::App(app::Home)
    ) {
        out.push(row("New", Action::NewGist, "New gist", "", None));
    }
}

/// The form's page.
pub fn build(state: &State, page: &mut Page, route: &Route) {
    let form = state.forms.shown::<Form>(route);
    let composing = |i| crate::forms::composing(state, route, i);
    let editing = matches!(route, Route::Gists(gists::EditGist { .. }));
    let mut rows = vec![FormRow {
        name: "Description",
        value: if form.description.is_empty() {
            "Gist description…".into()
        } else {
            form.description.clone()
        },
        unset: form.description.is_empty(),
        link: Some(Row::Description),
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
            let mut actions = vec![("rename", Row::Rename(i))];
            if form.files.len() > 1 {
                actions.push(("remove", Row::Remove(i)));
            }
            let markdown = crate::ui::pages::is_markdown(&f.name);
            TextBox {
                title: f.name.clone(),
                untitled: "Filename including extension…",
                body: if composing(i) { "" } else { &f.text },
                markdown,
                empty: "Nothing in it yet.",
                hint: f
                    .locked
                    .as_deref()
                    .unwrap_or("Enter writes it in the composer (ctrl-e for $EDITOR)"),
                link: Link::Form(Row::Text(i)),
                actions,
            }
        })
        .collect();
    let after = vec![FormRow {
        name: "",
        value: "+ Add file".into(),
        unset: false,
        link: Some(Row::AddFile),
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
        why_not: named(&form).err(),
        error: state.forms.sent(route).error.as_deref(),
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
