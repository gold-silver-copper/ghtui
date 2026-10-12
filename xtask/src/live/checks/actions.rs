//! Running a workflow by hand and seeing why it failed: `,E` on
//! `actions.yml` (which fails with annotations and uploads an artifact)
//! with its `seconds` input set, the page following the run it started
//! until it ends; the job's annotations; the artifact saved, and "Replace"
//! offered on saving it again.

use std::time::Duration;

use anyhow::{Context, Result, ensure};
use serde_json::Value;

use crate::live::{Check, Cx, gh, path, until};

pub const CHECK: Check = Check::new(
    "actions",
    ",E runs actions.yml with an input and follows the run; its job's annotations; its artifact saved, then Replace offered",
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

    cx.step(",E opens the form with the inputs the workflow declares")?;
    ghtui.keys(", E")?;
    ghtui.wait_for("Run workflow")?;
    ghtui.wait_for("How long the job writes its log, in seconds")?;
    ghtui.select("}", "the seconds input", |s| s.contains("⟦▌seconds"))?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("Set the input")?;
    ghtui.type_text(SECONDS)?;
    ghtui.keys("Enter")?;
    ghtui.wait_for(&format!("“{SECONDS}”"))?;

    cx.step(",E runs it; the page follows the run it started")?;
    let since = gh::iso8601(gh::epoch().saturating_sub(5));
    ghtui.keys(", E")?;
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

    cx.step("it ends failed, shown without a refresh, with its artifact")?;
    ghtui.wait_for_within(&format!("✗ actions #{number}"), RUN)?;
    ghtui.wait_for_within("ghtui-artifact", GITHUB)?;
    let ended = get_run(id)?;
    ensure!(
        ended["conclusion"] == "failure",
        "GitHub says the run's conclusion is {}",
        ended["conclusion"]
    );
    let jobs = gh::get(&format!("repos/{}/actions/runs/{id}/jobs", gh::REPO))?;
    let job = jobs["jobs"]
        .as_array()
        .and_then(|j| j.first())
        .cloned()
        .context("the run has no job")?;
    // The input reached the run: its log step took the seconds asked, not
    // the default minute.
    let step = job["steps"]
        .as_array()
        .and_then(|s| s.iter().find(|s| s["name"] == "Write the log slowly"))
        .cloned()
        .unwrap_or(Value::Null);
    let took = secs(&step["completed_at"]).saturating_sub(secs(&step["started_at"]));
    ensure!(
        step["conclusion"] == "success" && took < 30,
        "its log step took {took}s, not about {SECONDS}: {step}"
    );

    cx.step("↵ on the artifact saves it where you say")?;
    let dir = cx.env.work.join("artifacts");
    std::fs::create_dir_all(&dir)?;
    let artifacts = gh::get(&format!("repos/{}/actions/runs/{id}/artifacts", gh::REPO))?;
    let size = artifacts["artifacts"][0]["size_in_bytes"]
        .as_u64()
        .context("GitHub lists no artifact")?;
    ghtui.select("gg }", "the artifact", |s| s.contains("⟦▌▤ ghtui-artifact"))?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("Save the artifact")?;
    ghtui.type_text(&path(&dir))?;
    ghtui.wait_for(&format!(
        "Save to {}",
        path(&dir.join("ghtui-artifact.zip"))
    ))?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("Saved ghtui-artifact")?;
    let saved = dir.join("ghtui-artifact.zip");
    let len = std::fs::metadata(&saved)
        .with_context(|| format!("nothing at {}", path(&saved)))?
        .len();
    ensure!(len == size, "saved {len} bytes; GitHub says {size}");

    cx.step("saving it again offers to replace the file")?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("Save the artifact")?;
    ghtui.type_text(&path(&dir))?;
    ghtui.wait_for(&format!("Replace {}", path(&saved)))?;
    ghtui.keys("Escape")?;

    cx.step("the job's page lists its annotations")?;
    ghtui.select("gg", "the failed job", |s| s.contains("⟦▌✗ slow"))?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("Annotations")?;
    let job_id = job["id"].as_u64().context("no job id")?;
    let annotations = gh::get(&format!(
        "repos/{}/check-runs/{job_id}/annotations",
        gh::REPO
    ))?;
    let failing = annotations.as_array().and_then(|a| {
        a.iter()
            .find(|a| a["annotation_level"] == "failure" && a["path"] == "README.md")
            .cloned()
    });
    let failing = failing.context("GitHub has no failure annotation on README.md")?;
    ghtui.wait_for(&format!(
        "README.md:{}",
        failing["start_line"].as_u64().unwrap_or(1)
    ))?;
    ghtui.wait_for(failing["message"].as_str().unwrap_or("failing on purpose"))?;
    ghtui.wait_for("a warning for ghtui")?;
    Ok(())
}

/// `2026-10-12T14:03:56Z` as seconds into its day, enough to subtract two
/// times a few seconds apart (0 for anything else).
fn secs(time: &Value) -> u64 {
    let Some(clock) = time.as_str().and_then(|t| t.get(11..19)) else {
        return 0;
    };
    clock
        .split(':')
        .filter_map(|n| n.parse::<u64>().ok())
        .fold(0, |acc, n| acc * 60 + n)
}

fn get_run(id: u64) -> Result<Value> {
    gh::get(&format!("repos/{}/actions/runs/{id}", gh::REPO))
}
