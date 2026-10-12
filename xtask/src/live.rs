//! `cargo xtask live`: ghtui in the real app, against real GitHub.
//!
//! Each [`Check`] drives ghtui in a private fux server ([`fux`]), then
//! asks GitHub (`gh api`, [`gh`]) whether the change happened, and cleans
//! up what it made even when a step fails. Writes go only to
//! [`gh::REPO`]. The owner's browser is never opened: a stand-in
//! `xdg-open`, first on the server's `PATH`, records the URLs instead
//! ([`Cx::opened`]).
//!
//! Adding a check: write a `pub const CHECK: Check` in a file under
//! `live/checks/`, and list it in [`checks::ALL`].

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

pub mod checks;
pub mod fux;
pub mod gh;

pub use fux::{Launch, NARROW, Pane};

/// `cargo xtask live`.
#[derive(clap::Args)]
pub struct Args {
    /// Run only these checks (by name; `--list` names them).
    checks: Vec<String>,
    /// List the checks and stop.
    #[arg(long)]
    list: bool,
    /// Use this ghtui instead of building `target/release/ghtui`.
    #[arg(long, value_name = "PATH")]
    ghtui: Option<PathBuf>,
    /// The fux binary (default: `fux` on PATH, else ~/code/fux's release build).
    #[arg(long, value_name = "PATH")]
    fux: Option<PathBuf>,
}

/// A check: what it's called, what it shows, and how.
pub struct Check {
    pub name: &'static str,
    pub about: &'static str,
    pub run: fn(&mut Cx) -> Result<()>,
    /// Runs before any check starts, for what takes GitHub a while (a
    /// workflow to run), so the wait overlaps the other checks.
    pub prepare: Option<fn(&mut Cx) -> Result<()>>,
    /// Puts back what a killed run of this check may have left changed
    /// (a repository setting), when any run starts.
    pub sweep: Option<fn() -> Result<()>>,
}

impl Check {
    pub const fn new(
        name: &'static str,
        about: &'static str,
        run: fn(&mut Cx) -> Result<()>,
    ) -> Check {
        Check {
            name,
            about,
            run,
            prepare: None,
            sweep: None,
        }
    }

    pub const fn prepare(mut self, prepare: fn(&mut Cx) -> Result<()>) -> Check {
        self.prepare = Some(prepare);
        self
    }

    pub const fn sweep(mut self, sweep: fn() -> Result<()>) -> Check {
        self.sweep = Some(sweep);
        self
    }
}

/// Set by ctrl-c: the running check stops at its next step, and every
/// cleanup still runs.
static STOP: AtomicBool = AtomicBool::new(false);

pub fn stopping() -> bool {
    STOP.load(Ordering::Relaxed)
}

/// A check gives up without failing: something it needs is missing.
#[derive(Debug)]
pub struct Skipped(pub String);

impl std::fmt::Display for Skipped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Skipped {}

/// What every check shares.
pub struct Env {
    /// Everything the run makes on disk; emptied when a run starts.
    pub work: PathBuf,
    pub ghtui: PathBuf,
    pub fux: fux::Fux,
    /// The account the token is for.
    pub login: String,
    /// The token's scopes.
    pub scopes: Vec<String>,
    /// Unique to this run, for names and texts it writes.
    pub run_id: String,
    config: PathBuf,
    opened: PathBuf,
}

impl Env {
    /// The xdg-open stand-in's log: every URL ghtui opened.
    pub fn opened(&self) -> Vec<String> {
        std::fs::read_to_string(&self.opened)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

type Cleanup = Box<dyn FnOnce() -> Result<()>>;

/// One check's run: its panes, and what to undo when it ends.
pub struct Cx<'e> {
    pub env: &'e Env,
    name: &'static str,
    cleanups: Vec<(String, Cleanup)>,
    panes: Vec<Pane>,
}

impl Cx<'_> {
    /// Undoes something at the end of the check, pass or fail, in reverse
    /// order of being asked.
    pub fn defer(&mut self, what: impl Into<String>, undo: impl FnOnce() -> Result<()> + 'static) {
        self.cleanups.push((what.into(), Box::new(undo)));
    }

    /// Says what the check is doing (and stops it on ctrl-c).
    pub fn step(&self, what: &str) -> Result<()> {
        if stopping() {
            bail!("stopped");
        }
        say(&format!("    · {what}"));
        Ok(())
    }

    /// Starts ghtui at 216 columns, in the run's scratch directory.
    pub fn ghtui(&mut self, args: &[&str]) -> Result<Pane> {
        self.launch(Launch::new(args))
    }

    /// Starts ghtui as `launch` says. Its pane is closed when the check
    /// ends; only one runs at a time (a pane fux starts takes the focus,
    /// and ghtui out of focus waits minutes between live checks).
    pub fn launch(&mut self, launch: Launch) -> Result<Pane> {
        for pane in self.panes.drain(..) {
            pane.kill();
        }
        let name = format!("{}-{}", self.name, launch.size.0);
        let mut env = vec![
            (
                "XDG_CACHE_HOME".to_owned(),
                path(&self.env.work.join("xdg/cache")),
            ),
            (
                "XDG_DATA_HOME".to_owned(),
                path(&self.env.work.join("xdg/data")),
            ),
            (
                "XDG_STATE_HOME".to_owned(),
                path(&self.env.work.join("xdg/state")),
            ),
        ];
        env.extend(launch.env.iter().cloned());
        let cwd = launch.cwd.clone().unwrap_or_else(|| self.env.work.clone());
        let mut argv = vec![
            path(&self.env.ghtui),
            "--config".to_owned(),
            path(&self.env.config),
        ];
        argv.extend(launch.args.iter().cloned());
        let pane = self.env.fux.run(&name, launch.size, &cwd, &env, &argv)?;
        self.panes.push(pane.clone());
        Ok(pane)
    }

    /// URLs ghtui opened in the "browser" since `since` of them.
    pub fn opened_since(&self, since: usize) -> Vec<String> {
        self.env.opened().into_iter().skip(since).collect()
    }

    /// Gives up on this check without failing it.
    pub fn skip(why: impl Into<String>) -> anyhow::Error {
        anyhow::Error::new(Skipped(why.into()))
    }

    /// A text unique to this run, to find what the check wrote.
    pub fn mark(&self, what: &str) -> String {
        format!("[ghtui live] {what} {}", self.env.run_id)
    }
}

/// Everything a run makes goes under here.
pub fn default_work_dir() -> PathBuf {
    std::env::var_os("GHTUI_LIVE_DIR").map_or_else(
        || {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default();
            home.join(".cache/claude-tmp/ghtui-live")
        },
        PathBuf::from,
    )
}

pub fn path(p: &Path) -> String {
    p.display().to_string()
}

#[expect(clippy::print_stdout, reason = "the check's report")]
pub fn say(line: &str) {
    println!("{line}");
}

#[expect(clippy::print_stderr, reason = "a command-line tool")]
pub fn main(args: &Args) -> ExitCode {
    if args.list {
        for check in checks::ALL {
            say(&format!("{:<14} {}", check.name, check.about));
        }
        return ExitCode::SUCCESS;
    }
    match run(args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(err) => {
            eprintln!("live: {err:#}");
            ExitCode::FAILURE
        }
    }
}

enum Outcome {
    Passed,
    Failed(String),
    Skipped(String),
}

fn run(args: &Args) -> Result<bool> {
    let started = Instant::now();
    let selected: Vec<&Check> = if args.checks.is_empty() {
        checks::ALL.iter().collect()
    } else {
        let mut picked = Vec::new();
        for name in &args.checks {
            match checks::ALL.iter().find(|c| c.name == name.as_str()) {
                Some(check) => picked.push(check),
                None => bail!("no check named {name} (`cargo xtask live --list` names them)"),
            }
        }
        picked
    };
    on_ctrl_c();
    let work = default_work_dir();
    let ghtui = match &args.ghtui {
        Some(path) => std::fs::canonicalize(path)
            .with_context(|| format!("no ghtui at {}", path.display()))?,
        None => build_ghtui()?,
    };
    let (login, scopes) = gh::account()?;
    let fux_bin = fux::find(args.fux.as_deref())?;
    // A run that was killed left its server behind.
    fux::Fux::stop_at(&fux_bin, &work.join("run/fux.sock"));
    if work.exists() {
        std::fs::remove_dir_all(&work)
            .with_context(|| format!("cannot empty {}", work.display()))?;
    }
    for dir in [
        "run",
        "bin",
        "xdg/cache",
        "xdg/data",
        "xdg/state",
        "failures",
    ] {
        std::fs::create_dir_all(work.join(dir))?;
    }
    set_private(&work.join("run"))?;
    let opened = work.join("opened.txt");
    write_xdg_open(&work.join("bin/xdg-open"), &opened)?;
    let config = work.join("config.toml");
    std::fs::write(&config, "")?;
    let token = gh::token()?;
    let fux = fux::Fux::start(&fux_bin, &work, &[("GH_ENTERPRISE_TOKEN", token.as_str())])?;
    let run_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_default();
    let env = Env {
        work,
        ghtui,
        fux,
        login,
        scopes,
        run_id,
        config,
        opened,
    };
    say(&format!(
        "Live check of {} on {}, as {}, in {}",
        path(&env.ghtui),
        gh::REPO,
        env.login,
        path(&env.work)
    ));
    let swept = sweep();
    if let Err(err) = &swept {
        say(&format!("  leftovers from an earlier run: {err:#}"));
    }

    let mut cxs: Vec<(&Check, Cx, Option<Outcome>)> = selected
        .iter()
        .map(|&check| {
            let cx = Cx {
                env: &env,
                name: check.name,
                cleanups: Vec::new(),
                panes: Vec::new(),
            };
            (check, cx, None)
        })
        .collect();
    for (check, cx, outcome) in &mut cxs {
        if let Some(prepare) = check.prepare
            && let Err(err) = prepare(cx)
        {
            *outcome = Some(outcome_of(&err));
        }
    }
    let mut results = Vec::new();
    for (check, mut cx, prepared) in cxs {
        let began = Instant::now();
        say(&format!("▶ {}: {}", check.name, check.about));
        let outcome = match prepared {
            Some(outcome) => outcome,
            None if stopping() => Outcome::Failed("stopped".into()),
            None => match (check.run)(&mut cx) {
                Ok(()) => Outcome::Passed,
                Err(err) => outcome_of(&err),
            },
        };
        if matches!(outcome, Outcome::Failed(_)) {
            for pane in &cx.panes {
                let file = env
                    .work
                    .join("failures")
                    .join(format!("{}.txt", pane.name()));
                if let Ok(screen) = pane.screen_marked() {
                    let _ = std::fs::write(&file, screen);
                    say(&format!("    its last screen: {}", path(&file)));
                }
            }
        }
        let mut problems = Vec::new();
        for pane in cx.panes.drain(..) {
            pane.kill();
        }
        while let Some((what, undo)) = cx.cleanups.pop() {
            if let Err(err) = undo() {
                problems.push(format!("{what}: {err:#}"));
            }
        }
        let secs = began.elapsed().as_secs_f32();
        match &outcome {
            Outcome::Passed => say(&format!("✓ {} ({secs:.0}s)", check.name)),
            Outcome::Failed(why) => say(&format!("✗ {} ({secs:.0}s): {why}", check.name)),
            Outcome::Skipped(why) => say(&format!("– {} skipped: {why}", check.name)),
        }
        for problem in &problems {
            say(&format!("    cleanup failed: {problem}"));
        }
        results.push((check.name, outcome, problems));
    }

    env.fux.stop();
    let passed = results
        .iter()
        .filter(|(_, o, _)| matches!(o, Outcome::Passed))
        .count();
    let failed: Vec<&str> = results
        .iter()
        .filter(|(_, o, p)| matches!(o, Outcome::Failed(_)) || !p.is_empty())
        .map(|(name, _, _)| *name)
        .collect();
    let skipped = results
        .iter()
        .filter(|(_, o, _)| matches!(o, Outcome::Skipped(_)))
        .count();
    say(&format!(
        "\n{passed} passed, {} failed, {skipped} skipped in {:.0}s{}",
        failed.len(),
        started.elapsed().as_secs_f32(),
        if failed.is_empty() {
            String::new()
        } else {
            format!(": {}", failed.join(", "))
        }
    ));
    Ok(failed.is_empty())
}

fn outcome_of(err: &anyhow::Error) -> Outcome {
    match err.downcast_ref::<Skipped>() {
        Some(Skipped(why)) => Outcome::Skipped(why.clone()),
        None => Outcome::Failed(format!("{err:#}")),
    }
}

/// What an earlier run left behind when it was killed before cleaning up:
/// issues, pull requests and branches it named, its projects, and the
/// repository settings checks change.
fn sweep() -> Result<()> {
    for issue in gh::rest_list(&format!(
        "repos/{}/issues?state=all&creator=@me&per_page=100",
        gh::REPO
    ))? {
        let title = issue["title"].as_str().unwrap_or_default();
        let number = issue["number"].as_u64().unwrap_or_default();
        if !title.starts_with("[ghtui live]") {
            continue;
        }
        if issue.get("pull_request").is_some() {
            if issue["state"] == "open" {
                gh::close_pr(number)?;
            }
        } else {
            gh::delete_issue(number)?;
        }
    }
    for branch in gh::rest_list(&format!("repos/{}/branches?per_page=100", gh::REPO))? {
        if let Some(name) = branch["name"].as_str()
            && name.starts_with(gh::BRANCH_PREFIX)
        {
            gh::delete_branch(name)?;
        }
    }
    for check in checks::ALL {
        if let Some(sweep) = check.sweep {
            sweep()?;
        }
    }
    Ok(())
}

/// Builds the release ghtui, as a person runs it.
fn build_ghtui() -> Result<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("xtask is inside the repository")?
        .to_path_buf();
    say("Building ghtui (cargo build --release -j 2)…");
    let status =
        std::process::Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
            .args(["build", "--release", "--quiet", "-j", "2", "--bin", "ghtui"])
            .current_dir(&root)
            .status()?;
    if !status.success() {
        bail!("building ghtui failed");
    }
    let target =
        std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), PathBuf::from);
    Ok(target.join("release/ghtui"))
}

/// The browser ghtui gets: it writes down the URL and opens nothing.
fn write_xdg_open(script: &Path, log: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let log = path(log).replace('\'', r"'\''");
    std::fs::write(
        script,
        format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{log}'\n"),
    )?;
    std::fs::set_permissions(script, std::fs::Permissions::from_mode(0o755))?;
    Ok(())
}

fn set_private(dir: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

/// ctrl-c stops the run after cleaning up; the step running then fails
/// (its fux or gh call gets the signal too).
fn on_ctrl_c() {
    std::thread::spawn(|| {
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return;
        };
        runtime.block_on(async {
            while tokio::signal::ctrl_c().await.is_ok() {
                STOP.store(true, Ordering::Relaxed);
                say("\nStopping: cleaning up first…");
            }
        });
    });
}

/// Waits for `f` to answer, asking every two seconds, for GitHub's
/// answers to catch up (or a workflow to run).
pub fn until<T>(
    what: &str,
    timeout: Duration,
    mut f: impl FnMut() -> Result<Option<T>>,
) -> Result<T> {
    let began = Instant::now();
    loop {
        if let Some(found) = f()? {
            return Ok(found);
        }
        if stopping() {
            bail!("stopped while waiting for {what}");
        }
        if began.elapsed() > timeout {
            bail!("{what}: not after {}s", timeout.as_secs());
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}
