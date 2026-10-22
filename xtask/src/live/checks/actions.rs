//! Running a workflow by hand: `,E` on `actions.yml` (a job that fails on
//! purpose) with its `seconds` input set, the page following the run it
//! started until it ends. The run is deleted after.

use std::time::Duration;

use anyhow::{Context, Result, ensure};
use serde_json::Value;

use crate::live::{Check, Cx, gh, until};

pub const CHECK: Check = Check::new(
    "actions",
    ":run workflow runs actions.yml with an input and follows the run until it ends",
    run,
);

const GITHUB: Duration = Duration::from_secs(30);
/// Queued, run (a few seconds of log) and ended, as ghtui sees it.
const RUN: Duration = Duration::from_secs(240);
const SECONDS: &str = "3";

fn run(cx: &mut Cx) -> Result<()> {
    let login = cx.env.login.clone();
    let url = format!(
        "https://github.com/{}/actions/workflows/actions.yml",
        gh::REPO
    );
    let ghtui = cx.ghtui(&[&url])?;
    ghtui.wait_for(".github/workflows/actions.yml")?;
    ghtui.wait_followed()?;

    cx.step(":run workflow opens the form with the inputs the workflow declares")?;
    ghtui.palette("run workflow")?;
    ghtui.wait_for("Run workflow")?;
    ghtui.wait_for("How long the job writes its log, in seconds")?;
    ghtui.select("Down", "the seconds input", |s| s.contains("⟦▌seconds"))?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("Set the input")?;
    ghtui.type_text(SECONDS)?;
    ghtui.keys("Enter")?;
    ghtui.wait_for(&format!("“{SECONDS}”"))?;

    cx.step("ctrl-s runs it; the page follows the run it started")?;
    let since = gh::iso8601(gh::epoch().saturating_sub(5));
    ghtui.keys("C-s")?;
    let run = until("the run on GitHub", GITHUB, || {
        let runs = gh::get(&format!(
            "repos/{}/actions/workflows/actions.yml/runs?event=workflow_dispatch&per_page=5",
            gh::REPO
        ))?;
        Ok(runs["workflow_runs"].as_array().and_then(|runs| {
            runs.iter()
                .find(|r| {
                    r["actor"]["login"] == login.as_str()
                        && r["created_at"]
                            .as_str()
                            .is_some_and(|t| t >= since.as_str())
                })
                .cloned()
        }))
    })?;
    let id = run["id"].as_u64().context("no run id")?;
    let number = run["run_number"].as_u64().unwrap_or_default();
    cx.defer(format!("delete run {id}"), move || {
        // A run can't be deleted while it runs.
        until("the run to end", RUN, || {
            Ok((get_run(id)?["status"] == "completed").then_some(()))
        })?;
        gh::send(
            "DELETE",
            &format!("repos/{}/actions/runs/{id}", gh::REPO),
            None,
        )?;
        Ok(())
    });
    ghtui.wait_for(&format!("actions #{number}"))?;

    cx.step("it ends failed, shown without a refresh")?;
    ghtui.wait_for_within(&format!("✗ actions #{number}"), RUN)?;
    let ended = get_run(id)?;
    ensure!(
        ended["conclusion"] == "failure",
        "GitHub says the run's conclusion is {}",
        ended["conclusion"]
    );
    Ok(())
}

fn get_run(id: u64) -> Result<Value> {
    gh::get(&format!("repos/{}/actions/runs/{id}", gh::REPO))
}
