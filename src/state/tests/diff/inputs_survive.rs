use super::*;
use crate::diff_job::DiffFiles;

const PATH: &str = "lines.txt";

fn text(changed: bool) -> String {
    (1..=20)
        .map(|n| {
            if changed && n == 15 {
                "fifteen changed\n".to_owned()
            } else {
                format!("line {n}\n")
            }
        })
        .collect()
}

/// GitHub's diff of `lines.txt` covers only its first line.
fn patches() -> DiffMsg {
    DiffMsg::PatchesLoaded(Ok(vec![PatchFile {
        filename: PATH.into(),
        previous_filename: None,
        patch: Some("@@ -1 +1 @@\n-a\n+b".into()),
    }]))
}

/// The diff job's answer for `job`: `lines.txt`, line 15 changed.
fn deliver(s: &mut State, pr: &PrRef, job: crate::diff_job::JobId) {
    let files = DiffFiles {
        refs: crate::git::repo::PrRefs {
            head: Oid::parse(&"e".repeat(40)).unwrap(),
            base: Oid::parse(&"b".repeat(40)).unwrap(),
            merge_base: Oid::parse(&"c".repeat(40)).unwrap(),
        },
        files: vec![crate::git::files::ChangedFile {
            status: crate::git::files::FileStatus::Modified,
            old_path: Some(PATH.into()),
            new_path: Some(PATH.into()),
            old_mode: 0o100644,
            new_mode: 0o100644,
            old_oid: Oid::parse(&"1".repeat(40)).unwrap(),
            new_oid: Oid::parse(&"2".repeat(40)).unwrap(),
            similarity: None,
        }],
        generated: std::collections::HashSet::new(),
    };
    diff_msg(s, pr, DiffMsg::Job(job, JobMsg::Files(Box::new(files))));
    let diff = crate::diff::FileDiff::compute(
        PATH,
        Some(text(false).as_bytes()),
        Some(text(true).as_bytes()),
    );
    diff_msg(s, pr, DiffMsg::Job(job, JobMsg::File(0, Arc::new(diff))));
    let _ = s.settle_diff();
}

/// A diff screen on o/r#7 whose job hasn't answered yet, its
/// saved review read and its metadata (for the commit picker) in.
fn loading() -> (State, PrRef, crate::diff_job::JobId) {
    let mut s = state();
    s.size = (120, 40);
    let pr = PrRef::parse("o/r#7").unwrap();
    let of = DiffOf::Pr(pr.clone());
    let diff = DiffState::fresh();
    let job = diff.job;
    s.diffs.insert(of.clone(), diff);
    s.screens.push(Screen::Diff(Box::new(DiffScreen::new(
        of,
        DiffPrefs::fit(120),
    ))));
    crate::fixtures::pr_cached(&mut s, &pr, crate::fixtures::pr_detail());
    diff_msg(
        &mut s,
        &pr,
        DiffMsg::ReviewLoaded(Ok(ReviewState::default())),
    );
    (s, pr, job)
}

/// Where `c` on the changed line 15 would put a comment.
fn comment_target(s: &mut State) -> ComposeTarget {
    press(s, "/");
    press(s, "fifteen changed<Enter>");
    press(s, "c");
    let target = overlay!(s, Compose).target.clone();
    press(s, "<C-c>");
    s.overlay = None;
    target
}

fn is_file_comment(target: &ComposeTarget) -> bool {
    matches!(
        target,
        ComposeTarget::File {
            reason: Some(_),
            ..
        }
    )
}

/// Opens the commit picker on one commit and presses `keys` (`jj<Enter>`
/// picks the commit, `<Enter>` "All"); the new job's id.
fn pick_commit(s: &mut State, pr: &PrRef, keys: &str) -> crate::diff_job::JobId {
    act(s, Action::PickCommits);
    diff_msg(
        s,
        pr,
        DiffMsg::CommitsListed(Ok(vec![crate::git::repo::Commit {
            oid: Oid::parse(&"a".repeat(40)).unwrap(),
            subject: "first".into(),
        }])),
    );
    assert!(matches!(s.overlay, Some(Overlay::Picker(_))));
    let cmds = press(s, keys);
    cmds.iter()
        .find_map(|c| match c {
            Cmd::Git(Git::LoadDiff { job, .. }) => Some(*job),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{cmds:?}"))
}

/// GitHub's patches usually arrive before git has listed the
/// files; lines outside GitHub's diff still become file comments.
#[test]
fn patches_that_arrive_before_the_files_still_place_comments() {
    let (mut s, pr, job) = loading();
    diff_msg(&mut s, &pr, patches());
    deliver(&mut s, &pr, job);
    let target = comment_target(&mut s);
    assert!(is_file_comment(&target), "{target:?}");
}

/// Back to the whole PR after a commit, GitHub's patches still
/// decide where comments go.
#[test]
fn the_whole_pr_after_a_commit_keeps_githubs_patches() {
    let (mut s, pr, job) = loading();
    deliver(&mut s, &pr, job);
    diff_msg(&mut s, &pr, patches());
    let target = comment_target(&mut s);
    assert!(is_file_comment(&target), "before: {target:?}");

    let job = pick_commit(&mut s, &pr, "jj<Enter>");
    deliver(&mut s, &pr, job);
    let job = pick_commit(&mut s, &pr, "<Enter>");
    deliver(&mut s, &pr, job);
    let target = comment_target(&mut s);
    assert!(is_file_comment(&target), "after All: {target:?}");
}

/// Ignoring whitespace, split and the tree are the screen's
/// choices; picking a commit doesn't undo them.
#[test]
fn choosing_a_commit_keeps_the_view_toggles() {
    let (mut s, pr, job) = loading();
    deliver(&mut s, &pr, job);
    press(&mut s, "zw");
    press(&mut s, "zs");
    press(&mut s, "zT");
    let before = {
        let d = screen(&s);
        (
            d.prefs.ignore_whitespace,
            d.prefs.split_override,
            d.prefs.tree_visible,
        )
    };
    assert!(before.0 && before.1.is_some() && !before.2, "{before:?}");
    let job = pick_commit(&mut s, &pr, "jj<Enter>");
    deliver(&mut s, &pr, job);
    let d = screen(&s);
    assert_eq!(
        (
            d.prefs.ignore_whitespace,
            d.prefs.split_override,
            d.prefs.tree_visible
        ),
        before,
        "(ignore_whitespace, split_override, tree_visible)"
    );
}

/// A file marked viewed stays viewed when another range of
/// commits is shown.
#[test]
fn a_file_marked_viewed_stays_viewed_in_another_range() {
    let (mut s, pr, job) = loading();
    deliver(&mut s, &pr, job);
    diff_msg(
        &mut s,
        &pr,
        DiffMsg::ViewedLoaded(Box::new(Ok(ViewedFiles {
            pull_request_id: NodeId::new("PR_1"),
            states: std::collections::HashMap::new(),
        }))),
    );
    let cmds = press(&mut s, ",v");
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Cmd::Api(Api::SetViewed { viewed: true, .. }))),
        "{cmds:?}"
    );
    diff_msg(
        &mut s,
        &pr,
        DiffMsg::ViewedSaved {
            path: PATH.into(),
            previous: Viewed::Unviewed,
            result: Ok(()),
        },
    );
    let of = DiffOf::Pr(pr.clone());
    assert_eq!(s.diffs[&of].doc.files()[0].viewed, Viewed::Viewed);

    let job = pick_commit(&mut s, &pr, "jj<Enter>");
    deliver(&mut s, &pr, job);
    assert_eq!(s.diffs[&of].doc.files()[0].viewed, Viewed::Viewed);
}
