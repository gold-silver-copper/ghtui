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
use crate::ui::page::{Link, Page, WriteLink};
use crate::ui::pages::{Mark, TextBox, WriteFormView};

use crate::data::DataKey;
use crate::follow::{Expectation, Shown};
use crate::forms::{Fields, say};
use crate::keymap::Action;
use crate::nav::Doable;
use crate::review::ComposeTarget;
use crate::state::{Cmd, Overlay, State};
use crate::work::{self, Cx, Failure, Job};
use crate::writes::{Text, row};

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

impl Fields for Form {}

/// The wiki page being written on screen: its repository, the page (none
/// for a new one), and its form.
fn here(state: &State) -> Option<(Route, RepoId, Option<String>, Cow<'_, Form>)> {
    let route = state.route()?;
    let Route::Wiki(wiki::WikiEdit { repo, page }) = route else {
        return None;
    };
    let form = state.forms.shown(route);
    Some((route.clone(), repo.clone(), page.clone(), form))
}

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
#[must_use]
pub fn open_edit(state: &mut State) -> Vec<Cmd> {
    if let Some(why) = uneditable(state) {
        state.info(why);
        return Vec::new();
    }
    let Some((repo, w)) = shown(state) else {
        return Vec::new();
    };
    let page = w.title.as_deref().map(crate::wiki::slug);
    let route = Route::Wiki(wiki::WikiEdit { repo, page });
    // Started again from what's read now.
    state.forms.take::<Form>(&route);
    let mut cmds = state.push(route);
    cmds.extend(follow(state));
    cmds
}

/// `,W`: a new page of the wiki on screen (or of the repository's).
#[must_use]
pub fn open_new(state: &mut State) -> Vec<Cmd> {
    let repo = match state.route() {
        Some(Route::Wiki(wiki::WikiEdit { page: None, .. })) => return compose(state),
        Some(route) => route.repo().cloned(),
        None => None,
    };
    let Some(repo) = repo else {
        state.info("Open a repository's wiki first");
        return Vec::new();
    };
    if state.overview(&repo).is_some_and(|o| !o.has_wiki) {
        state.info(format!("{repo} has its wiki turned off"));
        return Vec::new();
    }
    let mut cmds = state.push(Route::Wiki(wiki::WikiEdit { repo, page: None }));
    cmds.extend(follow(state));
    cmds
}

/// Brings the form on screen up to date: filled in from the page once
/// it's read, then the composer opens on it.
#[must_use]
pub fn follow(state: &mut State) -> Vec<Cmd> {
    let Some((route, repo, page, form)) = here(state) else {
        return Vec::new();
    };
    if form.started {
        return Vec::new();
    }
    let Some(w) = state
        .picked::<WikiPage>(&key(&repo, page.as_ref()))
        .cloned()
    else {
        return Vec::new();
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
    Vec::new()
}

/// The composer, on the page.
#[must_use]
pub fn compose(state: &mut State) -> Vec<Cmd> {
    let Some((route, _, page, form)) = here(state) else {
        return Vec::new();
    };
    if state.forms.sent(&route).sending {
        state.info("It's being saved");
        return Vec::new();
    }
    if page.is_some() && form.page.is_none() {
        state.info("The page hasn't loaded yet");
        return Vec::new();
    }
    let text = form.text.clone();
    state.forms.touch(&route);
    let cmds = state.compose(ComposeTarget::Write(Text::Wiki(route)), &text);
    if let Some(Overlay::Compose(c)) = &mut state.overlay {
        c.input.move_cursor(ratatui_textarea::CursorMove::Top);
    }
    cmds
}

pub fn keep_text(state: &mut State, route: &Route, text: String) {
    state.forms.edit::<Form>(route).text = text;
    state.forms.sent_mut(route).error = None;
}

/// The composer's title.
pub fn title(state: &State, route: &Route) -> String {
    let page = (state.forms.get::<Form>(route)).and_then(|f| f.page.as_ref());
    match page {
        Some((title, ..)) => format!("Edit the wiki page {title}"),
        None => "New wiki page: its title on the first line, then its text".to_owned(),
    }
}

/// The page to save, or why it can't be.
fn to_save(state: &State, repo: &RepoId, form: &Form) -> Result<crate::wiki::Save, String> {
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
            if title.is_empty() {
                return Err("Write the page's title first: it's the first line".into());
            }
            if title.contains(['/', '\\']) || title.chars().any(char::is_control) {
                return Err("A page's title can't have / or \\ in it".into());
            }
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
#[must_use]
pub fn submit(state: &mut State) -> Vec<Cmd> {
    let Some((route, repo, page, form)) = here(state) else {
        return Vec::new();
    };
    if state.forms.sent(&route).sending {
        say(state, "It's being saved".into());
        return Vec::new();
    }
    if page.is_some() && form.page.is_none() {
        say(state, "The page hasn't loaded yet".into());
        return Vec::new();
    }
    let save = match to_save(state, &repo, &form) {
        Ok(save) => save,
        Err(why) => {
            say(state, why);
            return Vec::new();
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
    fn done(self, state: &mut State, result: Result<String, Failure>) -> Vec<Cmd> {
        let route = &self.route;
        let Route::Wiki(wiki::WikiEdit { repo, .. }) = route else {
            return Vec::new();
        };
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
                    return Vec::new();
                }
                // Back on the page it was opened from, if it's that one.
                let below = state.screens.below().and_then(|s| match s {
                    crate::state::Screen::Page(p) => match &p.route {
                        Route::Wiki(wiki::Wiki { repo: r, page }) if r == repo => {
                            let title = (state
                                .picked::<WikiPage>(&DataKey::Wiki(r.clone(), page.clone())))
                            .and_then(|w| w.title.as_deref().map(crate::wiki::slug));
                            (title.as_deref() == Some(slug.as_str())).then(|| p.route.clone())
                        }
                        _ => None,
                    },
                    crate::state::Screen::Diff(_) => None,
                });
                let page = below.clone().unwrap_or_else(|| {
                    Route::Wiki(wiki::Wiki {
                        repo: repo.clone(),
                        page: Some(slug),
                    })
                });
                let Route::Wiki(wiki::Wiki { page: name, .. }) = &page else {
                    return Vec::new();
                };
                let shows = Saved(DataKey::Wiki(repo.clone(), name.clone()), text);
                crate::follow::expect(state, page.clone(), shows);
                if below.is_some() {
                    let mut cmds = state.back();
                    state.forward.clear();
                    cmds.extend(state.ensure_route(&page, true));
                    cmds
                } else {
                    state.replace(page, true, None)
                }
            }
            Err(why) => {
                crate::forms::failed(state, route, why.to_string());
                Vec::new()
            }
        }
    }
}

/// The menu's rows: editing the page on screen and making one, and the
/// form's own.
pub fn doables(state: &State, route: &Route, out: &mut Vec<Doable>) {
    if let Some((route, repo, _, form)) = here(state) {
        let busy = (state.forms.sent(&route).sending).then(|| "it's being saved".to_owned());
        out.push(row(
            "This page",
            Action::Comment,
            "Write the page",
            "write",
            busy.clone(),
        ));
        let why = busy.or_else(|| to_save(state, &repo, &form).err());
        out.push(row(
            "This page",
            Action::Create,
            "Save the page",
            "save",
            why,
        ));
        return;
    }
    if let Route::Wiki(wiki::Wiki { .. }) = route {
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
    let Route::Wiki(wiki::WikiEdit { repo, .. }) = route else {
        return;
    };
    let form = state.forms.shown::<Form>(route);
    let composing = matches!(
        &state.overlay,
        Some(Overlay::Compose(c)) if c.target == ComposeTarget::Write(Text::Wiki(route.clone()))
    );
    let (heading, title, body, hint) = match &form.page {
        Some((title, file, _)) => (
            format!("Editing {title} in {repo}'s wiki"),
            title.clone(),
            form.text.clone(),
            format!("Saved as {file}: committed and pushed with your git credentials"),
        ),
        None => {
            let (title, body) = templates::split(&form.text);
            (
                format!("New page in {repo}'s wiki"),
                title,
                body,
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
        ));
    }
    let write = state.first_key(Action::Comment);
    let save_key = state.first_key(Action::Create);
    let body = if composing { String::new() } else { body };
    let view = WriteFormView {
        heading,
        base: Some(crate::ui::markdown::LinkBase::wiki(repo)),
        rows: Vec::new(),
        status,
        boxes: vec![TextBox {
            title: if composing { String::new() } else { title },
            untitled: "Title",
            body: &body,
            markdown,
            empty: "Nothing written yet.",
            hint: &hint,
            link: Link::Comment,
            actions: Vec::new(),
        }],
        after: Vec::new(),
        write: &write,
        button: "Save page".into(),
        save_key: &save_key,
        why_not: to_save(state, repo, &form).err(),
        error: state.forms.sent(route).error.as_deref(),
    };
    let _ = WriteLink::Save;
    crate::ui::pages::write_form(page, &view);
}
