//! Writes that used to need the browser: making a release
//! ([`crate::release`]), your gists ([`crate::gist`]), a wiki's pages
//! ([`crate::wiki::form`]), and what you're subscribed to
//! ([`crate::watch`]). This module is what they share: their keys and
//! menu rows, the composer's text on their forms, their pickers and
//! questions, the work they do in the background, and following GitHub
//! until the page shows what changed, as [`crate::act`] does for its
//! changes.

use crate::route::gists;
use crate::route::releases;
use crate::route::wiki;
use std::collections::HashMap;

use crate::api::GitHub;
use crate::api::model::RepoId;
use crate::api::watch::{SubscriptionState, Watched};
use crate::ui::overlays::PaletteItem;
use crate::ui::page::{Link, WriteLink};

use crate::data::DataKey;
use crate::diff_job::GitContext;
use crate::keymap::Action;
use crate::nav::Doable;
use crate::route::Route;
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

/// The forms being written, and the change GitHub accepted that the page
/// on screen doesn't show yet.
#[derive(Debug, Default)]
pub struct Writing {
    /// New and edited releases, by their form's page: leaving the form
    /// keeps it.
    pub releases: HashMap<Route, crate::release::Form>,
    /// New and edited gists, by their form's page.
    pub gists: HashMap<Route, crate::gist::Form>,
    /// New and edited wiki pages, by their form's page.
    pub wikis: HashMap<Route, crate::wiki::form::Form>,
    /// Labels, milestones, branches and tags, forks, new repositories
    /// and files.
    pub housekeeping: crate::housekeeping::Housekeeping,
    pub expect: Option<Expect>,
}

/// Work in the background.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Work {
    Release(crate::release::Work),
    Gist(crate::gist::Work),
    /// Commit and push a wiki page; the form's page asked.
    Wiki(Route, crate::wiki::Save),
    Watch(crate::watch::Work),
    /// A revert, a transfer, a new discussion, a branch checked out, and
    /// what their pickers list ([`crate::manage`]).
    Manage(crate::manage::Work),
    /// Starred lists and packages ([`crate::site`]).
    Site(crate::site::Work),
    Housekeeping(crate::housekeeping::Work),
}

/// Work's answer.
#[derive(Debug)]
pub enum Done {
    Release(crate::release::Done),
    Gist(crate::gist::Done),
    /// The page's title as its URL names it, or why it wasn't saved.
    Wiki(Route, Result<String, String>),
    Watch(crate::watch::Done),
    Manage(crate::manage::Done),
    Site(crate::site::Done),
    Housekeeping(Box<crate::housekeeping::Done>),
}

impl Done {
    /// Whether GitHub changed: what any page shows may be old.
    pub fn wrote(&self) -> bool {
        match self {
            Done::Release(d) => d.wrote(),
            Done::Gist(d) => d.wrote(),
            Done::Wiki(_, result) => result.is_ok(),
            Done::Watch(d) => d.wrote(),
            Done::Manage(d) => d.wrote(),
            Done::Site(d) => d.wrote(),
            Done::Housekeeping(d) => d.wrote(),
        }
    }
}

impl Work {
    /// Its answer, had it failed: for when the task running it panics.
    pub fn failed(&self) -> Done {
        let bug = "ghtui hit a bug".to_owned();
        match self {
            Work::Release(w) => Done::Release(w.failed(bug)),
            Work::Gist(w) => Done::Gist(w.failed(bug)),
            Work::Wiki(route, _) => Done::Wiki(route.clone(), Err(bug)),
            Work::Watch(w) => Done::Watch(w.failed(bug)),
            Work::Manage(w) => Done::Manage(w.failed(bug)),
            Work::Site(w) => Done::Site(w.failed(bug)),
            Work::Housekeeping(w) => Done::Housekeeping(Box::new(w.failed(bug))),
        }
    }

    pub async fn run(self, gh: &GitHub, git: &GitContext) -> Done {
        match self {
            Work::Release(w) => Done::Release(w.run(gh).await),
            Work::Gist(w) => Done::Gist(w.run(gh).await),
            Work::Wiki(route, save) => Done::Wiki(route, crate::wiki::save(git, &save).await),
            Work::Watch(w) => Done::Watch(w.run(gh).await),
            Work::Manage(w) => Done::Manage(w.run(gh, &git.cwd).await),
            Work::Site(w) => Done::Site(w.run(gh).await),
            Work::Housekeeping(w) => Done::Housekeeping(Box::new(w.run(gh).await)),
        }
    }
}

/// Work's answers.
#[must_use]
pub fn on_done(state: &mut State, done: Done) -> Vec<Cmd> {
    match done {
        Done::Release(d) => crate::release::on_done(state, d),
        Done::Gist(d) => crate::gist::on_done(state, d),
        Done::Wiki(route, result) => crate::wiki::form::on_saved(state, &route, result),
        Done::Watch(d) => crate::watch::on_done(state, d),
        Done::Manage(d) => crate::manage::on_done(state, d),
        Done::Site(d) => crate::site::on_done(state, d),
        Done::Housekeeping(d) => crate::housekeeping::on_done(state, *d),
    }
}

/// GitHub's reason, as the status bar says it.
pub fn reason(err: &crate::api::ApiError) -> String {
    use crate::api::ApiError;
    match err {
        ApiError::GraphQl(messages) => messages.join("; "),
        ApiError::Http { message, .. } => message.clone(),
        err => err.to_string(),
    }
}

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

    fn route(&self) -> Route {
        match self {
            Text::Release(route)
            | Text::GistFile(route, _)
            | Text::Wiki(route)
            | Text::Housekeeping(route) => route.clone(),
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
    state.data_gen += 1;
}

/// ctrl-s in the composer: keeps its text, and saves the form.
#[must_use]
pub fn save(state: &mut State) -> Vec<Cmd> {
    if let Some(Overlay::Compose(c)) = &state.overlay
        && let crate::review::ComposeTarget::Write(text) = &c.target
    {
        let (text, written) = (text.clone(), c.text());
        keep_text(state, &text, written);
        if state.route() != Some(&text.route()) {
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

/// Says `text` where it'll be seen: in the composer or the question when
/// one's open, else in the status bar.
pub(crate) fn say(state: &mut State, text: String) {
    match &mut state.overlay {
        Some(Overlay::Compose(c)) => {
            c.sending = false;
            c.error = Some(text);
        }
        Some(Overlay::Ask(a)) => {
            a.sending = false;
            a.error = Some(text);
        }
        _ => state.error(text),
    }
}

/// Marks the composer or the question that sent something as waiting
/// for GitHub.
pub(crate) fn sending(state: &mut State) {
    match &mut state.overlay {
        Some(Overlay::Compose(c)) => {
            c.sending = true;
            c.error = None;
        }
        Some(Overlay::Ask(a)) => {
            a.sending = true;
            a.error = None;
        }
        _ => {}
    }
    state.data_gen += 1;
}

/// Closes the composer or question that sent something to `route`'s
/// form, once it's done.
pub(crate) fn close_sender(state: &mut State, route: &Route) {
    let ours = match &state.overlay {
        Some(Overlay::Compose(c)) => {
            matches!(&c.target, crate::review::ComposeTarget::Write(t) if t.route() == *route)
        }
        Some(Overlay::Ask(a)) => a.route == *route,
        _ => false,
    };
    if ours {
        state.overlay = None;
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
    Subscribe(crate::watch::Work),
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
            close_sender(state, route);
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
    state.overlay = Some(Overlay::Ask(Box::new(crate::create::Ask {
        route,
        title,
        facts,
        choices,
        selected: 0,
        sending: false,
        error: None,
    })));
}

// ---- following GitHub ----------------------------------------------------------------

/// How many times a page is fetched again for a change before giving up:
/// a few minutes, at the live refresh's pace, as [`crate::act`] does.
const POLLS: u32 = 40;

/// A change GitHub accepted that the page it went to doesn't show yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expect {
    pub route: Route,
    pub what: Expected,
    polls: u32,
}

/// What the page shows once GitHub has the change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expected {
    /// A published release's page.
    Release(RepoId, String),
    /// A release's page, as it was edited.
    Edited {
        repo: RepoId,
        tag: String,
        name: String,
        notes: String,
        prerelease: bool,
    },
    /// A release no longer among the releases.
    ReleaseGone(RepoId, String),
    /// A release's page with its asset `name` (or without it).
    Asset {
        repo: RepoId,
        tag: String,
        name: String,
        there: bool,
    },
    /// A draft among the releases.
    Draft(RepoId, String),
    /// A gist with this description and these files' names and text.
    Gist {
        id: String,
        description: String,
        files: Vec<(String, String)>,
    },
    /// A gist no longer among its owner's.
    GistGone(String, String),
    Subscription(Watched, SubscriptionState),
    /// A wiki page with this text, as its page's data is keyed.
    Wiki(DataKey, String),
    /// Starred lists' and packages' ([`crate::site`]).
    Site(crate::site::Expected),
    Housekeeping(crate::housekeeping::Expected),
}

impl Expected {
    /// What's fetched again to see it.
    fn needs(&self, state: &State, route: &Route) -> Vec<DataKey> {
        match self {
            Expected::Subscription(w, _) => vec![DataKey::Subscription(w.clone())],
            Expected::Site(e) => e.needs(),
            _ => state.needs(route),
        }
    }

    /// What the status bar says meanwhile.
    fn waiting(&self) -> &'static str {
        match self {
            Expected::Release(..) | Expected::Draft(..) | Expected::Edited { .. } => {
                "Waiting for GitHub to show the release"
            }
            Expected::ReleaseGone(..) => "Waiting for GitHub to show the release gone",
            Expected::Asset { there: true, .. } => "Waiting for GitHub to show the asset",
            Expected::Asset { .. } => "Waiting for GitHub to show the asset gone",
            Expected::Gist { .. } => "Waiting for GitHub to show the gist",
            Expected::GistGone(..) => "Waiting for GitHub to show the gist gone",
            Expected::Subscription(..) => "Waiting for GitHub to show the subscription",
            Expected::Wiki(..) => "Waiting for GitHub to show the page",
            Expected::Site(e) => e.waiting(),
            Expected::Housekeeping(e) => e.waiting(),
        }
    }

    fn shown(&self, state: &State) -> bool {
        use crate::api::browse::{Gist, GistSummary, Release, Results, WikiPage};
        match self {
            Expected::Release(repo, tag) => {
                (state.picked::<Release>(&DataKey::Release(repo.clone(), tag.clone()))).is_some()
            }
            Expected::Edited {
                repo,
                tag,
                name,
                notes,
                prerelease,
            } => (state.picked::<Release>(&DataKey::Release(repo.clone(), tag.clone())))
                .is_some_and(|r| {
                    r.name == *name
                        && r.prerelease == *prerelease
                        && r.notes.as_deref().unwrap_or_default().trim() == notes.trim()
                }),
            Expected::ReleaseGone(repo, tag) => (state
                .picked::<Results<Release>>(&DataKey::Releases(repo.clone())))
            .is_some_and(|r| r.items.iter().all(|r| r.tag != *tag)),
            Expected::Asset {
                repo,
                tag,
                name,
                there,
            } => crate::release::assets::shows(state, repo, tag, name, *there),
            Expected::Draft(repo, tag) => (state
                .picked::<Results<Release>>(&DataKey::Releases(repo.clone())))
            .is_some_and(|r| r.items.iter().any(|r| r.tag == *tag && r.draft)),
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
            Expected::Subscription(w, want) => state
                .picked::<crate::api::watch::Subscription>(&DataKey::Subscription(w.clone()))
                .is_some_and(|s| s.state == Some(*want)),
            Expected::Wiki(key, text) => state
                .picked::<WikiPage>(key)
                .is_some_and(|w| w.text.as_deref().map(str::trim_end) == Some(text.trim_end())),
            Expected::Site(e) => e.shown(state),
            Expected::Housekeeping(e) => e.shown(state),
        }
    }
}

/// Waits, on `route`, for GitHub to show `what`.
pub(crate) fn expect(state: &mut State, route: Route, what: Expected) {
    state.writing.expect = Some(Expect {
        route,
        what,
        polls: 0,
    });
}

/// The change the page on screen waits to see, if any.
fn expect_here(state: &State) -> Option<&Expect> {
    (state.writing.expect.as_ref()).filter(|e| state.route() == Some(&e.route))
}

/// Whether the page on screen waits to see a change.
pub fn awaiting(state: &State) -> bool {
    expect_here(state).is_some()
}

/// What the status bar says while GitHub hasn't shown a change.
pub fn waiting(state: &State) -> Option<String> {
    expect_here(state).map(|e| e.what.waiting().to_owned())
}

/// What the live refresh fetches again for the change the page waits to
/// see. Past [`POLLS`], it stops waiting.
pub fn poll(state: &mut State) -> Vec<DataKey> {
    let Some(needs) = expect_here(state).map(|e| e.what.needs(state, &e.route)) else {
        return Vec::new();
    };
    if let Some(e) = &mut state.writing.expect {
        e.polls += 1;
        if e.polls > POLLS {
            state.writing.expect = None;
            let key = state.first_key(Action::Refresh);
            state.info(format!(
                "GitHub doesn't show the change yet: {key} fetches the page again"
            ));
            return Vec::new();
        }
    }
    needs
}

/// After GitHub's data arrives: stops waiting for a change the page now
/// shows, and brings the form on screen up to date.
#[must_use]
pub fn follow(state: &mut State) -> Vec<Cmd> {
    if let Some(e) = expect_here(state)
        && e.what.shown(state)
    {
        state.writing.expect = None;
    }
    let mut cmds = crate::release::follow(state);
    cmds.extend(crate::gist::follow(state));
    cmds.extend(crate::wiki::form::follow(state));
    cmds.extend(crate::housekeeping::follow(state));
    cmds
}
