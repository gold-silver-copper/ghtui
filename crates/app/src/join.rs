//! Diff work that needs several async inputs, derived at each settle from
//! what is present, so the order the inputs arrive in can't matter.

use ghtui_api::model::{NodeId, PrDetail};
use ghtui_ui::bars::Notice;

use crate::diff_job::JobId;
use crate::diff_screen::{DiffOf, LastReview};
use crate::keymap::Action;
use crate::review;
use crate::state::{Cmd, Git, Screen, State};

/// What each piece of a diff's joined work last ran for: it runs again
/// when that changes. Only [`join`] sees inside.
#[derive(Debug, Default)]
pub struct Joins {
    moves: Option<JobId>,
    /// The job, and the outdated threads mapped onto it.
    mapped: Option<(JobId, Vec<NodeId>)>,
    /// The job, and the PR's head it was checked against.
    checked: Option<(JobId, String)>,
}

/// True once per `key`: the work it was for is due.
fn claim<K: PartialEq>(done: &mut Option<K>, key: K) -> bool {
    let due = done.as_ref() != Some(&key);
    *done = Some(key);
    due
}

/// What a command only [`join`] may build carries: code elsewhere can
/// read it, but not make one or change it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Joined<T>(T);

impl<T> Joined<T> {
    pub fn get(&self) -> &T {
        &self.0
    }

    pub fn into_inner(self) -> T {
        self.0
    }
}

/// The joined work now due, for every diff and then for the one on screen.
#[must_use]
pub(crate) fn join(state: &mut State) -> Vec<Cmd> {
    let mut cmds = Vec::new();
    let shown = match state.screen() {
        Screen::Diff(screen) => Some(screen.of.clone()),
        Screen::Page(_) => None,
    };
    for (of, diff) in &mut state.diffs {
        // Asked for on a screen you've left: not wanted when you're back.
        if shown.as_ref() != Some(of) {
            diff.since_requested = false;
        }
        let Some(head) = diff.head() else { continue };
        let (job, files) = (diff.job, diff.doc.files().len());
        if files > 0 && diff.doc.ready_count() == files && claim(&mut diff.joins.moves, job) {
            let diffs = (diff.doc.files().iter().enumerate())
                .filter_map(|(i, f)| Some((i, f.diff.clone()?)))
                .collect();
            cmds.push(Cmd::Git(Git::DetectMoves(Joined((of.clone(), job, diffs)))));
        }
        let DiffOf::Pr(pr) = of else { continue };
        let threads = review::outdated_to_map(&diff.inputs().threads);
        let ids = threads.iter().map(|t| t.thread.clone()).collect();
        if claim(&mut diff.joins.mapped, (job, ids)) && !threads.is_empty() {
            let work = (pr.clone(), job, head, threads);
            cmds.push(Cmd::Git(Git::MapOutdated(Joined(work))));
        }
    }
    // What tells you something is for the diff on screen: a hidden one's
    // keys stay unclaimed until it's back.
    let Some(of @ DiffOf::Pr(pr)) = &shown else {
        return cmds;
    };
    let refresh = state.first_key(Action::Refresh);
    let Some(diff) = state.diffs.get_mut(of).filter(|d| d.range().is_none()) else {
        return cmds;
    };
    let Some(head) = diff.head() else { return cmds };
    // The whole PR's diff, checked against what GitHub says of it (once
    // that's fresh: a cached copy may be from before a push).
    if let Some(remote) = state.prs.get(pr)
        && let Some(detail) = remote.data.as_ref()
        && !remote.loading()
        && remote.cached_at.is_none()
        && claim(&mut diff.joins.checked, (diff.job, detail.head_oid.clone()))
    {
        let check = check_pr_diff(detail, &head, diff.doc.files().len(), &refresh);
        if check != Ok(None) {
            tracing::warn!(%pr, ?check, "the PR's diff doesn't match GitHub");
        }
        match check {
            Ok(None) => {}
            Ok(Some(warning)) => state.notice = Some(Notice::Error(warning)),
            Err(error) => diff.error = Some(error),
        }
    }
    // "Since my last review", compared with the whole PR's diff.
    if diff.since_requested && diff.inputs().last_review != LastReview::Unknown {
        diff.since_requested = false;
        let local = diff.inputs().review.last_reviewed_head.clone();
        let (old, unasked) = match &diff.inputs().last_review {
            LastReview::At(oid) => (Some(oid.to_string()), String::new()),
            LastReview::Failed(err) if local.is_none() => {
                let message = format!("Couldn't look up your last review: {err}");
                state.error(message);
                return cmds;
            }
            // Only the review made here can say, and GitHub may know a later one.
            LastReview::Failed(err) => (
                local,
                format!("Couldn't ask GitHub ({err}); going by the review made here. "),
            ),
            LastReview::Unknown | LastReview::None => (local, String::new()),
        };
        match old {
            None => state.info(format!("{unasked}You haven't reviewed this PR yet")),
            Some(old) if *old == *head => {
                state.info(format!(
                    "{unasked}Nothing new: you reviewed the current head"
                ));
            }
            Some(old_head) => {
                state.info(format!("{unasked}Comparing with your last review…"));
                cmds.push(Cmd::Git(Git::SinceReview(Joined((pr.clone(), old_head)))));
            }
        }
    }
    cmds
}

/// Checks the diff git made (its head, how many files) against GitHub's
/// PR: the same head, and about as many files. No files where GitHub has
/// some is the merged-PR bug's shape, an `Err`; a warning says it may be.
fn check_pr_diff(
    detail: &PrDetail,
    head: &str,
    files: usize,
    refresh: &str,
) -> Result<Option<String>, String> {
    let short = ghtui_ui::text::short_sha;
    if head != detail.head_oid {
        return Ok(Some(format!(
            "The PR's head is {} on GitHub but {} here: it moved while loading. {refresh} reloads",
            short(&detail.head_oid),
            short(head)
        )));
    }
    let (ours, theirs) = (files as u64, detail.changed_files);
    if ours == 0 && theirs > 0 {
        return Err(format!(
            "git found no changes, but GitHub says {theirs} file{} changed. {refresh} tries again",
            if theirs == 1 { "" } else { "s" }
        ));
    }
    Ok((ours.abs_diff(theirs) > 3.max(theirs / 10)).then(|| {
        format!(
            "git found {ours} changed files, GitHub says {theirs}: the diff may not be the PR's"
        )
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prs_diff_is_checked_against_github() {
        let mut pr = crate::snapshot_tests::pr_detail();
        pr.head_oid = "a".repeat(40);
        pr.changed_files = 12;
        let check = |head: &str, files| check_pr_diff(&pr, head, files, "r");
        assert_eq!(check(&"a".repeat(40), 12), Ok(None));
        // Renames and the like count differently; a few files either way
        // is no cause for alarm.
        assert_eq!(check(&"a".repeat(40), 10), Ok(None));
        let none = check(&"a".repeat(40), 0);
        assert!(none.is_err_and(|e| e.contains("GitHub says 12 files")));
        let off = check(&"a".repeat(40), 40);
        assert!(off.is_ok_and(|w| {
            w.is_some_and(|w| w.contains("git found 40 changed files, GitHub says 12"))
        }));
        let moved = check(&"b".repeat(40), 12);
        assert!(
            moved
                .is_ok_and(|w| w.is_some_and(|w| w.contains("aaaaaaa on GitHub but bbbbbbb here")))
        );
    }
}
