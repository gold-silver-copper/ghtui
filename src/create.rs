//! Making issues and pull requests.
//!
//! A new issue or pull request is a page (`issues::NewIssue`,
//! `pulls::NewPr`) holding a [`Form`]: its title and description are
//! written in the comment composer, which opens in place of the form's
//! text box, with the title on the first line. A new issue starts from the
//! repository's templates when it has some; a new pull request from its
//! commits, as GitHub fills one in. What runs in the background (git in
//! your clone, GitHub's answers) is a job ([`Push`], [`Submit`]).

mod page;
pub mod templates;

use crate::route::{Route, issues, pulls};
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;

use crate::api::create::{NewForm, NewIssue, NewPr};
use crate::api::model::{Capped, NodeId, PrRef, RepoId};
use crate::api::{ApiError, GitHub};
use crate::git::local::{self, Branch, CommitMessage};

use crate::data::{Data, DataKey};
use crate::forms::{Answer, Fields, Sent, say};
use crate::keymap::Action;
use crate::nav::{Doable, row};
use crate::picker::Choice;
use crate::state::{Cmd, State};
use crate::ui::form::Row;
use crate::work::{self, Cx, Failure, Job};
pub use page::build;
use templates::Chooser;

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
    /// The base and head its text was last filled in for.
    pub filled_for: Option<(String, String)>,
}

impl Fields for Form {
    fn text(&mut self, _: usize) -> Option<&mut String> {
        Some(&mut self.text)
    }

    fn title(&self, route: &Route, _: usize) -> String {
        match route {
            Route::Pulls(pulls::NewPr(_)) => {
                "New pull request: its title on the first line, then its description".into()
            }
            _ => "New issue: its title on the first line, then its description".into(),
        }
    }

    fn save(&self, route: &Route) -> &'static str {
        match route {
            Route::Pulls(pulls::NewPr(_)) => "create pull request…",
            _ => "create issue",
        }
    }
}

impl Form {
    /// Where its head stands against its base, as it's read.
    fn head_key(&self, repo: &RepoId) -> Option<DataKey> {
        let (base, head) = (self.base.clone()?, self.head.clone()?);
        (base != head).then(|| DataKey::Head {
            repo: repo.clone(),
            base,
            head,
        })
    }

    /// Where its head stands against its base, once it's read.
    fn head_state<'a>(
        &self,
        state: &'a State,
        repo: &RepoId,
    ) -> Option<Result<&'a HeadState, &'a str>> {
        state.answer(&self.head_key(repo)?)
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

/// `git push` the branch, from the repository's clone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Push {
    pub repo: RepoId,
    pub branch: String,
}

/// Makes it on GitHub; the form's page asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Submit {
    pub route: Route,
    pub new: Submission,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Submission {
    Issue(NewIssue),
    Pr(NewPr),
}

/// Where `head` stands against `base`: in the clone ghtui runs in, if
/// it's one of `repo`'s, and on GitHub.
pub async fn head_state(
    gh: &GitHub,
    cwd: &Path,
    repo: &RepoId,
    base: &str,
    head: &str,
) -> Result<HeadState, ApiError> {
    let mut local = None;
    if let Some(c) = local::clone_of(cwd, &repo.owner, &repo.name).await? {
        local = local::branch(&c.top, &c.remote, head).await?;
        // Commits GitHub doesn't have yet are this clone's to tell.
        if local.as_ref().is_some_and(|b| !b.pushed()) {
            let commits = local::commits(&c.top, &c.remote, base, head).await?;
            return Ok(HeadState {
                local,
                commits: commits.into(),
            });
        }
    }
    let spec = head_spec(repo, head, local.as_ref());
    let commits = gh.commit_messages(repo, base, &spec).await?;
    Ok(HeadState { local, commits })
}

/// The repository a form's page is for.
fn form_repo(route: &Route) -> Option<&RepoId> {
    match route {
        Route::Issues(issues::NewIssue(repo)) | Route::Pulls(pulls::NewPr(repo)) => Some(repo),
        _ => None,
    }
}

/// The form on screen (blank until it's started), and its page.
fn here(state: &State) -> Option<(Route, Cow<'_, Form>)> {
    let route = state.route().filter(|r| form_repo(r).is_some())?;
    Some((route.clone(), state.forms.shown(route)))
}

/// The form on screen, started if it wasn't (its page was opened by a
/// link, before anything arrived for it).
fn here_mut(state: &mut State) -> Option<&mut Form> {
    let route = state.route().filter(|r| form_repo(r).is_some())?.clone();
    Some(state.forms.edit(&route))
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

/// What new issues and pull requests add: making one, and pushing its
/// branch.
pub const FEATURE: crate::features::Feature = crate::features::Feature {
    owns: &[Action::NewIssue, Action::NewPr, Action::PushBranch],
    act: |state, action| match action {
        Action::PushBranch => ask_push(state),
        _ => open(state, action == Action::NewPr),
    },
    offers: |state, out| {
        if let Some(route) = state.route() {
            doables(state, route, out);
        }
    },
    ..crate::features::Feature::NONE
};

/// `,i` and `,p`: the repository's new issue or pull request page, with
/// what was written there before.
#[must_use]
fn open(state: &mut State, pr: bool) -> Vec<Cmd> {
    let Some(repo) = state.route().and_then(Route::repo).cloned() else {
        return state.just_say("Open a repository first: a new issue or pull request goes in one");
    };
    if let Some(why) = refused(state, &repo, pr) {
        return state.just_say(why);
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
    let mut cmds = Vec::new();
    if pr {
        cmds.extend(state.ensure(DataKey::Clone(repo.clone()), true));
        let form = state.forms.get::<Form>(&route);
        if let Some(key) = form.and_then(|f| f.head_key(&repo)) {
            cmds.extend(state.ensure(key, true));
        }
    }
    cmds.extend(state.push(route));
    cmds.extend(follow(state));
    cmds
}

/// What a new issue's or pull request's form does.
pub const FORM: crate::forms::Kind = crate::forms::Kind {
    compose: |state, _| compose(state),
    row: on_row,
    fill: |_, _, _, _| Vec::new(),
    submit,
    follow,
    offers,
};

/// Brings the form on screen up to date with what's arrived: starts it,
/// fills in what's known, asks for what's missing.
#[must_use]
fn follow(state: &mut State) -> Vec<Cmd> {
    let mut cmds = Vec::new();
    let Some(route) = state.route().filter(|r| form_repo(r).is_some()).cloned() else {
        return cmds;
    };
    let Some(repo) = form_repo(&route).cloned() else {
        return cmds;
    };
    if let Route::Pulls(pulls::NewPr(_)) = route {
        cmds.extend(follow_pr(state, &route, &repo));
        return cmds;
    }
    let Some(d) = form_data(&state.data, &repo) else {
        return cmds;
    };
    if (state.forms.get::<Form>(&route)).is_none_or(|f| f.chooser.is_none()) {
        let chooser = templates::chooser(&d.issue_templates);
        let form = state.forms.edit::<Form>(&route);
        // Without templates to choose from, it's a blank issue.
        if form.template.is_none() && !chooser.offers() {
            form.template = Some(String::new());
        }
        form.chooser = Some(chooser);
    }
    // Ready to write: the composer opens, once.
    let form = state.forms.shown::<Form>(&route);
    if form.template.is_some()
        && !form.opened
        && state.overlay.is_none()
        && refused(state, &repo, false).is_none()
    {
        cmds.extend(compose(state));
    }
    cmds
}

/// The composer, on the form on screen: `c`, or Enter on its text box.
#[must_use]
fn compose(state: &mut State) -> Vec<Cmd> {
    let Some((route, form)) = here(state) else {
        return Vec::new();
    };
    if matches!(route, Route::Issues(issues::NewIssue(_))) && form.template.is_none() {
        return state.just_say("Choose a template first");
    }
    state.forms.edit::<Form>(&route).opened = true;
    // The title first, as on GitHub.
    crate::forms::compose::<Form>(state, &route, 0, crate::forms::TITLE)
}

/// `,s`, ctrl-s in the composer, Enter on the form's button: makes the
/// issue; for a pull request, asks first whether it's a draft.
#[must_use]
fn submit(state: &mut State) -> Vec<Cmd> {
    let Some((route, form)) = here(state) else {
        return Vec::new();
    };
    let Some(repo) = form_repo(&route).cloned() else {
        return Vec::new();
    };
    let (title, body) = templates::split(&form.text);
    let Some(d) = form_data(&state.data, &repo) else {
        return say(state, "Still loading what it can have".into());
    };
    if let Some(why) = refused(state, &repo, matches!(route, Route::Pulls(pulls::NewPr(_)))) {
        return say(state, why);
    }
    if matches!(route, Route::Issues(issues::NewIssue(_))) && form.template.is_none() {
        return say(state, "Choose a template first".into());
    }
    if title.is_empty() {
        let key = state.first_key(Action::Comment);
        return say(
            state,
            format!("Write a title first: it's the first line ({key} writes)"),
        );
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
    crate::forms::form_sending(state, &route);
    state.info("Creating the issue…");
    work::start(Submit {
        route,
        new: Submission::Issue(new),
    })
}

/// A new pull request's form: its base the default branch and its head
/// the clone's, until they're chosen, and its text from the head's
/// commits once they're in, unless you've written it.
#[must_use]
fn follow_pr(state: &mut State, route: &Route, repo: &RepoId) -> Vec<Cmd> {
    let Some(d) = form_data(&state.data, repo) else {
        return Vec::new();
    };
    let (default, template) = (d.default_branch.clone(), d.pr_template.clone());
    let checked_out = state
        .clone_of(repo)
        .flatten()
        .and_then(|c| c.branch.clone());
    let form = state.forms.shown::<Form>(route);
    let base = form.base.clone().or(default);
    // Inside a clone, its branch is the one to propose.
    let head = (form.head.clone()).or_else(|| checked_out.filter(|b| Some(b) != base.as_ref()));
    let filled = form.filled_for.clone();
    if (&form.base, &form.head) != (&base, &head) {
        let form = state.forms.edit::<Form>(route);
        (form.base, form.head) = (base.clone(), head.clone());
    }
    let (Some(base), Some(head)) = (base, head) else {
        return Vec::new();
    };
    let Some(key) = state.forms.shown::<Form>(route).head_key(repo) else {
        return Vec::new();
    };
    let cmds = state.ensure(key.clone(), false);
    if filled != Some((base.clone(), head.clone()))
        && let Some(Ok(hs)) = state.answer::<HeadState>(&key)
    {
        let text = templates::pr_text(&head, &hs.commits, template.as_deref());
        let form = state.forms.edit::<Form>(route);
        // Filled in as GitHub would, unless you've written.
        if form.untouched() {
            form.filled.clone_from(&text);
            form.text = text;
        }
        form.filled_for = Some((base, head));
    }
    cmds
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
fn head_problem(state: &State, repo: &RepoId, form: &Form) -> Result<HeadState, String> {
    let push = state.first_key(Action::PushBranch);
    let (Some(base), Some(head)) = (&form.base, &form.head) else {
        return Err("Choose the branch with the changes: Enter on “Compare”".into());
    };
    if base == head {
        return Err(format!(
            "{head} is the base too: choose another branch to compare"
        ));
    }
    let hs = match form.head_state(state, repo) {
        None => return Err(format!("Still comparing {head} with {base}")),
        Some(Err(why)) => return Err(format!("Couldn't compare {head} with {base}: {why}")),
        Some(Ok(hs)) => hs,
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
    let Some(form) = state.forms.get::<Form>(route) else {
        return Vec::new();
    };
    let Some(repo) = form_repo(route) else {
        return Vec::new();
    };
    let hs = match head_problem(state, repo, form) {
        Ok(hs) => hs,
        Err(why) => {
            return say(state, why);
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
    let choices = vec![
        (
            "Create pull request".into(),
            Answer::CreatePr { draft: false },
        ),
        (
            "Create draft pull request".into(),
            Answer::CreatePr { draft: true },
        ),
    ];
    let title = format!("Open a pull request in {repo}?");
    crate::forms::ask(state, route.clone(), title, facts, choices);
    Vec::new()
}

/// Asks before pushing the form's head branch from this clone.
#[must_use]
fn ask_push(state: &mut State) -> Vec<Cmd> {
    let Some((route, form)) =
        here(state).filter(|(r, _)| matches!(r, Route::Pulls(pulls::NewPr(_))))
    else {
        return state.just_say("Pushing works on a new pull request's branch (,p)");
    };
    let Some(repo) = form_repo(&route).cloned() else {
        return Vec::new();
    };
    let why = match (
        state.clone_of(&repo),
        form.head_state(state, &repo),
        &form.head,
    ) {
        (_, _, None) => Some("Choose the branch to push first".to_owned()),
        (Some(None), ..) => Some(format!(
            "ghtui isn't running in a clone of {repo}: push it with git"
        )),
        (_, Some(Ok(HeadState { local: None, .. })), Some(head)) => Some(format!(
            "{head} isn't a branch of this clone: there's nothing here to push"
        )),
        (_, Some(Ok(HeadState { local: Some(b), .. })), _) if b.pushed() => {
            Some(format!("GitHub has every commit of {} already", b.name))
        }
        (_, Some(Ok(_)), _) => None,
        _ => Some("Still comparing the branch with GitHub's".to_owned()),
    };
    if let Some(why) = why {
        return state.just_say(why);
    }
    let Some(Ok(HeadState { local: Some(b), .. })) = form.head_state(state, &repo) else {
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
    let title = format!("Push {} to {}?", b.name, b.remote);
    crate::forms::ask(
        state,
        route,
        title,
        facts,
        vec![("Push".into(), Answer::Push)],
    );
    Vec::new()
}

/// A pull request's question was answered: push its branch (`None`), or
/// make it, a draft or not.
#[must_use]
pub fn answer(state: &mut State, route: &Route, draft: Option<bool>) -> Vec<Cmd> {
    let Some(form) = state.forms.get::<Form>(route) else {
        return Vec::new();
    };
    let Some(repo) = form_repo(route).cloned() else {
        return Vec::new();
    };
    let head = form.head.clone().unwrap_or_default();
    let work: work::Work = match draft {
        None => Push { repo, branch: head }.into(),
        Some(draft) => {
            let hs = match head_problem(state, &repo, form) {
                Ok(hs) => hs,
                Err(why) => {
                    return say(state, why);
                }
            };
            let Some(d) = form_data(&state.data, &repo) else {
                return Vec::new();
            };
            let (title, body) = templates::split(&form.text);
            let new = Submission::Pr(NewPr {
                repo: d.repo_id.clone(),
                base: form.base.clone().unwrap_or_default(),
                head: head_spec(&repo, &head, hs.local.as_ref()),
                title,
                body,
                draft,
            });
            let route = route.clone();
            Submit { route, new }.into()
        }
    };
    crate::forms::form_sending(state, route);
    work::start(work)
}

impl Job for Push {
    type Out = ();
    const WRITES: bool = false;

    async fn run(&self, cx: &Cx) -> Result<(), Failure> {
        let (repo, branch) = (&self.repo, &self.branch);
        let c = crate::clone::here(cx, repo).await?;
        let b = (local::branch(&c.top, &c.remote, branch).await?)
            .ok_or_else(|| Failure::msg(format!("{branch} isn't a branch here")))?;
        Ok(local::push(&c.top, &b.remote, branch, !b.tracked).await?)
    }

    fn done(self, state: &mut State, result: Result<(), Failure>) -> Vec<Cmd> {
        let Push { repo, branch } = self;
        let route = Route::Pulls(pulls::NewPr(repo.clone()));
        match result {
            Ok(()) => {
                crate::forms::close_sender(state, &route);
                state.info(format!("Pushed {branch}"));
                // Compared again, now GitHub has it.
                if state.forms.get::<Form>(&route).is_none() {
                    return Vec::new();
                }
                *state.forms.sent_mut(&route) = Sent::default();
                let form = state.forms.edit::<Form>(&route);
                form.filled_for = None;
                let key = form.head_key(&repo);
                key.map_or_else(Vec::new, |key| state.ensure(key, true))
            }
            Err(err) => {
                let text = format!("Couldn't push {branch}: {err}");
                crate::forms::failed(state, &route, text)
            }
        }
    }
}

impl Job for Submit {
    type Out = u64;

    async fn run(&self, cx: &Cx) -> Result<u64, Failure> {
        let gh = &cx.gh;
        Ok(match &self.new {
            Submission::Issue(new) => gh.create_issue(new).await?,
            Submission::Pr(new) => gh.create_pull_request(new).await?,
        })
    }

    /// GitHub made it (and its page opens in place of the form's) or said why
    /// not.
    fn done(self, state: &mut State, result: Result<u64, Failure>) -> Vec<Cmd> {
        let route = &self.route;
        let Some(repo) = form_repo(route).cloned() else {
            return Vec::new();
        };
        match result {
            Ok(number) => {
                crate::forms::done::<Form>(state, route);
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
                let text = format!("GitHub didn't create {what}: {err}");
                crate::forms::failed(state, route, text)
            }
        }
    }
}

// ---- links, pickers and the menu ---------------------------------------------

/// Enter on one of a form's rows.
#[must_use]
fn on_row(state: &mut State, row: &Row) -> Vec<Cmd> {
    match row {
        Row::Template(file) => choose_template(state, file),
        Row::Templates => {
            if let Some(form) = here_mut(state) {
                form.template = None;
            }
            Vec::new()
        }
        Row::Labels => open_pick(state, Pick::Labels),
        Row::Assignees => open_pick(state, Pick::Assignees),
        Row::Milestone => open_pick(state, Pick::Milestone),
        Row::Base => open_pick(state, Pick::Base),
        Row::Head => open_pick(state, Pick::Head),
        Row::Push => ask_push(state),
        _ => Vec::new(),
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
    let Some(d) = form_data(&state.data, &repo) else {
        return Vec::new();
    };
    let form = state.forms.edit::<Form>(&route);
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
        return state.just_say(format!(
            "Setting {what} takes triage access to {repo}: GitHub would leave them out"
        ));
    }
    state.open_picker(crate::picker::Kind::Form(pick))
}

/// The rows of a form's picker matching `q`.
pub fn picker_rows(state: &State, pick: Pick, q: &str) -> crate::picker::Rows {
    let item = crate::picker::item;
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
        let refs =
            match crate::picker::listed::<crate::api::browse::Refs, _>(state, &key, "the branches")
            {
                Ok(refs) => refs,
                Err(why) => return why,
            };
        let chosen = if pick == Pick::Base {
            &form.base
        } else {
            &form.head
        };
        let current = state
            .clone_of(repo)
            .flatten()
            .and_then(|c| c.branch.clone());
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
    follow(state)
}

/// What the menu offers on a form: writing, pushing and making it.
fn offers(state: &State, out: &mut Vec<Doable>) {
    let Some((route, form)) = here(state) else {
        return;
    };
    let Some(repo) = form_repo(&route) else {
        return;
    };
    let pr = matches!(route, Route::Pulls(pulls::NewPr(_)));
    let why = (!pr && form.template.is_none()).then(|| "choose a template first".to_owned());
    let label = "Write the title and description";
    out.push(row("This page", Action::Comment, label, "write", why));
    let why = match (pr, refused(state, repo, pr)) {
        (_, Some(why)) => Some(why),
        (true, None) => head_problem(state, repo, &form).err(),
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
    out.push(row("This page", Action::Create, label, "create", why));
    if pr {
        let push = state.first_key(Action::PushBranch);
        let why = match form.head_state(state, repo) {
            Some(Ok(HeadState { local: Some(b), .. })) if !b.pushed() => None,
            Some(Ok(HeadState { local: Some(_), .. })) => {
                Some("GitHub has all of it already".into())
            }
            Some(Ok(_)) => Some("it isn't a branch of this clone".into()),
            _ => Some(format!("no branch here to push ({push})")),
        };
        let label = "Push the branch to GitHub";
        out.push(row("This page", Action::PushBranch, label, "", why));
    }
}

/// What the menu offers on a repository's pages: a new issue or pull
/// request.
fn doables(state: &State, route: &Route, out: &mut Vec<Doable>) {
    let Some(repo) = route.repo() else {
        return;
    };
    let why = |pr| refused(state, repo, pr);
    out.push(row("New", Action::NewIssue, "New issue", "", why(false)));
    out.push(row("New", Action::NewPr, "New pull request", "", why(true)));
}

/// "3 commits".
fn count(n: u64, word: &str) -> String {
    format!("{n} {}", crate::ui::text::noun(n, word))
}
