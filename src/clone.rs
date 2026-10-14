//! Checking a pull request out in the clone ghtui runs in, or a branch
//! made for an issue ([`crate::manage`]), as a job ([`Checkout`]).

use crate::api::model::{PrDetail, PrRef, RepoId};
use crate::git::local;

use crate::act::Subject;
use crate::keymap::Action;
use crate::nav::{Doable, row};
use crate::state::{Cmd, State};
use crate::work::{self, Cx, Failure, Job};

/// What checking out adds: `,c` on a pull request, once it's loaded.
pub const FEATURE: crate::features::Feature = crate::features::Feature {
    owns: &[Action::Checkout],
    act: |state, _| checkout(state),
    offers: |state, out| out.extend(doable(state)),
    ..crate::features::Feature::NONE
};

/// The clone ghtui runs in, if it's `repo`'s.
pub(crate) async fn here(cx: &Cx, repo: &RepoId) -> Result<local::Clone, Failure> {
    let found = local::clone_of(&cx.git.cwd, &repo.owner, &repo.name).await?;
    found.ok_or_else(|| Failure::msg(format!("ghtui isn't running in a clone of {repo}")))
}

/// Checks out a pull request's head (`pr`), or else the branch `name`,
/// in the clone ghtui runs in: on branch `name`, tracking `track`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkout {
    pub repo: RepoId,
    pub pr: Option<u64>,
    pub name: String,
    pub track: Option<String>,
}

impl Job for Checkout {
    type Out = local::Checkout;
    const WRITES: bool = false;

    async fn run(&self, cx: &Cx) -> Result<local::Checkout, Failure> {
        let c = here(cx, &self.repo).await?;
        let source = match self.pr {
            Some(n) => format!("refs/pull/{n}/head"),
            None => format!("refs/heads/{}", self.name),
        };
        let track = self.track.as_deref();
        Ok(local::checkout(&c.top, &c.remote, &source, &self.name, track).await?)
    }

    fn done(self, state: &mut State, result: Result<local::Checkout, Failure>) -> Vec<Cmd> {
        use local::Checkout as C;
        let text = match (result, self.pr) {
            (Ok(C::Created(b)), Some(n)) => {
                format!("Checked out #{n}: you're on {b}, a new branch at its head")
            }
            (Ok(C::Switched(b)), Some(n)) => format!("You're on {b}, at #{n}'s head"),
            (Ok(C::Created(b) | C::Switched(b)), None) => format!("You're on {b}"),
            (Ok(C::Diverged(b)), pr) => {
                let theirs = pr.map_or_else(|| "GitHub's".to_owned(), |n| format!("#{n}"));
                format!("You're on {b}, which has commits {theirs} doesn't: it's left as it was")
            }
            (Ok(C::Uncommitted(files)), _) => {
                let first = files.first().cloned().unwrap_or_default();
                let files = match files.len() {
                    1 => first,
                    n => format!(
                        "{first} and {} {}",
                        n - 1,
                        crate::ui::text::noun(n as u64 - 1, "other")
                    ),
                };
                format!(
                    "Not checked out: {files} has changes that aren't committed. Commit or stash them first"
                )
            }
            (Err(err), pr) => {
                let what = pr.map_or_else(|| self.name.clone(), |n| format!("#{n}"));
                state.error(format!("Couldn't check out {what}: {err}"));
                return Vec::new();
            }
        };
        state.just_say(text)
    }
}

/// `,c`: the pull request on screen (or its row) checked out here, once
/// it's loaded.
#[must_use]
fn checkout(state: &mut State) -> Vec<Cmd> {
    let Some(Subject::Pr(pr)) = crate::act::subject(state, Action::Checkout) else {
        return state.just_say("Checking out works on a pull request (or its row in a list)");
    };
    if let Some(None) = state.clone_of(&pr.repo) {
        return state.just_say(format!("ghtui isn't running in a clone of {}", pr.repo));
    }
    match state.pr(&pr) {
        Some(d) => start_checkout(state, pr.clone(), &d.clone()),
        None => crate::act::wait(state, Action::Checkout, Subject::Pr(pr), loaded),
    }
}

/// The pull request waited for has loaded, or couldn't be.
#[must_use]
fn loaded(state: &mut State, about: Subject, _: Action) -> Vec<Cmd> {
    let Subject::Pr(pr) = about else {
        return Vec::new();
    };
    if let Some(d) = state.pr(&pr) {
        let d = d.clone();
        return start_checkout(state, pr, &d);
    }
    let err = state.pr_remote(&pr).and_then(|r| r.error.clone());
    state.error(format!("Couldn't load {pr}: {}", err.unwrap_or_default()));
    Vec::new()
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
    work::start(Checkout {
        repo: pr.repo,
        pr: Some(pr.number),
        name,
        track,
    })
}

/// The menu's row for checking out the pull request here, if there is one.
fn doable(state: &State) -> Option<Doable> {
    let Some(crate::act::Subject::Pr(pr)) = crate::act::subject(state, Action::Checkout) else {
        return None;
    };
    let why = matches!(state.clone_of(&pr.repo), Some(None))
        .then(|| format!("ghtui isn't running in a clone of {}", pr.repo));
    let label = "Check out the pull request here";
    Some(row("Change", Action::Checkout, label, "", why))
}
