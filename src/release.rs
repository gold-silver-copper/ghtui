//! Making a release, as GitHub's "Draft a new release" does.
//!
//! The form is a page ([`Route::NewRelease`]): a tag (one GitHub has, or
//! a new one GitHub makes at a branch or commit when the release is
//! published), a title and notes written in the composer (the title on
//! the first line), GitHub's generated notes offered as a start, and
//! whether it's a pre-release or the latest. Making it asks whether to
//! publish it or save a draft, as GitHub's two buttons do; then the
//! release's page (a draft's: the releases) follows GitHub until it
//! shows it.

use crate::api::GitHub;
use crate::api::browse::{BranchInfo, Results, TagInfo};
use crate::api::model::RepoId;
use crate::api::release::{Made, NewRelease, Notes, TagLook};
use crate::create::Look;
use crate::create::templates;
use crate::ui::overlays::PaletteItem;
use crate::ui::page::{Link, Page, WriteLink};
use crate::ui::pages::{FormRow, Mark, TextBox, WriteFormView};

use crate::browse::DataKey;
use crate::keymap::Action;
use crate::nav::Doable;
use crate::picker::fuzzy_score;
use crate::review::ComposeTarget;
use crate::route::Route;
use crate::state::{Cmd, Overlay, State};
use crate::writes::{Answer, Chosen, Expected, Pick, Text, Work as Writes, item, row};

/// A new release, as written so far.
#[derive(Debug, Clone)]
pub struct Form {
    /// The title on the first line, then the notes.
    pub text: String,
    /// The text GitHub's notes filled in: while it's still that,
    /// generating again replaces it.
    pub filled: String,
    /// The tag picker has opened on it.
    pub started: bool,
    pub tag: Option<String>,
    /// What GitHub has of the tag, for the tag it was asked for.
    pub look: Option<(String, Look<TagLook>)>,
    /// What a new tag is made at: a branch or a commit (`None`: the
    /// default branch).
    pub target: Option<String>,
    pub prerelease: bool,
    pub latest: bool,
    /// GitHub is writing the notes.
    pub generating: bool,
    pub sending: bool,
    pub error: Option<String>,
}

impl Default for Form {
    fn default() -> Self {
        Self {
            text: String::new(),
            filled: String::new(),
            started: false,
            tag: None,
            look: None,
            target: None,
            prerelease: false,
            latest: true,
            generating: false,
            sending: false,
            error: None,
        }
    }
}

impl Form {
    /// What GitHub has of the tag, once it's said.
    fn tag_look(&self) -> Option<&Look<TagLook>> {
        let (tag, look) = self.look.as_ref()?;
        (Some(tag) == self.tag.as_ref()).then_some(look)
    }

    /// The tag is a new one: GitHub makes it at the target.
    fn new_tag(&self) -> bool {
        matches!(
            self.tag_look(),
            Some(Look::Found(TagLook { exists: false, .. }))
        )
    }
}

/// Work in the background.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Work {
    /// Whether the repository has the tag, and a release of it.
    Look(RepoId, String),
    /// GitHub's notes for a release of the tag.
    Notes {
        repo: RepoId,
        tag: String,
        target: Option<String>,
    },
    Create(NewRelease),
}

#[derive(Debug)]
pub enum Done {
    Looked(RepoId, String, Result<TagLook, String>),
    Notes(RepoId, String, Result<Notes, String>),
    Created(RepoId, Result<Made, String>),
}

impl Done {
    pub fn wrote(&self) -> bool {
        matches!(self, Done::Created(_, Ok(_)))
    }
}

impl Work {
    pub fn failed(&self, why: String) -> Done {
        match self.clone() {
            Work::Look(repo, tag) => Done::Looked(repo, tag, Err(why)),
            Work::Notes { repo, tag, .. } => Done::Notes(repo, tag, Err(why)),
            Work::Create(new) => Done::Created(new.repo, Err(why)),
        }
    }

    pub async fn run(self, gh: &GitHub) -> Done {
        let why = |err: crate::api::ApiError| crate::writes::reason(&err);
        match self {
            Work::Look(repo, tag) => {
                let look = gh.tag_look(&repo, &tag).await.map_err(why);
                Done::Looked(repo, tag, look)
            }
            Work::Notes { repo, tag, target } => {
                let notes = gh.release_notes(&repo, &tag, target.as_deref()).await;
                Done::Notes(repo, tag, notes.map_err(why))
            }
            Work::Create(new) => {
                let made = gh.create_release(&new).await.map_err(why);
                Done::Created(new.repo, made)
            }
        }
    }
}

/// The repository whose new release is on screen, and its form (blank
/// until it's started).
fn here(state: &State) -> Option<(RepoId, &Form)> {
    static BLANK: std::sync::LazyLock<Form> = std::sync::LazyLock::new(Form::default);
    let Some(Route::NewRelease(repo)) = state.route() else {
        return None;
    };
    let form = state.writing.releases.get(repo).unwrap_or(&BLANK);
    Some((repo.clone(), form))
}

fn form_mut<'a>(state: &'a mut State, repo: &RepoId) -> &'a mut Form {
    state.writing.releases.entry(repo.clone()).or_default()
}

/// Why `repo` takes no release from you, from what's known.
fn refused(state: &State, repo: &RepoId) -> Option<String> {
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
    let route = Route::NewRelease(repo);
    if state.route() == Some(&route) {
        return compose(state);
    }
    let mut cmds = state.push(route);
    cmds.extend(follow(state));
    cmds
}

/// Brings the form on screen up to date: starts it by choosing its tag,
/// and asks GitHub about a tag it hasn't yet.
#[must_use]
pub fn follow(state: &mut State) -> Vec<Cmd> {
    let Some((repo, _)) = here(state) else {
        return Vec::new();
    };
    let mut cmds = Vec::new();
    let loaded = state.overview(&repo).is_some();
    let form = form_mut(state, &repo);
    if let Some(tag) = form.tag.clone()
        && form.tag_look().is_none()
    {
        form.look = Some((tag.clone(), Look::Asking));
        cmds.push(Cmd::Write(Writes::Release(Work::Look(repo.clone(), tag))));
    }
    if loaded && !form.started && state.overlay.is_none() && refused(state, &repo).is_none() {
        form_mut(state, &repo).started = true;
        cmds.extend(crate::writes::open_picker(state, Pick::Tag(repo)));
    }
    cmds
}

/// The composer, on the title and notes.
#[must_use]
pub fn compose(state: &mut State) -> Vec<Cmd> {
    let Some((repo, form)) = here(state) else {
        return Vec::new();
    };
    if form.sending {
        state.info("It's being made");
        return Vec::new();
    }
    let text = form.text.clone();
    state.data_gen += 1;
    let cmds = state.compose(ComposeTarget::Write(Text::Release(repo)), &text);
    if let Some(Overlay::Compose(c)) = &mut state.overlay {
        c.input.move_cursor(ratatui_textarea::CursorMove::Top);
        c.input.move_cursor(ratatui_textarea::CursorMove::End);
    }
    cmds
}

pub fn keep_text(state: &mut State, repo: &RepoId, text: String) {
    let form = form_mut(state, repo);
    form.text = text;
    form.error = None;
}

/// Enter on one of the form's rows.
#[must_use]
pub fn follow_link(state: &mut State, link: &WriteLink) -> Vec<Cmd> {
    let Some((repo, form)) = here(state) else {
        return Vec::new();
    };
    match link {
        WriteLink::Tag => crate::writes::open_picker(state, Pick::Tag(repo)),
        WriteLink::Target if form.new_tag() => {
            crate::writes::open_picker(state, Pick::Target(repo))
        }
        WriteLink::Target => {
            state.info("The tag is GitHub's already: it's where it is");
            Vec::new()
        }
        WriteLink::Notes => generate(state),
        WriteLink::Prerelease => {
            let form = form_mut(state, &repo);
            form.prerelease = !form.prerelease;
            state.data_gen += 1;
            Vec::new()
        }
        WriteLink::Latest if form.prerelease => {
            state.info("A pre-release is never the latest release");
            Vec::new()
        }
        WriteLink::Latest => {
            let form = form_mut(state, &repo);
            form.latest = !form.latest;
            state.data_gen += 1;
            Vec::new()
        }
        WriteLink::Save => submit(state),
        _ => Vec::new(),
    }
}

/// Asks GitHub to write the notes.
#[must_use]
fn generate(state: &mut State) -> Vec<Cmd> {
    let Some((repo, form)) = here(state) else {
        return Vec::new();
    };
    let Some(tag) = form.tag.clone() else {
        state.info("Choose a tag first: GitHub writes the notes since the release before it");
        return Vec::new();
    };
    if form.generating {
        return Vec::new();
    }
    let target = form.target.clone().filter(|_| form.new_tag());
    form_mut(state, &repo).generating = true;
    state.data_gen += 1;
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

/// The tag picker's rows: a new tag typed, then the tags GitHub has.
pub fn tag_rows(state: &State, repo: &RepoId, q: &str) -> Vec<(PaletteItem, Option<Chosen>)> {
    let tags = state.picked::<Results<TagInfo>>(&DataKey::Tags(repo.clone()));
    let mut rows = Vec::new();
    let known = tags.is_some_and(|t| t.items.iter().any(|t| t.name == q));
    if !q.is_empty() && !known {
        rows.push(match tag_problem(q) {
            Some(why) => item(why, "", None),
            None => item(
                format!("Create new tag: {q}"),
                "on publish",
                Some(Chosen::Tag(repo.clone(), q.to_owned())),
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
            Some(Chosen::Tag(repo.clone(), t.name.clone())),
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
pub fn target_rows(state: &State, repo: &RepoId, q: &str) -> Vec<(PaletteItem, Option<Chosen>)> {
    let branches = state.picked::<Results<BranchInfo>>(&DataKey::Branches(repo.clone()));
    let mut rows = Vec::new();
    let known = branches.is_some_and(|b| b.items.iter().any(|b| b.name == q));
    if !q.is_empty() && !known {
        rows.push(item(
            format!("Use {q}"),
            "a branch or a commit's SHA",
            Some(Chosen::Target(repo.clone(), q.to_owned())),
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
            Some(Chosen::Target(repo.clone(), b.name.clone())),
        ));
    }
    rows
}

#[must_use]
pub fn choose_tag(state: &mut State, repo: &RepoId, tag: String) -> Vec<Cmd> {
    let form = form_mut(state, repo);
    form.tag = Some(tag);
    form.error = None;
    let blank = form.text.trim().is_empty();
    state.data_gen += 1;
    let mut cmds = follow(state);
    // Then its title and notes, as on GitHub.
    if blank && state.overlay.is_none() {
        cmds.extend(compose(state));
    }
    cmds
}

#[must_use]
pub fn choose_target(state: &mut State, repo: &RepoId, target: String) -> Vec<Cmd> {
    form_mut(state, repo).target = Some(target);
    state.data_gen += 1;
    Vec::new()
}

/// The default branch, for what a new tag is made at by default.
fn default_branch(state: &State, repo: &RepoId) -> String {
    (state.overview(repo))
        .and_then(|o| o.default_branch.clone())
        .unwrap_or_else(|| "the default branch".into())
}

/// Why the release can't be made yet, if it can't.
fn problem(state: &State, repo: &RepoId, form: &Form) -> Option<String> {
    if let Some(why) = refused(state, repo) {
        return Some(why);
    }
    let tag = form.tag.as_deref()?;
    match form.tag_look() {
        None | Some(Look::Asking) => Some(format!("Still looking for {tag} on GitHub")),
        Some(Look::Failed(why)) => Some(format!("Couldn't look for {tag} on GitHub: {why}")),
        Some(Look::Found(TagLook {
            release: Some((true, _)),
            ..
        })) => Some(format!(
            "{tag} has a draft release already: publish or delete that one"
        )),
        Some(Look::Found(TagLook {
            release: Some(_), ..
        })) => Some(format!("{tag} has a release already")),
        Some(Look::Found(_)) => None,
    }
}

/// `,s`, ctrl-s, or its button: asks whether to publish it or save a
/// draft.
#[must_use]
pub fn submit(state: &mut State) -> Vec<Cmd> {
    let Some((repo, form)) = here(state) else {
        return Vec::new();
    };
    if form.sending {
        crate::writes::say(state, "It's being made".into());
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
        if form.new_tag() {
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
    // A draft's tag isn't made until it's published.
    let route = Route::NewRelease(repo.clone());
    crate::writes::ask(
        state,
        route,
        format!("Publish {tag} in {repo}?"),
        facts,
        vec![
            ("Publish release".into(), Answer::Release { draft: false }),
            ("Save draft".into(), Answer::Release { draft: true }),
        ],
    );
    Vec::new()
}

/// Makes the release: published, or a draft.
#[must_use]
pub fn create(state: &mut State, route: &Route, draft: bool) -> Vec<Cmd> {
    let Route::NewRelease(repo) = route else {
        return Vec::new();
    };
    let Some(form) = state.writing.releases.get(repo) else {
        return Vec::new();
    };
    let Some(tag) = form.tag.clone() else {
        return Vec::new();
    };
    let (name, notes) = templates::split(&form.text);
    let new = NewRelease {
        repo: repo.clone(),
        tag,
        target: form.target.clone().filter(|_| form.new_tag()),
        name,
        notes,
        draft,
        prerelease: form.prerelease,
        latest: form.latest,
    };
    form_mut(state, repo).sending = true;
    crate::writes::sending(state);
    vec![Cmd::Write(Writes::Release(Work::Create(new)))]
}

/// Work's answers.
#[must_use]
pub fn on_done(state: &mut State, done: Done) -> Vec<Cmd> {
    match done {
        Done::Looked(repo, tag, result) => {
            if let Some(form) = state.writing.releases.get_mut(&repo)
                && form.look.as_ref().is_some_and(|(t, _)| *t == tag)
            {
                form.look = Some((
                    tag,
                    match result {
                        Ok(look) => Look::Found(look),
                        Err(why) => Look::Failed(why),
                    },
                ));
            }
            Vec::new()
        }
        Done::Notes(repo, tag, result) => {
            let Some(form) = state.writing.releases.get_mut(&repo) else {
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
        Done::Created(repo, result) => created(state, &repo, result),
    }
}

/// GitHub made the release: its page opens in place of the form's, and
/// follows GitHub until it shows it.
#[must_use]
fn created(state: &mut State, repo: &RepoId, result: Result<Made, String>) -> Vec<Cmd> {
    let route = Route::NewRelease(repo.clone());
    match result {
        Ok(made) => {
            state.writing.releases.remove(repo);
            crate::writes::close_sender(state, &route);
            let tag = made.tag_name;
            let (page, what, said) = if made.draft {
                (
                    Route::Releases(repo.clone()),
                    Expected::Draft(repo.clone(), tag.clone()),
                    format!("Saved a draft release of {tag}"),
                )
            } else {
                (
                    Route::Release {
                        repo: repo.clone(),
                        tag: tag.clone(),
                    },
                    Expected::Release(repo.clone(), tag.clone()),
                    format!("Published {tag}"),
                )
            };
            state.info(said);
            if state.route() != Some(&route) {
                return Vec::new();
            }
            crate::writes::expect(state, page.clone(), what);
            state.replace(page, true, None)
        }
        Err(why) => {
            if let Some(form) = state.writing.releases.get_mut(repo) {
                form.sending = false;
            }
            crate::writes::say(state, format!("GitHub didn't make the release: {why}"));
            Vec::new()
        }
    }
}

/// The menu's rows: making a release from a repository's pages, and the
/// form's own.
pub fn doables(state: &State, route: &Route, out: &mut Vec<Doable>) {
    let Some(repo) = route.repo() else {
        return;
    };
    if let Some((repo, form)) = here(state) {
        let mut write = row(
            "This page",
            Action::Comment,
            "Write the title and notes",
            "write",
            None,
        );
        write.unavailable = form.sending.then(|| "it's being made".into());
        out.push(write);
        let why = match &form.tag {
            None => Some("choose a tag first".to_owned()),
            Some(_) => problem(state, &repo, form),
        };
        let publish = "Publish the release, or save a draft";
        out.push(row("This page", Action::Create, publish, "publish", why));
        return;
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
pub fn build(state: &State, page: &mut Page, repo: &RepoId) {
    let blank = Form::default();
    let form = state.writing.releases.get(repo).unwrap_or(&blank);
    let composing = matches!(
        &state.overlay,
        Some(Overlay::Compose(c)) if c.target == ComposeTarget::Write(Text::Release(repo.clone()))
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
        let (text, mark) = match form.tag_look() {
            None | Some(Look::Asking) => (format!("Looking for {tag} on GitHub…"), Mark::Waiting),
            Some(Look::Failed(why)) => (format!("Couldn't look for {tag}: {why}"), Mark::Problem),
            Some(Look::Found(TagLook {
                release: Some(_), ..
            })) => (
                problem(state, repo, form).unwrap_or_default(),
                Mark::Problem,
            ),
            Some(Look::Found(TagLook { exists: true, .. })) => {
                (format!("{tag} is on GitHub already"), Mark::Fine)
            }
            Some(Look::Found(_)) => (
                format!("{tag} is a new tag: GitHub makes it when the release is published"),
                Mark::Fine,
            ),
        };
        status.push((text, mark));
    }
    if form.new_tag() {
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
    let view = WriteFormView {
        heading: format!("New release in {repo}"),
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
        button: "Publish release…".into(),
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
