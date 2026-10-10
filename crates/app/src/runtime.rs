//! The message loop: terminal events and background results become [`Msg`]s;
//! [`Cmd`]s run as tokio tasks. The UI task never awaits network or git work.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{Event, EventStream, KeyEventKind, MouseEventKind};
use futures::StreamExt;
use ghtui_api::model::{PendingReview, PrRef, RepoId, ReviewEvent};
use ghtui_api::{ApiError, GitHub};
use ghtui_store::{DraftComment, ReviewState, Reviews};
use ghtui_ui::bars::Notice;
use ratatui::DefaultTerminal;
use tokio::sync::mpsc;

use crate::browse::{Data, DataKey};
use crate::diff_job::{self, GitContext, JobControl, JobMsg, JobTx, RepoGit};
use crate::diff_screen::DiffOf;
use crate::review::EditPurpose;
use crate::review::{self, SubmitOutcome};
use crate::state::{Api, Cmd, DiffMsg, Failure, Git, Msg, Problem, State, apply_msg, timers};
use crate::view::view;

struct Effects {
    gh: GitHub,
    git: GitContext,
    tx: mpsc::UnboundedSender<Msg>,
    /// Running diff jobs by PR.
    jobs: HashMap<DiffOf, (Arc<JobControl>, tokio::task::JoinHandle<()>)>,
    /// Where review drafts live, or why they can't be saved.
    reviews: Result<Reviews, String>,
    /// Review saves go through one writer, in order.
    save_review: mpsc::UnboundedSender<(PrRef, ReviewState)>,
    writer: tokio::task::JoinHandle<()>,
}

pub async fn run(
    terminal: &mut DefaultTerminal,
    state: State,
    gh: GitHub,
    git: GitContext,
    reviews: Result<Reviews, String>,
    initial: Vec<Cmd>,
    started: Instant,
) -> Result<()> {
    let (tx, rx) = mpsc::unbounded_channel::<Msg>();
    let (save_review, writer) = review_writer(reviews.clone(), &tx);
    let mut effects = Effects {
        gh,
        git,
        tx,
        jobs: HashMap::new(),
        save_review,
        writer,
        reviews,
    };
    let result = drive(terminal, state, &mut effects, rx, initial, started).await;
    // Drafts saved just before quitting still reach the disk.
    drop(effects.save_review);
    #[expect(clippy::disallowed_methods, reason = "a file write: no progress")]
    if tokio::time::timeout(Duration::from_secs(2), effects.writer)
        .await
        .is_err()
    {
        tracing::warn!("review state was still being written at exit");
    }
    result
}

async fn drive(
    terminal: &mut DefaultTerminal,
    mut state: State,
    effects: &mut Effects,
    mut rx: mpsc::UnboundedReceiver<Msg>,
    initial: Vec<Cmd>,
    started: Instant,
) -> Result<()> {
    let tx = effects.tx.clone();
    let mut events = EventStream::new();
    let mut stop = StopSignals::new()?;

    terminal.draw(|frame| view(&state, frame, ghtui_store::now()))?;
    tracing::info!(elapsed_ms = started.elapsed().as_millis(), "first paint");

    for cmd in initial {
        if effects.run(cmd).is_some() {
            tracing::warn!("an edit can't start before the first frame");
        }
    }
    check_git(tx.clone());

    loop {
        let msg = tokio::select! {
            event = events.next() => match event {
                Some(Ok(Event::Key(key))) if key.kind != KeyEventKind::Release => Msg::Key(key),
                Some(Ok(Event::Resize(w, h))) => Msg::Resize(w, h),
                Some(Ok(Event::Mouse(m))) if matches!(
                    m.kind,
                    MouseEventKind::Down(_) | MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
                ) => Msg::Mouse(m),
                Some(Ok(_)) => continue,
                Some(Err(err)) => return Err(err.into()),
                None => return Ok(()),
            },
            Some(msg) = rx.recv() => msg,
            signal = stop.recv() => {
                tracing::info!(signal, "stopping");
                save_tabs(&effects.gh, &state).await;
                return Ok(());
            }
        };
        let mut cmds = apply_msg(&mut state, msg);
        // Drain whatever else is ready so a burst of results is settled
        // and drawn once.
        while let Ok(msg) = rx.try_recv() {
            cmds.extend(apply_msg(&mut state, msg));
        }
        cmds.extend(state.settle());
        cmds.extend(timers(&mut state));
        for cmd in cmds {
            // The editor needs the terminal: run here, not spawned.
            if let Some((purpose, text)) = effects.run(cmd) {
                let (stream, result) = edit_externally(terminal, events, &text).await;
                events = stream;
                let _ = tx.send(Msg::Edited(purpose, result));
            }
        }
        if state.quit {
            save_tabs(&effects.gh, &state).await;
            return Ok(());
        }
        terminal.draw(|frame| view(&state, frame, ghtui_store::now()))?;
    }
}

impl Effects {
    /// Runs `cmd` in the background. The editor needs the terminal, so an
    /// edit comes back for the loop to run.
    fn run(&mut self, cmd: Cmd) -> Option<(EditPurpose, String)> {
        let replies = panic_replies(&cmd);
        match cmd {
            Cmd::Api(Api::Fetch {
                key: DataKey::Wiki(repo, page),
                ..
            }) => self.wiki(repo, page, replies),
            Cmd::Api(api) => spawn(api, replies, &self.gh, &self.tx),
            Cmd::Git(git) => self.git(git, replies),
            Cmd::OpenUrl(url) => {
                spawn_guarded(&self.tx, replies, |tx| async move {
                    if let Err(err) = open_url(&url).await {
                        let text = format!("Couldn't open browser: {err}");
                        let _ = tx.send(Msg::Notice(Notice::Error(text)));
                    }
                });
            }
            // Clipboard writes go to the terminal, right away.
            Cmd::Copy(text) => copy_to_clipboard(&text),
            Cmd::Timer(timer, ms) => {
                spawn_guarded(&self.tx, replies, |tx| async move {
                    tokio::time::sleep(Duration::from_millis(ms)).await;
                    let _ = tx.send(Msg::Timer(timer));
                });
            }
            Cmd::SuggestLater(q) => {
                spawn_guarded(&self.tx, replies, |tx| async move {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    let _ = tx.send(Msg::SuggestDue(q));
                });
            }
            Cmd::LoadReview(pr) => self.load_review(pr, replies),
            Cmd::SaveReview(pr, review) => {
                let _ = self.save_review.send((pr, review));
            }
            Cmd::Edit { purpose, text } => return Some((purpose, text)),
            Cmd::EditHome {
                path,
                shown,
                edit,
                what,
            } => {
                spawn_guarded(&self.tx, replies, |tx| async move {
                    let made = edit.clone();
                    let result =
                        blocking(move || crate::home::edit_file(&path, &shown, &made)).await;
                    let _ = tx.send(Msg::HomeEdited { edit, result, what });
                });
            }
        }
        None
    }

    fn git(&mut self, git: Git, replies: Vec<Msg>) {
        match git {
            Git::LoadDiff {
                of,
                job,
                base,
                range,
            } => {
                if let Some((_, handle)) = self.jobs.remove(&of) {
                    handle.abort();
                }
                let control = Arc::new(JobControl::default());
                let out = JobTx {
                    tx: self.tx.clone(),
                    of: of.clone(),
                    job,
                };
                let run = diff_job::run(self.git.clone(), base, range, out, control.clone());
                let handle = spawn_guarded(&self.tx, replies, |_| run);
                self.jobs.insert(of, (control, handle));
            }
            Git::Prioritize(of, files) => {
                if let Some((control, _)) = self.jobs.get(&of) {
                    control.prioritize(&files);
                }
            }
            Git::DetectMoves(joined) => {
                let (of, job, files) = joined.into_inner();
                let out = JobTx {
                    tx: self.tx.clone(),
                    of,
                    job,
                };
                spawn_guarded(&self.tx, replies, |_| async move {
                    let moves = blocking(move || {
                        let inputs: Vec<_> = files
                            .iter()
                            .filter_map(|(i, d)| match &d.content {
                                ghtui_diff::Content::Text(t) => Some((*i, &**t)),
                                _ => None,
                            })
                            .collect();
                        ghtui_diff::moves::detect_moves(&inputs)
                    })
                    .await;
                    out.send(JobMsg::Moves(moves));
                });
            }
            Git::SinceReview(joined) => {
                let (pr, job, old_head) = joined.into_inner();
                let Some(git) = self.job_git(&DiffOf::Pr(pr.clone())) else {
                    let result = Err(Failure::msg("the diff isn't ready"));
                    let msg = DiffMsg::Job(job, JobMsg::Since(old_head, result));
                    let _ = self.tx.send(Msg::Diff(pr.into(), msg));
                    return;
                };
                spawn_guarded(&self.tx, replies, |tx| async move {
                    let (repo, reader) = git;
                    let result =
                        diff_job::since_review_hashes(&repo, &reader, pr.number, &old_head)
                            .await
                            .map_err(Failure::from);
                    let msg = DiffMsg::Job(job, JobMsg::Since(old_head, result));
                    let _ = tx.send(Msg::Diff(pr.into(), msg));
                });
            }
            Git::ListCommits(pr) => {
                let Some(git) = self.job_git(&DiffOf::Pr(pr.clone())) else {
                    let _ = self.tx.send(Msg::Diff(
                        pr.into(),
                        DiffMsg::CommitsListed(Err(Failure::msg("the diff isn't ready"))),
                    ));
                    return;
                };
                spawn_guarded(&self.tx, replies, |tx| async move {
                    let (repo, _) = git;
                    let result = async {
                        let head = repo
                            .rev_parse(&format!("refs/ghtui/pr/{}/head", pr.number))
                            .await?;
                        let base = repo
                            .rev_parse(&format!("refs/ghtui/pr/{}/base", pr.number))
                            .await?;
                        let merge_base = repo.merge_base(&base, &head).await?;
                        repo.commits(&merge_base, &head).await
                    }
                    .await
                    .map_err(Failure::from);
                    let _ = tx.send(Msg::Diff(pr.into(), DiffMsg::CommitsListed(result)));
                });
            }
            Git::MapOutdated(joined) => {
                let (pr, job, head, threads) = joined.into_inner();
                let Some(git) = self.job_git(&DiffOf::Pr(pr.clone())) else {
                    return;
                };
                spawn_guarded(&self.tx, replies, |tx| async move {
                    let (repo, reader) = git;
                    let mut mapped = Vec::new();
                    for t in threads {
                        let to = diff_job::map_outdated(
                            &repo, &reader, &head, &t.path, &t.commit, t.line,
                        )
                        .await;
                        mapped.push((t.thread, to));
                    }
                    let msg = DiffMsg::Job(job, JobMsg::Mapped(mapped));
                    let _ = tx.send(Msg::Diff(pr.into(), msg));
                });
            }
        }
    }

    fn load_review(&self, pr: PrRef, replies: Vec<Msg>) {
        let reviews = self.reviews.clone();
        spawn_guarded(&self.tx, replies, |tx| async move {
            let review = match reviews {
                Ok(reviews) => {
                    let key = (pr.repo.owner.clone(), pr.repo.name.clone(), pr.number);
                    blocking(move || reviews.get(&key.0, &key.1, key.2))
                        .await
                        .map_err(Failure::from)
                }
                Err(err) => Err(Failure::msg(err)),
            };
            let _ = tx.send(Msg::Diff(pr.into(), DiffMsg::ReviewLoaded(review)));
        });
    }

    /// Reads a wiki page through git.
    fn wiki(&self, repo: RepoId, page: Option<String>, replies: Vec<Msg>) {
        let git = self.git.clone();
        spawn_guarded(&self.tx, replies, |tx| async move {
            let result = crate::wiki::page(&git, &repo, page.as_deref()).await;
            let _ = tx.send(Msg::Fetched {
                key: DataKey::Wiki(repo, page),
                result: result.map(|w| Data::Wiki(Box::new(w))),
                cached_at: None,
            });
        });
    }

    fn job_git(&self, of: &DiffOf) -> Option<RepoGit> {
        self.jobs.get(of).and_then(|(control, _)| control.git())
    }
}

/// Runs a GitHub request and sends its answer, then the rate limits (and
/// the token's state, when it changed).
fn spawn(api: Api, replies: Vec<Msg>, gh: &GitHub, tx: &mpsc::UnboundedSender<Msg>) {
    let gh = gh.clone();
    spawn_guarded(tx, replies, |tx| async move {
        let msg = match api {
            Api::FetchViewer => Msg::Viewer(gh.viewer_login().await),
            Api::FetchPr(pr) => {
                let result = gh.pull_request(&pr).await;
                Msg::Pr(pr, Box::new(result))
            }
            Api::Fetch { key, cached } => {
                if cached {
                    let (gh, lookup) = (gh.clone(), key.clone());
                    if let Some((data, at)) = blocking(move || cached_data(&gh, &lookup)).await {
                        let _ = tx.send(Msg::Fetched {
                            key: key.clone(),
                            result: Ok(data),
                            cached_at: Some(at),
                        });
                    }
                }
                // Nothing on screen yet: a search shows before its checks.
                let early = cached.then_some(&tx);
                let result = fetch(&gh, &key, None, early).await;
                Msg::Fetched {
                    key,
                    result,
                    cached_at: None,
                }
            }
            Api::FetchMore { key, after } => {
                let result = fetch(&gh, &key, Some(after.clone()), None).await;
                Msg::FetchedMore(key, after, result)
            }
            Api::Change(change, by) => {
                let result = gh.change(&change).await;
                Msg::Changed(change, by, result)
            }
            Api::Suggest(q) => {
                let result = gh
                    .search(ghtui_api::browse::SearchKind::Repos, &q, None)
                    .await
                    .map(|r| match r {
                        ghtui_api::browse::SearchResults::Repos(r) => {
                            r.items.into_iter().take(6).collect()
                        }
                        _ => Vec::new(),
                    });
                Msg::Suggested(q, result)
            }
            Api::SaveVisits(visits) => {
                gh.remember(ghtui_api::browse::keys::VISITS, visits).await;
                return;
            }
            Api::FetchViewed(pr) => {
                let result = gh.viewed_files(&pr).await;
                Msg::Diff(pr.into(), DiffMsg::ViewedLoaded(Box::new(result)))
            }
            Api::SetViewed {
                pr,
                pull_request_id,
                path,
                viewed,
                previous,
            } => {
                let result = gh.set_viewed(&pull_request_id, &path, viewed).await;
                Msg::Diff(
                    pr.into(),
                    DiffMsg::ViewedSaved {
                        path,
                        previous,
                        result,
                    },
                )
            }
            Api::FetchThreads(pr) => {
                let result = gh.review_threads(&pr).await;
                Msg::Diff(pr.into(), DiffMsg::ThreadsLoaded(result))
            }
            Api::FetchPatches(pr) => {
                let result = gh.pr_patches(&pr).await;
                Msg::Diff(pr.into(), DiffMsg::PatchesLoaded(result))
            }
            Api::SetResolved {
                pr,
                thread_id,
                resolved,
            } => {
                let result = gh.set_resolved(&thread_id, resolved).await;
                Msg::Diff(
                    pr.into(),
                    DiffMsg::ResolvedSet {
                        thread_id,
                        resolved,
                        result,
                    },
                )
            }
            Api::SubmitReview {
                pr,
                head,
                drafts,
                event,
                body,
                seen,
            } => {
                let review = Review {
                    drafts,
                    event,
                    body: &body,
                    seen,
                };
                let outcome = submit_review(&gh, &pr, &head, review).await;
                Msg::Diff(pr.into(), DiffMsg::ReviewSubmitted(outcome))
            }
            Api::FetchPendingReview(pr) => {
                let result = gh.pending_review(&pr).await;
                let comments = result.map(|(_, pending)| pending.map_or(0, |p| p.comments));
                Msg::Diff(pr.into(), DiffMsg::PendingReview(comments))
            }
            Api::FetchLastReview { pr, login } => {
                let result = gh.last_review_commit(&pr, &login).await;
                Msg::Diff(pr.into(), DiffMsg::LastReview(result))
            }
        };
        let _ = tx.send(msg);
        let _ = tx.send(Msg::RateLimits(gh.rate_limits()));
        let left_out = gh.take_left_out();
        if !left_out.is_empty() {
            let text = format!("GitHub left some data out: {}", left_out.join("; "));
            let _ = tx.send(Msg::Notice(Notice::Error(text)));
        }
        let doubts = gh.take_doubts();
        if !doubts.is_empty() {
            let text = format!("GitHub's answer doesn't add up: {}", doubts.join("; "));
            let _ = tx.send(Msg::Notice(Notice::Error(text)));
        }
        if let Some(rejected) = gh.token_rejected_change() {
            let text =
                "GitHub rejected the token: run `gh auth login` or set GH_TOKEN, then restart";
            let _ = tx.send(Msg::Problem(Problem::Auth, rejected.then(|| text.into())));
        }
    });
}

/// Spawns the task `start` makes from a sender of its own. If it panics,
/// the panic is logged and reported, and `replies` (what it would have
/// answered, as errors) go out instead, so no screen waits forever.
fn spawn_guarded<F: Future<Output = ()> + Send + 'static>(
    tx: &mpsc::UnboundedSender<Msg>,
    replies: Vec<Msg>,
    start: impl FnOnce(mpsc::UnboundedSender<Msg>) -> F,
) -> tokio::task::JoinHandle<()> {
    use futures::FutureExt;
    let task = start(tx.clone());
    let tx = tx.clone();
    tokio::spawn(async move {
        if let Err(panic) = std::panic::AssertUnwindSafe(task).catch_unwind().await {
            let what = panic_message(&*panic);
            tracing::error!(what, "background task panicked");
            let _ = tx.send(Msg::Notice(Notice::Error(format!(
                "Internal error: {what} (this is a bug; details are in the log)"
            ))));
            for reply in replies {
                let _ = tx.send(reply);
            }
        }
    })
}

/// Runs `f` on the blocking pool. A panic in `f` resumes in the caller, so
/// [`spawn_guarded`] reports it.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    match tokio::task::spawn_blocking(f).await {
        Ok(value) => value,
        Err(err) => match err.try_into_panic() {
            Ok(panic) => std::panic::resume_unwind(panic),
            // Only at runtime shutdown.
            Err(err) => std::panic::resume_unwind(Box::new(err.to_string())),
        },
    }
}

fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".into())
}

/// The replies `cmd` owes the UI, as failures: sent if its task panics.
fn panic_replies(cmd: &Cmd) -> Vec<Msg> {
    fn api<T>() -> Result<T, ApiError> {
        Err(ApiError::Internal("ghtui hit a bug".into()))
    }
    fn git<T>() -> Result<T, Failure> {
        Err(Failure::msg("ghtui hit a bug"))
    }
    let msg = match cmd {
        Cmd::Api(Api::FetchViewer) => Msg::Viewer(api()),
        Cmd::Api(Api::FetchPr(pr)) => Msg::Pr(pr.clone(), Box::new(api())),
        Cmd::Api(Api::Fetch { key, .. }) => Msg::Fetched {
            key: key.clone(),
            result: api(),
            cached_at: None,
        },
        Cmd::Api(Api::FetchMore { key, after }) => {
            Msg::FetchedMore(key.clone(), after.clone(), api())
        }
        Cmd::Api(Api::Change(change, by)) => Msg::Changed(change.clone(), by.clone(), api()),
        Cmd::Api(Api::Suggest(q)) => Msg::Suggested(q.clone(), api()),
        Cmd::Api(Api::FetchViewed(pr)) => {
            Msg::Diff(pr.clone().into(), DiffMsg::ViewedLoaded(Box::new(api())))
        }
        Cmd::Api(Api::SetViewed {
            pr, path, previous, ..
        }) => Msg::Diff(
            pr.clone().into(),
            DiffMsg::ViewedSaved {
                path: path.clone(),
                previous: *previous,
                result: api(),
            },
        ),
        Cmd::Api(Api::FetchThreads(pr)) => {
            Msg::Diff(pr.clone().into(), DiffMsg::ThreadsLoaded(api()))
        }
        Cmd::Api(Api::FetchPatches(pr)) => {
            Msg::Diff(pr.clone().into(), DiffMsg::PatchesLoaded(api()))
        }
        Cmd::Git(Git::MapOutdated(joined)) => {
            let (pr, job, _, threads) = joined.get();
            let none = threads.iter().map(|t| (t.thread.clone(), None)).collect();
            Msg::Diff(pr.clone().into(), DiffMsg::Job(*job, JobMsg::Mapped(none)))
        }
        Cmd::Api(Api::SetResolved {
            pr,
            thread_id,
            resolved,
        }) => Msg::Diff(
            pr.clone().into(),
            DiffMsg::ResolvedSet {
                thread_id: thread_id.clone(),
                resolved: *resolved,
                result: api(),
            },
        ),
        Cmd::LoadReview(pr) => Msg::Diff(pr.clone().into(), DiffMsg::ReviewLoaded(git())),
        Cmd::EditHome { edit, what, .. } => Msg::HomeEdited {
            edit: edit.clone(),
            result: Err("ghtui hit a bug".into()),
            what: what.clone(),
        },
        Cmd::Api(Api::SubmitReview { pr, .. }) => Msg::Diff(
            pr.clone().into(),
            DiffMsg::ReviewSubmitted(SubmitOutcome {
                error: Some("Couldn't submit: ghtui hit a bug".into()),
                ..SubmitOutcome::default()
            }),
        ),
        Cmd::Api(Api::FetchPendingReview(pr)) => {
            Msg::Diff(pr.clone().into(), DiffMsg::PendingReview(api()))
        }
        Cmd::Api(Api::FetchLastReview { pr, .. }) => {
            Msg::Diff(pr.clone().into(), DiffMsg::LastReview(api()))
        }
        Cmd::Git(Git::LoadDiff { of, job, .. }) => Msg::Diff(
            of.clone(),
            DiffMsg::Job(*job, JobMsg::Failed(Failure::msg("ghtui hit a bug"))),
        ),
        Cmd::Git(Git::DetectMoves(joined)) => {
            let (of, job, _) = joined.get();
            Msg::Diff(of.clone(), DiffMsg::Job(*job, JobMsg::Moves(Vec::new())))
        }
        Cmd::Git(Git::SinceReview(joined)) => {
            let (pr, job, old) = joined.get();
            let msg = DiffMsg::Job(*job, JobMsg::Since(old.clone(), git()));
            Msg::Diff(pr.clone().into(), msg)
        }
        Cmd::Git(Git::ListCommits(pr)) => {
            Msg::Diff(pr.clone().into(), DiffMsg::CommitsListed(git()))
        }
        // Nothing waits on these.
        Cmd::OpenUrl(_)
        | Cmd::Copy(_)
        | Cmd::Timer(..)
        | Cmd::SuggestLater(_)
        | Cmd::Api(Api::SaveVisits(_))
        | Cmd::Git(Git::Prioritize(..))
        | Cmd::SaveReview(..)
        | Cmd::Edit { .. } => return Vec::new(),
    };
    vec![msg]
}

/// Writes review state in the order it was saved (only the latest per PR
/// when several are waiting), and reports when drafts stop or resume
/// reaching the disk.
fn review_writer(
    reviews: Result<Reviews, String>,
    tx: &mpsc::UnboundedSender<Msg>,
) -> (
    mpsc::UnboundedSender<(PrRef, ReviewState)>,
    tokio::task::JoinHandle<()>,
) {
    let (save, mut saves) = mpsc::unbounded_channel::<(PrRef, ReviewState)>();
    let gone = Msg::Problem(
        Problem::Drafts,
        Some("Drafts are no longer being saved (internal error; see the log)".into()),
    );
    let writer = spawn_guarded(tx, vec![gone], |out| async move {
        let mut failing = false;
        while let Some(first) = saves.recv().await {
            let mut latest: Vec<(PrRef, ReviewState)> = vec![first];
            while let Ok((pr, review)) = saves.try_recv() {
                match latest.iter_mut().find(|(p, _)| *p == pr) {
                    Some(slot) => slot.1 = review,
                    None => latest.push((pr, review)),
                }
            }
            let result = match reviews.clone() {
                Ok(reviews) => blocking(move || {
                    latest.iter().try_for_each(|(pr, review)| {
                        reviews.put(&pr.repo.owner, &pr.repo.name, pr.number, review)
                    })
                })
                .await
                .map_err(|e| e.to_string()),
                Err(err) => Err(err),
            };
            match result {
                Ok(()) if failing => {
                    failing = false;
                    let _ = out.send(Msg::Problem(Problem::Drafts, None));
                }
                Ok(()) => {}
                Err(err) => {
                    tracing::warn!(%err, "saving review state failed");
                    failing = true;
                    let text = format!("Drafts are not being saved: {err}");
                    let _ = out.send(Msg::Problem(Problem::Drafts, Some(text)));
                }
            }
        }
    });
    (save, writer)
}

/// The signals that ask ghtui to stop: quitting on them restores the
/// terminal (a plain `kill` used to leave it in raw mode with mouse
/// reporting on).
struct StopSignals {
    #[cfg(unix)]
    signals: Vec<(&'static str, tokio::signal::unix::Signal)>,
}

impl StopSignals {
    fn new() -> std::io::Result<Self> {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            Ok(Self {
                signals: vec![
                    ("SIGTERM", signal(SignalKind::terminate())?),
                    ("SIGHUP", signal(SignalKind::hangup())?),
                    ("SIGQUIT", signal(SignalKind::quit())?),
                ],
            })
        }
        #[cfg(not(unix))]
        Ok(Self {})
    }

    /// The name of the next stop signal to arrive.
    async fn recv(&mut self) -> &'static str {
        #[cfg(unix)]
        {
            let waits = self.signals.iter_mut().map(|(name, signal)| {
                Box::pin(async move {
                    signal.recv().await;
                    *name
                })
            });
            futures::future::select_all(waits).await.0
        }
        #[cfg(not(unix))]
        std::future::pending().await
    }
}

/// Remembers the open tabs for next time. One tab isn't remembered:
/// ghtui starts at home, as it always has.
async fn save_tabs(gh: &GitHub, state: &State) {
    let urls = if state.tab_count() > 1 {
        state.tab_urls()
    } else {
        Vec::new()
    };
    gh.remember(ghtui_api::browse::keys::TABS, urls).await;
}

/// What the cache has for a page, and when it was fetched.
pub(crate) fn cached_data(gh: &GitHub, key: &DataKey) -> Option<(Data, u64)> {
    use ghtui_api::browse::keys;
    fn at<T>(c: ghtui_store::Cached<T>, wrap: impl FnOnce(T) -> Data) -> (Data, u64) {
        (wrap(c.value), c.fetched_at)
    }
    Some(match key {
        DataKey::Repo(repo) => at(gh.cached(&keys::repo(repo))?, |v| Data::Repo(Box::new(v))),
        DataKey::Readme(repo) => at(gh.cached(&keys::readme(repo))?, |v: Option<_>| {
            Data::Readme(v.map(Box::new))
        }),
        DataKey::Tree(repo, rev, path) => at(gh.cached(&keys::tree(repo, rev, path))?, Data::Tree),
        DataKey::Search(kind, query) => at(gh.cached(&keys::search(*kind, query))?, |v| {
            Data::Search(Box::new(v))
        }),
        DataKey::Counts(searches) => at(gh.cached(&keys::counts(searches))?, Data::Counts),
        DataKey::Issue(repo, number) => at(gh.cached(&keys::issue(repo, *number))?, |v| {
            Data::Issue(Some(Box::new(v)))
        }),
        DataKey::PrActivity(pr) => at(gh.cached(&keys::pr_activity(pr))?, |v| {
            Data::PrActivity(Box::new(v))
        }),
        DataKey::Profile(login) => at(gh.cached(&keys::profile(login))?, |v| {
            Data::Profile(Box::new(v))
        }),
        DataKey::Refs(repo) => at(gh.cached(&keys::refs(repo))?, |v| Data::Refs(Box::new(v))),
        DataKey::Commit(repo, oid) => at(gh.cached(&keys::commit(repo, oid))?, |v| {
            Data::Commit(Box::new(v))
        }),
        DataKey::PrChecks(pr) => at(gh.cached(&keys::pr_checks(pr))?, |v| {
            Data::Checks(Box::new(v))
        }),
        DataKey::Users(list) => at(gh.cached(&keys::users(list))?, |v| Data::Users(Box::new(v))),
        DataKey::Forks(repo) => at(gh.cached(&keys::forks(repo))?, |v| {
            Data::RepoPage(Box::new(v))
        }),
        DataKey::OwnerRepos(login, sort) => at(gh.cached(&keys::owner_repos(login, *sort))?, |v| {
            Data::RepoPage(Box::new(v))
        }),
        DataKey::Stars(login) => at(gh.cached(&keys::starred(login))?, |v| {
            Data::RepoPage(Box::new(v))
        }),
        DataKey::Releases(repo) => at(gh.cached(&keys::releases(repo))?, |v| {
            Data::Releases(Box::new(v))
        }),
        DataKey::Release(repo, tag) => at(gh.cached(&keys::release(repo, tag))?, |v| {
            Data::Release(Box::new(v))
        }),
        DataKey::Tags(repo) => at(gh.cached(&keys::tags(repo))?, |v| Data::Tags(Box::new(v))),
        DataKey::Milestones(repo, closed) => {
            at(gh.cached(&keys::milestones(repo, *closed))?, |v| {
                Data::Milestones(Box::new(v))
            })
        }
        DataKey::Milestone(repo, n) => at(gh.cached(&keys::milestone(repo, *n))?, |v| {
            Data::Milestone(Box::new(v))
        }),
        DataKey::Deployments(repo, env) => {
            at(gh.cached(&keys::deployments(repo, env.as_deref()))?, |v| {
                Data::Deployments(Box::new(v))
            })
        }
        DataKey::Compare(repo, spec) => at(gh.cached(&keys::compare(repo, spec))?, |v| {
            Data::Compare(Box::new(v))
        }),
        DataKey::Blame(repo, rev, path) => at(gh.cached(&keys::blame(repo, rev, path))?, |v| {
            Data::Blame(Box::new(v))
        }),
        DataKey::Gist(id) => at(gh.cached(&keys::gist(id))?, |v| Data::Gist(Box::new(v))),
        DataKey::Gists(login) => at(gh.cached(&keys::gists(login))?, |v| {
            Data::Gists(Box::new(v))
        }),
        DataKey::Teams(org) => at(gh.cached(&keys::teams(org))?, |v| Data::Teams(Box::new(v))),
        DataKey::Team(org, slug) => at(gh.cached(&keys::team(org, slug))?, |v| {
            Data::Team(Box::new(v))
        }),
        DataKey::Advisories(repo) => at(gh.cached(&keys::advisories(repo.as_ref()))?, |v| {
            Data::Advisories(Box::new(v))
        }),
        DataKey::Advisory(repo, ghsa) => {
            at(gh.cached(&keys::advisory(repo.as_ref(), ghsa))?, |v| {
                Data::Advisory(Box::new(v))
            })
        }
        DataKey::Branches(repo) => at(gh.cached(&keys::branches(repo))?, |v| {
            Data::Branches(Box::new(v))
        }),
        DataKey::Run(repo, run, attempt) => at(gh.cached(&keys::run(repo, *run, *attempt))?, |v| {
            Data::Run(Box::new(v))
        }),
        DataKey::Discussions(of, category) => at(
            gh.cached(&keys::discussions(of, category.as_deref()))?,
            |v| Data::Discussions(Box::new(v)),
        ),
        DataKey::Discussion(of, n) => at(gh.cached(&keys::discussion(of, *n))?, |v| {
            Data::Discussion(Box::new(v))
        }),
        DataKey::Job(repo, job) => at(gh.cached(&keys::job(repo, *job))?, |v| {
            Data::Job(Box::new(v))
        }),
        DataKey::Workflow(repo, file) => at(gh.cached(&keys::workflow(repo, file))?, |v| {
            Data::Workflow(Box::new(v))
        }),
        DataKey::WorkflowRuns(repo, file) => {
            at(gh.cached(&keys::workflow_runs(repo, file))?, |v| {
                Data::Runs(Box::new(v))
            })
        }
        DataKey::CommitChecks(repo, oid) => at(gh.cached(&keys::commit_checks(repo, oid))?, |v| {
            Data::Checks(Box::new(v))
        }),
        DataKey::BranchChecks(repo) => at(gh.cached(&keys::branch_checks(repo))?, |v| {
            Data::Checks(Box::new(v))
        }),
        DataKey::History(repo, rev, path) => at(gh.cached(&keys::history(repo, rev, path))?, |v| {
            Data::History(Box::new(v))
        }),
        DataKey::LastCommits(repo, rev, path) => {
            at(gh.cached(&keys::last_commits(repo, rev, path))?, |v| {
                Data::LastCommits(std::sync::Arc::new(v))
            })
        }
        // Files aren't cached; they can be large.
        // A wiki's clone is its cache.
        // Where a ref ends is asked anew each session.
        DataKey::Blob(..)
        | DataKey::Files(..)
        | DataKey::JobLog(..)
        | DataKey::Wiki(..)
        | DataKey::RefIn(..) => {
            return None;
        }
    })
}

/// A page's data; for a list, the page after `after` (the first without).
/// With `early`, a search's page is sent there as soon as it's found, as
/// a first look while its pull requests' checks are read.
pub(crate) async fn fetch(
    gh: &GitHub,
    key: &DataKey,
    after: Option<String>,
    early: Option<&mpsc::UnboundedSender<Msg>>,
) -> Result<Data, ApiError> {
    Ok(match key {
        DataKey::Repo(repo) => Data::Repo(Box::new(gh.repo(repo).await?)),
        DataKey::Readme(repo) => Data::Readme(gh.readme(repo).await?.map(Box::new)),
        DataKey::Tree(repo, rev, path) => Data::Tree(gh.tree(repo, rev, path).await?),
        DataKey::Blob(repo, rev, path) => Data::Blob(Box::new(gh.blob(repo, rev, path).await?)),
        DataKey::Search(kind, query) => {
            let found = |r: &ghtui_api::browse::SearchResults| {
                if let Some(tx) = early {
                    let _ = tx.send(Msg::Fetched {
                        key: key.clone(),
                        result: Ok(Data::Search(Box::new(r.clone()))),
                        cached_at: Some(ghtui_store::now()),
                    });
                }
            };
            let results = gh.search_showing(*kind, query, after, found).await?;
            Data::Search(Box::new(results))
        }
        DataKey::Counts(searches) => Data::Counts(gh.search_counts(searches).await?),
        DataKey::Issue(repo, number) => Data::Issue(gh.issue(repo, *number).await?.map(Box::new)),
        DataKey::PrActivity(pr) => Data::PrActivity(Box::new(gh.pr_activity(pr).await?)),
        DataKey::Profile(login) => Data::Profile(Box::new(gh.profile(login).await?)),
        DataKey::Files(repo, rev) => {
            let (files, truncated) = gh.file_list(repo, rev).await?;
            Data::Files(std::sync::Arc::new(files), truncated)
        }
        DataKey::Refs(repo) => Data::Refs(Box::new(gh.refs(repo).await?)),
        DataKey::RefIn(repo, spot) => Data::RefIn(gh.ref_in(repo, spot).await?),
        DataKey::Commit(repo, oid) => Data::Commit(Box::new(gh.commit(repo, oid).await?)),
        DataKey::PrChecks(pr) => Data::Checks(Box::new(gh.pr_checks(pr).await?)),
        DataKey::Users(list) => Data::Users(Box::new(gh.users(list, after).await?)),
        DataKey::Forks(repo) => Data::RepoPage(Box::new(gh.forks(repo, after).await?)),
        DataKey::OwnerRepos(login, sort) => {
            Data::RepoPage(Box::new(gh.owner_repos(login, *sort, after).await?))
        }
        DataKey::Stars(login) => Data::RepoPage(Box::new(gh.starred(login, after).await?)),
        DataKey::Releases(repo) => Data::Releases(Box::new(gh.releases(repo, after).await?)),
        DataKey::Release(repo, tag) => Data::Release(Box::new(gh.release(repo, tag).await?)),
        DataKey::Tags(repo) => Data::Tags(Box::new(gh.tags(repo, after).await?)),
        DataKey::Branches(repo) => Data::Branches(Box::new(gh.branches(repo, after).await?)),
        // Wikis come through git (see `Effects::wiki`).
        DataKey::Wiki(repo, _) => {
            return Err(ApiError::NotFound(format!(
                "{repo}'s wiki, which git reads"
            )));
        }
        DataKey::Advisories(repo) => {
            Data::Advisories(Box::new(gh.advisories(repo.as_ref(), after).await?))
        }
        DataKey::Advisory(repo, ghsa) => {
            Data::Advisory(Box::new(gh.advisory(repo.as_ref(), ghsa).await?))
        }
        DataKey::Teams(org) => Data::Teams(Box::new(gh.teams(org, after).await?)),
        DataKey::Team(org, slug) => Data::Team(Box::new(gh.team(org, slug).await?)),
        DataKey::Gist(id) => Data::Gist(Box::new(gh.gist(id).await?)),
        DataKey::Gists(login) => Data::Gists(Box::new(gh.gists(login, after).await?)),
        DataKey::Blame(repo, rev, path) => Data::Blame(Box::new(gh.blame(repo, rev, path).await?)),
        DataKey::Compare(repo, spec) => Data::Compare(Box::new(gh.compare(repo, spec).await?)),
        DataKey::Deployments(repo, env) => {
            Data::Deployments(Box::new(gh.deployments(repo, env.as_deref(), after).await?))
        }
        DataKey::Milestones(repo, closed) => {
            Data::Milestones(Box::new(gh.milestones(repo, *closed, after).await?))
        }
        DataKey::Milestone(repo, n) => {
            Data::Milestone(Box::new(gh.milestone(repo, *n, after).await?))
        }
        DataKey::Run(repo, run, attempt) => {
            Data::Run(Box::new(gh.workflow_run(repo, *run, *attempt).await?))
        }
        DataKey::Discussions(of, category) => Data::Discussions(Box::new(
            gh.discussions(of, category.as_deref(), after).await?,
        )),
        DataKey::Discussion(of, n) => Data::Discussion(Box::new(gh.discussion(of, *n).await?)),
        DataKey::Job(repo, job) => Data::Job(Box::new(gh.job(repo, *job).await?)),
        DataKey::JobLog(repo, job) => Data::Log(Arc::new(gh.job_log(repo, *job).await?)),
        DataKey::Workflow(repo, file) => Data::Workflow(Box::new(gh.workflow(repo, file).await?)),
        DataKey::WorkflowRuns(repo, file) => {
            Data::Runs(Box::new(gh.workflow_runs(repo, file, after).await?))
        }
        DataKey::CommitChecks(repo, oid) => {
            Data::Checks(Box::new(gh.commit_checks(repo, oid).await?))
        }
        DataKey::BranchChecks(repo) => Data::Checks(Box::new(gh.branch_checks(repo).await?)),
        DataKey::History(repo, rev, path) => {
            Data::History(Box::new(gh.history(repo, rev, path, after).await?))
        }
        DataKey::LastCommits(repo, rev, path) => {
            let names: Vec<String> = gh
                .tree(repo, rev, path)
                .await?
                .into_iter()
                .map(|e| e.name)
                .collect();
            Data::LastCommits(std::sync::Arc::new(
                gh.last_commits(repo, rev, path, &names).await?,
            ))
        }
    })
}

/// Puts `text` on the clipboard with OSC 52 (through tmux too, when its
/// `set-clipboard` allows).
fn copy_to_clipboard(text: &str) {
    use base64::Engine;
    use std::io::Write;
    let encoded = base64::engine::general_purpose::STANDARD.encode(text);
    let sequence = if std::env::var_os("TMUX").is_some() {
        format!("\x1bPtmux;\x1b\x1b]52;c;{encoded}\x07\x1b\\")
    } else {
        format!("\x1b]52;c;{encoded}\x07")
    };
    let mut out = std::io::stdout();
    let _ = out.write_all(sequence.as_bytes());
    let _ = out.flush();
}

/// Opens a web or email link in the default handler. Nothing else is
/// opened, whatever the link came from.
async fn open_url(url: &str) -> std::io::Result<()> {
    if ghtui_ui::markdown::scheme(url)
        .is_none_or(|s| !matches!(s.as_str(), "http" | "https" | "mailto"))
    {
        return Err(std::io::Error::other("only web and email links are opened"));
    }
    // Never through a shell: `cmd /C start` would run `&` in a URL.
    let (program, args): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("open", vec![url])
    } else if cfg!(windows) {
        ("rundll32", vec!["url.dll,FileProtocolHandler", url])
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

/// Diffs need a recent git. Check in the background and
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

fn api_message(err: &ApiError) -> String {
    match err {
        ApiError::GraphQl(messages) => messages.join("; "),
        err => err.to_string(),
    }
}

/// A review to submit.
struct Review<'a> {
    drafts: Vec<DraftComment>,
    event: ReviewEvent,
    body: &'a str,
    /// The comments in your pending review on GitHub you were shown.
    seen: u64,
}

/// Submits a review: reuses the viewer's pending review on GitHub (or
/// starts one on `head`), adds each draft as a thread, and submits only if
/// GitHub accepted every draft. Each draft's fate is reported, so rejected
/// ones keep their text. A pending review holding comments you weren't
/// shown isn't submitted: they'd be published unseen.
async fn submit_review(gh: &GitHub, pr: &PrRef, head: &str, review: Review<'_>) -> SubmitOutcome {
    let mut outcome = SubmitOutcome::default();
    let (review_id, held) = match gh.pending_review(pr).await {
        Ok((_, Some(pending))) if pending.comments > review.seen => {
            outcome.on_github = Some(pending.comments);
            outcome.error = Some(
                "Not sent: your pending review on GitHub holds comments you hadn't seen".into(),
            );
            return outcome;
        }
        Ok((_, Some(pending))) => match reusable(&pending, head) {
            Ok(()) => (pending.id, pending.comments),
            Err(message) => {
                outcome.error = Some(message);
                return outcome;
            }
        },
        Ok((pr_id, None)) => match gh.start_review(&pr_id, head).await {
            Ok(id) => (id, 0),
            Err(err) => {
                outcome.error = Some(format!("Couldn't start the review: {}", api_message(&err)));
                return outcome;
            }
        },
        Err(err) => {
            outcome.error = Some(format!("Couldn't reach the PR: {}", api_message(&err)));
            return outcome;
        }
    };
    for draft in review.drafts {
        match gh
            .add_review_thread(&review_id, &review::new_thread(&draft))
            .await
        {
            Ok(_) => outcome.accepted.push(draft.id),
            Err(err) => outcome.rejected.push((draft.id, api_message(&err))),
        }
    }
    // Those accepted are in it now, and were yours to see.
    outcome.on_github = Some(held + outcome.accepted.len() as u64);
    if !outcome.rejected.is_empty() {
        return outcome;
    }
    match gh
        .submit_review(&review_id, review.event, review.body)
        .await
    {
        Ok(()) => outcome.submitted = true,
        Err(err) => outcome.error = Some(format!("Couldn't submit: {}", api_message(&err))),
    }
    outcome
}

/// Whether the drafts can join your pending review: it must be on `head`.
/// One on another commit has its comments placed on that commit's lines,
/// and the drafts' line numbers are from this diff, so they're not mixed;
/// nor is the pending review discarded, since it may hold comments written
/// on GitHub.
fn reusable(pending: &PendingReview, head: &str) -> Result<(), String> {
    match &pending.commit {
        Some(commit) if commit != head => Err(format!(
            "You have a pending review on another commit ({}): finish or discard it on GitHub, then submit again",
            ghtui_ui::text::short_sha(commit)
        )),
        _ => Ok(()),
    }
}

/// A temporary file holding `text` for the editor: created exclusively
/// (never through a symlink someone planted), readable only by you, and
/// removed when dropped.
fn draft_file(text: &str) -> std::io::Result<tempfile::NamedTempFile> {
    let mut file = tempfile::Builder::new()
        .prefix("ghtui-")
        .suffix(".md")
        .tempfile()?;
    std::io::Write::write_all(&mut file, text.as_bytes())?;
    Ok(file)
}

/// Suspends the TUI and edits `text` in `$VISUAL`/`$EDITOR` (default `vi`).
/// Terminal input is released for the editor and taken back after.
async fn edit_externally(
    terminal: &mut DefaultTerminal,
    events: EventStream,
    text: &str,
) -> (EventStream, Result<String, Failure>) {
    use crossterm::ExecutableCommand;
    // Stop reading keys so the editor gets them.
    drop(events);
    let result = async {
        let file = draft_file(text)?;
        let path = file.path().to_owned();
        crate::restore_terminal();
        let editor = std::env::var("VISUAL")
            .or_else(|_| std::env::var("EDITOR"))
            .unwrap_or_else(|_| "vi".into());
        // Through the shell, so EDITOR may carry arguments ("code --wait").
        let status = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(format!("{editor} \"$1\""))
            .arg("sh")
            .arg(&path)
            .status()
            .await;
        let restored = crossterm::terminal::enable_raw_mode()
            .and_then(|()| {
                std::io::stdout()
                    .execute(crossterm::terminal::EnterAlternateScreen)
                    .map(drop)
            })
            .and_then(|()| terminal.clear());
        crate::set_mouse(true);
        if let Err(err) = restored {
            tracing::error!(%err, "couldn't restore the terminal after the editor");
        }
        match status {
            // By path: editors may replace the file rather than write it.
            Ok(status) if status.success() => Ok(std::fs::read_to_string(&path)?),
            Ok(status) => Err(Failure::msg(format!("{editor} exited with {status}"))),
            Err(err) => Err(Failure::msg(format!("couldn't start {editor}: {err}"))),
        }
    }
    .await;
    (EventStream::new(), result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghtui_api::model::NodeId;

    /// A panicking task still answers: a notice, then the failure reply
    /// the screen is waiting for.
    #[tokio::test]
    async fn panicking_task_sends_its_replies() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let pr = PrRef::parse("o/r#1").unwrap();
        let cmd = Cmd::Api(Api::FetchThreads(pr.clone()));
        let handle = spawn_guarded(&tx, panic_replies(&cmd), |_| async { panic!("boom") });
        handle.await.unwrap();
        assert!(matches!(
            rx.recv().await,
            Some(Msg::Notice(Notice::Error(e))) if e.contains("boom")
        ));
        assert!(matches!(
            rx.recv().await,
            Some(Msg::Diff(p, DiffMsg::ThreadsLoaded(Err(ApiError::Internal(_))))) if p == DiffOf::Pr(pr.clone())
        ));
    }

    #[test]
    fn a_pending_review_on_another_commit_is_not_reused() {
        let on = |commit: Option<&str>| PendingReview {
            id: NodeId::new("R_1"),
            commit: commit.map(Into::into),
            comments: 0,
        };
        assert_eq!(reusable(&on(Some("abc")), "abc"), Ok(()));
        assert_eq!(reusable(&on(None), "abc"), Ok(()));
        let err = reusable(&on(Some("0123456789")), "abc").unwrap_err();
        assert!(err.contains("another commit (0123456)"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn drafts_for_the_editor_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let a = draft_file("secret").unwrap();
        let b = draft_file("secret").unwrap();
        assert_ne!(a.path(), b.path());
        let mode = std::fs::metadata(a.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "{mode:o}");
        assert_eq!(std::fs::read_to_string(a.path()).unwrap(), "secret");
    }

    #[tokio::test]
    async fn only_web_and_email_links_open() {
        for url in [
            "file:///etc/passwd",
            "vscode://x",
            "ghtui:star",
            "/relative",
        ] {
            let err = open_url(url).await.unwrap_err();
            assert!(err.to_string().contains("only web"), "{url}");
        }
    }

    #[tokio::test]
    async fn panics_on_the_blocking_pool_reach_the_guard() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let handle = spawn_guarded(&tx, Vec::new(), |_| async {
            blocking(|| panic!("deep")).await;
        });
        handle.await.unwrap();
        assert!(matches!(
            rx.recv().await,
            Some(Msg::Notice(Notice::Error(e))) if e.contains("deep")
        ));
    }
}
