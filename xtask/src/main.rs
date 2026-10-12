//! ghtui's development tasks, run with `cargo xtask …`.
//!
//! - `cargo xtask live` runs ghtui for real, in a private fux server,
//!   against real GitHub, and checks each write and live behaviour against
//!   GitHub's own answer (README, Development).
//! - `cargo xtask ghes` serves the stand-in Enterprise Server the live
//!   check uses, to try ghtui against by hand.

use std::process::ExitCode;

use clap::Parser;

mod ghes;
mod live;

#[derive(Parser)]
#[command(name = "cargo xtask", about = "ghtui's development tasks")]
enum Task {
    /// Check ghtui in the real app, against real GitHub.
    Live(live::Args),
    /// Serve the stand-in GitHub Enterprise Server on https://localhost:PORT.
    Ghes(ghes::Args),
}

fn main() -> ExitCode {
    match Task::parse() {
        Task::Live(args) => live::main(&args),
        Task::Ghes(args) => ghes::main(&args),
    }
}
