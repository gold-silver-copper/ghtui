//! Writes made on forms and pickers: making a release
//! ([`crate::release`]), your gists ([`crate::gist`]), a wiki's pages
//! ([`crate::wiki::form`]), and what you're subscribed to
//! ([`crate::watch`]). This module is what they share: their keys and
//! menu rows, their pickers and questions.

use crate::route::{Route, releases};

use crate::api::watch::Watched;
use crate::ui::overlays::PaletteItem;

use crate::data::DataKey;
use crate::state::{Cmd, Overlay, State};

// ---- the forms' text and rows -----------------------------------------------------

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
    pub fn start(&self, state: &State) -> Option<String> {
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
