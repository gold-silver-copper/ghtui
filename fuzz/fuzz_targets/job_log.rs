//! Any job log, cut to its last 2 MB or not, split into steps whose times
//! are anything, renders as the job's page without panicking, whatever
//! step and line a link points at or the filter holds.
#![no_main]

use ghtui_api::browse::{CheckOutcome, Job, JobLog, Step};
use ghtui_api::model::RepoId;
use ghtui_ui::page::Page;
use ghtui_ui::pages::{JobAt, Keys};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: (&str, Vec<(u8, &str, &str)>, (u8, u16), &str)| {
    let (log, steps, (step, line), query) = input;
    let outcomes = [
        CheckOutcome::Success,
        CheckOutcome::Failure,
        CheckOutcome::Pending,
        CheckOutcome::Skipped,
    ];
    let steps: Vec<Step> = steps
        .into_iter()
        .take(20)
        .enumerate()
        .map(|(i, (outcome, start, end))| Step {
            number: u32::try_from(i + 1).unwrap_or(1),
            name: format!("Run step {i}"),
            outcome: outcomes[usize::from(outcome) % outcomes.len()],
            started_at: Some(start.to_owned()),
            completed_at: Some(end.to_owned()),
        })
        .collect();
    let job = Job {
        id: 1,
        run_id: 1,
        name: "job".into(),
        outcome: CheckOutcome::Failure,
        started_at: None,
        completed_at: None,
        steps,
    };
    let (cut, text) = ghtui_api::browse::log::cut(log);
    let log = JobLog {
        cut,
        text: text.to_owned(),
        running: false,
    };
    let at = JobAt {
        step: Some((u32::from(step), u32::from(line))),
        query,
        keys: Keys::default(),
    };
    let mut page = Page::new(100);
    ghtui_ui::pages::job(&mut page, &RepoId::new("o", "r"), &job, Some(Ok(&log)), at, 0);
});
