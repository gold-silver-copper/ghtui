//! Writes made on forms and pickers: making a release
//! ([`crate::release`]), your gists ([`crate::gist`]), a wiki's pages
//! ([`crate::wiki::form`]), and what you're subscribed to
//! ([`crate::watch`]). This module is what they share: their keys and
//! menu rows, the composer's text on their forms, their pickers and
//! questions.

use crate::route::{Route, gists, releases, wiki};

use crate::api::watch::Watched;
use crate::ui::overlays::PaletteItem;
use crate::ui::page::{Link, WriteLink};

use crate::data::DataKey;
use crate::keymap::Action;
use crate::nav::Doable;
use crate::state::{Cmd, Overlay, State};

/// The actions these writes add.
pub const ACTIONS: [Action; 6] = [
    Action::Watch,
    Action::NewRelease,
    Action::UploadAsset,
    Action::NewGist,
    Action::NewWikiPage,
    Action::Lists,
];

// ---- where they work -------------------------------------------------------------

/// The forms' pages.
pub fn is_form(route: &Route) -> bool {
    matches!(
        route,
        Route::Releases(releases::NewRelease(_))
            | Route::Releases(releases::EditRelease { .. })
            | Route::Gists(gists::NewGist)
            | Route::Gists(gists::EditGist { .. })
            | Route::Wiki(wiki::WikiEdit { .. })
            | Route::Housekeeping(_)
    )
}

/// Whether `action` is these writes' here: their own actions anywhere,
/// and editing, deleting and making on their pages.
pub fn claims(state: &State, action: Action) -> bool {
    let route = state.route();
    ACTIONS.contains(&action)
        || match action {
            Action::EditComment => matches!(
                route,
                Some(
                    Route::Wiki(wiki::Wiki { .. })
                        | Route::Gists(gists::Gist { .. })
                        | Route::Releases(releases::Release { .. })
                )
            ),
            Action::DeleteComment => {
                matches!(
                    route,
                    Some(
                        Route::Gists(gists::Gist { .. })
                            | Route::Releases(releases::Release { .. })
                    )
                ) || crate::site::deletes_here(route)
            }
            Action::Create => route.is_some_and(is_form),
            _ => false,
        }
}

/// Runs one of the actions [`claims`] says are these writes'.
#[must_use]
pub fn act(state: &mut State, action: Action) -> Vec<Cmd> {
    let route = state.route().cloned();
    match (action, &route) {
        (Action::Watch, _) => crate::watch::open(state),
        (Action::NewRelease, _) => crate::release::open(state),
        (Action::UploadAsset, _) => crate::release::assets::open_upload(state),
        (Action::EditComment, Some(Route::Releases(releases::Release { .. }))) => {
            crate::release::open_edit(state)
        }
        (Action::DeleteComment, Some(Route::Releases(releases::Release { .. }))) => {
            crate::release::ask_delete(state)
        }
        (Action::NewGist, _) => crate::gist::open_new(state),
        (Action::NewWikiPage, _) => crate::wiki::form::open_new(state),
        (Action::EditComment, Some(Route::Wiki(wiki::Wiki { .. }))) => {
            crate::wiki::form::open_edit(state)
        }
        (Action::EditComment, Some(Route::Gists(gists::Gist { .. }))) => {
            crate::gist::open_edit(state)
        }
        (Action::DeleteComment, Some(Route::Gists(gists::Gist { .. }))) => {
            crate::gist::ask_delete(state)
        }
        (Action::Lists, _) => crate::site::lists::open(state),
        (Action::DeleteComment, Some(Route::Site(_))) => crate::site::ask_delete(state),
        (
            Action::Create,
            Some(
                Route::Releases(releases::NewRelease(_))
                | Route::Releases(releases::EditRelease { .. }),
            ),
        ) => crate::release::submit(state),
        (
            Action::Create,
            Some(Route::Gists(gists::NewGist) | Route::Gists(gists::EditGist { .. })),
        ) => crate::gist::submit(state),
        (Action::Create, Some(Route::Wiki(wiki::WikiEdit { .. }))) => {
            crate::wiki::form::submit(state)
        }
        (Action::Create, Some(Route::Housekeeping(_))) => crate::housekeeping::act(state, action),
        _ => {
            state.info(action.not_here());
            Vec::new()
        }
    }
}

/// The menu's rows for what changes GitHub, wherever you are.
pub fn change_doables(state: &State) -> Vec<Doable> {
    let mut out = Vec::from_iter(crate::watch::doable(state));
    out.extend(crate::site::lists::doable(state));
    out.extend(crate::housekeeping::change_doables(state));
    out
}

/// The menu's rows for the page on screen.
pub fn page_doables(state: &State, route: &Route, out: &mut Vec<Doable>) {
    crate::release::doables(state, route, out);
    crate::gist::doables(state, route, out);
    crate::wiki::form::doables(state, route, out);
    crate::site::doables(state, route, out);
    crate::housekeeping::doables(state, route, out);
}

/// A menu row.
pub(crate) fn row(
    section: &'static str,
    action: Action,
    label: impl Into<String>,
    short: &'static str,
    unavailable: Option<String>,
) -> Doable {
    Doable {
        section,
        action,
        label: label.into(),
        short,
        unavailable,
    }
}

// ---- the forms' text and rows -----------------------------------------------------

/// What the composer writes on one of these forms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Text {
    /// A release's title and notes, on its form's page.
    Release(Route),
    /// One of a gist's files, by its place in the form.
    GistFile(Route, usize),
    /// A wiki page's text (a new page's title first).
    Wiki(Route),
    /// A milestone's description, a file's text.
    Housekeeping(Route),
}

impl Text {
    /// The composer's title.
    pub fn title(&self, state: &State) -> String {
        match self {
            Text::Release(Route::Releases(releases::EditRelease { tag, .. })) => {
                format!("Editing {tag}: its title on the first line, then its notes")
            }
            Text::Release(_) => {
                "New release: its title on the first line, then its notes".to_owned()
            }
            Text::GistFile(route, i) => crate::gist::file_title(state, route, *i),
            Text::Wiki(route) => crate::wiki::form::title(state, route),
            Text::Housekeeping(route) => crate::housekeeping::compose_title(route),
        }
    }

    /// What ctrl-s does, for the composer's hint.
    pub fn save(&self) -> &'static str {
        match self {
            Text::Release(Route::Releases(releases::EditRelease { .. })) => "save…",
            Text::Release(_) => "publish…",
            Text::GistFile(Route::Gists(gists::NewGist), _) => "create gist…",
            Text::GistFile(..) => "save gist",
            Text::Wiki(_) => "save page",
            Text::Housekeeping(route) => crate::housekeeping::compose_save(route),
        }
    }

    /// The row of the page the composer opens in place of.
    pub fn anchor(&self) -> Link {
        match self {
            Text::GistFile(_, i) => Link::Write(WriteLink::File(*i)),
            Text::Release(_) | Text::Wiki(_) | Text::Housekeeping(_) => Link::Comment,
        }
    }

    pub fn route(&self) -> &Route {
        match self {
            Text::Release(route)
            | Text::GistFile(route, _)
            | Text::Wiki(route)
            | Text::Housekeeping(route) => route,
        }
    }
}

/// `c`, or Enter on a form's text: the composer on it.
#[must_use]
pub fn compose(state: &mut State) -> Vec<Cmd> {
    match state.route() {
        Some(
            Route::Releases(releases::NewRelease(_))
            | Route::Releases(releases::EditRelease { .. }),
        ) => crate::release::compose(state),
        Some(Route::Gists(gists::NewGist) | Route::Gists(gists::EditGist { .. })) => {
            let selected = match state.screen() {
                crate::state::Screen::Page(p) => p.selected_link().cloned(),
                crate::state::Screen::Diff(_) => None,
            };
            let file = match selected {
                Some(Link::Write(WriteLink::File(i) | WriteLink::Rename(i))) => i,
                _ => 0,
            };
            crate::gist::compose(state, file)
        }
        Some(Route::Wiki(wiki::WikiEdit { .. })) => crate::wiki::form::compose(state),
        Some(Route::Housekeeping(_)) => crate::housekeeping::compose(state),
        _ => Vec::new(),
    }
}

/// The composer's text, kept with its form (esc, or before saving).
pub fn keep_text(state: &mut State, text: &Text, written: String) {
    match text {
        Text::Release(route) => crate::release::keep_text(state, route, written),
        Text::GistFile(route, i) => crate::gist::keep_text(state, route, *i, written),
        Text::Wiki(route) => crate::wiki::form::keep_text(state, route, written),
        Text::Housekeeping(route) => crate::housekeeping::keep_text(state, route, written),
    }
}

/// ctrl-s in the composer: keeps its text, and saves the form.
#[must_use]
pub fn save(state: &mut State) -> Vec<Cmd> {
    if let Some(Overlay::Compose(c)) = &state.overlay
        && let crate::review::ComposeTarget::Write(text) = &c.target
    {
        let (text, written) = (text.clone(), c.text());
        keep_text(state, &text, written);
        if state.route() != Some(text.route()) {
            return Vec::new();
        }
    }
    act(state, Action::Create)
}

/// Enter on one of a form's rows.
#[must_use]
pub fn follow_link(state: &mut State, link: &WriteLink) -> Vec<Cmd> {
    if let WriteLink::Housekeeping(row) = link {
        return crate::housekeeping::follow_link(state, *row);
    }
    match state.route() {
        Some(
            Route::Releases(releases::NewRelease(_))
            | Route::Releases(releases::EditRelease { .. }),
        ) => crate::release::follow_link(state, link),
        Some(Route::Releases(releases::Release { .. })) => match link {
            WriteLink::Asset(name) => crate::release::assets::open(state, name),
            _ => Vec::new(),
        },
        Some(Route::Gists(gists::NewGist) | Route::Gists(gists::EditGist { .. })) => {
            crate::gist::follow_link(state, link)
        }
        Some(Route::Wiki(wiki::WikiEdit { .. })) => crate::wiki::form::submit(state),
        Some(Route::Housekeeping(_)) => crate::housekeeping::act(state, Action::Create),
        _ => Vec::new(),
    }
}

/// What Enter on one of a form's rows does, for the menu.
pub fn describe(link: &WriteLink) -> &'static str {
    match link {
        WriteLink::Tag => "Choose a tag, or make one",
        WriteLink::Target => "Choose the branch or commit to tag",
        WriteLink::Notes => "Have GitHub write the release notes",
        WriteLink::Prerelease => "Mark it a pre-release, or not",
        WriteLink::Latest => "Make it the latest release, or not",
        WriteLink::Description => "Write the gist's description",
        WriteLink::File(_) => "Write the file",
        WriteLink::Rename(_) => "Rename the file",
        WriteLink::Remove(_) => "Remove the file",
        WriteLink::AddFile => "Add a file",
        WriteLink::Save => "Save it",
        WriteLink::Asset(_) => "Save the asset",
        WriteLink::Housekeeping(row) => crate::housekeeping::describe(*row),
    }
}

// ---- pickers ------------------------------------------------------------------------

/// What one of these writes' pickers lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// How you're notified of it.
    Watch(Watched),
    /// A release's tag, on its form's page.
    Tag(Route),
    /// What a new tag is made at.
    Target(Route),
    /// Where to save a release's asset.
    SaveAsset(crate::release::assets::Of),
    /// A file to upload to a release.
    Upload(crate::release::assets::Of),
    /// The gist's description.
    Description(Route),
    /// A file's name (`None`: a new file's).
    FileName(Route, Option<usize>),
    /// Starred lists' ([`crate::site`]).
    Site(crate::site::Pick),
    Housekeeping(crate::housekeeping::Pick),
}

impl Pick {
    pub fn title(&self) -> &'static str {
        match self {
            Pick::Watch(Watched::Repo(_)) => "Watch the repository",
            Pick::Watch(_) => "Notifications",
            Pick::Tag(_) => "Choose a tag",
            Pick::Target(_) => "Target: a branch or commit",
            Pick::Description(_) => "Gist description",
            Pick::FileName(_, None) => "Add a file",
            Pick::FileName(..) => "Rename the file",
            Pick::Site(pick) => pick.title(),
            Pick::SaveAsset(_) => "Save the asset",
            Pick::Upload(_) => "Upload an asset",
            Pick::Housekeeping(pick) => pick.title(),
        }
    }

    pub fn placeholder(&self) -> &'static str {
        match self {
            Pick::Watch(_) => "Type to filter",
            Pick::Tag(_) => "Find a tag, or type a new one",
            Pick::Target(_) => "Find a branch, or type a commit's SHA",
            Pick::Description(_) => "Type the description",
            Pick::FileName(..) => "Type the file's name, with its extension",
            Pick::Site(pick) => pick.placeholder(),
            Pick::SaveAsset(_) => "A file or folder (↵ saves in the current folder)",
            Pick::Upload(_) => "A file's path (a folder lists its files)",
            Pick::Housekeeping(pick) => pick.placeholder(),
        }
    }

    /// What it lists, to load when it opens.
    pub fn need(&self) -> Option<DataKey> {
        match self {
            Pick::Watch(w) => Some(DataKey::Subscription(w.clone())),
            Pick::Tag(route) => route.repo().map(|r| DataKey::Tags(r.clone())),
            Pick::Target(route) => route.repo().map(|r| DataKey::Branches(r.clone())),
            Pick::Description(_) | Pick::FileName(..) | Pick::SaveAsset(_) | Pick::Upload(_) => {
                None
            }
            Pick::Site(pick) => pick.need(),
            Pick::Housekeeping(pick) => pick.need(),
        }
    }

    /// The text the field starts with.
    fn start(&self, state: &State) -> Option<String> {
        match self {
            Pick::Description(route) => crate::gist::description(state, route),
            Pick::FileName(route, Some(i)) => crate::gist::file_name(state, route, *i),
            Pick::Housekeeping(pick) => pick.start(state),
            _ => None,
        }
    }
}

/// What choosing a row does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Chosen {
    Subscribe(crate::watch::Subscribe),
    Tag(Route, String),
    Target(Route, String),
    SaveAsset {
        of: crate::release::assets::Of,
        path: std::path::PathBuf,
        replace: bool,
    },
    Upload {
        of: crate::release::assets::Of,
        path: std::path::PathBuf,
    },
    Description(Route, String),
    FileName(Route, Option<usize>, String),
    Site(crate::site::Chosen),
    Housekeeping(crate::housekeeping::Chosen),
}

/// Opens one of these pickers, its field holding what's set now.
#[must_use]
pub fn open_picker(state: &mut State, pick: Pick) -> Vec<Cmd> {
    let start = pick.start(state);
    let cmds = state.open_picker(crate::picker::Kind::Write(pick));
    if let (Some(start), Some(Overlay::Picker(p))) = (start, &mut state.overlay) {
        p.input.insert_str(start);
    }
    cmds
}

/// A picker's rows: what each says, and what choosing it does.
pub fn rows(state: &State, pick: &Pick, q: &str) -> Vec<(PaletteItem, Option<Chosen>)> {
    match pick {
        Pick::Watch(w) => crate::watch::rows(state, w, q),
        Pick::Tag(route) => crate::release::tag_rows(state, route, q),
        Pick::Target(route) => crate::release::target_rows(state, route, q),
        Pick::SaveAsset(of) => crate::release::assets::save_rows(of, q),
        Pick::Upload(of) => crate::release::assets::upload_rows(state, of, q),
        Pick::Description(route) => crate::gist::description_rows(route, q),
        Pick::FileName(route, file) => crate::gist::name_rows(state, route, *file, q),
        Pick::Site(pick) => crate::site::rows(state, pick, q),
        Pick::Housekeeping(pick) => crate::housekeeping::rows(state, pick, q),
    }
}

/// A picker's row.
pub(crate) fn item(
    label: impl Into<String>,
    hint: impl Into<String>,
    chosen: Option<Chosen>,
) -> (PaletteItem, Option<Chosen>) {
    let item = PaletteItem {
        label: label.into(),
        hint: hint.into(),
    };
    (item, chosen)
}

#[must_use]
pub fn choose(state: &mut State, chosen: Chosen) -> Vec<Cmd> {
    match chosen {
        Chosen::Subscribe(work) => crate::watch::choose(state, work),
        Chosen::Tag(route, tag) => crate::release::choose_tag(state, &route, tag),
        Chosen::Target(route, target) => crate::release::choose_target(state, &route, target),
        chosen @ (Chosen::SaveAsset { .. } | Chosen::Upload { .. }) => {
            crate::release::assets::choose(state, chosen)
        }
        Chosen::Description(route, text) => crate::gist::set_description(state, &route, text),
        Chosen::FileName(route, file, name) => crate::gist::set_name(state, &route, file, name),
        Chosen::Site(chosen) => crate::site::choose(state, chosen),
        Chosen::Housekeeping(chosen) => crate::housekeeping::choose(state, chosen),
    }
}

// ---- questions -------------------------------------------------------------------

/// The answers to these writes' questions, asked in [`crate::create::Ask`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Release {
        draft: bool,
    },
    DeleteRelease,
    /// The deleted release's tag too.
    DeleteTag,
    KeepTag,
    /// The release's asset of this name.
    DeleteAsset(String),
    Gist {
        public: bool,
    },
    DeleteGist,
    Site(crate::site::Answer),
    Housekeeping(crate::housekeeping::Answer),
}

impl Answer {
    /// What's happening while GitHub answers.
    pub fn busy(&self) -> &'static str {
        match self {
            Answer::Release { draft: true } => "Saving the draft…",
            Answer::Release { .. } => "Saving the release…",
            Answer::DeleteRelease => "Deleting the release…",
            Answer::DeleteTag => "Deleting the tag…",
            Answer::KeepTag => "",
            Answer::DeleteAsset(_) => "Deleting the asset…",
            Answer::Gist { .. } => "Creating the gist…",
            Answer::DeleteGist => "Deleting the gist…",
            Answer::Site(a) => a.busy(),
            Answer::Housekeeping(a) => a.busy(),
        }
    }
}

/// The question about `route` was answered.
#[must_use]
pub fn answer(state: &mut State, route: &Route, answer: Answer) -> Vec<Cmd> {
    match answer {
        Answer::Release { draft } => crate::release::create(state, route, draft),
        Answer::DeleteRelease => crate::release::delete(state, route),
        Answer::DeleteTag => crate::release::delete_tag(state, route),
        Answer::KeepTag => {
            crate::forms::close_sender(state, route);
            if let Route::Releases(releases::Release { tag, .. }) = route {
                state.info(format!("Kept the tag {tag}"));
            }
            Vec::new()
        }
        Answer::DeleteAsset(name) => crate::release::assets::delete(state, &name),
        Answer::Gist { public } => crate::gist::create(state, route, public),
        Answer::DeleteGist => crate::gist::delete(state, route),
        Answer::Site(a) => crate::site::answer(state, route, a),
        Answer::Housekeeping(a) => crate::housekeeping::answer(state, route, a),
    }
}

/// Asks `title` about `route`, with what's worth knowing and the ways to
/// go ahead.
pub(crate) fn ask(
    state: &mut State,
    route: Route,
    title: String,
    facts: Vec<(String, bool)>,
    choices: Vec<(String, Answer)>,
) {
    let choices = (choices.into_iter())
        .map(|(label, a)| (label, crate::create::Answer::Write(a)))
        .collect();
    let ask = crate::create::Ask::new(route, title, facts, choices);
    state.overlay = Some(Overlay::Ask(Box::new(ask)));
}

/// After GitHub's data arrives: brings the form on screen up to date.
#[must_use]
pub fn follow(state: &mut State) -> Vec<Cmd> {
    let mut cmds = crate::release::follow(state);
    cmds.extend(crate::gist::follow(state));
    cmds.extend(crate::wiki::form::follow(state));
    cmds.extend(crate::housekeeping::follow(state));
    cmds
}
