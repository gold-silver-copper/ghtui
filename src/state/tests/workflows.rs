//! Saving a run's artifacts, and running a workflow by hand.

use super::*;
use crate::route::runs;
use crate::ui::form::Row;
use crate::ui::page::Link;
use crate::workflows::{Dispatch, SaveArtifact};

const WORKFLOW: &str = "name: CI\non:\n  workflow_dispatch:\n    inputs:\n      outcome:\n        type: choice\n        options: [fail, pass]\n        default: fail\n      seconds:\n        type: string\n        default: \"60\"\n";

/// `,E` on a workflow opens the form of its inputs, read from its file;
/// an input is set from its choices, `,E` runs it on the default branch
/// with every input's value, and the page follows the run it started.
#[test]
fn a_workflow_runs_by_hand_with_its_inputs_and_is_followed() {
    let mut s = with_workflow();
    act(&mut s, Action::RunWorkflow);
    let dispatch = Route::Runs(runs::Dispatch {
        repo: ghtui(),
        file: "ci.yml".into(),
    });
    assert_eq!(route(&s), dispatch);
    let blob = crate::fixtures::file(".github/workflows/ci.yml", WORKFLOW);
    let key = DataKey::Blob(ghtui(), "HEAD".into(), ".github/workflows/ci.yml".into());
    fetched(&mut s, key, Data::from(blob));
    fixtures::select_link(&mut s, &Link::Form(Row::Input("outcome".into())));
    press(&mut s, "<Enter>");
    press(&mut s, "pass<Enter>");
    let cmds = act(&mut s, Action::RunWorkflow);
    let dispatch = Dispatch {
        repo: ghtui(),
        file: "ci.yml".into(),
        git_ref: "main".into(),
        inputs: vec![
            ("outcome".into(), "pass".into()),
            ("seconds".into(), "60".into()),
        ],
    };
    assert_eq!(crate::fixtures::sent::<Dispatch>(&cmds), dispatch);
    crate::fixtures::done(&mut s, dispatch, Ok(Some(55)));
    let run = Route::Runs(runs::WorkflowRun {
        repo: ghtui(),
        run: 55,
        attempt: None,
    });
    assert_eq!(route(&s), run);
    says(&s, "Started ci.yml: following its run");
}

/// A workflow without a `workflow_dispatch` trigger says so, and `,E`
/// says why it can't run.
#[test]
fn a_workflow_that_cant_run_by_hand_says_so() {
    let mut s = with_workflow();
    act(&mut s, Action::RunWorkflow);
    let blob = crate::fixtures::file(".github/workflows/ci.yml", "on: push\n");
    let key = DataKey::Blob(ghtui(), "HEAD".into(), ".github/workflows/ci.yml".into());
    fetched(&mut s, key, Data::from(blob));
    assert!(fetches(act(&mut s, Action::RunWorkflow)).is_empty());
    says(&s, "can't be run by hand");
}

/// Enter on an artifact asks where to save it: the current folder by
/// default; a file that's there is replaced only when the row says so.
#[test]
fn an_artifact_is_saved_where_you_say() {
    let mut s = with_run();
    let artifact = Link::Artifact {
        id: 31,
        name: "coverage-report".into(),
    };
    fixtures::select_link(&mut s, &artifact);
    press(&mut s, "<Enter>");
    let cmds = press(&mut s, "<Enter>");
    let saved = SaveArtifact {
        repo: ghtui(),
        id: 31,
        name: "coverage-report".into(),
        path: std::path::Path::new(".").join("coverage-report.zip"),
        replace: false,
    };
    assert_eq!(crate::fixtures::sent::<SaveArtifact>(&cmds), saved);
    // Cargo.toml is there: the row asks to replace it.
    fixtures::select_link(&mut s, &artifact);
    press(&mut s, "<Enter>");
    press(&mut s, "Cargo.toml");
    let Some(Overlay::Picker(p)) = &s.overlay else {
        panic!("no picker")
    };
    let rows = s.picker_rows(p);
    assert_eq!(rows[0].0.label, "Replace Cargo.toml");
    let cmds = press(&mut s, "<Enter>");
    assert!(crate::fixtures::sent::<SaveArtifact>(&cmds).replace);
}

/// A check's annotation on a file shows in Files changed on its line, as
/// on GitHub, named by its check; one about the run doesn't.
#[test]
fn check_annotations_show_on_their_lines_in_files_changed() {
    use crate::ui::annotations::{AnnotationKey, Place};
    let (mut s, pr) = super::diff::diff_state(120);
    let mut checks = crate::fixtures::checks();
    let mut on_run = crate::fixtures::check_annotation();
    on_run.path = None;
    let mut on_file = crate::fixtures::check_annotation();
    on_file.path = Some("src/point.rs".into());
    checks.items.items[0].annotations = vec![on_file, on_run].into();
    fetched(&mut s, DataKey::PrChecks(pr.clone()), Data::from(checks));
    let anns = s.diffs[&crate::diff_screen::DiffOf::Pr(pr)]
        .doc
        .annotations();
    let shown: Vec<_> = (anns.iter())
        .filter(|a| matches!(a.key, AnnotationKey::Check(_)))
        .collect();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].path, "src/point.rs");
    assert!(matches!(shown[0].place, Place::Line { at, .. } if at.line == 3));
    assert!(shown[0].comments[0].author.contains("test (ubuntu)"));
}
