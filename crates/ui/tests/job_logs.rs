//! Real job logs (recorded by `contract_record_logs`, see `logs/`), split
//! into their steps and rendered, against golden summaries: for each step,
//! how many lines it got (and how many were cut from a log over 2 MB), and
//! how its first and last lines show. The cases: a BOM, a composite
//! action, groups and `[command]` lines (ratatui); a re-run with debug
//! logging (deno); a log over 2 MB (rust-lang/rust); a `##[warning]`
//! (cli/cli); a container job's `##[command]` lines (iputils).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests: failing loudly is the point"
)]

use std::io::Read;
use std::path::PathBuf;

use ghtui_api::browse::{Job, log};
use ghtui_ui::pages::{log_seg, step_lines};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/logs")
}

/// A recorded log, gzipped if it's big.
fn read_log(name: &str) -> String {
    let plain = dir().join(format!("{name}.log"));
    if plain.exists() {
        return std::fs::read_to_string(plain).unwrap();
    }
    let gz = std::fs::File::open(dir().join(format!("{name}.log.gz"))).unwrap();
    let mut text = String::new();
    flate2::read::GzDecoder::new(gz)
        .read_to_string(&mut text)
        .unwrap();
    text
}

fn shown(line: &str) -> String {
    let seg = log_seg(line);
    let text: String = seg.text().chars().take(90).collect();
    format!("{:?}: {text}", seg.role)
}

fn golden(name: &str) -> String {
    let raw = read_log(name);
    let job: Job = serde_json::from_str(
        &std::fs::read_to_string(dir().join(format!("{name}.job.json"))).unwrap(),
    )
    .unwrap();
    let (cut, kept) = log::cut(&raw);
    let total = raw.lines().count();
    let mut out = format!(
        "{} bytes, {total} lines; {} cut\n",
        raw.len(),
        cut.lines().count()
    );
    let mut placed = 0;
    for (number, cut, lines) in step_lines(&job, &cut, kept) {
        let step = job.steps.iter().find(|s| s.number == number).unwrap();
        placed += cut + lines.len();
        out.push_str(&format!(
            "\nstep {number} {:?} {}: {cut} cut, {} lines\n",
            step.outcome,
            step.name,
            lines.len()
        ));
        let ends: Vec<&&str> = if lines.len() <= 4 {
            lines.iter().collect()
        } else {
            lines
                .iter()
                .take(2)
                .chain(lines.iter().skip(lines.len() - 2))
                .collect()
        };
        for line in ends {
            out.push_str(&format!("  {}\n", shown(line)));
        }
    }
    // Every line is in a step: none lost, none twice.
    assert_eq!(placed, total, "{name}");
    out
}

macro_rules! cases {
    ($($test:ident: $name:literal,)*) => {$(
        #[test]
        fn $test() {
            insta::assert_snapshot!($name, golden($name));
        }
    )*};
}

cases! {
    a_composite_action_with_groups_and_commands: "ratatui-clippy",
    a_rerun_with_debug_logging: "deno-rerun-debug",
    a_log_over_2_mb: "rust-dist-big",
    a_warning: "cli-warning",
    a_container_jobs_commands: "iputils-container",
}
