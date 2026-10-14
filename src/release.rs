//! Making a release, as GitHub's "Draft a new release" does, editing
//! one, and deleting one (then, asked again, its tag).
//!
//! The form is a page (`releases::NewRelease`, or `releases::EditRelease`
//! filled from the release): a tag (one GitHub has, or a new one GitHub
//! makes at a branch or commit when the release is published), a title
//! and notes written in the composer (the title on the first line),
//! GitHub's generated notes offered as a start, and whether it's a
//! pre-release or the latest. Saving asks whether to publish it or keep
//! it a draft, as GitHub's two buttons do; then the release's page (a
//! draft's: the releases) follows GitHub until it shows it. A release's
//! assets are [`assets`]'.

pub mod assets;

use std::borrow::Cow;

use crate::api::browse::{BranchInfo, Release, Results, TagInfo};
use crate::api::model::RepoId;
use crate::api::release::{Made, NewRelease, Notes, TagLook};
use crate::create::templates;
use crate::route::{Route, releases};
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
use crate::picker::fuzzy_score;
use crate::picker::{Choice, entry};
use crate::state::{Cmd, State};
use crate::work::{self, Cx, Failure, Job};

/// A release, as written so far: a new one, or one being edited.
#[derive(Debug, Clone)]
pub struct Form {
    /// The title on the first line, then the notes.
    pub text: String,
    /// The text GitHub's notes filled in: while it's still that,
    /// generating again replaces it.
    pub filled: String,
    /// The tag picker has opened on it (a new release), or it's been
    /// filled from the release (an edit).
    pub started: bool,
    pub tag: Option<String>,
    /// What a new tag is made at: a branch or a commit (`None`: the
    /// default branch).
    pub target: Option<String>,
    pub prerelease: bool,
    pub latest: bool,
    /// The release this edits, if it does.
    pub editing: Option<Editing>,
    /// GitHub is writing the notes.
    pub generating: bool,
}

impl Fields for Form {
    fn text(&mut self, _: usize) -> Option<&mut String> {
        Some(&mut self.text)
    }

    fn title(&self, route: &Route, _: usize) -> String {
        match route {
            Route::Releases(releases::EditRelease { tag, .. }) => {
                format!("Editing {tag}: its title on the first line, then its notes")
            }
            _ => "New release: its title on the first line, then its notes".into(),
        }
    }

    fn save(&self, route: &Route) -> &'static str {
        match route {
            Route::Releases(releases::EditRelease { .. }) => "save…",
            _ => "publish…",
        }
    }
}

/// The release a form edits, as it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Editing {
    /// REST's ID for it.
    pub id: u64,
    pub tag: String,
    pub draft: bool,
}

impl Default for Form {
    fn default() -> Self {
        Self {
            text: String::new(),
            filled: String::new(),
            started: false,
            tag: None,
            target: None,
            prerelease: false,
            latest: true,
            editing: None,
            generating: false,
        }
    }
}

impl Form {
    /// A form editing `r`, with what it has.
    fn of(r: &Release, id: u64) -> Self {
        let text = templates::join(&r.name, r.notes.as_deref().unwrap_or_default());
        Self {
            text,
            started: true,
            tag: Some(r.tag.clone()),
            prerelease: r.prerelease,
            latest: r.latest,
            editing: Some(Editing {
                id,
                tag: r.tag.clone(),
                draft: r.draft,
            }),
            ..Self::default()
        }
    }

    /// What GitHub has of the tag, once it's said: an edit's own tag is
    /// its release's.
    fn tag_look<'a>(
        &self,
        state: &'a State,
        repo: &RepoId,
    ) -> Option<Result<&'a TagLook, &'a str>> {
        static OWN: TagLook = TagLook {
            exists: true,
            release: None,
        };
        if self.own_tag() {
            return Some(Ok(&OWN));
        }
        state.answer(&DataKey::TagLook(repo.clone(), self.tag.clone()?))
    }

    /// The tag is a new one: GitHub makes it at the target.
    fn new_tag(&self, state: &State, repo: &RepoId) -> bool {
        matches!(
            self.tag_look(state, repo),
            Some(Ok(TagLook { exists: false, .. }))
        )
    }

    /// The tag's release is the one this edits.
    fn own_tag(&self) -> bool {
        (self.editing.as_ref()).is_some_and(|e| self.tag.as_ref() == Some(&e.tag))
    }
}

/// GitHub's notes for a release of the tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Generate {
    pub repo: RepoId,
    pub tag: String,
    pub target: Option<String>,
}

/// Makes the release written on the form at `route`, or (with an ID)
/// changes that release to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Save {
    pub route: Route,
    pub id: Option<u64>,
    pub new: NewRelease,
}

/// Deletes a release; its answer says whether its tag is on GitHub (a
/// draft's new tag never was).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delete {
    pub repo: RepoId,
    pub tag: String,
    pub id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteTag {
    pub repo: RepoId,
    pub tag: String,
}

/// What a release's page, or the releases, show once GitHub has a change.
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
}

impl Expectation for Expected {
    fn waiting(&self) -> &'static str {
        match self {
            Expected::Release(..) | Expected::Draft(..) | Expected::Edited { .. } => {
                "Waiting for GitHub to show the release"
            }
            Expected::ReleaseGone(..) => "Waiting for GitHub to show the release gone",
            Expected::Asset { there: true, .. } => "Waiting for GitHub to show the asset",
            Expected::Asset { .. } => "Waiting for GitHub to show the asset gone",
        }
    }

    fn shown(&self, state: &State, _: &Route) -> Shown {
        let release = |repo: &RepoId, tag: &String| {
            state.picked::<Release>(&DataKey::Release(repo.clone(), tag.clone()))
        };
        let releases =
            |repo: &RepoId| state.picked::<Results<Release>>(&DataKey::Releases(repo.clone()));
        match self {
            Expected::Release(repo, tag) => release(repo, tag).is_some(),
            Expected::Edited {
                repo,
                tag,
                name,
                notes,
                prerelease,
            } => release(repo, tag).is_some_and(|r| {
                r.name == *name
                    && r.prerelease == *prerelease
                    && r.notes.as_deref().unwrap_or_default().trim() == notes.trim()
            }),
            Expected::ReleaseGone(repo, tag) => {
                releases(repo).is_some_and(|r| r.items.iter().all(|r| r.tag != *tag))
            }
            Expected::Asset {
                repo,
                tag,
                name,
                there,
            } => assets::shows(state, repo, tag, name, *there),
            Expected::Draft(repo, tag) => {
                releases(repo).is_some_and(|r| r.items.iter().any(|r| r.tag == *tag && r.draft))
            }
        }
        .into()
    }
}

/// The release form on screen: its page, its repository, and the form
/// (blank until it's started).
fn here(state: &State) -> Option<(Route, RepoId, Cow<'_, Form>)> {
    let route = state.route()?;
    let repo = form_repo(route)?;
    Some((route.clone(), repo.clone(), state.forms.shown(route)))
}

/// What releases add: making one wherever a repository is, and editing,
/// deleting and uploading to the one on screen.
pub const FEATURE: crate::features::Feature = crate::features::Feature {
    owns: &[Action::NewRelease, Action::UploadAsset],
    takes: |state, action| {
        matches!(action, Action::EditComment | Action::DeleteComment)
            && matches!(
                state.route(),
                Some(Route::Releases(releases::Release { .. }))
            )
    },
    act: |state, action| match action {
        Action::NewRelease => open(state),
        Action::UploadAsset => assets::open_upload(state),
        Action::EditComment => open_edit(state),
        _ => ask_delete(state),
    },
    offers: doables,
    ..crate::features::Feature::NONE
};

/// Why `repo` takes no release from you (nor changes to one), from
/// what's known.
pub(crate) fn refused(state: &State, repo: &RepoId) -> Option<String> {
    let o = state.overview(repo)?;
    if o.summary.archived {
        return Some(format!("{repo} is archived: it takes nothing new"));
    }
    (!o.can_write).then(|| format!("Releases take write access to {repo}"))
}

/// `,V`: the repository's new release, with what was written before.
#[must_use]
pub fn open(state: &mut State) -> Vec<Cmd> {
    let Some(repo) = state.route().and_then(Route::repo).cloned() else {
        return state.just_say("Open a repository first: a release is of one");
    };
    if let Some(why) = refused(state, &repo) {
        return state.just_say(why);
    }
    let route = Route::Releases(releases::NewRelease(repo));
    if state.route() == Some(&route) {
        return compose(state);
    }
    let mut cmds = state.push(route);
    cmds.extend(follow(state));
    cmds
}

/// The release on screen (its page's), as loaded.
pub(crate) fn shown(state: &State) -> Option<(RepoId, String, &Release)> {
    let Some(Route::Releases(releases::Release { repo, tag })) = state.route() else {
        return None;
    };
    let r = state.picked::<Release>(&DataKey::Release(repo.clone(), tag.clone()))?;
    Some((repo.clone(), tag.clone(), r))
}

/// Why the release on screen can't be changed, if it can't.
pub(crate) fn unchangeable(state: &State) -> Option<String> {
    let Some((repo, _, r)) = shown(state) else {
        return Some("The release hasn't loaded yet".into());
    };
    if let Some(why) = refused(state, &repo) {
        return Some(why);
    }
    r.id.is_none()
        .then(|| "GitHub didn't say the release's ID, which changing it takes".into())
}

/// `,e` on a release's page: the form editing it.
#[must_use]
pub fn open_edit(state: &mut State) -> Vec<Cmd> {
    if let Some(why) = unchangeable(state) {
        return state.just_say(why);
    }
    let Some((repo, tag, _)) = shown(state) else {
        return Vec::new();
    };
    let mut cmds = state.push(Route::Releases(releases::EditRelease { repo, tag }));
    cmds.extend(follow(state));
    cmds
}

/// Brings the form on screen up to date: fills an edit's from its
/// release, starts a new one by choosing its tag, and asks GitHub about
/// a tag it hasn't yet.
#[must_use]
fn follow(state: &mut State) -> Vec<Cmd> {
    let Some((route, repo, form)) = here(state) else {
        return Vec::new();
    };
    if let Route::Releases(releases::EditRelease { tag, .. }) = &route
        && !form.started
        && let Some(r) = state.picked::<Release>(&DataKey::Release(repo.clone(), tag.clone()))
        && let Some(id) = r.id
    {
        *state.forms.edit(&route) = Form::of(r, id);
    }
    let mut cmds = Vec::new();
    let loaded = state.overview(&repo).is_some();
    let form = state.forms.shown::<Form>(&route);
    let (started, tag) = (form.started, form.tag.clone().filter(|_| !form.own_tag()));
    if let Some(tag) = tag {
        cmds.extend(state.ensure(DataKey::TagLook(repo.clone(), tag), false));
    }
    let new = matches!(route, Route::Releases(releases::NewRelease(_)));
    if new && loaded && !started && state.overlay.is_none() && refused(state, &repo).is_none() {
        state.forms.edit::<Form>(&route).started = true;
        cmds.extend(state.open_picker(crate::picker::Kind::Tag(route)));
    }
    cmds
}

/// The composer, on the title and notes.
#[must_use]
fn compose(state: &mut State) -> Vec<Cmd> {
    let Some((route, ..)) = here(state) else {
        return Vec::new();
    };
    crate::forms::compose::<Form>(state, &route, 0, crate::forms::TITLE)
}

/// What the release form does.
pub const FORM: crate::forms::Kind = crate::forms::Kind {
    compose: |state, _| compose(state),
    row: on_row,
    submit,
    follow,
    offers,
};

/// Enter on one of the form's rows.
#[must_use]
fn on_row(state: &mut State, link: &Row) -> Vec<Cmd> {
    let Some((route, repo, form)) = here(state) else {
        return Vec::new();
    };
    match link {
        Row::Tag => state.open_picker(crate::picker::Kind::Tag(route)),
        Row::Target if form.new_tag(state, &repo) => {
            state.open_picker(crate::picker::Kind::Target(route))
        }
        Row::Target => state.just_say("The tag is GitHub's already: it's where it is"),
        Row::Notes => generate(state),
        Row::Prerelease => {
            let form = state.forms.edit::<Form>(&route);
            form.prerelease = !form.prerelease;
            Vec::new()
        }
        Row::Latest if form.prerelease => {
            state.just_say("A pre-release is never the latest release")
        }
        Row::Latest => {
            let form = state.forms.edit::<Form>(&route);
            form.latest = !form.latest;
            Vec::new()
        }
        _ => Vec::new(),
    }
}

/// Asks GitHub to write the notes.
#[must_use]
fn generate(state: &mut State) -> Vec<Cmd> {
    let Some((route, repo, form)) = here(state) else {
        return Vec::new();
    };
    let Some(tag) = form.tag.clone() else {
        return state
            .just_say("Choose a tag first: GitHub writes the notes since the release before it");
    };
    if form.generating {
        return Vec::new();
    }
    let target = form.target.clone().filter(|_| form.new_tag(state, &repo));
    state.forms.edit::<Form>(&route).generating = true;
    state.info("Asking GitHub for release notes…");
    work::start(Generate { repo, tag, target })
}

/// Whether `tag` can be a tag's name, as git's `check-ref-format` has it,
/// or why not.
fn tag_problem(tag: &str) -> Option<&'static str> {
    let bad = |c: char| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(c);
    if tag.is_empty() {
        Some("A tag needs a name")
    } else if tag.chars().any(bad) || tag.contains("..") || tag.contains("@{") {
        Some("A tag's name can't have spaces, .., or any of ~^:?*[\\")
    } else if tag.starts_with(['/', '.', '-'])
        || tag.ends_with(['/', '.'])
        || tag.ends_with(".lock")
    {
        Some("A tag's name can't start with / . or -, or end with / . or .lock")
    } else {
        None
    }
}

/// The repository a form's page is of.
fn form_repo(route: &Route) -> Option<&RepoId> {
    match route {
        Route::Releases(releases::NewRelease(repo))
        | Route::Releases(releases::EditRelease { repo, .. }) => Some(repo),
        _ => None,
    }
}

/// The tag picker's rows: a new tag typed, then the tags GitHub has.
pub fn tag_rows(state: &State, route: &Route, q: &str) -> Vec<(PaletteItem, Option<Choice>)> {
    let Some(repo) = form_repo(route) else {
        return Vec::new();
    };
    let tags = state.picked::<Results<TagInfo>>(&DataKey::Tags(repo.clone()));
    let mut rows = Vec::new();
    let known = tags.is_some_and(|t| t.items.iter().any(|t| t.name == q));
    if !q.is_empty() && !known {
        rows.push(match tag_problem(q) {
            Some(why) => entry(why, "", None),
            None => entry(
                format!("Create new tag: {q}"),
                "on publish",
                Some(Choice::Tag(route.clone(), q.to_owned())),
            ),
        });
    }
    let Some(tags) = tags else {
        rows.push(entry("Loading the tags…", "", None));
        return rows;
    };
    let mut hits: Vec<(usize, &TagInfo)> = (tags.items.iter())
        .filter_map(|t| Some((fuzzy_score(q, &t.name)?, t)))
        .collect();
    hits.sort_by_key(|(score, _)| *score);
    for (_, t) in hits {
        let hint = t
            .date
            .as_deref()
            .map_or("tag", |d| d.get(..10).unwrap_or(d));
        rows.push(entry(
            t.name.clone(),
            hint,
            Some(Choice::Tag(route.clone(), t.name.clone())),
        ));
    }
    let more = tags.total.saturating_sub(tags.items.len() as u64);
    if more > 0 {
        let what = crate::ui::text::noun(more, "tag");
        rows.push(entry(
            format!("{more} older {what} on GitHub: type one's name"),
            "",
            None,
        ));
    }
    rows
}

/// The target picker's rows: a commit typed, then the branches.
pub fn target_rows(state: &State, route: &Route, q: &str) -> Vec<(PaletteItem, Option<Choice>)> {
    let Some(repo) = form_repo(route) else {
        return Vec::new();
    };
    let branches = state.picked::<Results<BranchInfo>>(&DataKey::Branches(repo.clone()));
    let mut rows = Vec::new();
    let known = branches.is_some_and(|b| b.items.iter().any(|b| b.name == q));
    if !q.is_empty() && !known {
        rows.push(entry(
            format!("Use {q}"),
            "a branch or a commit's SHA",
            Some(Choice::Target(route.clone(), q.to_owned())),
        ));
    }
    let Some(branches) = branches else {
        rows.push(entry("Loading the branches…", "", None));
        return rows;
    };
    let mut hits: Vec<(usize, &BranchInfo)> = (branches.items.iter())
        .filter_map(|b| Some((fuzzy_score(q, &b.name)?, b)))
        .collect();
    hits.sort_by_key(|(score, b)| (!b.default, *score));
    for (_, b) in hits {
        let hint = if b.default {
            "default branch"
        } else {
            "branch"
        };
        rows.push(entry(
            b.name.clone(),
            hint,
            Some(Choice::Target(route.clone(), b.name.clone())),
        ));
    }
    rows
}

#[must_use]
pub fn choose_tag(state: &mut State, route: &Route, tag: String) -> Vec<Cmd> {
    let form = state.forms.edit::<Form>(route);
    form.tag = Some(tag);
    let blank = form.text.trim().is_empty();
    state.forms.sent_mut(route).error = None;
    let mut cmds = follow(state);
    // Then its title and notes, as on GitHub.
    if blank && state.overlay.is_none() {
        cmds.extend(compose(state));
    }
    cmds
}

#[must_use]
pub fn choose_target(state: &mut State, route: &Route, target: String) -> Vec<Cmd> {
    state.forms.edit::<Form>(route).target = Some(target);
    Vec::new()
}

/// The default branch, for what a new tag is made at by default.
fn default_branch(state: &State, repo: &RepoId) -> String {
    (state.overview(repo))
        .and_then(|o| o.default_branch.clone())
        .unwrap_or_else(|| "the default branch".into())
}

/// Why the release can't be saved yet, if it can't.
fn problem(state: &State, repo: &RepoId, form: &Form) -> Option<String> {
    if let Some(why) = refused(state, repo) {
        return Some(why);
    }
    let tag = form.tag.as_deref()?;
    match form.tag_look(state, repo) {
        None => Some(format!("Still looking for {tag} on GitHub")),
        Some(Err(why)) => Some(format!("Couldn't look for {tag} on GitHub: {why}")),
        // Its own release, being edited.
        Some(Ok(_)) if form.own_tag() => None,
        Some(Ok(TagLook {
            release: Some((true, _)),
            ..
        })) => Some(format!(
            "{tag} has a draft release already: publish or delete that one"
        )),
        Some(Ok(TagLook {
            release: Some(_), ..
        })) => Some(format!("{tag} has a release already")),
        Some(Ok(_)) => None,
    }
}

/// `,s`, ctrl-s, or its button: asks whether to publish it or keep a
/// draft.
#[must_use]
fn submit(state: &mut State) -> Vec<Cmd> {
    let Some((route, repo, form)) = here(state) else {
        return Vec::new();
    };
    let Some(tag) = form.tag.clone() else {
        return say(state, "Choose a tag first (Enter on Tag)".into());
    };
    if let Some(why) = problem(state, &repo, &form) {
        return say(state, why);
    }
    let (title, notes) = templates::split(&form.text);
    let mut facts = vec![(
        if form.new_tag(state, &repo) {
            let at = form
                .target
                .clone()
                .unwrap_or_else(|| default_branch(state, &repo));
            format!("A new tag, {tag}, made at {at}")
        } else {
            format!("Tag {tag}, as GitHub has it")
        },
        false,
    )];
    if let Some(was) = form.editing.as_ref().filter(|e| e.tag != tag) {
        facts.push((format!("Its tag was {}: that tag stays", was.tag), true));
    }
    facts.push((
        if title.is_empty() {
            format!("Titled {tag}: it has no title of its own")
        } else {
            format!("“{title}”")
        },
        false,
    ));
    if notes.is_empty() {
        facts.push(("It has no notes".into(), true));
    }
    facts.push((
        match (form.prerelease, form.latest) {
            (true, _) => "A pre-release".to_owned(),
            (false, true) => "It becomes the latest release".to_owned(),
            (false, false) => "It doesn't become the latest release".to_owned(),
        },
        false,
    ));
    let (question, choices) = match &form.editing {
        None => (
            format!("Publish {tag} in {repo}?"),
            vec![
                ("Publish release".into(), Answer::Release { draft: false }),
                ("Save draft".into(), Answer::Release { draft: true }),
            ],
        ),
        Some(e) if e.draft => (
            format!("Save the draft of {tag}?"),
            vec![
                ("Publish release".into(), Answer::Release { draft: false }),
                ("Save draft".into(), Answer::Release { draft: true }),
            ],
        ),
        Some(_) => (
            format!("Update {tag}?"),
            vec![
                ("Update release".into(), Answer::Release { draft: false }),
                (
                    "Make it a draft again".into(),
                    Answer::Release { draft: true },
                ),
            ],
        ),
    };
    crate::forms::ask(state, route, question, facts, choices);
    Vec::new()
}

/// Saves the release: published, or a draft.
#[must_use]
pub fn create(state: &mut State, route: &Route, draft: bool) -> Vec<Cmd> {
    let Some(repo) = form_repo(route).cloned() else {
        return Vec::new();
    };
    let Some(form) = state.forms.get::<Form>(route) else {
        return Vec::new();
    };
    let Some(tag) = form.tag.clone() else {
        return Vec::new();
    };
    let (name, notes) = templates::split(&form.text);
    let new = NewRelease {
        target: form.target.clone().filter(|_| form.new_tag(state, &repo)),
        repo,
        tag,
        name,
        notes,
        draft,
        prerelease: form.prerelease,
        latest: form.latest,
    };
    let id = form.editing.as_ref().map(|e| e.id);
    crate::forms::form_sending(state, route);
    work::start(Save {
        route: route.clone(),
        id,
        new,
    })
}

/// `,D` on a release's page: deleting the asset selected, or the release,
/// after asking.
#[must_use]
pub fn ask_delete(state: &mut State) -> Vec<Cmd> {
    if let Some(why) = unchangeable(state) {
        return state.just_say(why);
    }
    if let Some(name) = assets::selected(state) {
        return assets::ask_delete(state, &name);
    }
    let Some((repo, tag, r)) = shown(state) else {
        return Vec::new();
    };
    let mut facts = Vec::new();
    let n = r.assets.total();
    if n > 0 {
        let what = crate::ui::text::noun(n, "asset");
        facts.push((format!("Its {n} {what} go with it"), true));
    }
    facts.push((
        format!("Its tag, {tag}, stays: you're asked about it next"),
        false,
    ));
    let title = format!("Delete the release {}?", r.name);
    let route = Route::Releases(releases::Release { repo, tag });
    let choices = vec![("Delete release".into(), Answer::DeleteRelease)];
    crate::forms::ask(state, route, title, facts, choices);
    Vec::new()
}

/// The release on `route`'s page was asked to go: deletes it.
#[must_use]
pub fn delete(state: &mut State, route: &Route) -> Vec<Cmd> {
    let Route::Releases(releases::Release { repo, tag }) = route else {
        return Vec::new();
    };
    let key = DataKey::Release(repo.clone(), tag.clone());
    let Some(id) = state.picked::<Release>(&key).and_then(|r| r.id) else {
        return Vec::new();
    };
    crate::forms::sending(state);
    work::start(Delete {
        repo: repo.clone(),
        tag: tag.clone(),
        id,
    })
}

/// Deletes the tag of the release deleted from `route`, as asked.
#[must_use]
pub fn delete_tag(state: &mut State, route: &Route) -> Vec<Cmd> {
    let Route::Releases(releases::Release { repo, tag }) = route else {
        return Vec::new();
    };
    crate::forms::sending(state);
    work::start(DeleteTag {
        repo: repo.clone(),
        tag: tag.clone(),
    })
}

impl Job for Generate {
    type Out = Notes;
    const WRITES: bool = false;

    async fn run(&self, cx: &Cx) -> Result<Notes, Failure> {
        let notes = cx
            .gh
            .release_notes(&self.repo, &self.tag, self.target.as_deref());
        Ok(notes.await?)
    }

    fn done(self, state: &mut State, result: Result<Notes, Failure>) -> Vec<Cmd> {
        let Generate { repo, tag, .. } = self;
        let Some(route) = state
            .route()
            .cloned()
            .filter(|r| form_repo(r) == Some(&repo))
        else {
            return Vec::new();
        };
        let Some(form) = state.forms.get_mut::<Form>(&route) else {
            return Vec::new();
        };
        form.generating = false;
        match result {
            Ok(notes) => {
                // GitHub's title and notes in place of what's there,
                // unless you've written your own: then below them.
                let (title, _) = templates::split(&form.text);
                if form.text.trim().is_empty() || form.text == form.filled {
                    let title = if title.is_empty() { notes.name } else { title };
                    form.text = templates::join(&title, &notes.body);
                    form.filled = form.text.clone();
                } else {
                    form.text = format!("{}\n\n{}", form.text.trim_end(), notes.body);
                }
                let write = state.first_key(Action::Comment);
                state.info(format!(
                    "GitHub wrote the notes for {tag}: {write} edits them"
                ));
            }
            Err(why) => state.error(format!("GitHub didn't write the notes: {why}")),
        }
        Vec::new()
    }
}

impl Job for DeleteTag {
    type Out = ();

    async fn run(&self, cx: &Cx) -> Result<(), Failure> {
        Ok(cx.gh.delete_tag(&self.repo, &self.tag).await?)
    }

    fn done(self, state: &mut State, result: Result<(), Failure>) -> Vec<Cmd> {
        let DeleteTag { repo, tag } = self;
        let route = Route::Releases(releases::Release {
            repo: repo.clone(),
            tag: tag.clone(),
        });
        match result {
            Ok(()) => {
                crate::forms::close_sender(state, &route);
                state.info(format!("Deleted the tag {tag} on GitHub"));
                // The tags list, if it's shown, no longer has it.
                state.ensure(DataKey::Tags(repo), true)
            }
            Err(why) => say(state, format!("GitHub didn't delete the tag: {why}")),
        }
    }
}

impl Job for Save {
    type Out = Made;

    async fn run(&self, cx: &Cx) -> Result<Made, Failure> {
        let gh = &cx.gh;
        Ok(match self.id {
            Some(id) => gh.update_release(id, &self.new).await?,
            None => gh.create_release(&self.new).await?,
        })
    }

    /// GitHub saved the release: its page opens in place of the form's,
    /// and follows GitHub until it shows it.
    fn done(self, state: &mut State, result: Result<Made, Failure>) -> Vec<Cmd> {
        let route = &self.route;
        let Some(repo) = form_repo(route).cloned() else {
            return Vec::new();
        };
        match result {
            Ok(made) => {
                let form: Form = crate::forms::done(state, route);
                let tag = made.tag_name;
                let editing = form.editing.is_some();
                let (page, what, said) = if made.draft {
                    let said = format!("Saved a draft release of {tag}");
                    (
                        Route::Releases(releases::Releases(repo.clone())),
                        Expected::Draft(repo, tag),
                        said,
                    )
                } else {
                    let (name, notes) = templates::split(&form.text);
                    let what = if editing {
                        Expected::Edited {
                            repo: repo.clone(),
                            tag: tag.clone(),
                            name: if name.is_empty() { tag.clone() } else { name },
                            notes,
                            prerelease: form.prerelease,
                        }
                    } else {
                        Expected::Release(repo.clone(), tag.clone())
                    };
                    let said = if editing {
                        format!("Updated {tag}")
                    } else {
                        format!("Published {tag}")
                    };
                    (Route::Releases(releases::Release { repo, tag }), what, said)
                };
                state.info(said);
                if state.route() != Some(route) {
                    return Vec::new();
                }
                crate::follow::expect(state, page.clone(), what);
                state.replace(page, true, None)
            }
            Err(why) => {
                let why = format!("GitHub didn't save the release: {why}");
                crate::forms::failed(state, route, why)
            }
        }
    }
}

impl Job for Delete {
    type Out = bool;

    async fn run(&self, cx: &Cx) -> Result<bool, Failure> {
        let (gh, repo) = (&cx.gh, &self.repo);
        gh.delete_release(repo, self.id).await?;
        // Whether there's a tag to ask about.
        Ok(gh.tag_look(repo, &self.tag).await.is_ok_and(|l| l.exists))
    }

    /// GitHub deleted the release: the releases follow until it's gone, and
    /// deleting its tag too is asked, apart.
    fn done(self, state: &mut State, result: Result<bool, Failure>) -> Vec<Cmd> {
        let Delete { repo, tag, .. } = &self;
        let route = Route::Releases(releases::Release {
            repo: repo.clone(),
            tag: tag.to_owned(),
        });
        let tagged = match result {
            Ok(tagged) => tagged,
            Err(why) => {
                return say(state, format!("GitHub didn't delete the release: {why}"));
            }
        };
        crate::forms::close_sender(state, &route);
        state.info(format!("Deleted the release of {tag}"));
        let mut cmds = Vec::new();
        if state.route() == Some(&route) {
            let list = Route::Releases(releases::Releases(repo.clone()));
            let gone = Expected::ReleaseGone(repo.clone(), tag.to_owned());
            crate::follow::expect(state, list.clone(), gone);
            cmds = state.replace(list, true, None);
        }
        if !tagged {
            return cmds;
        }
        crate::forms::ask(
            state,
            route,
            format!("Delete the tag {tag} as well?"),
            vec![
                (
                    format!(
                        "The release is deleted; the tag {tag} stays on GitHub unless you delete it"
                    ),
                    false,
                ),
                ("A deleted tag can't be brought back from here".into(), true),
            ],
            vec![
                (format!("Delete the tag {tag}"), Answer::DeleteTag),
                ("Keep the tag".into(), Answer::KeepTag),
            ],
        );
        cmds
    }
}

/// The form's rows in the menu: writing it, and saving it.
fn offers(state: &State, out: &mut Vec<Doable>) {
    if let Some((_, repo, form)) = here(state) {
        let why = match &form.tag {
            None => Some("choose a tag first".to_owned()),
            Some(_) => problem(state, &repo, &form),
        };
        let publish = if form.editing.is_some() {
            "Save the release"
        } else {
            "Publish the release, or save a draft"
        };
        let write = Some("Write the title and notes");
        crate::forms::offer(state, write, (publish, "publish"), why, out);
    }
}

/// The menu's rows: making a release from a repository's pages, and
/// changing the release on screen.
fn doables(state: &State, out: &mut Vec<Doable>) {
    let Some(route) = state.route().filter(|_| here(state).is_none()) else {
        return;
    };
    let Some(repo) = route.repo() else {
        return;
    };
    if matches!(route, Route::Releases(releases::Release { .. })) {
        let why = unchangeable(state);
        out.push(row(
            "Edit",
            Action::EditComment,
            "Edit the release",
            "edit",
            why.clone(),
        ));
        let delete = if assets::selected(state).is_some() {
            "Delete the asset"
        } else {
            "Delete the release"
        };
        out.push(row(
            "Edit",
            Action::DeleteComment,
            delete,
            "delete",
            why.clone(),
        ));
        out.push(row(
            "Change",
            Action::UploadAsset,
            "Upload an asset",
            "upload",
            why,
        ));
    }
    out.push(row(
        "New",
        Action::NewRelease,
        "New release",
        "",
        refused(state, repo),
    ));
}

/// The form's page.
pub fn build(state: &State, page: &mut Page, route: &Route, repo: &RepoId) {
    let form = state.forms.shown::<Form>(route);
    let form = form.as_ref();
    let composing = crate::forms::composing(state, route, 0);
    let mut rows = vec![FormRow {
        name: "Tag",
        value: form
            .tag
            .as_ref()
            .map_or_else(|| "Choose a tag ▾".into(), |t| format!("{t} ▾")),
        unset: form.tag.is_none(),
        link: Some(Row::Tag),
    }];
    let mut status = Vec::new();
    if let Some(tag) = &form.tag {
        let (text, mark) = match form.tag_look(state, repo) {
            None => (format!("Looking for {tag} on GitHub…"), Mark::Waiting),
            Some(Err(why)) => (format!("Couldn't look for {tag}: {why}"), Mark::Problem),
            Some(Ok(_)) if form.own_tag() => (format!("{tag} is this release's tag"), Mark::Fine),
            Some(Ok(TagLook {
                release: Some(_), ..
            })) => (
                problem(state, repo, form).unwrap_or_default(),
                Mark::Problem,
            ),
            Some(Ok(TagLook { exists: true, .. })) => {
                (format!("{tag} is on GitHub already"), Mark::Fine)
            }
            Some(Ok(_)) => (
                format!("{tag} is a new tag: GitHub makes it when the release is published"),
                Mark::Fine,
            ),
        };
        status.push((text, mark));
    }
    if form.new_tag(state, repo) {
        let target = (form.target.clone())
            .unwrap_or_else(|| format!("{} (default branch)", default_branch(state, repo)));
        rows.push(FormRow {
            name: "Target",
            value: format!("{target} ▾"),
            unset: false,
            link: Some(Row::Target),
        });
    }
    let check = |on: bool| if on { "☑" } else { "☐" };
    let notes = if form.generating {
        "GitHub is writing them…"
    } else {
        "From the pull requests since the last release"
    };
    let after = vec![
        FormRow {
            name: "Notes",
            value: format!("Generate release notes · {notes}"),
            unset: false,
            link: Some(Row::Notes),
        },
        FormRow {
            name: "Pre-release",
            value: format!("{} Set as a pre-release", check(form.prerelease)),
            unset: false,
            link: Some(Row::Prerelease),
        },
        FormRow {
            name: "Latest",
            value: if form.prerelease {
                "☐ A pre-release isn't the latest".to_owned()
            } else {
                format!("{} Set as the latest release", check(form.latest))
            },
            unset: form.prerelease,
            link: Some(Row::Latest),
        },
    ];
    let (title, notes) = templates::split(&form.text);
    let write = state.first_key(Action::Comment);
    let save_key = state.first_key(Action::Create);
    let why_not = match &form.tag {
        None => Some("Choose a tag first".to_owned()),
        Some(_) => problem(state, repo, form),
    };
    let (title, notes) = if composing {
        (String::new(), String::new())
    } else {
        (title, notes)
    };
    let (heading, button) = match &form.editing {
        Some(e) => (
            format!("Editing {} in {repo}", e.tag),
            if e.draft {
                "Save the draft…"
            } else {
                "Update release…"
            },
        ),
        None => (format!("New release in {repo}"), "Publish release…"),
    };
    let view = WriteFormView {
        heading,
        base: Some(crate::ui::markdown::LinkBase::new(repo, "HEAD", "")),
        rows,
        status,
        boxes: vec![TextBox {
            title,
            untitled: "Release title (the tag's name, if none)",
            body: &notes,
            markdown: true,
            empty: "No notes yet.",
            hint: "The first line is the title; then the notes, in Markdown (ctrl-e for $EDITOR)",
            link: Link::Form(crate::ui::form::Row::Text(0)),
            actions: Vec::new(),
        }],
        after,
        write: &write,
        button: button.into(),
        save_key: &save_key,
        why_not,
        error: state.forms.sent(route).error.as_deref(),
    };
    crate::ui::pages::write_form(page, &view);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_names_follow_gits_rules() {
        assert_eq!(tag_problem("v1.2.0"), None);
        assert_eq!(tag_problem("release/2026-10"), None);
        assert!(tag_problem("v1 2").is_some());
        assert!(tag_problem("v1..2").is_some());
        assert!(tag_problem("v1.lock").is_some());
        assert!(tag_problem("-v1").is_some());
        assert!(tag_problem("").is_some());
    }
}
