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

use crate::api::GitHub;
use crate::api::browse::{BranchInfo, Release, Results, TagInfo};
use crate::api::model::RepoId;
use crate::api::release::{Made, NewRelease, Notes, TagLook};
use crate::create::templates;
use crate::route::{Route, releases};
use crate::ui::overlays::PaletteItem;
use crate::ui::page::{Link, Page, WriteLink};
use crate::ui::pages::{FormRow, Mark, TextBox, WriteFormView};

use crate::data::DataKey;
use crate::keymap::Action;
use crate::nav::Doable;
use crate::picker::fuzzy_score;
use crate::review::ComposeTarget;
use crate::state::{Cmd, Overlay, State};
use crate::writes::{Answer, Chosen, Expected, Pick, Text, Work as Writes, item, row};

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
    pub sending: bool,
    pub error: Option<String>,
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
            sending: false,
            error: None,
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

/// Work in the background.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Work {
    /// GitHub's notes for a release of the tag.
    Notes {
        repo: RepoId,
        tag: String,
        target: Option<String>,
    },
    /// Makes the release written on the form at `route`, or (with an ID)
    /// changes that release to it.
    Save {
        route: Route,
        id: Option<u64>,
        new: NewRelease,
    },
    Delete {
        repo: RepoId,
        tag: String,
        id: u64,
    },
    DeleteTag(RepoId, String),
    Asset(assets::Work),
}

#[derive(Debug)]
pub enum Done {
    Notes(RepoId, String, Result<Notes, String>),
    Saved(Route, Result<Made, String>),
    /// The release deleted, and whether its tag is on GitHub (a draft's
    /// new tag never was).
    Deleted(RepoId, String, Result<bool, String>),
    TagDeleted(RepoId, String, Result<(), String>),
    Asset(assets::Done),
}

impl Done {
    pub fn wrote(&self) -> bool {
        match self {
            Done::Notes(..) => false,
            Done::Saved(_, r) => r.is_ok(),
            Done::Deleted(.., r) => r.is_ok(),
            Done::TagDeleted(.., r) => r.is_ok(),
            Done::Asset(d) => d.wrote(),
        }
    }
}

impl Work {
    pub fn failed(&self, why: String) -> Done {
        match self.clone() {
            Work::Notes { repo, tag, .. } => Done::Notes(repo, tag, Err(why)),
            Work::Save { route, .. } => Done::Saved(route, Err(why)),
            Work::Delete { repo, tag, .. } => Done::Deleted(repo, tag, Err(why)),
            Work::DeleteTag(repo, tag) => Done::TagDeleted(repo, tag, Err(why)),
            Work::Asset(w) => Done::Asset(w.failed(why)),
        }
    }

    pub async fn run(self, gh: &GitHub) -> Done {
        let why = |err: crate::api::ApiError| crate::writes::reason(&err);
        match self {
            Work::Notes { repo, tag, target } => {
                let notes = gh.release_notes(&repo, &tag, target.as_deref()).await;
                Done::Notes(repo, tag, notes.map_err(why))
            }
            Work::Save { route, id, new } => {
                let made = match id {
                    Some(id) => gh.update_release(id, &new).await,
                    None => gh.create_release(&new).await,
                };
                Done::Saved(route, made.map_err(why))
            }
            Work::Delete { repo, tag, id } => {
                let done = match gh.delete_release(&repo, id).await {
                    // Whether there's a tag to ask about.
                    Ok(()) => Ok(gh.tag_look(&repo, &tag).await.is_ok_and(|l| l.exists)),
                    Err(err) => Err(why(err)),
                };
                Done::Deleted(repo, tag, done)
            }
            Work::DeleteTag(repo, tag) => {
                let done = gh.delete_tag(&repo, &tag).await.map_err(why);
                Done::TagDeleted(repo, tag, done)
            }
            Work::Asset(w) => Done::Asset(w.run(gh).await),
        }
    }
}

/// The release form on screen: its page, its repository, and the form
/// (blank until it's started).
fn here(state: &State) -> Option<(Route, RepoId, &Form)> {
    static BLANK: std::sync::LazyLock<Form> = std::sync::LazyLock::new(Form::default);
    let route = state.route()?;
    let (Route::Releases(releases::NewRelease(repo))
    | Route::Releases(releases::EditRelease { repo, .. })) = route
    else {
        return None;
    };
    let form = state.writing.releases.get(route).unwrap_or(&BLANK);
    Some((route.clone(), repo.clone(), form))
}

fn form_mut<'a>(state: &'a mut State, route: &Route) -> &'a mut Form {
    state.writing.releases.entry(route.clone()).or_default()
}

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
        state.info("Open a repository first: a release is of one");
        return Vec::new();
    };
    if let Some(why) = refused(state, &repo) {
        state.info(why);
        return Vec::new();
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
        state.info(why);
        return Vec::new();
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
pub fn follow(state: &mut State) -> Vec<Cmd> {
    let Some((route, repo, form)) = here(state) else {
        return Vec::new();
    };
    if let Route::Releases(releases::EditRelease { tag, .. }) = &route
        && !form.started
        && let Some(r) = state.picked::<Release>(&DataKey::Release(repo.clone(), tag.clone()))
        && let Some(id) = r.id
    {
        let form = Form::of(r, id);
        state.writing.releases.insert(route.clone(), form);
        state.shown_gen += 1;
    }
    let mut cmds = Vec::new();
    let loaded = state.overview(&repo).is_some();
    let form = form_mut(state, &route);
    let (started, tag) = (form.started, form.tag.clone().filter(|_| !form.own_tag()));
    if let Some(tag) = tag {
        cmds.extend(state.ensure(DataKey::TagLook(repo.clone(), tag), false));
    }
    let new = matches!(route, Route::Releases(releases::NewRelease(_)));
    if new && loaded && !started && state.overlay.is_none() && refused(state, &repo).is_none() {
        form_mut(state, &route).started = true;
        cmds.extend(crate::writes::open_picker(state, Pick::Tag(route)));
    }
    cmds
}

/// The composer, on the title and notes.
#[must_use]
pub fn compose(state: &mut State) -> Vec<Cmd> {
    let Some((route, _, form)) = here(state) else {
        return Vec::new();
    };
    if form.sending {
        state.info("It's being saved");
        return Vec::new();
    }
    let text = form.text.clone();
    state.shown_gen += 1;
    let cmds = state.compose(ComposeTarget::Write(Text::Release(route)), &text);
    if let Some(Overlay::Compose(c)) = &mut state.overlay {
        c.input.move_cursor(ratatui_textarea::CursorMove::Top);
        c.input.move_cursor(ratatui_textarea::CursorMove::End);
    }
    cmds
}

pub fn keep_text(state: &mut State, route: &Route, text: String) {
    let form = form_mut(state, route);
    form.text = text;
    form.error = None;
}

/// Enter on one of the form's rows.
#[must_use]
pub fn follow_link(state: &mut State, link: &WriteLink) -> Vec<Cmd> {
    let Some((route, repo, form)) = here(state) else {
        return Vec::new();
    };
    match link {
        WriteLink::Tag => crate::writes::open_picker(state, Pick::Tag(route)),
        WriteLink::Target if form.new_tag(state, &repo) => {
            crate::writes::open_picker(state, Pick::Target(route))
        }
        WriteLink::Target => {
            state.info("The tag is GitHub's already: it's where it is");
            Vec::new()
        }
        WriteLink::Notes => generate(state),
        WriteLink::Prerelease => {
            let form = form_mut(state, &route);
            form.prerelease = !form.prerelease;
            state.shown_gen += 1;
            Vec::new()
        }
        WriteLink::Latest if form.prerelease => {
            state.info("A pre-release is never the latest release");
            Vec::new()
        }
        WriteLink::Latest => {
            let form = form_mut(state, &route);
            form.latest = !form.latest;
            state.shown_gen += 1;
            Vec::new()
        }
        WriteLink::Save => submit(state),
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
        state.info("Choose a tag first: GitHub writes the notes since the release before it");
        return Vec::new();
    };
    if form.generating {
        return Vec::new();
    }
    let target = form.target.clone().filter(|_| form.new_tag(state, &repo));
    form_mut(state, &route).generating = true;
    state.shown_gen += 1;
    state.info("Asking GitHub for release notes…");
    vec![Cmd::Write(Writes::Release(Work::Notes {
        repo,
        tag,
        target,
    }))]
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
pub fn tag_rows(state: &State, route: &Route, q: &str) -> Vec<(PaletteItem, Option<Chosen>)> {
    let Some(repo) = form_repo(route) else {
        return Vec::new();
    };
    let tags = state.picked::<Results<TagInfo>>(&DataKey::Tags(repo.clone()));
    let mut rows = Vec::new();
    let known = tags.is_some_and(|t| t.items.iter().any(|t| t.name == q));
    if !q.is_empty() && !known {
        rows.push(match tag_problem(q) {
            Some(why) => item(why, "", None),
            None => item(
                format!("Create new tag: {q}"),
                "on publish",
                Some(Chosen::Tag(route.clone(), q.to_owned())),
            ),
        });
    }
    let Some(tags) = tags else {
        rows.push(item("Loading the tags…", "", None));
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
        rows.push(item(
            t.name.clone(),
            hint,
            Some(Chosen::Tag(route.clone(), t.name.clone())),
        ));
    }
    let more = tags.total.saturating_sub(tags.items.len() as u64);
    if more > 0 {
        let what = crate::ui::text::noun(more, "tag");
        rows.push(item(
            format!("{more} older {what} on GitHub: type one's name"),
            "",
            None,
        ));
    }
    rows
}

/// The target picker's rows: a commit typed, then the branches.
pub fn target_rows(state: &State, route: &Route, q: &str) -> Vec<(PaletteItem, Option<Chosen>)> {
    let Some(repo) = form_repo(route) else {
        return Vec::new();
    };
    let branches = state.picked::<Results<BranchInfo>>(&DataKey::Branches(repo.clone()));
    let mut rows = Vec::new();
    let known = branches.is_some_and(|b| b.items.iter().any(|b| b.name == q));
    if !q.is_empty() && !known {
        rows.push(item(
            format!("Use {q}"),
            "a branch or a commit's SHA",
            Some(Chosen::Target(route.clone(), q.to_owned())),
        ));
    }
    let Some(branches) = branches else {
        rows.push(item("Loading the branches…", "", None));
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
        rows.push(item(
            b.name.clone(),
            hint,
            Some(Chosen::Target(route.clone(), b.name.clone())),
        ));
    }
    rows
}

#[must_use]
pub fn choose_tag(state: &mut State, route: &Route, tag: String) -> Vec<Cmd> {
    let form = form_mut(state, route);
    form.tag = Some(tag);
    form.error = None;
    let blank = form.text.trim().is_empty();
    state.shown_gen += 1;
    let mut cmds = follow(state);
    // Then its title and notes, as on GitHub.
    if blank && state.overlay.is_none() {
        cmds.extend(compose(state));
    }
    cmds
}

#[must_use]
pub fn choose_target(state: &mut State, route: &Route, target: String) -> Vec<Cmd> {
    form_mut(state, route).target = Some(target);
    state.shown_gen += 1;
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
pub fn submit(state: &mut State) -> Vec<Cmd> {
    let Some((route, repo, form)) = here(state) else {
        return Vec::new();
    };
    if form.sending {
        crate::writes::say(state, "It's being saved".into());
        return Vec::new();
    }
    let Some(tag) = form.tag.clone() else {
        crate::writes::say(state, "Choose a tag first (Enter on Tag)".into());
        return Vec::new();
    };
    if let Some(why) = problem(state, &repo, form) {
        crate::writes::say(state, why);
        return Vec::new();
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
    crate::writes::ask(state, route, question, facts, choices);
    Vec::new()
}

/// Saves the release: published, or a draft.
#[must_use]
pub fn create(state: &mut State, route: &Route, draft: bool) -> Vec<Cmd> {
    let Some(repo) = form_repo(route).cloned() else {
        return Vec::new();
    };
    let Some(form) = state.writing.releases.get(route) else {
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
    form_mut(state, route).sending = true;
    crate::writes::sending(state);
    vec![Cmd::Write(Writes::Release(Work::Save {
        route: route.clone(),
        id,
        new,
    }))]
}

/// `,D` on a release's page: deleting the asset selected, or the release,
/// after asking.
#[must_use]
pub fn ask_delete(state: &mut State) -> Vec<Cmd> {
    if let Some(why) = unchangeable(state) {
        state.info(why);
        return Vec::new();
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
    crate::writes::ask(state, route, title, facts, choices);
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
    crate::writes::sending(state);
    vec![Cmd::Write(Writes::Release(Work::Delete {
        repo: repo.clone(),
        tag: tag.clone(),
        id,
    }))]
}

/// Deletes the tag of the release deleted from `route`, as asked.
#[must_use]
pub fn delete_tag(state: &mut State, route: &Route) -> Vec<Cmd> {
    let Route::Releases(releases::Release { repo, tag }) = route else {
        return Vec::new();
    };
    crate::writes::sending(state);
    vec![Cmd::Write(Writes::Release(Work::DeleteTag(
        repo.clone(),
        tag.clone(),
    )))]
}

/// Work's answers.
#[must_use]
pub fn on_done(state: &mut State, done: Done) -> Vec<Cmd> {
    match done {
        Done::Notes(repo, tag, result) => {
            let Some(route) = state
                .route()
                .cloned()
                .filter(|r| form_repo(r) == Some(&repo))
            else {
                return Vec::new();
            };
            let Some(form) = state.writing.releases.get_mut(&route) else {
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
        Done::Saved(route, result) => saved(state, &route, result),
        Done::Deleted(repo, tag, result) => deleted(state, &repo, &tag, result),
        Done::TagDeleted(repo, tag, result) => {
            let route = Route::Releases(releases::Release {
                repo: repo.clone(),
                tag: tag.clone(),
            });
            match result {
                Ok(()) => {
                    crate::writes::close_sender(state, &route);
                    state.info(format!("Deleted the tag {tag} on GitHub"));
                    // The tags list, if it's shown, no longer has it.
                    state.ensure(DataKey::Tags(repo), true)
                }
                Err(why) => {
                    crate::writes::say(state, format!("GitHub didn't delete the tag: {why}"));
                    Vec::new()
                }
            }
        }
        Done::Asset(d) => assets::on_done(state, d),
    }
}

/// GitHub saved the release: its page opens in place of the form's,
/// and follows GitHub until it shows it.
#[must_use]
fn saved(state: &mut State, route: &Route, result: Result<Made, String>) -> Vec<Cmd> {
    let Some(repo) = form_repo(route).cloned() else {
        return Vec::new();
    };
    match result {
        Ok(made) => {
            let form = state.writing.releases.remove(route).unwrap_or_default();
            crate::writes::close_sender(state, route);
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
            crate::writes::expect(state, page.clone(), what);
            state.replace(page, true, None)
        }
        Err(why) => {
            if let Some(form) = state.writing.releases.get_mut(route) {
                form.sending = false;
            }
            crate::writes::say(state, format!("GitHub didn't save the release: {why}"));
            Vec::new()
        }
    }
}

/// GitHub deleted the release: the releases follow until it's gone, and
/// deleting its tag too is asked, apart.
#[must_use]
fn deleted(state: &mut State, repo: &RepoId, tag: &str, result: Result<bool, String>) -> Vec<Cmd> {
    let route = Route::Releases(releases::Release {
        repo: repo.clone(),
        tag: tag.to_owned(),
    });
    let tagged = match result {
        Ok(tagged) => tagged,
        Err(why) => {
            crate::writes::say(state, format!("GitHub didn't delete the release: {why}"));
            return Vec::new();
        }
    };
    crate::writes::close_sender(state, &route);
    state.info(format!("Deleted the release of {tag}"));
    let mut cmds = Vec::new();
    if state.route() == Some(&route) {
        let list = Route::Releases(releases::Releases(repo.clone()));
        let gone = Expected::ReleaseGone(repo.clone(), tag.to_owned());
        crate::writes::expect(state, list.clone(), gone);
        cmds = state.replace(list, true, None);
    }
    if !tagged {
        return cmds;
    }
    crate::writes::ask(
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

/// The menu's rows: making a release from a repository's pages, the
/// form's own, and changing the release on screen.
pub fn doables(state: &State, route: &Route, out: &mut Vec<Doable>) {
    let Some(repo) = route.repo() else {
        return;
    };
    if let Some((_, repo, form)) = here(state) {
        let mut write = row(
            "This page",
            Action::Comment,
            "Write the title and notes",
            "write",
            None,
        );
        write.unavailable = form.sending.then(|| "it's being saved".into());
        out.push(write);
        let why = match &form.tag {
            None => Some("choose a tag first".to_owned()),
            Some(_) => problem(state, &repo, form),
        };
        let publish = if form.editing.is_some() {
            "Save the release"
        } else {
            "Publish the release, or save a draft"
        };
        out.push(row("This page", Action::Create, publish, "publish", why));
        return;
    }
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
    let blank = Form::default();
    let form = state.writing.releases.get(route).unwrap_or(&blank);
    let composing = matches!(
        &state.overlay,
        Some(Overlay::Compose(c)) if c.target == ComposeTarget::Write(Text::Release(route.clone()))
    );
    let mut rows = vec![FormRow {
        name: "Tag",
        value: form
            .tag
            .as_ref()
            .map_or_else(|| "Choose a tag ▾".into(), |t| format!("{t} ▾")),
        unset: form.tag.is_none(),
        link: Some(WriteLink::Tag),
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
            link: Some(WriteLink::Target),
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
            link: Some(WriteLink::Notes),
        },
        FormRow {
            name: "Pre-release",
            value: format!("{} Set as a pre-release", check(form.prerelease)),
            unset: false,
            link: Some(WriteLink::Prerelease),
        },
        FormRow {
            name: "Latest",
            value: if form.prerelease {
                "☐ A pre-release isn't the latest".to_owned()
            } else {
                format!("{} Set as the latest release", check(form.latest))
            },
            unset: form.prerelease,
            link: Some(WriteLink::Latest),
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
            link: Link::Comment,
            actions: Vec::new(),
        }],
        after,
        write: &write,
        button: button.into(),
        save_key: &save_key,
        why_not,
        error: form.error.as_deref(),
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
