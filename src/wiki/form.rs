//! Writing a wiki's pages: editing one (`,e` on it) or making one (`,W`),
//! in the composer or `$EDITOR`. The form is a page
//! ([`Route::WikiEdit`]); saving commits the page on the wiki's latest
//! and pushes it with your git credentials ([`super::save`]). A page
//! changed on GitHub since it was opened is never overwritten: saving
//! says so, and keeps your text.

use std::borrow::Cow;

use crate::api::browse::WikiPage;
use crate::api::model::RepoId;
use crate::create::templates;
use crate::route::{Route, wiki};
use crate::ui::page::{Link, Page};
use crate::ui::pages::{Mark, TextBox, WriteFormView};

use crate::data::DataKey;
use crate::follow::{Expectation, Shown};
use crate::forms::{Fields, say};
use crate::keymap::Action;
use crate::nav::{Doable, row};
use crate::state::{Cmds, State};
use crate::work::{self, Cx, Failure, Job};

/// Commits and pushes a wiki page; its form's page asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Save {
    pub route: Route,
    pub save: crate::wiki::Save,
}

/// A page being written.
#[derive(Debug, Clone, Default)]
pub struct Form {
    /// A page's text; a new page's title on its first line, then its text.
    pub text: String,
    /// The page changed: its title, file, and the commit it was read at.
    pub page: Option<(String, String, String)>,
    /// Filled in, and the composer opened once.
    pub started: bool,
}

impl Fields for Form {
    fn text(&mut self, _: usize) -> Option<&mut String> {
        Some(&mut self.text)
    }

    fn title(&self, _: &Route, _: usize) -> String {
        match &self.page {
            Some((title, ..)) => format!("Edit the wiki page {title}"),
            None => "New wiki page: its title on the first line, then its text".into(),
        }
    }

    fn save(&self, _: &Route) -> &'static str {
        "save page"
    }
}

/// The wiki page being written on screen: its repository, the page (none
/// for a new one), and its form.
fn here(state: &State) -> Option<(Route, RepoId, Option<String>, Cow<'_, Form>)> {
    let route = state.route()?;
    let Route::Wiki(wiki::WikiEdit { repo, page }) = route else { return None };
    let form = state.forms.shown(route);
    Some((route.clone(), repo.clone(), page.clone(), form))
}

/// What a wiki's pages add: making one, and editing the one on screen.
pub const FEATURE: crate::features::Feature = crate::features::Feature {
    owns: &[Action::NewWikiPage],
    takes: |state, action| {
        action == Action::EditComment
            && matches!(state.route(), Some(Route::Wiki(wiki::Wiki { .. })))
    },
    act: |state, action| match action {
        Action::NewWikiPage => open_new(state),
        _ => open_edit(state),
    },
    offers: doables,
    ..crate::features::Feature::NONE
};

/// What a form reads: the page, or for a new one the list of pages.
pub fn key(repo: &RepoId, page: Option<&String>) -> DataKey {
    let page = page.cloned().unwrap_or_else(|| "_pages".into());
    DataKey::Wiki(repo.clone(), Some(page))
}

/// The wiki page on screen, as read.
fn shown(state: &State) -> Option<(RepoId, &WikiPage)> {
    let Some(Route::Wiki(wiki::Wiki { repo, page })) = state.route() else {
        return None;
    };
    let w = state.picked::<WikiPage>(&DataKey::Wiki(repo.clone(), page.clone()))?;
    Some((repo.clone(), w))
}

/// Why the page on screen can't be edited here, if it can't.
fn uneditable(state: &State) -> Option<String> {
    let Some((_, w)) = shown(state) else {
        return Some("The page hasn't loaded yet".into());
    };
    match (&w.title, &w.file) {
        (Some(_), Some(_)) => None,
        _ => Some("This is the list of pages: open one to edit it".into()),
    }
}

/// `,e` on a wiki page: its form.
pub fn open_edit(state: &mut State) -> Cmds {
    if let Some(why) = uneditable(state) {
        return state.just_say(why);
    }
    let Some((repo, w)) = shown(state) else { return Cmds::new() };
    let page = w.title.as_deref().map(crate::wiki::slug);
    let route = Route::Wiki(wiki::WikiEdit { repo, page });
    // Started again from what's read now.
    state.forms.take::<Form>(&route);
    crate::forms::open(state, route)
}

/// `,W`: a new page of the wiki on screen (or of the repository's).
pub fn open_new(state: &mut State) -> Cmds {
    let repo = match state.route() {
        Some(Route::Wiki(wiki::WikiEdit { page: None, .. })) => return compose(state),
        Some(route) => route.repo().cloned(),
        None => None,
    };
    let Some(repo) = repo else {
        return state.just_say("Open a repository's wiki first");
    };
    if state.overview(&repo).is_some_and(|o| !o.has_wiki) {
        return state.just_say(format!("{repo} has its wiki turned off"));
    }
    crate::forms::open(state, Route::Wiki(wiki::WikiEdit { repo, page: None }))
}

/// Brings the form on screen up to date: filled in from the page once
/// it's read, then the composer opens on it.
fn follow(state: &mut State) -> Cmds {
    let Some((route, repo, page, form)) = here(state) else { return Cmds::new() };
    if form.started {
        return Cmds::new();
    }
    let Some(w) = state
        .picked::<WikiPage>(&key(&repo, page.as_ref()))
        .cloned()
    else {
        return Cmds::new();
    };
    let form = state.forms.edit::<Form>(&route);
    form.started = true;
    if page.is_some()
        && let (Some(title), Some(file), Some(commit)) = (w.title, w.file, w.commit)
    {
        form.text = w.text.unwrap_or_default();
        form.page = Some((title, file, commit));
    }
    if state.overlay.is_none() {
        return compose(state);
    }
    Cmds::new()
}

/// The composer, on the page.
fn compose(state: &mut State) -> Cmds {
    let Some((route, _, page, form)) = here(state) else { return Cmds::new() };
    if page.is_some() && form.page.is_none() {
        return state.just_say("The page hasn't loaded yet");
    }
    let top = [ratatui_textarea::CursorMove::Top];
    crate::forms::compose::<Form>(state, &route, 0, &top)
}

/// Why the page can't be saved, if it can't: its page asks each frame,
/// so nothing is copied.
fn problem(form: &Form) -> Option<&'static str> {
    // A new page's title is its first line, as `templates::split` reads it.
    let title = form.text.lines().next().unwrap_or_default().trim();
    if form.page.is_some() {
        None
    } else if title.is_empty() {
        Some("Write the page's title first: it's the first line")
    } else if title.contains(['/', '\\']) || title.chars().any(char::is_control) {
        Some("A page's title can't have / or \\ in it")
    } else {
        None
    }
}

/// The page to save, or why it can't be.
fn to_save(state: &State, repo: &RepoId, form: &Form) -> Result<crate::wiki::Save, String> {
    if let Some(why) = problem(form) {
        return Err(why.into());
    }
    let login = state.viewer.clone();
    match &form.page {
        Some((title, file, commit)) => Ok(crate::wiki::Save {
            repo: repo.clone(),
            title: title.clone(),
            file: Some(file.clone()),
            base: Some(commit.clone()),
            text: form.text.clone(),
            login,
        }),
        None => {
            let (title, text) = templates::split(&form.text);
            Ok(crate::wiki::Save {
                repo: repo.clone(),
                title,
                file: None,
                base: None,
                text,
                login,
            })
        }
    }
}

/// `,s`, ctrl-s or its button: commits and pushes the page.
fn submit(state: &mut State) -> Cmds {
    let Some((route, repo, page, form)) = here(state) else { return Cmds::new() };
    if page.is_some() && form.page.is_none() {
        return say(state, "The page hasn't loaded yet".into());
    }
    let save = match to_save(state, &repo, &form) {
        Ok(save) => save,
        Err(why) => {
            return say(state, why);
        }
    };
    crate::forms::form_sending(state, &route);
    state.info(format!("Saving {}…", save.title));
    work::start(Save { route, save })
}

/// A wiki page with this text, as its page's data is keyed.
#[derive(Debug)]
struct Saved(DataKey, String);

impl Expectation for Saved {
    fn waiting(&self) -> &'static str {
        "Waiting for GitHub to show the page"
    }

    fn shown(&self, state: &State, _: &Route) -> Shown {
        let text = |w: &WikiPage| w.text.as_deref().map(str::trim_end) == Some(self.1.trim_end());
        state.picked::<WikiPage>(&self.0).is_some_and(text).into()
    }
}

impl Job for Save {
    type Out = String;

    async fn run(&self, cx: &Cx) -> Result<String, Failure> {
        crate::wiki::save(&cx.git, &self.save)
            .await
            .map_err(Failure::msg)
    }

    /// Saved: the page opens (back on it, when it's the page before), and
    /// follows GitHub until it shows the text saved.
    fn done(self, state: &mut State, result: Result<String, Failure>) -> Cmds {
        let route = &self.route;
        let Route::Wiki(wiki::WikiEdit { repo, .. }) = route else { return Cmds::new() };
        match result {
            Ok(slug) => {
                let form: Form = crate::forms::done(state, route);
                let text = match &form.page {
                    Some(_) => form.text.clone(),
                    None => templates::split(&form.text).1,
                };
                state.info(match &form.page {
                    Some((title, ..)) => format!("Saved {title}"),
                    None => format!("Created {}", slug.replace('-', " ")),
                });
                if state.route() != Some(route) {
                    return Cmds::new();
                }
                // Back on the page it was opened from, if it's that one.
                let below = state.screens.below().and_then(|s| match &s.page()?.route {
                    shown @ Route::Wiki(wiki::Wiki { repo: r, page }) if r == repo => {
                        let title = (state
                            .picked::<WikiPage>(&DataKey::Wiki(r.clone(), page.clone())))
                        .and_then(|w| w.title.as_deref().map(crate::wiki::slug));
                        (title.as_deref() == Some(slug.as_str())).then(|| shown.clone())
                    }
                    _ => None,
                });
                let page = below.unwrap_or_else(|| {
                    Route::Wiki(wiki::Wiki {
                        repo: repo.clone(),
                        page: Some(slug),
                    })
                });
                let Route::Wiki(wiki::Wiki { page: name, .. }) = &page else { return Cmds::new() };
                let shows = Saved(DataKey::Wiki(repo.clone(), name.clone()), text);
                crate::follow::expect(state, page.clone(), shows);
                crate::gist::back_to(state, page)
            }
            Err(why) => crate::forms::failed(state, route, why.to_string()),
        }
    }
}

/// What a wiki page's form does.
pub const FORM: crate::forms::Kind = crate::forms::Kind {
    compose: |state, _| compose(state),
    row: |_, _| Cmds::new(),
    fill: |_, _, _, _| Cmds::new(),
    submit,
    follow,
    offers,
};

/// The form's rows in the menu: writing it, and saving it.
fn offers(state: &State, out: &mut Vec<Doable>) {
    if let Some((.., form)) = here(state) {
        let why = problem(&form).map(str::to_owned);
        let save = ("Save the page", "save");
        crate::forms::offer(state, Some("Write the page"), save, why, out);
    }
}

/// The menu's rows: editing the page on screen and making one.
fn doables(state: &State, out: &mut Vec<Doable>) {
    if let Some(Route::Wiki(wiki::Wiki { .. })) = state.route() {
        out.push(row(
            "Edit",
            Action::EditComment,
            "Edit this page",
            "edit",
            uneditable(state),
        ));
        out.push(row("New", Action::NewWikiPage, "New wiki page", "", None));
    }
}

/// The form's page.
pub fn build(state: &State, page: &mut Page, route: &Route) {
    let Route::Wiki(wiki::WikiEdit { repo, .. }) = route else { return };
    let form = state.forms.shown::<Form>(route);
    let composing = crate::forms::composing(state, route, 0);
    let (heading, title, body, hint) = match &form.page {
        Some((title, file, _)) => (
            format!("Editing {title} in {repo}'s wiki"),
            title.clone(),
            Cow::Borrowed(form.text.as_str()),
            format!("Saved as {file}: committed and pushed with your git credentials"),
        ),
        None => {
            let (title, body) = templates::split(&form.text);
            (
                format!("New page in {repo}'s wiki"),
                title,
                Cow::Owned(body),
                "The first line is the title; then the page, in Markdown (ctrl-e for $EDITOR)"
                    .to_owned(),
            )
        }
    };
    let markdown = form
        .page
        .as_ref()
        .is_none_or(|(_, file, _)| crate::ui::pages::is_markdown(file));
    let mut status = Vec::new();
    if let Some((_, _, commit)) = &form.page {
        let at = crate::ui::text::short_sha(commit);
        status.push((
            format!("As the wiki had it at {at}: if it changes meanwhile, saving says so"),
            Mark::Fine,
            None,
        ));
    }
    let body = if composing { "" } else { &body };
    let view = WriteFormView {
        heading,
        base: Some(crate::ui::markdown::LinkBase::wiki(repo)),
        status,
        boxes: vec![TextBox {
            title: if composing { String::new() } else { title },
            untitled: "Title",
            body,
            markdown,
            empty: "Nothing written yet.",
            hint: &hint,
            link: Link::Form(crate::ui::form::Row::Text(0)),
            actions: Vec::new(),
        }],
        button: "Save page".into(),
        why_not: problem(&form).map(str::to_owned),
        ..WriteFormView::default()
    };
    crate::forms::write_form(state, route, page, view);
}
