//! Conversations, beyond building their pages: long reviews folded and
//! unfolded, and a review's rows leading to the changes it reviewed (from
//! git, as they were at its commit) and to what changed since.

use std::collections::HashSet;

use crate::api::model::PrRef;
use crate::diff_screen::{DiffOf, RangeView};
use crate::keymap::Action;
use crate::route::Route;
use crate::state::{Cmd, Git, Screen, State};
use crate::ui::page::{Link, ReviewLink};
use crate::ui::pages::{Marks, PrTab};
use crate::ui::text::short_sha;

/// What conversations keep while you read them.
#[derive(Debug, Default)]
pub struct Talk {
    /// Long reviews you unfolded, by their anchors
    /// (`pullrequestreview-…`).
    pub unfolded: HashSet<String>,
}

/// What a conversation marks: what came in on the pull request watched
/// since you started looking, the reviews you unfolded, and who you are.
pub fn marks<'a>(state: &'a State, pr: Option<&PrRef>) -> Marks<'a> {
    let none = Marks::none();
    let watch = pr.and_then(|pr| crate::live::fresh(state, pr));
    Marks {
        comments: watch.map_or(none.comments, |w| &w.new_comments),
        reviews: watch.map_or(none.reviews, |w| &w.new_reviews),
        threads: watch.map_or(none.threads, |w| &w.new_threads),
        unfolded: &state.talk.unfolded,
        viewer: state.viewer.as_deref(),
    }
}

/// What following a review's row does, for the menu.
pub fn describe(link: &ReviewLink) -> &'static str {
    match link {
        ReviewLink::Fold(_) => "Fold or unfold the review's threads",
        ReviewLink::Open { .. } => "Open the review",
        ReviewLink::AsReviewed { .. } => "The changes as the review saw them",
        ReviewLink::Since { .. } => "What changed since the review",
    }
}

#[must_use]
pub fn follow(state: &mut State, link: &ReviewLink) -> Vec<Cmd> {
    match link {
        ReviewLink::Fold(anchor) => {
            toggle(state, anchor, None);
            Vec::new()
        }
        ReviewLink::Open { pr, id } => state.push(Route::Pr {
            pr: pr.clone(),
            tab: PrTab::Reviewed(*id),
        }),
        ReviewLink::AsReviewed { pr, commit } => as_reviewed(state, pr, commit),
        ReviewLink::Since { pr, commit } => since(state, pr, commit),
    }
}

/// Folds (`Some(false)`), unfolds (`Some(true)`) or toggles the review
/// `anchor` names.
fn toggle(state: &mut State, anchor: &str, open: Option<bool>) {
    let unfolded = &mut state.talk.unfolded;
    let open = open.unwrap_or(!unfolded.contains(anchor));
    let changed = if open {
        unfolded.insert(anchor.to_owned())
    } else {
        unfolded.remove(anchor)
    };
    if changed {
        state.data_gen += 1;
    }
}

/// `zc`, `zo` and `za` on a page: the review the selection is in, folded
/// or unfolded. `None` where there's none (the key then says so).
#[must_use]
pub fn fold_key(state: &mut State, action: Action) -> Option<Vec<Cmd>> {
    let Screen::Page(p) = state.screen() else {
        return None;
    };
    let page = p.page();
    let at = page.items.get(p.selected?)?.start;
    // The review whose box holds the selection: the nearest anchor at or
    // above it, if a fold row of that review follows.
    let (anchor, _) = (page.anchors.iter())
        .filter(|(name, line)| name.starts_with("pullrequestreview-") && **line <= at)
        .max_by_key(|(_, line)| **line)?;
    let folds = page
        .links
        .iter()
        .any(|l| matches!(l, Link::Review(ReviewLink::Fold(a)) if a == anchor));
    if !folds {
        return None;
    }
    let anchor = anchor.clone();
    let open = match action {
        Action::FoldOpen => Some(true),
        Action::FoldClose => Some(false),
        _ => None,
    };
    toggle(state, &anchor, open);
    Some(Vec::new())
}

/// The pull request's diff as a review saw it: from where its branch
/// left the base to the review's commit, from git.
fn as_reviewed(state: &mut State, pr: &PrRef, commit: &str) -> Vec<Cmd> {
    let range = RangeView {
        label: format!("as reviewed at {}", short_sha(commit)),
        // Empty: from the merge base with the PR's base (`diff_job`).
        from: String::new(),
        to: commit.to_owned(),
    };
    open_range(state, pr, Some(range), None)
}

/// What changed in the pull request since a review's commit: its whole
/// diff, with what was there at that commit marked as seen, as "since your
/// last review" does.
fn since(state: &mut State, pr: &PrRef, commit: &str) -> Vec<Cmd> {
    open_range(state, pr, None, Some(commit.to_owned()))
}

/// Opens the pull request's Files changed on `range` (or the whole of it),
/// compared with `since` once it's in.
fn open_range(
    state: &mut State,
    pr: &PrRef,
    range: Option<RangeView>,
    since: Option<String>,
) -> Vec<Cmd> {
    let mut cmds = state.open_diff(DiffOf::Pr(pr.clone()), None);
    let Some(base) = state.pr_base(pr) else {
        state.info("Loading the pull request first");
        return cmds;
    };
    let Some((screen, diff)) = state.diff_parts() else {
        return cmds;
    };
    let same = diff.range() == range.as_ref() && diff.error.is_none();
    if !same {
        // The diff starts again on the range; a whole diff's start is
        // dropped for it.
        let cmd_range = range.as_ref().map(|r| (r.from.clone(), r.to.clone()));
        diff.restart(range);
        screen.restart();
        cmds.retain(|c| !matches!(c, Cmd::Git(Git::LoadDiff { .. })));
        cmds.push(Cmd::Git(Git::LoadDiff {
            of: pr.clone().into(),
            job: diff.job,
            base,
            range: cmd_range,
        }));
    }
    if let Some(commit) = since {
        // Another comparison's marks go: this one's are made anew.
        if diff.since_commit.as_ref() != Some(&commit) || !diff.doc.since_active() {
            diff.doc.set_since(None, false);
            diff.since_commit = Some(commit.clone());
            diff.since_requested = true;
            state.info(format!("Comparing with {}…", short_sha(&commit)));
        }
    }
    cmds
}
