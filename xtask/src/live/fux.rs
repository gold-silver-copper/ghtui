//! Driving ghtui through a private fux server (`fux help agents`): its own
//! socket and `XDG_RUNTIME_DIR` under the run's directory, so it can't
//! reach anyone's own fux session.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde_json::Value;

use super::stopping;

/// About as wide as a wide terminal, and as narrow as a split one.
pub const WIDE: (u16, u16) = (216, 60);
pub const NARROW: (u16, u16) = (90, 40);

/// How long a screen waits for something by default.
const WAIT: Duration = Duration::from_secs(30);

/// The fux binary: `--fux`, else `fux` on PATH, else ~/code/fux's build;
/// one new enough to have `fux help agents` (its `-S`, `run` and waits).
pub fn find(given: Option<&Path>) -> Result<PathBuf> {
    if let Some(given) = given {
        return Ok(given.to_path_buf());
    }
    let recent = |p: &PathBuf| {
        p.is_file()
            && Command::new(p)
                .args(["help", "agents"])
                .output()
                .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("# Driving programs"))
    };
    let on_path = std::env::var_os("PATH")
        .map(|p| {
            std::env::split_paths(&p)
                .map(|d| d.join("fux"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    on_path
        .into_iter()
        .chain([home.join("code/fux/target/release/fux")])
        .find(recent)
        .context("no fux new enough to have `fux help agents`: install it, or pass --fux PATH")
}

/// A private fux server.
#[derive(Clone)]
pub struct Fux {
    bin: PathBuf,
    socket: PathBuf,
    runtime: PathBuf,
    path: OsString,
}

impl Fux {
    /// Starts a server with its own socket under `work/run`. Its programs
    /// get this environment, `work/bin` (the xdg-open stand-in) first on
    /// `PATH`, and `extra`.
    pub fn start(bin: &Path, work: &Path, extra: &[(&str, &str)]) -> Result<Fux> {
        let mut paths = vec![work.join("bin")];
        paths.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        let fux = Fux {
            bin: bin.to_path_buf(),
            socket: work.join("run/fux.sock"),
            runtime: work.join("run"),
            path: std::env::join_paths(paths)?,
        };
        let mut cmd = fux.command();
        cmd.args(["server", "--background"]);
        for (k, v) in extra {
            cmd.env(k, v);
        }
        check(&cmd.output()?, "fux server")?;
        Ok(fux)
    }

    /// Stops a server left at `socket`, if one answers there.
    pub fn stop_at(bin: &Path, socket: &Path) {
        if socket.exists() {
            let _ = Command::new(bin)
                .arg("-S")
                .arg(socket)
                .args(["kill-server", "--force"])
                .env_remove("FUX_PANE")
                .env_remove("FUX_SERVER")
                .output();
        }
    }

    pub fn stop(&self) {
        let _ = self.command().args(["kill-server", "--force"]).output();
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(&self.bin);
        cmd.arg("-S")
            .arg(&self.socket)
            .env_remove("FUX_PANE")
            .env_remove("FUX_SERVER")
            .env("FUX_SOCKET", &self.socket)
            .env("XDG_RUNTIME_DIR", &self.runtime)
            .env("PATH", &self.path);
        cmd
    }

    /// Starts `argv` in a tab of its own named `name`.
    pub fn run(
        &self,
        name: &str,
        (cols, rows): (u16, u16),
        cwd: &Path,
        env: &[(String, String)],
        argv: &[String],
    ) -> Result<Pane> {
        let mut cmd = self.command();
        cmd.args(["run", "-n", name, "--size", &format!("{cols}x{rows}")])
            .arg("--cwd")
            .arg(cwd);
        for (k, v) in env {
            cmd.arg("--env").arg(format!("{k}={v}"));
        }
        cmd.arg("--").args(argv);
        check(&cmd.output()?, "fux run")?;
        Ok(Pane {
            name: name.to_owned(),
            fux: self.clone(),
        })
    }
}

fn check(out: &Output, what: &str) -> Result<()> {
    if !out.status.success() {
        bail!(
            "{what} failed ({}): {}{}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim(),
            String::from_utf8_lossy(&out.stdout).trim()
        );
    }
    Ok(())
}

/// ghtui running in a pane.
#[derive(Clone)]
pub struct Pane {
    name: String,
    fux: Fux,
}

impl Pane {
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Presses keys, fux's way: each word a key name (`Enter`, `C-s`,
    /// `Escape`, `Tab`, `Space`) or text (`,m`, `gg`), then waits for the
    /// screen to settle.
    pub fn keys(&self, keys: &str) -> Result<()> {
        if stopping() {
            bail!("stopped");
        }
        let mut cmd = self.fux.command();
        cmd.args(["send-keys", "-t", &self.name]);
        cmd.args(keys.split_whitespace());
        cmd.args(["--wait-idle", "400", "--timeout", "20"]);
        settled(&cmd.output()?, &format!("keys `{keys}`"))
    }

    /// Presses `keys` until the marked screen (`⟦…⟧` highlights) shows
    /// what `selected` looks for: a page that is still following GitHub
    /// may redraw, and move the selection, just after a key.
    pub fn select(&self, keys: &str, what: &str, selected: impl Fn(&str) -> bool) -> Result<()> {
        for _ in 0..5 {
            self.keys(keys)?;
            if selected(&self.screen_marked()?) {
                return Ok(());
            }
            let mut cmd = self.fux.command();
            cmd.args([
                "wait",
                "-t",
                &self.name,
                "--idle",
                "1000",
                "--timeout",
                "10",
            ]);
            let _ = cmd.output();
        }
        bail!("`{keys}` doesn't select {what}")
    }

    /// Types `text` as it is, then waits for the screen to settle.
    pub fn type_text(&self, text: &str) -> Result<()> {
        let mut cmd = self.fux.command();
        cmd.args(["send-keys", "-t", &self.name, "--text", text]);
        cmd.args(["--wait-idle", "200", "--timeout", "20"]);
        settled(&cmd.output()?, "typing")
    }

    /// The screen, as text.
    pub fn screen(&self) -> Result<String> {
        Ok(self.capture()?.0)
    }

    /// The screen with highlights marked `⟦…⟧` (a selection, a menu's row).
    pub fn screen_marked(&self) -> Result<String> {
        let out = self
            .fux
            .command()
            .args(["capture-pane", "-t", &self.name, "--marks"])
            .output()?;
        check(&out, "fux capture-pane")?;
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// The screen and its revision.
    fn capture(&self) -> Result<(String, u64)> {
        let out = self
            .fux
            .command()
            .args(["capture-pane", "-t", &self.name, "--json"])
            .output()?;
        check(&out, "fux capture-pane")?;
        let json: Value = serde_json::from_slice(&out.stdout)?;
        let lines = json["lines"]
            .as_array()
            .map(|lines| {
                lines
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        Ok((lines, json["revision"].as_u64().unwrap_or_default()))
    }

    /// Waits until the screen shows `text`; returns the line it's on.
    pub fn wait_for(&self, text: &str) -> Result<String> {
        self.wait_for_within(text, WAIT)
    }

    pub fn wait_for_within(&self, text: &str, timeout: Duration) -> Result<String> {
        self.wait_until(&format!("`{text}` on screen"), timeout, |screen| {
            screen.lines().find(|l| l.contains(text)).map(str::to_owned)
        })
    }

    /// Waits until the page has followed GitHub to the last change: the
    /// status bar no longer says it's waiting for GitHub or loading. Until
    /// then ghtui refuses the next change (what's on screen may be old).
    pub fn wait_followed(&self) -> Result<()> {
        let busy = |screen: &str| {
            let status = screen.lines().rev().find(|l| !l.trim().is_empty());
            status.is_some_and(|s| s.contains("Waiting for GitHub") || s.contains("Loading"))
        };
        for _ in 0..10 {
            self.wait_until(
                "the page to follow GitHub",
                Duration::from_secs(90),
                |screen| (!busy(screen)).then_some(()),
            )?;
            // Settled for a second: it may not have started waiting yet.
            let mut cmd = self.fux.command();
            cmd.args([
                "wait",
                "-t",
                &self.name,
                "--idle",
                "1000",
                "--timeout",
                "10",
            ]);
            let _ = cmd.output();
            if !busy(&self.screen()?) {
                return Ok(());
            }
        }
        bail!("the page goes on loading")
    }

    /// Waits until the screen no longer shows `text`.
    pub fn wait_gone(&self, text: &str) -> Result<()> {
        self.wait_until(&format!("`{text}` gone"), WAIT, |screen| {
            (!screen.contains(text)).then_some(())
        })
    }

    /// Waits until `found` finds something on screen, waking at each
    /// change of it (fux's `wait --change --since`), so nothing is missed.
    pub fn wait_until<T>(
        &self,
        what: &str,
        timeout: Duration,
        mut found: impl FnMut(&str) -> Option<T>,
    ) -> Result<T> {
        let began = Instant::now();
        loop {
            let (screen, revision) = self.capture()?;
            if let Some(it) = found(&screen) {
                return Ok(it);
            }
            let left = timeout.saturating_sub(began.elapsed());
            if left.is_zero() || stopping() {
                let last = screen
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
                bail!(
                    "waited {}s for {what}; the status bar says: {last}",
                    timeout.as_secs()
                );
            }
            let out = self
                .fux
                .command()
                .args(["wait", "-t", &self.name, "--change", "--since"])
                .arg(revision.to_string())
                .args(["--timeout", &left.as_secs().max(1).to_string(), "--json"])
                .output()?;
            match out.status.code() {
                Some(0 | 124) => {}
                Some(3) => bail!("ghtui exited while waiting for {what}: {}", tail(&out)),
                _ => check(&out, "fux wait")?,
            }
        }
    }

    /// Checks that `full` is shown whole or cut with `…`, never cut off
    /// bare: "this text ends in …". It's found by its first twenty
    /// characters, and runs to the next gap of two spaces, or its `…`.
    pub fn expect_whole_or_cut(&self, full: &str) -> Result<()> {
        let start: String = full.chars().take(20).collect();
        let screen = self.screen()?;
        let Some(line) = screen.lines().find(|l| l.contains(&start)) else {
            bail!("`{start}` isn't on screen");
        };
        let from = line.find(&start).unwrap_or_default();
        let rest = line.get(from..).unwrap_or_default();
        let shown = rest.split("  ").next().unwrap_or_default().trim_end();
        let shown = match shown.find('…') {
            Some(at) => shown.get(..at + '…'.len_utf8()).unwrap_or(shown),
            None => shown,
        };
        let cut = shown
            .strip_suffix('…')
            .is_some_and(|kept| full.starts_with(kept.trim_end()));
        if shown != full && !cut {
            bail!("`{shown}` is `{full}` cut without …");
        }
        Ok(())
    }

    pub fn kill(&self) {
        let _ = self
            .fux
            .command()
            .args(["kill-tab", "-t", &self.name])
            .output();
    }
}

/// `send-keys --wait-idle`: settled (0), or the keys changed nothing (5).
fn settled(out: &Output, what: &str) -> Result<()> {
    match out.status.code() {
        Some(0 | 5) => Ok(()),
        Some(3) => bail!("ghtui exited after {what}: {}", tail(out)),
        _ => check(out, &format!("fux send-keys ({what})")),
    }
}

fn tail(out: &Output) -> String {
    let text = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines
        .iter()
        .rev()
        .take(3)
        .rev()
        .map(|l| l.trim())
        .collect::<Vec<_>>()
        .join(" / ")
}

/// How ghtui is started: its arguments, size, directory and environment.
pub struct Launch {
    pub args: Vec<String>,
    pub size: (u16, u16),
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
}

impl Launch {
    pub fn new(args: &[&str]) -> Launch {
        Launch {
            args: args.iter().map(|&a| a.to_owned()).collect(),
            size: WIDE,
            cwd: None,
            env: Vec::new(),
        }
    }

    pub fn size(mut self, size: (u16, u16)) -> Launch {
        self.size = size;
        self
    }

    pub fn cwd(mut self, cwd: &Path) -> Launch {
        self.cwd = Some(cwd.to_path_buf());
        self
    }

    pub fn env(mut self, vars: impl IntoIterator<Item = (String, String)>) -> Launch {
        self.env.extend(vars);
        self
    }
}
