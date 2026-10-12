//! Saving a run's artifacts, and running a workflow by hand.

use super::*;
use crate::actions::{Done, Work};
use crate::ui::page::{DispatchLink, Link};

/// Selects the row on screen whose link is `link`.
fn select_link(s: &mut State, link: &Link) {
    let height = s.page_height();
    let Screen::Page(p) = s.screen_mut() else {
        panic!("not on a page")
    };
    let page = p.page();
    let i = (0..page.items.len())
        .find(|&i| page.target(page.items[i].link) == Some(link))
        .unwrap_or_else(|| panic!("no {link:?} row"));
    p.select(i, height);
}

/// The one write `cmds` holds.
fn work(cmds: &[Cmd]) -> Work {
    let works: Vec<&Work> = (cmds.iter())
        .filter_map(|c| match c {
            Cmd::Api(Api::Actions(work)) => Some(work),
            _ => None,
        })
        .collect();
    match works[..] {
        [work] => work.clone(),
        _ => panic!("{cmds:?}"),
    }
}

const WORKFLOW: &str = "name: CI\non:\n  workflow_dispatch:\n    inputs:\n      outcome:\n        type: choice\n        options: [fail, pass]\n        default: fail\n      seconds:\n        type: string\n        default: \"60\"\n";

fn on_workflow() -> State {
    let mut s = with_repo();
    let _ = s.push(Route::Workflow {
        repo: repo(),
        file: "ci.yml".into(),
    });
    let (workflow, runs) = crate::fixtures::workflow_runs();
    fetched(
        &mut s,
        DataKey::Workflow(repo(), "ci.yml".into()),
        Data::Workflow(Box::new(workflow)),
    );
    let key = DataKey::Runs(repo(), "workflow:ci.yml".into());
    fetched(&mut s, key, Data::Runs(Box::new(runs)));
    s
}

/// `,E` on a workflow opens the form of its inputs, read from its file;
/// an input is set from its choices, `,E` runs it on the default branch
/// with every input's value, and the page follows the run it started.
#[test]
fn a_workflow_runs_by_hand_with_its_inputs_and_is_followed() {
    let mut s = on_workflow();
    press(&mut s, ",E");
    let dispatch = Route::Dispatch {
        repo: repo(),
        file: "ci.yml".into(),
    };
    assert_eq!(route(&s), dispatch);
    let blob = crate::api::browse::Blob {
        path: ".github/workflows/ci.yml".into(),
        text: Some(WORKFLOW.into()),
        size: WORKFLOW.len() as u64,
        truncated: false,
    };
    let key = DataKey::Blob(repo(), "HEAD".into(), ".github/workflows/ci.yml".into());
    fetched(&mut s, key, Data::Blob(Box::new(blob)));
    select_link(
        &mut s,
        &Link::Dispatch(DispatchLink::Input("outcome".into())),
    );
    press(&mut s, "<Enter>");
    press(&mut s, "pass<Enter>");
    let cmds = press(&mut s, ",E");
    assert_eq!(
        work(&cmds),
        Work::Dispatch {
            repo: repo(),
            file: "ci.yml".into(),
            git_ref: "main".into(),
            inputs: vec![
                ("outcome".into(), "pass".into()),
                ("seconds".into(), "60".into()),
            ],
        }
    );
    let done = Done::Dispatched {
        repo: repo(),
        file: "ci.yml".into(),
        result: Ok(Some(55)),
    };
    update(&mut s, Msg::Actions(done));
    let run = Route::WorkflowRun {
        repo: repo(),
        run: 55,
        attempt: None,
    };
    assert_eq!(route(&s), run);
    says(&s, "Started ci.yml: following its run");
}

/// A workflow without a `workflow_dispatch` trigger says so, and `,E`
/// says why it can't run.
#[test]
fn a_workflow_that_cant_run_by_hand_says_so() {
    let mut s = on_workflow();
    press(&mut s, ",E");
    let blob = crate::api::browse::Blob {
        path: ".github/workflows/ci.yml".into(),
        text: Some("on: push\n".into()),
        size: 9,
        truncated: false,
    };
    let key = DataKey::Blob(repo(), "HEAD".into(), ".github/workflows/ci.yml".into());
    fetched(&mut s, key, Data::Blob(Box::new(blob)));
    assert!(fetches(press(&mut s, ",E")).is_empty());
    says(&s, "can't be run by hand");
}

/// Enter on an artifact asks where to save it: the current folder by
/// default; a file that's there is replaced only when the row says so.
#[test]
fn an_artifact_is_saved_where_you_say() {
    let mut s = with_repo();
    let _ = s.push(Route::WorkflowRun {
        repo: repo(),
        run: 7,
        attempt: None,
    });
    let run = crate::fixtures::workflow_run();
    fetched(
        &mut s,
        DataKey::Run(repo(), 7, None),
        Data::Run(Box::new(run)),
    );
    let artifact = Link::Artifact {
        id: 31,
        name: "coverage-report".into(),
    };
    select_link(&mut s, &artifact);
    press(&mut s, "<Enter>");
    let cmds = press(&mut s, "<Enter>");
    let saved = Work::Save {
        repo: repo(),
        id: 31,
        name: "coverage-report".into(),
        path: std::path::Path::new(".").join("coverage-report.zip"),
        replace: false,
    };
    assert_eq!(work(&cmds), saved);
    // Cargo.toml is there: the row asks to replace it.
    select_link(&mut s, &artifact);
    press(&mut s, "<Enter>");
    press(&mut s, "Cargo.toml");
    let Some(Overlay::Picker(p)) = &s.overlay else {
        panic!("no picker")
    };
    let rows = s.picker_rows(p);
    assert_eq!(rows[0].0.label, "Replace Cargo.toml");
    let cmds = press(&mut s, "<Enter>");
    assert!(matches!(work(&cmds), Work::Save { replace: true, .. }));
}
