//! Resolving a pull request's conflicts: git merges, the diff shows each
//! conflict, `<`, `>` and `e` resolve them, and the merge is committed
//! after asking, or given up on.

use std::sync::Arc;

use super::*;
use crate::conflicts::{File, Finish, How, Merging, Start};
use crate::diff::FileDiff;
use crate::diff::anchor::{LinePos, Side};
use crate::diff_job::{DiffFiles, JobMsg};
use crate::forms::Answer;
use crate::git::Oid;
use crate::git::files::{ChangedFile, FileStatus};
use crate::git::merge::{Content, Entry, Part};
use crate::git::repo::PrRefs;

fn oid(c: char) -> Oid {
    Oid::parse(&c.to_string().repeat(40)).unwrap()
}

const OURS: &str = "one\nmine\nend\n";

/// What git made of merging main into the fixture pull request's branch:
/// a line both changed, and a file the base deleted.
fn merged() -> Merging {
    let x = Entry {
        mode: "100644".into(),
        oid: oid('e'),
    };
    Merging {
        pr: pr(),
        base_ref: "main".into(),
        head_ref: "syntax-palette".into(),
        head_repo: RepoId::new("octocat", "ghtui"),
        head: oid('a'),
        base: oid('b'),
        tree: oid('c'),
        view: oid('d'),
        files: vec![
            File {
                path: "src/a.rs".into(),
                ours: Some(OURS.into()),
                how: How::Hunks(vec![
                    Part::Merged("one\n".into()),
                    Part::Conflict {
                        ours: "mine\n".into(),
                        theirs: "theirs\n".into(),
                        chosen: None,
                    },
                    Part::Merged("end\n".into()),
                ]),
            },
            File {
                path: "gone.rs".into(),
                ours: Some(b"x\n".to_vec()),
                how: How::Whole {
                    ours: Some((x, b"x\n".to_vec())),
                    theirs: None,
                    chosen: None,
                },
            },
        ],
    }
}

/// The text the diff shows for `src/a.rs` while its conflict waits.
const MARKED: &str = "one\n<<<<<<< syntax-palette\nmine\n=======\ntheirs\n>>>>>>> main\nend\n";

fn changed(path: &str, status: FileStatus) -> ChangedFile {
    ChangedFile {
        status,
        old_path: Some(path.into()),
        new_path: (status != FileStatus::Deleted).then(|| path.into()),
        old_mode: 0o100644,
        new_mode: 0o100644,
        old_oid: oid('1'),
        new_oid: if status == FileStatus::Deleted {
            oid('0')
        } else {
            oid('2')
        },
        similarity: None,
    }
}

/// The fixture pull request's page, its conflicts being resolved on their
/// diff, which git has listed and diffed.
fn resolving() -> State {
    let mut s = with_pr();
    let cmds = act(&mut s, Action::ResolveConflicts);
    let start = fixtures::sent::<Start>(&cmds);
    assert_eq!(
        (start.base_ref.as_str(), start.head_ref.as_str()),
        ("main", "syntax-palette")
    );
    assert_eq!(start.head_repo, RepoId::new("octocat", "ghtui"));
    let cmds = fixtures::done(&mut s, start, Ok(Some(merged())));
    let of = DiffOf::Conflicts(pr(), oid('a'), oid('d'));
    assert_eq!(on_diff(&s).of, of);
    listed(&mut s, &cmds);
    s
}

/// The conflicts' diff job `cmds` start lists and diffs their files, as
/// they were merged.
fn listed(s: &mut State, cmds: &[Cmd]) {
    let job = fixtures::diff_job(cmds);
    let of = DiffOf::Conflicts(pr(), oid('a'), oid('d'));
    let refs = PrRefs {
        head: oid('d'),
        base: oid('a'),
        merge_base: oid('a'),
    };
    let files = vec![
        changed("src/a.rs", FileStatus::Modified),
        changed("gone.rs", FileStatus::Deleted),
    ];
    let listed = DiffFiles {
        refs,
        files,
        generated: Default::default(),
    };
    let job_msg = |msg| Msg::Diff(of.clone(), DiffMsg::Job(job, msg));
    update(s, job_msg(JobMsg::Files(Box::new(listed))));
    let a = FileDiff::compute("src/a.rs", Some(OURS.as_bytes()), Some(MARKED.as_bytes()));
    let gone = FileDiff::compute("gone.rs", Some(b"x\n"), None);
    update(s, job_msg(JobMsg::File(0, Arc::new(a))));
    update(s, job_msg(JobMsg::File(1, Arc::new(gone))));
}

/// Line `line` of file `file`'s new side, as the diff shows it now.
fn shown(s: &State, file: usize, line: u32) -> String {
    let doc = &s.diffs[&on_diff(s).of].doc;
    let text = doc.files()[file].text().unwrap();
    text.line(LinePos {
        side: Side::Right,
        line,
    })
    .to_owned()
}

/// Resolving: `>` takes the base's side of the conflict under the cursor
/// and the diff shows it; committing waits until every conflict is
/// resolved, `<` keeps your branch's file the base deleted, and the
/// merge is committed and pushed after asking. The pull request then
/// follows GitHub until it shows the merge.
#[test]
fn conflicts_are_resolved_one_by_one_and_the_merge_pushed() {
    let mut s = resolving();
    press(&mut s, "/mine<Enter>");
    press(&mut s, ">");
    says(&s, "Took main's side · 1 conflict left");
    assert_eq!(shown(&s, 0, 2), "theirs");
    // Loaded again, the diff shows what was resolved.
    let cmds = press(&mut s, "r");
    listed(&mut s, &cmds);
    assert_eq!(shown(&s, 0, 2), "theirs");
    act(&mut s, Action::CommitMerge);
    says(&s, "1 conflict left");
    assert!(s.overlay.is_none());

    press(&mut s, "<S-Down><lt>");
    says(&s, "Took syntax-palette's side · all resolved");

    act(&mut s, Action::CommitMerge);
    assert_eq!(
        overlay!(s, Ask).title,
        "Commit the merge and push it to syntax-palette?"
    );
    let Some((_, Answer::CommitMerge(job))) = overlay!(s, Ask).choices.first() else {
        panic!()
    };
    let job: Finish = (**job).clone();
    let cmds = press(&mut s, "<Enter>");
    assert_eq!(fixtures::sent::<Finish>(&cmds), job);
    assert_eq!(job.message, "Merge branch 'main' into syntax-palette");
    assert_eq!((job.head.clone(), job.base.clone()), (oid('a'), oid('b')));
    let x = Entry {
        mode: "100644".into(),
        oid: oid('e'),
    };
    assert_eq!(
        job.files,
        [
            (
                "src/a.rs".into(),
                Content::Text("one\ntheirs\nend\n".into())
            ),
            ("gone.rs".into(), Content::Entry(x)),
        ]
    );

    fixtures::done(&mut s, job, Ok(oid('f')));
    assert!(s.merging.is_none() && s.overlay.is_none());
    assert_eq!(route(&s), Route::pr(pr()));
    says(&s, "Merged into syntax-palette and pushed as fffffff");
    assert!(crate::follow::waiting(&s).is_some());
    pr_shows(&mut s, |d| d.head_oid = "f".repeat(40));
    assert!(crate::follow::waiting(&s).is_none());
}

/// `e` opens the conflict under the cursor in `$EDITOR`, markers and all;
/// what comes back resolves it, unless it still has markers.
#[test]
fn a_conflict_is_edited_in_the_editor() {
    let mut s = resolving();
    press(&mut s, "/theirs<Enter>");
    let cmds = press(&mut s, "e");
    let Some(Cmd::Edit { purpose, text }) = cmds.into_iter().next() else { panic!() };
    assert_eq!(
        text,
        "<<<<<<< syntax-palette\nmine\n=======\ntheirs\n>>>>>>> main\n"
    );
    update(&mut s, Msg::Edited(purpose.clone(), Ok(text)));
    says(&s, "still has conflict markers");
    update(&mut s, Msg::Edited(purpose, Ok("both".into())));
    says(&s, "Edited · 1 conflict left");
    assert_eq!(shown(&s, 0, 2), "both");
    // Off a conflict, a side says where to go.
    press(&mut s, "<Home>>");
    says(&s, "Move into a conflict");
}

/// Stopping leaves GitHub as it was: the conflicts' diff closes, and
/// nothing was sent.
#[test]
fn resolving_is_given_up_on_cleanly() {
    let mut s = resolving();
    press(&mut s, "/mine<Enter>>");
    let cmds = act(&mut s, Action::AbortMerge);
    assert!(fixtures::jobs::<Finish>(&cmds).is_empty());
    assert!(s.merging.is_none());
    assert_eq!(route(&s), Route::pr(pr()));
    let of = DiffOf::Conflicts(pr(), oid('a'), oid('d'));
    assert!(!s.diffs.contains_key(&of));
    says(&s, "GitHub's branch is as it was");
    // Without conflicts left to resolve, its keys say where they work.
    press(&mut s, "M");
    says(
        &s,
        "It conflicts with main: :resolve conflicts resolves that",
    );
}

/// Where it can't be done, the menu says why; without conflicts, it
/// isn't offered.
#[test]
fn resolving_is_offered_only_where_it_conflicts() {
    let row = |s: &State| {
        let d = s
            .doables()
            .into_iter()
            .find(|d| d.action == Action::ResolveConflicts);
        d.map(|d| d.unavailable)
    };
    assert_eq!(row(&with_pr()), Some(None));
    let s = pr_with(|d| {
        d.mergeable = crate::api::model::Mergeable::Conflicting;
        d.may.push_head = false;
    });
    let why = row(&s).flatten().unwrap();
    assert!(why.contains("doesn't let maintainers push"), "{why}");
    assert_eq!(row(&pr_with(|_| {})), None);
}
