//! Loading a diff: a pull request's, from its base, and again.

use super::*;

/// `ghtui <pr>/files` with the PR in the disk cache: the cached copy is
/// from an earlier session, so the PR is fetched again (it may have
/// been merged or force-pushed since).
#[test]
fn a_pr_diff_opened_from_a_cached_pr_fetches_the_pr() {
    let mut state = state();
    let pr = PrRef::parse("o/r#1").unwrap();
    state.prs.insert(
        pr.clone(),
        Remote::cached(Some(crate::store::Cached {
            value: crate::snapshot_tests::pr_detail(),
            fetched_at: 0,
        })),
    );
    let cmds = state.start_at(Target::Files(DiffOf::Pr(pr.clone())).into());
    assert!(
        cmds.contains(&Cmd::Api(Api::FetchPr(pr))),
        "the cached PR is never refreshed: {cmds:?}"
    );
}

/// A commit's diff starts the diff job and reads the commit's comments;
/// what belongs to pull requests says so.
#[test]
fn a_commit_diff_has_no_review() {
    let mut s = state();
    let of = DiffOf::Commit(RepoId::new("o", "r"), Oid::parse(&"a".repeat(40)).unwrap());
    let cmds = s.open_diff(of.clone(), None);
    assert!(
        matches!(
            &cmds[..],
            [
                Cmd::Git(Git::LoadDiff { of: o, base, .. }),
                Cmd::Api(Api::Fetch {
                    key: crate::browse::DataKey::CommitComments(..),
                    ..
                }),
            ] if *o == of && *base == crate::diff_job::PrBase::default()
        ),
        "{cmds:?}"
    );
    s.diffs.insert(of, crate::snapshot_tests::diff_fixture());
    for action in [Action::Comment, Action::ToggleViewed, Action::MarkReviewed] {
        s.notices.dismiss();
        assert!(apply(&mut s, action).is_empty(), "{action:?}");
        says(&s, "pull request");
    }
}

/// An open PR's diff is from its base branch; a merged or closed one's
/// from the base it had then, which a merge commit has moved past.
#[test]
fn a_merged_prs_diff_is_from_its_own_base() {
    let pr = PrRef::parse("o/r#1").unwrap();
    for (pr_state, oid) in [
        (PrState::Open, None),
        (
            PrState::Merged,
            Some("0123456789abcdef0123456789abcdef01234567"),
        ),
        (
            PrState::Closed,
            Some("0123456789abcdef0123456789abcdef01234567"),
        ),
    ] {
        let mut state = state();
        let mut detail = crate::snapshot_tests::pr_detail();
        detail.state = pr_state;
        let _ = update(&mut state, Msg::Pr(pr.clone(), Box::new(Ok(detail))));
        let cmds = state.open_diff(DiffOf::Pr(pr.clone()), None);
        let base = cmds.iter().find_map(|c| match c {
            Cmd::Git(Git::LoadDiff { base, .. }) => Some(base.clone()),
            _ => None,
        });
        assert_eq!(base.and_then(|b| b.oid).as_deref(), oid, "{pr_state:?}");
    }
}

/// git finding no changes in a PR GitHub says has some (as a merged
/// PR's diff once did) is shown as an error, not as an empty diff.
#[test]
fn an_empty_diff_of_a_pr_with_changes_is_an_error() {
    let mut state = state();
    let pr = PrRef::parse("o/r#1").unwrap();
    let detail = crate::snapshot_tests::pr_detail();
    let head = crate::git::Oid::parse(&detail.head_oid).unwrap();
    let _ = update(&mut state, Msg::Pr(pr.clone(), Box::new(Ok(detail))));
    let cmds = state.open_diff(DiffOf::Pr(pr.clone()), None);
    let job = cmds
        .iter()
        .find_map(|c| match c {
            Cmd::Git(Git::LoadDiff { job, .. }) => Some(*job),
            _ => None,
        })
        .unwrap();
    let refs = crate::git::repo::PrRefs {
        head,
        base: crate::git::Oid::parse(&"b".repeat(40)).unwrap(),
        merge_base: crate::git::Oid::parse(&"b".repeat(40)).unwrap(),
    };
    let files = crate::diff_job::DiffFiles {
        refs,
        files: Vec::new(),
        generated: std::collections::HashSet::new(),
    };
    let msg = DiffMsg::Job(job, crate::diff_job::JobMsg::Files(Box::new(files)));
    let _ = diff_msg(&mut state, &pr, msg);
    let error = state.diffs[&DiffOf::Pr(pr)].error.clone();
    assert!(error.is_some_and(|e| e.contains("GitHub says 7 files changed")));
}

/// Opens `pr`'s diff (its metadata already in) and returns the job's id.
fn start_pr_diff(state: &mut State, pr: &PrRef) -> crate::diff_job::JobId {
    let cmds = state.open_diff(DiffOf::Pr(pr.clone()), None);
    cmds.iter()
        .find_map(|c| match c {
            Cmd::Git(Git::LoadDiff { job, .. }) => Some(*job),
            _ => None,
        })
        .unwrap()
}

/// git's listing of a PR's diff: at `head`, with no files.
fn no_files(job: crate::diff_job::JobId, head: &str) -> DiffMsg {
    let refs = crate::git::repo::PrRefs {
        head: crate::git::Oid::parse(head).unwrap(),
        base: crate::git::Oid::parse(&"b".repeat(40)).unwrap(),
        merge_base: crate::git::Oid::parse(&"b".repeat(40)).unwrap(),
    };
    let files = crate::diff_job::DiffFiles {
        refs,
        files: Vec::new(),
        generated: std::collections::HashSet::new(),
    };
    DiffMsg::Job(job, crate::diff_job::JobMsg::Files(Box::new(files)))
}

/// Review threads from GraphQL often beat git's fetch on an uncached
/// repo: the outdated ones are still mapped once the diff's head is known.
#[test]
fn outdated_threads_that_beat_the_diff_are_mapped_when_it_lists() {
    let mut state = state();
    let pr = PrRef::parse("o/r#1").unwrap();
    let detail = crate::snapshot_tests::pr_detail();
    let head = detail.head_oid.clone();
    let _ = update(&mut state, Msg::Pr(pr.clone(), Box::new(Ok(detail))));
    let job = start_pr_diff(&mut state, &pr);
    let early = diff_msg(
        &mut state,
        &pr,
        DiffMsg::ThreadsLoaded(Ok(vec![thread("old", None, false, true)])),
    );
    assert!(
        !early
            .iter()
            .any(|c| matches!(c, Cmd::Git(Git::MapOutdated(_)))),
        "no head to map onto yet"
    );
    let cmds = diff_msg(&mut state, &pr, no_files(job, &head));
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Cmd::Git(Git::MapOutdated(j)) if j.get().3.len() == 1)),
        "the outdated thread is never mapped: {cmds:?}"
    );
}

/// A PR shown from the cache may have been merged since: once GitHub's
/// fresh copy arrives, git's diff is still checked against it.
#[test]
fn a_diff_listed_under_a_cached_pr_is_checked_when_the_pr_refreshes() {
    let mut state = state();
    let pr = PrRef::parse("o/r#1").unwrap();
    let detail = crate::snapshot_tests::pr_detail();
    let head = detail.head_oid.clone();
    state.prs.insert(
        pr.clone(),
        Remote::cached(Some(crate::store::Cached {
            value: detail.clone(),
            fetched_at: 0,
        })),
    );
    let job = start_pr_diff(&mut state, &pr);
    let _ = diff_msg(&mut state, &pr, no_files(job, &head));
    assert!(
        state.diffs[&DiffOf::Pr(pr.clone())].error.is_none(),
        "a cached PR isn't trusted for the check"
    );
    let _ = update(&mut state, Msg::Pr(pr.clone(), Box::new(Ok(detail))));
    let error = state.diffs[&DiffOf::Pr(pr.clone())].error.clone();
    assert!(
        error
            .as_deref()
            .is_some_and(|e| e.contains("GitHub says 7 files changed")),
        "the empty diff was never checked against the fresh PR: error = {error:?}"
    );
}

/// After a push, git's diff is at a newer head than the PR we hold;
/// refreshing the diff fetches the PR again, so the check can agree.
#[test]
fn refreshing_a_pr_diff_refetches_the_pr() {
    let mut state = state();
    let pr = PrRef::parse("o/r#1").unwrap();
    let detail = crate::snapshot_tests::pr_detail();
    let _ = update(&mut state, Msg::Pr(pr.clone(), Box::new(Ok(detail))));
    let job = start_pr_diff(&mut state, &pr);
    let _ = diff_msg(&mut state, &pr, no_files(job, &"c".repeat(40)));
    warns(&state, "moved while loading");
    let cmds = act(&mut state, Action::Refresh);
    assert!(
        cmds.contains(&Cmd::Api(Api::FetchPr(pr.clone()))),
        "r reloads the diff but not the PR it's checked against: {cmds:?}"
    );
}

/// A PR held as open may have merged since: r starts its diff again
/// from the PR it fetches, not the one it had.
#[test]
fn refreshing_a_pr_diff_waits_for_the_pr() {
    let mut state = state();
    let pr = PrRef::parse("o/r#1").unwrap();
    let mut detail = crate::snapshot_tests::pr_detail();
    detail.state = PrState::Open;
    let _ = update(
        &mut state,
        Msg::Pr(pr.clone(), Box::new(Ok(detail.clone()))),
    );
    let _ = start_pr_diff(&mut state, &pr);
    let load = |cmds: &[Cmd]| {
        cmds.iter().find_map(|c| match c {
            Cmd::Git(Git::LoadDiff { base, .. }) => Some(base.clone()),
            _ => None,
        })
    };
    let cmds = act(&mut state, Action::Refresh);
    assert_eq!(load(&cmds), None, "started from the PR it had: {cmds:?}");
    detail.state = PrState::Merged;
    detail.base_oid = "c".repeat(40);
    let cmds = update(&mut state, Msg::Pr(pr.clone(), Box::new(Ok(detail))));
    assert_eq!(
        load(&cmds).and_then(|b| b.oid),
        Oid::parse(&"c".repeat(40)).ok()
    );
}

/// A diff you've left isn't checked when its PR refreshes (on the PR's
/// page, "moved while loading" would be wrong), only once it's back.
#[test]
fn a_hidden_diff_is_checked_when_it_shows_again() {
    let mut state = state();
    let pr = PrRef::parse("o/r#1").unwrap();
    let mut detail = crate::snapshot_tests::pr_detail();
    detail.changed_files = 0;
    let head = detail.head_oid.clone();
    let _ = update(
        &mut state,
        Msg::Pr(pr.clone(), Box::new(Ok(detail.clone()))),
    );
    let job = start_pr_diff(&mut state, &pr);
    let _ = diff_msg(&mut state, &pr, no_files(job, &head));
    let _ = state.back();
    state.notices.dismiss();
    detail.head_oid = "d".repeat(40);
    let _ = update(&mut state, Msg::Pr(pr.clone(), Box::new(Ok(detail))));
    assert_eq!(state.notices.shown(), None, "a diff off screen was checked");
    let _ = state.open_diff(DiffOf::Pr(pr.clone()), None);
    let _ = state.settle();
    warns(&state, "moved while loading");
}

/// "Since my last review" asked for while the diff loads says it'll
/// wait, and is dropped if you leave before it's in.
#[test]
fn since_review_asked_on_a_diff_you_left_is_dropped() {
    let mut state = state();
    let pr = PrRef::parse("o/r#1").unwrap();
    let mut detail = crate::snapshot_tests::pr_detail();
    detail.changed_files = 0;
    let head = detail.head_oid.clone();
    let _ = update(&mut state, Msg::Pr(pr.clone(), Box::new(Ok(detail))));
    let job = start_pr_diff(&mut state, &pr);
    let _ = diff_msg(
        &mut state,
        &pr,
        DiffMsg::LastReview(Ok(Some("a".repeat(40)))),
    );
    let _ = act(&mut state, Action::ToggleSinceReview);
    says(&state, "once the diff has loaded");
    let _ = state.back();
    let _ = diff_msg(&mut state, &pr, no_files(job, &head));
    let mut cmds = state.open_diff(DiffOf::Pr(pr.clone()), None);
    cmds.extend(state.settle());
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, Cmd::Git(Git::SinceReview(_)))),
        "a request from before you left came back: {cmds:?}"
    );
}

#[test]
fn files_tab_opens_the_diff_once_the_pr_loads() {
    let mut state = state();
    let pr = PrRef::parse("o/r#1").unwrap();
    let _ = state.push(Route::pr(pr.clone()));
    assert!(press(&mut state, "4gt").is_empty(), "waits for the PR");
    assert!(matches!(state.screen(), Screen::Diff(_)));
    assert_eq!(state.chrome().active, Some(3));
    assert_eq!(state.busy().as_deref(), Some("Loading o/r#1"));
    let cmds = update(
        &mut state,
        Msg::Pr(pr.clone(), Box::new(Ok(crate::snapshot_tests::pr_detail()))),
    );
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Cmd::Git(Git::LoadDiff { .. }))),
        "{cmds:?}"
    );
    press(&mut state, "2gt");
    assert_eq!(
        route(&state),
        Route::Pr {
            pr,
            tab: PrTab::Commits
        }
    );
    assert_eq!(state.screens.len(), 2, "back on the PR page, not a new one");
}
