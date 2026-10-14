//! Making issues and pull requests, and checking a pull request out in
//! the clone ghtui runs in.
//!
//! A new issue or pull request is a page (`issues::NewIssue`,
//! `pulls::NewPr`) holding a [`Form`]: its title and description are
//! written in the comment composer, which opens in place of the form's
//! text box, with the title on the first line. A new issue starts from the
//! repository's templates when it has some; a new pull request from its
//! commits, as GitHub fills one in. What runs in the background (git in
//! your clone, GitHub's answers) is a [`Work`], answered with a [`Done`].

mod page;
pub mod templates;
pub(crate) mod yaml;

use crate::route::{Route, issues, pulls};
use std::collections::HashMap;
use std::path::Path;

use crate::api::create::{NewForm, NewIssue, NewPr};
use crate::api::model::{Capped, NodeId, PrDetail, PrRef, RepoId};
use crate::api::{ApiError, GitHub};
use crate::git::local::{self, Branch, Checkout, Clone, CommitMessage};
use crossterm::event::{KeyCode, KeyEvent};

use crate::data::{Data, DataKey};
use crate::keymap::Action;
use crate::nav::Doable;
use crate::picker::Choice;
use crate::review::ComposeTarget;
use crate::state::{Cmd, Failure, Overlay, State};
use crate::ui::overlays::PaletteItem;
use crate::ui::page::FormLink;
pub use page::build;
use templates::Chooser;

/// What git or GitHub answers, while it's on its way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Look<T> {
    Asking,
    Found(T),
    Failed(String),
}

/// What's being made, and what's known of the clones ghtui runs in.
#[derive(Debug, Default)]
pub struct Creating {
    /// The forms being written, by their page: leaving one keeps it.
    pub forms: HashMap<Route, Form>,
    /// Whether ghtui runs in a clone of each repository asked about.
    pub clones: HashMap<RepoId, Look<Option<Clone>>>,
    /// A pull request to check out once it's loaded.
    pub checkout: Option<PrRef>,
}

/// A new issue or pull request, as written so far.
#[derive(Debug, Clone, Default)]
pub struct Form {
    /// The title on the first line, then the description.
    pub text: String,
    /// The text last filled in for you: while it's still that, choosing
    /// again fills it in again.
    pub filled: String,
    /// The composer has opened on it.
    pub opened: bool,
    /// An issue's template, by its file (empty for a blank issue); none
    /// until it's chosen.
    pub template: Option<String>,
    pub chooser: Option<Chooser>,
    pub labels: Vec<NodeId>,
    pub assignees: Vec<NodeId>,
    pub milestone: Option<NodeId>,
    /// A pull request's branches: into `base` from `head`.
    pub base: Option<String>,
    pub head: Option<String>,
    /// What's known of `head` against `base`, for the two it was asked for.
    pub branch: Option<(String, String, Look<HeadState>)>,
    pub sending: bool,
    pub error: Option<String>,
}

impl Form {
    /// What's known of its head against its base, if it's for them.
    fn head_state(&self) -> Option<&Look<HeadState>> {
        let (base, head, look) = self.branch.as_ref()?;
        (Some(base) == self.base.as_ref() && Some(head) == self.head.as_ref()).then_some(look)
    }

    /// Text you haven't written yourself.
    fn untouched(&self) -> bool {
        self.text == self.filled
    }
}

/// A pull request's head against its base: the branch here, when it's
/// one of this clone's, and the commits it would bring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadState {
    pub local: Option<Branch>,
    pub commits: Capped<CommitMessage>,
}

/// Work in the background.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Work {
    /// Whether ghtui runs in a clone of the repository.
    Clone(RepoId),
    /// Where `head` stands against `base`: here and on GitHub.
    Head {
        repo: RepoId,
        base: String,
        head: String,
    },
    /// `git push` the branch, from the repository's clone.
    Push { repo: RepoId, branch: String },
    /// Make it on GitHub; the form's page asked.
    Submit(Route, Submission),
    /// Check out the pull request on branch `name`, tracking `track`.
    Checkout {
        pr: PrRef,
        name: String,
        track: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Submission {
    Issue(NewIssue),
    Pr(NewPr),
}

/// Work's answer.
#[derive(Debug)]
pub enum Done {
    Clone(RepoId, Result<Option<Clone>, Failure>),
    Head {
        repo: RepoId,
        base: String,
        head: String,
        result: Result<HeadState, Failure>,
    },
    Pushed(RepoId, String, Result<(), Failure>),
    Created(Route, Result<u64, ApiError>),
    CheckedOut(PrRef, Result<Checkout, Failure>),
}

impl Work {
    /// Its answer, had it failed: for when the task running it panics.
    pub fn failed(&self) -> Done {
        let bug = || Failure::msg("ghtui hit a bug");
        match self.clone() {
            Work::Clone(repo) => Done::Clone(repo, Err(bug())),
            Work::Head { repo, base, head } => Done::Head {
                repo,
                base,
                head,
                result: Err(bug()),
            },
            Work::Push { repo, branch } => Done::Pushed(repo, branch, Err(bug())),
            Work::Submit(route, _) => {
                Done::Created(route, Err(ApiError::Internal("ghtui hit a bug".into())))
            }
            Work::Checkout { pr, .. } => Done::CheckedOut(pr, Err(bug())),
        }
    }

    /// Does it, with `gh` and the clone `cwd` is in.
    pub async fn run(self, gh: &GitHub, cwd: &Path) -> Done {
        match self {
            Work::Clone(repo) => {
                let result = local::clone_of(cwd, &repo.owner, &repo.name)
                    .await
                    .map_err(Failure::from);
                Done::Clone(repo, result)
            }
            Work::Head { repo, base, head } => {
                let result = async {
                    let mut local = None;
                    if let Some(c) = local::clone_of(cwd, &repo.owner, &repo.name).await? {
                        local = local::branch(&c.top, &c.remote, &head).await?;
                        // Commits GitHub doesn't have yet are this clone's
                        // to tell.
                        if local.as_ref().is_some_and(|b| !b.pushed()) {
                            let commits = local::commits(&c.top, &c.remote, &base, &head).await?;
                            return Ok(HeadState {
                                local,
                                commits: commits.into(),
                            });
                        }
                    }
                    let spec = head_spec(&repo, &head, local.as_ref());
                    let commits = gh.commit_messages(&repo, &base, &spec).await?;
                    Ok(HeadState { local, commits })
                }
                .await;
                Done::Head {
                    repo,
                    base,
                    head,
                    result,
                }
            }
            Work::Push { repo, branch } => {
                let result = async {
                    let c = local::clone_of(cwd, &repo.owner, &repo.name)
                        .await?
                        .ok_or_else(|| not_a_clone(&repo))?;
                    let b = (local::branch(&c.top, &c.remote, &branch).await?)
                        .ok_or_else(|| Failure::msg(format!("{branch} isn't a branch here")))?;
                    local::push(&c.top, &b.remote, &branch, !b.tracked).await?;
                    Ok(())
                }
                .await;
                Done::Pushed(repo, branch, result)
            }
            Work::Submit(route, submission) => {
                let result = match &submission {
                    Submission::Issue(new) => gh.create_issue(new).await,
                    Submission::Pr(new) => gh.create_pull_request(new).await,
                };
                Done::Created(route, result)
            }
            Work::Checkout { pr, name, track } => {
                let result = async {
                    let c = local::clone_of(cwd, &pr.repo.owner, &pr.repo.name)
                        .await?
                        .ok_or_else(|| not_a_clone(&pr.repo))?;
                    let source = format!("refs/pull/{}/head", pr.number);
                    let done = local::checkout(&c.top, &c.remote, &source, &name, track.as_deref())
                        .await?;
                    Ok(done)
                }
                .await;
                Done::CheckedOut(pr, result)
            }
        }
    }
}

fn not_a_clone(repo: &RepoId) -> Failure {
    Failure::msg(format!("ghtui isn't running in a clone of {repo}"))
}

/// What the confirmation for a pull request offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Push,
    Create {
        draft: bool,
    },
    /// A release's, a gist's ([`crate::writes`]).
    Write(crate::writes::Answer),
}

/// A question before a pull request is pushed or made: what, what's
/// worth knowing, and the ways to go ahead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    pub route: Route,
    pub title: String,
    pub facts: Vec<(String, bool)>,
    pub choices: Vec<(String, Answer)>,
    pub selected: usize,
    pub sending: bool,
    pub error: Option<String>,
}

impl Ask {
    /// What's happening while git or GitHub answers.
    pub fn busy(&self) -> Option<&'static str> {
        let (_, answer) = self.choices.get(self.selected).filter(|_| self.sending)?;
        Some(match answer {
            Answer::Push => "Pushing…",
            Answer::Create { .. } => "Creating the pull request…",
            Answer::Write(answer) => answer.busy(),
        })
    }
}

/// The repository a form's page is for.
fn form_repo(route: &Route) -> Option<&RepoId> {
    match route {
        Route::Issues(issues::NewIssue(repo)) | Route::Pulls(pulls::NewPr(repo)) => Some(repo),
        _ => None,
    }
}

/// The form on screen (blank until it's started), and its page.
fn here(state: &State) -> Option<(Route, &Form)> {
    static BLANK: std::sync::LazyLock<Form> = std::sync::LazyLock::new(Form::default);
    let route = state.route().filter(|r| form_repo(r).is_some())?;
    let form = state.creating.forms.get(route).unwrap_or(&BLANK);
    Some((route.clone(), form))
}

/// The form on screen, started if it wasn't (its page was opened by a
/// link, before anything arrived for it).
fn here_mut(state: &mut State) -> Option<&mut Form> {
    let route = state.route().filter(|r| form_repo(r).is_some())?.clone();
    Some(state.creating.forms.entry(route).or_default())
}

/// What a new issue or pull request in `repo` can have, once loaded.
fn form_data<'a>(
    data: &'a HashMap<DataKey, crate::state::Remote<Data>>,
    repo: &RepoId,
) -> Option<&'a NewForm> {
    match data.get(&DataKey::NewForm(repo.clone()))?.data.as_ref()? {
        Data::NewForm(form) => Some(form),
        _ => None,
    }
}

/// Says `text` where it'll be seen: in the composer or the question when
/// one's open, else in the status bar.
fn say(state: &mut State, text: String) {
    match &mut state.overlay {
        Some(Overlay::Compose(c)) => c.error = Some(text),
        Some(Overlay::Ask(a)) => a.error = Some(text),
        _ => state.info(text),
    }
}

/// Why `repo` takes no new issue (or pull request), from what's known.
fn refused(state: &State, repo: &RepoId, pr: bool) -> Option<String> {
    if state.overview(repo).is_some_and(|o| o.summary.archived)
        || form_data(&state.data, repo).is_some_and(|d| d.archived)
    {
        return Some(format!("{repo} is archived: it takes nothing new"));
    }
    if pr {
        return None;
    }
    let off = state.overview(repo).is_some_and(|o| !o.has_issues)
        || form_data(&state.data, repo).is_some_and(|d| !d.has_issues);
    if off {
        return Some(format!("{repo} has issues turned off"));
    }
    form_data(&state.data, repo)
        .filter(|d| !d.can_create_issues)
        .map(|_| format!("GitHub doesn't let you open issues in {repo}"))
}

/// The actions this module runs.
#[must_use]
pub fn action(state: &mut State, action: Action) -> Vec<Cmd> {
    match action {
        Action::NewIssue | Action::NewPr => open(state, action == Action::NewPr),
        Action::Create => submit(state),
        Action::PushBranch => ask_push(state),
        Action::Checkout => checkout(state),
        _ => Vec::new(),
    }
}

/// `,i` and `,p`: the repository's new issue or pull request page, with
/// what was written there before.
#[must_use]
fn open(state: &mut State, pr: bool) -> Vec<Cmd> {
    let Some(repo) = state.route().and_then(Route::repo).cloned() else {
        state.info("Open a repository first: a new issue or pull request goes in one");
        return Vec::new();
    };
    if let Some(why) = refused(state, &repo, pr) {
        state.info(why);
        return Vec::new();
    }
    let route = if pr {
        Route::Pulls(pulls::NewPr(repo.clone()))
    } else {
        Route::Issues(issues::NewIssue(repo.clone()))
    };
    if state.route() == Some(&route) {
        return compose(state);
    }
    // The clone may have moved on since: its branch is looked at again.
    if pr {
        state.creating.clones.remove(&repo);
        if let Some(form) = state.creating.forms.get_mut(&route) {
            form.branch = None;
        }
    }
    let mut cmds = state.push(route);
    cmds.extend(follow(state));
    cmds
}

/// Brings the form on screen up to date with what's arrived: starts it,
/// fills in what's known, asks for what's missing; and checks out a pull
/// request that was waiting to load.
#[must_use]
pub fn follow(state: &mut State) -> Vec<Cmd> {
    let mut cmds = waiting_checkout(state);
    let Some(route) = state.route().filter(|r| form_repo(r).is_some()).cloned() else {
        return cmds;
    };
    let Some(repo) = form_repo(&route).cloned() else {
        return cmds;
    };
    let pr = matches!(route, Route::Pulls(pulls::NewPr(_)));
    if pr && !state.creating.clones.contains_key(&repo) {
        state.creating.clones.insert(repo.clone(), Look::Asking);
        cmds.push(Cmd::Create(Work::Clone(repo.clone())));
    }
    let State {
        creating,
        data,
        overlay,
        ..
    } = &mut *state;
    let form = creating.forms.entry(route).or_default();
    let Some(d) = form_data(data, &repo) else {
        return cmds;
    };
    if pr {
        if form.base.is_none() {
            form.base.clone_from(&d.default_branch);
        }
        // Inside a clone, its branch is the one to propose.
        if form.head.is_none()
            && let Some(Look::Found(Some(clone))) = creating.clones.get(&repo)
            && clone.branch != form.base
        {
            form.head.clone_from(&clone.branch);
        }
        if form.head_state().is_none()
            && let (Some(base), Some(head)) = (form.base.clone(), form.head.clone())
            && base != head
        {
            form.branch = Some((base.clone(), head.clone(), Look::Asking));
            cmds.push(Cmd::Create(Work::Head { repo, base, head }));
        }
        return cmds;
    }
    let chooser = form
        .chooser
        .get_or_insert_with(|| templates::chooser(&d.issue_templates));
    // Without templates to choose from, it's a blank issue.
    if form.template.is_none() && !chooser.offers() {
        form.template = Some(String::new());
    }
    // Ready to write: the composer opens, once.
    if form.template.is_some()
        && !form.opened
        && overlay.is_none()
        && refused(state, &repo, false).is_none()
    {
        cmds.extend(compose(state));
    }
    cmds
}

/// The composer, on the form on screen: `c`, or Enter on its text box.
#[must_use]
pub fn compose(state: &mut State) -> Vec<Cmd> {
    let Some((route, form)) = here(state) else {
        return Vec::new();
    };
    if form.sending {
        state.info("It's being created");
        return Vec::new();
    }
    if matches!(route, Route::Issues(issues::NewIssue(_))) && form.template.is_none() {
        state.info("Choose a template first");
        return Vec::new();
    }
    let text = form.text.clone();
    if let Some(form) = here_mut(state) {
        form.opened = true;
    }
    // Its box on the page makes way.
    state.data_gen += 1;
    let cmds = state.compose(ComposeTarget::Form(route), &text);
    // The title first, as on GitHub.
    if let Some(Overlay::Compose(c)) = &mut state.overlay {
        c.input.move_cursor(ratatui_textarea::CursorMove::Top);
        c.input.move_cursor(ratatui_textarea::CursorMove::End);
    }
    cmds
}

/// The composer's text, kept with its form (esc, or before making it).
pub fn keep_text(state: &mut State, route: &Route, text: String) {
    let form = state.creating.forms.entry(route.clone()).or_default();
    form.text = text;
    form.error = None;
    state.data_gen += 1;
}

/// `,s`, ctrl-s in the composer, Enter on the form's button: makes the
/// issue; for a pull request, asks first whether it's a draft.
#[must_use]
pub fn submit(state: &mut State) -> Vec<Cmd> {
    if let Some(Overlay::Compose(c)) = &state.overlay
        && let ComposeTarget::Form(route) = &c.target
    {
        let (route, text) = (route.clone(), c.text());
        keep_text(state, &route, text);
    }
    let Some((route, form)) = here(state) else {
        state.info("That makes the new issue or pull request on its page (,i or ,p)");
        return Vec::new();
    };
    let Some(repo) = form_repo(&route).cloned() else {
        return Vec::new();
    };
    if form.sending {
        say(state, "It's being created".into());
        return Vec::new();
    }
    let (title, body) = templates::split(&form.text);
    let Some(d) = form_data(&state.data, &repo) else {
        say(state, "Still loading what it can have".into());
        return Vec::new();
    };
    if let Some(why) = refused(state, &repo, matches!(route, Route::Pulls(pulls::NewPr(_)))) {
        say(state, why);
        return Vec::new();
    }
    if matches!(route, Route::Issues(issues::NewIssue(_))) && form.template.is_none() {
        say(state, "Choose a template first".into());
        return Vec::new();
    }
    if title.is_empty() {
        let key = state.first_key(Action::Comment);
        say(
            state,
            format!("Write a title first: it's the first line ({key} writes)"),
        );
        return Vec::new();
    }
    if let Route::Pulls(pulls::NewPr(_)) = route {
        return ask_create(state, &route);
    }
    let triage = |ids: &[NodeId]| {
        if d.can_triage {
            ids.to_vec()
        } else {
            Vec::new()
        }
    };
    let new = NewIssue {
        repo: d.repo_id.clone(),
        title,
        body,
        labels: triage(&form.labels),
        assignees: triage(&form.assignees),
        milestone: form.milestone.clone().filter(|_| d.can_triage),
    };
    sending(state, true);
    state.info("Creating the issue…");
    vec![Cmd::Create(Work::Submit(route, Submission::Issue(new)))]
}

/// Marks the form, and the composer or question that sent it, as waiting
/// for GitHub (or not).
fn sending(state: &mut State, on: bool) {
    if let Some(form) = here_mut(state) {
        form.sending = on;
        form.error = None;
    }
    state.data_gen += 1;
    match &mut state.overlay {
        Some(Overlay::Compose(c)) => {
            c.sending = on;
            c.error = None;
        }
        Some(Overlay::Ask(a)) => {
            a.sending = on;
            a.error = None;
        }
        _ => {}
    }
}

/// A pull request's head as `createPullRequest` takes it: a fork's
/// branch is `owner:branch`.
fn head_spec(repo: &RepoId, head: &str, local: Option<&Branch>) -> String {
    let owner = local.and_then(|b| b.owner.as_deref());
    match owner {
        Some(owner) if !owner.eq_ignore_ascii_case(&repo.owner) => format!("{owner}:{head}"),
        _ => head.to_owned(),
    }
}

/// Why the form's head can't be proposed yet, if it can't: it isn't
/// chosen, isn't on GitHub as it is here, or brings nothing.
fn head_problem(state: &State, form: &Form) -> Result<HeadState, String> {
    let push = state.first_key(Action::PushBranch);
    let (Some(base), Some(head)) = (&form.base, &form.head) else {
        return Err("Choose the branch with the changes: Enter on “compare”".into());
    };
    if base == head {
        return Err(format!(
            "{head} is the base too: choose another branch to compare"
        ));
    }
    let look = form.head_state().unwrap_or(&Look::Asking);
    let hs = match look {
        Look::Asking => return Err(format!("Still comparing {head} with {base}")),
        Look::Failed(why) => return Err(format!("Couldn't compare {head} with {base}: {why}")),
        Look::Found(hs) => hs,
    };
    if let Some(b) = &hs.local
        && !b.pushed()
    {
        return Err(match (&b.on_github, b.ahead, b.behind) {
            (None, ..) => format!("{head} isn't on GitHub: {push} pushes it to {}", b.remote),
            (_, Some(ahead), Some(0)) => format!(
                "GitHub's {head} is {} behind yours: {push} pushes them",
                count(ahead, "commit")
            ),
            _ => format!(
                "GitHub's {head} and yours have each moved on: pull GitHub's into yours, then {push} pushes"
            ),
        });
    }
    if hs.commits.total() == 0 {
        return Err(format!(
            "{head} has nothing {base} doesn't: there's nothing to merge"
        ));
    }
    Ok(hs.clone())
}

/// Asks whether to open the pull request as a draft.
#[must_use]
fn ask_create(state: &mut State, route: &Route) -> Vec<Cmd> {
    let Some(form) = state.creating.forms.get(route) else {
        return Vec::new();
    };
    let Some(repo) = form_repo(route) else {
        return Vec::new();
    };
    let hs = match head_problem(state, form) {
        Ok(hs) => hs,
        Err(why) => {
            say(state, why);
            return Vec::new();
        }
    };
    let (base, head) = (
        form.base.clone().unwrap_or_default(),
        form.head.clone().unwrap_or_default(),
    );
    let (title, _) = templates::split(&form.text);
    let commits = count(hs.commits.total(), "commit");
    let facts = vec![
        (
            format!(
                "Into {base} from {}",
                head_spec(repo, &head, hs.local.as_ref())
            ),
            false,
        ),
        (format!("{commits}: “{title}”"), false),
    ];
    let ask = Ask {
        route: route.clone(),
        title: format!("Open a pull request in {repo}?"),
        facts,
        choices: vec![
            (
                "Create pull request".into(),
                Answer::Create { draft: false },
            ),
            (
                "Create draft pull request".into(),
                Answer::Create { draft: true },
            ),
        ],
        selected: 0,
        sending: false,
        error: None,
    };
    state.overlay = Some(Overlay::Ask(Box::new(ask)));
    Vec::new()
}

/// Asks before pushing the form's head branch from this clone.
#[must_use]
fn ask_push(state: &mut State) -> Vec<Cmd> {
    let Some((route, form)) =
        here(state).filter(|(r, _)| matches!(r, Route::Pulls(pulls::NewPr(_))))
    else {
        state.info("Pushing works on a new pull request's branch (,p)");
        return Vec::new();
    };
    let Some(repo) = form_repo(&route).cloned() else {
        return Vec::new();
    };
    let why = match (
        state.creating.clones.get(&repo),
        form.head_state(),
        &form.head,
    ) {
        (_, _, None) => Some("Choose the branch to push first".to_owned()),
        (Some(Look::Found(None)), ..) => Some(format!(
            "ghtui isn't running in a clone of {repo}: push it with git"
        )),
        (_, Some(Look::Found(HeadState { local: None, .. })), Some(head)) => Some(format!(
            "{head} isn't a branch of this clone: there's nothing here to push"
        )),
        (_, Some(Look::Found(HeadState { local: Some(b), .. })), _) if b.pushed() => {
            Some(format!("GitHub has every commit of {} already", b.name))
        }
        (_, Some(Look::Found(_)), _) => None,
        _ => Some("Still comparing the branch with GitHub's".to_owned()),
    };
    if let Some(why) = why {
        state.info(why);
        return Vec::new();
    }
    let Some(Look::Found(HeadState { local: Some(b), .. })) = form.head_state() else {
        return Vec::new();
    };
    let upstream = if b.tracked { "" } else { " --set-upstream" };
    let mut facts = vec![(
        format!("Runs git push{upstream} {} {}", b.remote, b.name),
        false,
    )];
    match (b.ahead, b.behind) {
        (Some(ahead), Some(0)) => facts.push((
            format!("{} GitHub doesn't have", count(ahead, "commit")),
            false,
        )),
        (_, Some(behind)) if behind > 0 => facts.push((
            format!(
                "GitHub's {} has {} yours doesn't: git refuses until you pull them",
                b.name,
                count(behind, "commit")
            ),
            true,
        )),
        _ if b.on_github.is_none() => facts.push((format!("GitHub has no {} yet", b.name), false)),
        _ => {}
    }
    let ask = Ask {
        route,
        title: format!("Push {} to {}?", b.name, b.remote),
        facts,
        choices: vec![("Push".into(), Answer::Push)],
        selected: 0,
        sending: false,
        error: None,
    };
    state.overlay = Some(Overlay::Ask(Box::new(ask)));
    Vec::new()
}

/// Keys in the question: choose, go ahead, or cancel.
#[must_use]
pub fn on_ask_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    let Some(Overlay::Ask(ask)) = &mut state.overlay else {
        return Vec::new();
    };
    let n = ask.choices.len().max(1);
    match key.code {
        KeyCode::Esc => state.overlay = None,
        _ if ask.sending => {}
        KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => ask.selected = (ask.selected + 1) % n,
        KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
            ask.selected = (ask.selected + n - 1) % n;
        }
        KeyCode::Enter => {
            let Some((_, answer)) = ask.choices.get(ask.selected).cloned() else {
                return Vec::new();
            };
            let route = ask.route.clone();
            return answer_ask(state, &route, answer);
        }
        _ => {}
    }
    Vec::new()
}

#[must_use]
fn answer_ask(state: &mut State, route: &Route, answer: Answer) -> Vec<Cmd> {
    if let Answer::Write(answer) = answer {
        return crate::writes::answer(state, route, answer);
    }
    let Some(form) = state.creating.forms.get(route) else {
        return Vec::new();
    };
    let Some(repo) = form_repo(route).cloned() else {
        return Vec::new();
    };
    let head = form.head.clone().unwrap_or_default();
    let work = match answer {
        Answer::Write(_) => return Vec::new(),
        Answer::Push => Work::Push { repo, branch: head },
        Answer::Create { draft } => {
            let hs = match head_problem(state, form) {
                Ok(hs) => hs,
                Err(why) => {
                    say(state, why);
                    return Vec::new();
                }
            };
            let Some(d) = form_data(&state.data, &repo) else {
                return Vec::new();
            };
            let (title, body) = templates::split(&form.text);
            Work::Submit(
                route.clone(),
                Submission::Pr(NewPr {
                    repo: d.repo_id.clone(),
                    base: form.base.clone().unwrap_or_default(),
                    head: head_spec(&repo, &head, hs.local.as_ref()),
                    title,
                    body,
                    draft,
                }),
            )
        }
    };
    sending(state, true);
    vec![Cmd::Create(work)]
}

/// Work's answers.
#[must_use]
pub fn on_done(state: &mut State, done: Done) -> Vec<Cmd> {
    match done {
        Done::Clone(repo, result) => {
            let look = match result {
                Ok(clone) => Look::Found(clone),
                Err(err) => Look::Failed(err.to_string()),
            };
            state.creating.clones.insert(repo, look);
            Vec::new()
        }
        Done::Head {
            repo,
            base,
            head,
            result,
        } => {
            let route = Route::Pulls(pulls::NewPr(repo.clone()));
            let template = form_data(&state.data, &repo).and_then(|d| d.pr_template.clone());
            let Some(form) = state.creating.forms.get_mut(&route) else {
                return Vec::new();
            };
            if form
                .branch
                .as_ref()
                .is_none_or(|(b, h, _)| (b, h) != (&base, &head))
            {
                return Vec::new();
            }
            let look = match result {
                Ok(hs) => {
                    // Filled in as GitHub would, unless you've written.
                    if form.untouched() {
                        form.text = templates::pr_text(&head, &hs.commits, template.as_deref());
                        form.filled = form.text.clone();
                    }
                    Look::Found(hs)
                }
                Err(err) => Look::Failed(err.to_string()),
            };
            form.branch = Some((base, head, look));
            Vec::new()
        }
        Done::Pushed(repo, branch, result) => {
            let route = Route::Pulls(pulls::NewPr(repo));
            match result {
                Ok(()) => {
                    if matches!(&state.overlay, Some(Overlay::Ask(a)) if a.route == route) {
                        state.overlay = None;
                    }
                    if let Some(form) = state.creating.forms.get_mut(&route) {
                        form.sending = false;
                        form.branch = None;
                    }
                    state.info(format!("Pushed {branch}"));
                    Vec::new()
                }
                Err(err) => {
                    if let Some(form) = state.creating.forms.get_mut(&route) {
                        form.sending = false;
                    }
                    let text = format!("Couldn't push {branch}: {err}");
                    match &mut state.overlay {
                        Some(Overlay::Ask(a)) if a.route == route => {
                            a.sending = false;
                            a.error = Some(text);
                        }
                        _ => state.error(text),
                    }
                    Vec::new()
                }
            }
        }
        Done::Created(route, result) => created(state, &route, result),
        Done::CheckedOut(pr, result) => {
            let n = pr.number;
            match result {
                Ok(Checkout::Created(b)) => {
                    state.info(format!(
                        "Checked out #{n}: you're on {b}, a new branch at its head"
                    ));
                }
                Ok(Checkout::Switched(b)) => state.info(format!("You're on {b}, at #{n}'s head")),
                Ok(Checkout::Diverged(b)) => state.info(format!(
                    "You're on {b}, which has commits #{n} doesn't: it's left as it was"
                )),
                Ok(Checkout::Uncommitted(files)) => {
                    let first = files.first().cloned().unwrap_or_default();
                    let files = match files.len() {
                        1 => first,
                        n => format!("{first} and {}", count(n as u64 - 1, "other")),
                    };
                    state.info(format!(
                        "Not checked out: {files} has changes that aren't committed. Commit or stash them first"
                    ));
                }
                Err(err) => state.error(format!("Couldn't check out #{n}: {err}")),
            }
            Vec::new()
        }
    }
}

/// GitHub made it (and its page opens in place of the form's) or said why
/// not.
#[must_use]
fn created(state: &mut State, route: &Route, result: Result<u64, ApiError>) -> Vec<Cmd> {
    let ours = |o: &Option<Overlay>| match o {
        Some(Overlay::Compose(c)) => c.target == ComposeTarget::Form(route.clone()),
        Some(Overlay::Ask(a)) => a.route == *route,
        _ => false,
    };
    let Some(repo) = form_repo(route).cloned() else {
        return Vec::new();
    };
    match result {
        Ok(number) => {
            state.creating.forms.remove(route);
            if ours(&state.overlay) {
                state.overlay = None;
            }
            let (page, what) = match route {
                Route::Pulls(pulls::NewPr(_)) => {
                    (Route::pr(PrRef { repo, number }), "pull request")
                }
                _ => (Route::Issues(issues::Issue { repo, number }), "issue"),
            };
            state.info(format!("Opened {what} #{number}"));
            if state.route() == Some(route) {
                return state.replace(page, true, None);
            }
            Vec::new()
        }
        Err(err) => {
            tracing::warn!(%err, "creating failed");
            let what = match route {
                Route::Pulls(pulls::NewPr(_)) => "the pull request",
                _ => "the issue",
            };
            let reason = match &err {
                ApiError::GraphQl(messages) => messages.join("; "),
                ApiError::Http { message, .. } => message.clone(),
                err => err.to_string(),
            };
            let text = format!("GitHub didn't create {what}: {reason}");
            if let Some(form) = state.creating.forms.get_mut(route) {
                form.sending = false;
                form.error = Some(text.clone());
            }
            match &mut state.overlay {
                Some(Overlay::Compose(c)) if c.target == ComposeTarget::Form(route.clone()) => {
                    c.sending = false;
                    c.error = Some(text);
                }
                Some(Overlay::Ask(a)) if a.route == *route => {
                    a.sending = false;
                    a.error = Some(text);
                }
                _ => state.error(text),
            }
            Vec::new()
        }
    }
}

// ---- checking out --------------------------------------------------------------

/// `,c`: the pull request on screen (or its row) checked out here, once
/// it's loaded.
#[must_use]
fn checkout(state: &mut State) -> Vec<Cmd> {
    let Some(crate::act::Subject::Pr(pr)) = crate::act::subject(state, Action::Checkout) else {
        state.info("Checking out works on a pull request (or its row in a list)");
        return Vec::new();
    };
    if let Some(Look::Found(None)) = state.creating.clones.get(&pr.repo) {
        state.info(format!("ghtui isn't running in a clone of {}", pr.repo));
        return Vec::new();
    }
    match state.pr(&pr) {
        Some(d) => start_checkout(state, pr.clone(), &d.clone()),
        None => {
            state.info(format!("Loading {pr}…"));
            state.creating.checkout = Some(pr.clone());
            state.ensure(DataKey::Pr(pr), false)
        }
    }
}

/// A pull request that was waiting to load before it's checked out.
#[must_use]
fn waiting_checkout(state: &mut State) -> Vec<Cmd> {
    let Some(pr) = state.creating.checkout.clone() else {
        return Vec::new();
    };
    let Some(remote) = state.pr_remote(&pr) else {
        return Vec::new();
    };
    match (state.pr(&pr), &remote.error) {
        (Some(d), _) => {
            let d = d.clone();
            state.creating.checkout = None;
            start_checkout(state, pr, &d)
        }
        (None, Some(err)) if !remote.loading => {
            let text = format!("Couldn't load {pr}: {err}");
            state.creating.checkout = None;
            state.error(text);
            Vec::new()
        }
        _ => Vec::new(),
    }
}

/// Checks out `pr`: on its own branch's name when it's from this
/// repository (tracking it), else `owner/branch` (a fork's).
#[must_use]
fn start_checkout(state: &mut State, pr: PrRef, d: &PrDetail) -> Vec<Cmd> {
    let same =
        (d.head_repo.as_deref()).is_some_and(|r| r.eq_ignore_ascii_case(&pr.repo.to_string()));
    let fork = d
        .head_repo
        .as_deref()
        .and_then(|r| r.split_once('/'))
        .map(|(o, _)| o);
    let (name, track) = match fork {
        _ if same => (d.head_ref.clone(), Some(d.head_ref.clone())),
        Some(owner) => (format!("{owner}/{}", d.head_ref), None),
        None => (format!("pr-{}", pr.number), None),
    };
    state.info(format!("Checking out {pr} on {name}…"));
    vec![Cmd::Create(Work::Checkout { pr, name, track })]
}

// ---- links, pickers and the menu ---------------------------------------------

/// Enter on one of a form's rows.
#[must_use]
pub fn follow_link(state: &mut State, link: &FormLink) -> Vec<Cmd> {
    match link {
        FormLink::Template(file) => choose_template(state, file),
        FormLink::Templates => {
            if let Some(form) = here_mut(state) {
                form.template = None;
            }
            state.data_gen += 1;
            Vec::new()
        }
        FormLink::Labels => open_pick(state, Pick::Labels),
        FormLink::Assignees => open_pick(state, Pick::Assignees),
        FormLink::Milestone => open_pick(state, Pick::Milestone),
        FormLink::Base => open_pick(state, Pick::Base),
        FormLink::Head => open_pick(state, Pick::Head),
        FormLink::Push => ask_push(state),
        FormLink::Create => submit(state),
    }
}

/// What Enter on one of a form's rows does, for the menu.
pub fn describe(link: &FormLink) -> &'static str {
    match link {
        FormLink::Template(file) if file.is_empty() => "Start a blank issue",
        FormLink::Template(_) => "Start from this template",
        FormLink::Templates => "Choose another template",
        FormLink::Labels => "Choose labels",
        FormLink::Assignees => "Choose assignees",
        FormLink::Milestone => "Choose a milestone",
        FormLink::Base => "Choose the branch to merge into",
        FormLink::Head => "Choose the branch with the changes",
        FormLink::Push => "Push the branch to GitHub",
        FormLink::Create => "Create it",
    }
}

/// Starts the issue from template `file` (empty: a blank one): its title,
/// text, labels and assignees, unless you've written your own text.
#[must_use]
fn choose_template(state: &mut State, file: &str) -> Vec<Cmd> {
    let Some((route, _)) = here(state) else {
        return Vec::new();
    };
    let Some(repo) = form_repo(&route).cloned() else {
        return Vec::new();
    };
    let State { creating, data, .. } = &mut *state;
    let (Some(form), Some(d)) = (creating.forms.get_mut(&route), form_data(data, &repo)) else {
        return Vec::new();
    };
    let template = (form.chooser.as_ref())
        .and_then(|c| c.templates.iter().find(|t| t.file == file))
        .cloned();
    form.template = Some(file.to_owned());
    if let Some(t) = &template {
        let ids = |names: &[String], from: &Capped<crate::api::create::Choice>| -> Vec<NodeId> {
            (from.iter())
                .filter(|c| names.iter().any(|n| n.eq_ignore_ascii_case(&c.name)))
                .map(|c| c.id.clone())
                .collect()
        };
        form.labels = ids(&t.labels, &d.labels);
        form.assignees = ids(&t.assignees, &d.assignees);
    }
    if form.untouched() {
        form.text = template
            .map(|t| templates::join(&t.title, &t.body))
            .unwrap_or_default();
        form.filled = form.text.clone();
    }
    state.data_gen += 1;
    compose(state)
}

/// A form's picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    Labels,
    Assignees,
    Milestone,
    Base,
    Head,
}

impl Pick {
    pub fn title(self) -> &'static str {
        match self {
            Pick::Labels => "Labels",
            Pick::Assignees => "Assignees",
            Pick::Milestone => "Milestone",
            Pick::Base => "Base branch: into",
            Pick::Head => "Compare branch: from",
        }
    }

    /// Several can be picked: choosing one keeps the picker open.
    pub fn several(self) -> bool {
        matches!(self, Pick::Labels | Pick::Assignees)
    }
}

/// What choosing one of a form picker's rows does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Picked {
    Label(NodeId),
    Assignee(NodeId),
    Milestone(Option<NodeId>),
    Base(String),
    Head(String),
}

#[must_use]
fn open_pick(state: &mut State, pick: Pick) -> Vec<Cmd> {
    let Some((route, _)) = here(state) else {
        return Vec::new();
    };
    let Some(repo) = form_repo(&route).cloned() else {
        return Vec::new();
    };
    let what = match pick {
        Pick::Labels => "labels",
        Pick::Assignees => "assignees",
        Pick::Milestone => "a milestone",
        Pick::Base | Pick::Head => "",
    };
    let triage = form_data(&state.data, &repo).map(|d| d.can_triage);
    if !what.is_empty() && triage == Some(false) {
        state.info(format!(
            "Setting {what} takes triage access to {repo}: GitHub would leave them out"
        ));
        return Vec::new();
    }
    state.open_picker(crate::picker::Kind::Form(pick))
}

/// The rows of a form's picker matching `q`.
pub fn picker_rows(state: &State, pick: Pick, q: &str) -> Vec<(PaletteItem, Option<Choice>)> {
    let item = |label: String, hint: String| PaletteItem { label, hint };
    let note = |text: String| vec![(item(text, String::new()), None)];
    let Some((route, form)) = here(state) else {
        return note("The form isn't on screen".into());
    };
    let Some(repo) = form_repo(&route) else {
        return Vec::new();
    };
    let fits = |s: &str| crate::picker::fuzzy_score(q, s).is_some();
    if matches!(pick, Pick::Base | Pick::Head) {
        let key = DataKey::Refs(repo.clone());
        let Some(Data::Refs(refs)) = state.get(&key) else {
            let failed = state.remote(&key).and_then(|r| r.error.clone());
            return note(failed.map_or_else(
                || "Loading branches…".to_owned(),
                |e| format!("Couldn't load the branches: {e}"),
            ));
        };
        let chosen = if pick == Pick::Base {
            &form.base
        } else {
            &form.head
        };
        let current = match state.creating.clones.get(repo) {
            Some(Look::Found(Some(c))) => c.branch.clone(),
            _ => None,
        };
        let default = form_data(&state.data, repo).and_then(|d| d.default_branch.clone());
        let mut names: Vec<&String> = refs.branches.iter().collect();
        // A branch only this clone has can be compared (and pushed).
        if pick == Pick::Head
            && let Some(c) = current.as_ref().filter(|c| !refs.branches.contains(c))
        {
            names.insert(0, c);
        }
        let mut rows: Vec<_> = (names.into_iter())
            .filter(|n| fits(n))
            .map(|name| {
                let hint = match () {
                    () if chosen.as_ref() == Some(name) => "chosen",
                    () if current.as_ref() == Some(name) => "checked out here",
                    () if default.as_ref() == Some(name) => "default",
                    () => "",
                };
                let picked = match pick {
                    Pick::Base => Picked::Base(name.clone()),
                    _ => Picked::Head(name.clone()),
                };
                (item(name.clone(), hint.into()), Some(Choice::Form(picked)))
            })
            .collect();
        if refs.branches.left_out() > 0 {
            let n = refs.branches.left_out();
            rows.extend(note(format!(
                "{n} more {} on GitHub",
                crate::ui::text::noun(n, "branch")
            )));
        }
        return rows;
    }
    let Some(d) = form_data(&state.data, repo) else {
        return note("Loading…".into());
    };
    let (list, chosen, one): (_, Vec<&NodeId>, &str) = match pick {
        Pick::Labels => (&d.labels, form.labels.iter().collect(), "label"),
        Pick::Assignees => (&d.assignees, form.assignees.iter().collect(), "person"),
        _ => (&d.milestones, form.milestone.iter().collect(), "milestone"),
    };
    let mut rows = Vec::new();
    if pick == Pick::Milestone && fits("No milestone") {
        let hint = if chosen.is_empty() { "chosen" } else { "" };
        rows.push((
            item("No milestone".into(), hint.into()),
            Some(Choice::Form(Picked::Milestone(None))),
        ));
    }
    for c in list.iter().filter(|c| fits(&c.name) || fits(&c.about)) {
        let on = chosen.contains(&&c.id);
        let label = format!("{} {}", if on { "✓" } else { " " }, c.name);
        let picked = match pick {
            Pick::Labels => Picked::Label(c.id.clone()),
            Pick::Assignees => Picked::Assignee(c.id.clone()),
            _ => Picked::Milestone(Some(c.id.clone())),
        };
        rows.push((item(label, c.about.clone()), Some(Choice::Form(picked))));
    }
    if list.is_empty() && list.left_out() == 0 {
        rows.extend(note(format!("{repo} has no {one} to choose")));
    }
    if list.left_out() > 0 {
        let n = list.left_out();
        rows.extend(note(format!("{n} more on GitHub")));
    }
    rows
}

/// Choosing one of a form picker's rows.
#[must_use]
pub fn pick(state: &mut State, picked: Picked) -> Vec<Cmd> {
    let Some(form) = here_mut(state) else {
        return Vec::new();
    };
    let toggle = |list: &mut Vec<NodeId>, id: NodeId| match list.iter().position(|i| *i == id) {
        Some(at) => {
            list.remove(at);
        }
        None => list.push(id),
    };
    match picked {
        Picked::Label(id) => toggle(&mut form.labels, id),
        Picked::Assignee(id) => toggle(&mut form.assignees, id),
        Picked::Milestone(id) => form.milestone = id,
        Picked::Base(name) => form.base = Some(name),
        Picked::Head(name) => form.head = Some(name),
    }
    state.data_gen += 1;
    follow(state)
}

/// What the menu offers here: a new issue or pull request from a
/// repository's pages, and writing, pushing and making one on its form.
pub fn doables(state: &State, route: &Route, out: &mut Vec<Doable>) {
    let row = |section, action: Action, label: &str, why: Option<String>| Doable {
        section,
        action,
        label: label.to_owned(),
        short: "",
        unavailable: why,
    };
    let Some(repo) = route.repo() else {
        return;
    };
    if let Some((route, form)) = here(state) {
        let pr = matches!(route, Route::Pulls(pulls::NewPr(_)));
        let mut write = row(
            "This page",
            Action::Comment,
            "Write the title and description",
            None,
        );
        write.short = "write";
        if !pr && form.template.is_none() {
            write.unavailable = Some("choose a template first".into());
        }
        out.push(write);
        let why = match (pr, refused(state, repo, pr)) {
            (_, Some(why)) => Some(why),
            (true, None) => head_problem(state, form).err(),
            (false, None) => templates::split(&form.text)
                .0
                .is_empty()
                .then(|| "it needs a title".to_owned()),
        };
        let label = if pr {
            "Create the pull request (draft or ready)"
        } else {
            "Create the issue"
        };
        let mut create = row("This page", Action::Create, label, why);
        create.short = "create";
        out.push(create);
        if pr {
            let push = state.first_key(Action::PushBranch);
            let why = match form.head_state() {
                Some(Look::Found(HeadState { local: Some(b), .. })) if !b.pushed() => None,
                Some(Look::Found(HeadState { local: Some(_), .. })) => {
                    Some("GitHub has all of it already".into())
                }
                Some(Look::Found(_)) => Some("it isn't a branch of this clone".into()),
                _ => Some(format!("no branch here to push ({push})")),
            };
            out.push(row(
                "This page",
                Action::PushBranch,
                "Push the branch to GitHub",
                why,
            ));
        }
    }
    out.push(row(
        "New",
        Action::NewIssue,
        "New issue",
        refused(state, repo, false),
    ));
    out.push(row(
        "New",
        Action::NewPr,
        "New pull request",
        refused(state, repo, true),
    ));
}

/// The menu's row for checking out the pull request here, if there is one.
pub fn checkout_doable(state: &State) -> Option<Doable> {
    let Some(crate::act::Subject::Pr(pr)) = crate::act::subject(state, Action::Checkout) else {
        return None;
    };
    let why = match state.creating.clones.get(&pr.repo) {
        Some(Look::Found(None)) => Some(format!("ghtui isn't running in a clone of {}", pr.repo)),
        _ => None,
    };
    Some(Doable {
        section: "Change",
        action: Action::Checkout,
        label: "Check out the pull request here".into(),
        short: "",
        unavailable: why,
    })
}

/// "3 commits".
fn count(n: u64, word: &str) -> String {
    format!("{n} {}", crate::ui::text::noun(n, word))
}
