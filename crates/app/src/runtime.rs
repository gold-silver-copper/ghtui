//! The message loop: terminal events and background results become [`Msg`]s;
//! [`Cmd`]s run as tokio tasks. The UI task never awaits network or git work.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use crossterm::event::{Event, EventStream, KeyEventKind};
use futures::StreamExt;
use ghtui_api::GitHub;
use ghtui_api::model::PrRef;
use ghtui_ui::bars::Notice;
use ratatui::DefaultTerminal;
use tokio::sync::mpsc;

use crate::diff_job::{self, GitContext, JobControl};
use crate::state::{Cmd, Msg, State, update};

/// Running diff jobs by PR.
#[derive(Default)]
struct Jobs {
    running: HashMap<PrRef, (Arc<JobControl>, tokio::task::JoinHandle<()>)>,
}

struct Effects {
    gh: GitHub,
    git: GitContext,
    tx: mpsc::UnboundedSender<Msg>,
    jobs: Jobs,
}
use crate::view::view;

pub async fn run(
    terminal: &mut DefaultTerminal,
    mut state: State,
    gh: GitHub,
    git: GitContext,
    initial: Vec<Cmd>,
    started: Instant,
) -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Msg>();
    let mut events = EventStream::new();
    let mut effects = Effects {
        gh,
        git,
        tx: tx.clone(),
        jobs: Jobs::default(),
    };

    terminal.draw(|frame| view(&state, frame, ghtui_store::now()))?;
    tracing::info!(elapsed_ms = started.elapsed().as_millis(), "first paint");

    for cmd in initial {
        effects.run(cmd);
    }
    check_git(tx.clone());

    loop {
        let msg = tokio::select! {
            event = events.next() => match event {
                Some(Ok(Event::Key(key))) if key.kind != KeyEventKind::Release => Msg::Key(key),
                Some(Ok(Event::Resize(w, h))) => Msg::Resize(w, h),
                Some(Ok(_)) => continue,
                Some(Err(err)) => return Err(err.into()),
                None => return Ok(()),
            },
            Some(msg) = rx.recv() => msg,
        };
        let mut cmds = update(&mut state, msg);
        // Drain whatever else is ready so a burst of results draws once.
        while let Ok(msg) = rx.try_recv() {
            cmds.extend(update(&mut state, msg));
        }
        for cmd in cmds {
            effects.run(cmd);
        }
        if state.quit {
            return Ok(());
        }
        terminal.draw(|frame| view(&state, frame, ghtui_store::now()))?;
    }
}

impl Effects {
    fn run(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::LoadDiff { pr, base_ref } => {
                if let Some((_, handle)) = self.jobs.running.remove(&pr) {
                    handle.abort();
                }
                let control = Arc::new(JobControl::default());
                let handle = tokio::spawn(diff_job::run(
                    self.git.clone(),
                    pr.clone(),
                    base_ref,
                    self.tx.clone(),
                    control.clone(),
                ));
                self.jobs.running.insert(pr, (control, handle));
            }
            Cmd::Prioritize(pr, files) => {
                if let Some((control, _)) = self.jobs.running.get(&pr) {
                    control.prioritize(&files);
                }
            }
            cmd => spawn(cmd, &self.gh, &self.tx),
        }
    }
}

fn spawn(cmd: Cmd, gh: &GitHub, tx: &mpsc::UnboundedSender<Msg>) {
    let gh = gh.clone();
    let tx = tx.clone();
    tokio::spawn(async move {
        let msg = match cmd {
            Cmd::FetchViewer => Msg::Viewer(gh.viewer_login().await),
            Cmd::FetchInbox => Msg::Inbox(gh.inbox().await),
            Cmd::FetchPr(pr) => {
                let result = gh.pull_request(&pr).await;
                Msg::Pr(pr, Box::new(result))
            }
            Cmd::OpenUrl(url) => match open_url(&url).await {
                Ok(()) => return,
                Err(err) => Msg::Notice(Notice::Error(format!("Couldn't open browser: {err}"))),
            },
            Cmd::FetchViewed(pr) => {
                let result = gh.viewed_files(&pr).await;
                Msg::ViewedLoaded(pr, Box::new(result))
            }
            Cmd::SetViewed {
                pr,
                pull_request_id,
                path,
                file,
                viewed,
                previous,
            } => {
                let result = gh.set_viewed(&pull_request_id, &path, viewed).await;
                Msg::ViewedSaved {
                    pr,
                    file,
                    previous,
                    result,
                }
            }
            Cmd::LoadReview(pr) => {
                let store = gh.store().clone();
                let key = pr.to_string();
                let review = tokio::task::spawn_blocking(move || store.review_get(&key))
                    .await
                    .unwrap_or_default();
                Msg::ReviewLoaded(pr, review)
            }
            Cmd::SaveReview(pr, review) => {
                let store = gh.store().clone();
                let key = pr.to_string();
                if let Err(err) =
                    tokio::task::spawn_blocking(move || store.review_put(&key, &review)).await
                {
                    tracing::warn!(%pr, %err, "saving review marks failed");
                }
                return;
            }
            Cmd::LoadDiff { .. } | Cmd::Prioritize(..) => unreachable!("handled by Effects::run"),
        };
        let _ = tx.send(msg);
        let _ = tx.send(Msg::RateLimits(gh.rate_limits()));
    });
}

async fn open_url(url: &str) -> std::io::Result<()> {
    let (program, args): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("open", vec![url])
    } else if cfg!(windows) {
        ("cmd", vec!["/C", "start", "", url])
    } else {
        ("xdg-open", vec![url])
    };
    let status = tokio::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "{program} exited with {status}"
        )))
    }
}

/// Diffs (next milestone) need a recent git. Check in the background and
/// warn instead of blocking startup.
fn check_git(tx: mpsc::UnboundedSender<Msg>) {
    tokio::spawn(async move {
        let problem = match ghtui_git::version().await {
            Ok(v) if v >= ghtui_git::MIN_VERSION => {
                tracing::info!(version = %v, "git");
                return;
            }
            Ok(v) => format!(
                "git {v} is too old; ghtui needs {} or newer",
                ghtui_git::MIN_VERSION
            ),
            Err(err) => format!("git not available: {err}"),
        };
        tracing::warn!("{problem}");
        let _ = tx.send(Msg::Notice(Notice::Error(problem)));
    });
}
