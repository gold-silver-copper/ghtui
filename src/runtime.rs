//! The message loop: terminal events and background results become [`Msg`]s;
//! [`Cmd`]s run as tokio tasks. The UI task never awaits network or git work.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::api::model::{PendingReview, PrRef, ReviewEvent};
use crate::api::{ApiError, GitHub};
use crate::store::{DraftComment, ReviewState, Reviews};
use crate::ui::bars::Notice;
use anyhow::Result;
use crossterm::event::{Event, EventStream, KeyEventKind, MouseEventKind};
use futures::{FutureExt, StreamExt};
use ratatui::DefaultTerminal;
use tokio::sync::mpsc;

use crate::data::{self, DataKey};
use crate::diff_job::{self, GitContext, JobControl, JobMsg, JobTx, RepoGit};
use crate::diff_screen::DiffOf;
use crate::review::EditPurpose;
use crate::review::{self, SubmitOutcome};
use crate::state::{Api, Cmd, DiffMsg, Git, Msg, Problem, State, apply_msg, timers};
use crate::view::view;
use crate::work::Failure;

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
    /// What git answers in place of GitHub's API.
    local: Arc<crate::source::Local>,
    /// Repositories' code indexes.
    index: Arc<crate::code_index::Indexer>,
    /// Writes waiting for GitHub, and the repositories kept offline
    /// ([`crate::offline`]).
    queue: mpsc::UnboundedSender<crate::offline::queue::Op>,
    keep: mpsc::UnboundedSender<crate::offline::keep::Op>,
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
    let index = Arc::new(crate::code_index::Indexer::new(git.clone()));
    let local = crate::source::Local::new(git.clone());
    follow_reach(&gh, &tx);
    let queue = crate::offline::queue::start(gh.clone(), tx.clone());
    let limits = state.offline.limits;
    let keep = crate::offline::keep::start(gh.clone(), local.clone(), limits, tx.clone());
    let mut effects = Effects {
        index,
        gh,
        local,
        queue,
        keep,
        git,
        tx,
        jobs: HashMap::new(),
        save_review,
        writer,
        reviews,
    };
    let result = drive(terminal, state, &mut effects, rx, initial, started).await;
    // Answers cached just before quitting still reach the disk, and so do
    // drafts.
    let store = effects.gh.store().clone();
    blocking(move || store.sync()).await;
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

    // A scripted demo (demo/tour.py) reads each frame's text to know when
    // what it waits for is on screen.
    let dump = std::env::var_os("GHTUI_FRAME_DUMP");
    draw(terminal, &state, dump.as_deref())?;
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
                Some(Ok(event)) => match event_msg(event) {
                    Some(msg) => msg,
                    None => continue,
                },
                Some(Err(err)) => return Err(err.into()),
                None => return Ok(()),
            },
            Some(msg) = rx.recv() => msg,
            signal = stop.recv() => {
                tracing::info!(signal, "stopping");
                save_tabs(&effects.gh, &state);
                return Ok(());
            }
        };
        let mut cmds = apply_msg(&mut state, msg);
        // Every key, scroll or resize that's ready is drawn once; each is
        // settled first, as the next may act on the page it opened. They
        // wait while the editor is to open, or ghtui to quit.
        let editing = |cmds: &[Cmd]| cmds.iter().any(|c| matches!(c, Cmd::Edit { .. }));
        while !state.quit
            && !editing(&cmds)
            && let Some(event) = events.next().now_or_never()
        {
            // Ended: the loop sees it next.
            let Some(event) = event.transpose()? else {
                break;
            };
            let Some(msg) = event_msg(event) else {
                continue;
            };
            cmds.extend(state.settle());
            cmds.extend(apply_msg(&mut state, msg));
        }
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
            save_tabs(&effects.gh, &state);
            return Ok(());
        }
        draw(terminal, &state, dump.as_deref())?;
    }
}

/// What a terminal event means to ghtui, if anything.
fn event_msg(event: Event) -> Option<Msg> {
    Some(match event {
        Event::Key(key) if key.kind != KeyEventKind::Release => Msg::Key(key),
        Event::Resize(w, h) => Msg::Resize(w, h),
        Event::Paste(text) => Msg::Paste(text),
        Event::FocusGained => Msg::Focus(true),
        Event::FocusLost => Msg::Focus(false),
        Event::Mouse(m)
            if matches!(
                m.kind,
                MouseEventKind::Down(_) | MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
            ) =>
        {
            Msg::Mouse(m)
        }
        _ => return None,
    })
}

/// Draws `state`, and with a `dump` path writes the frame's text there.
fn draw(
    terminal: &mut DefaultTerminal,
    state: &State,
    dump: Option<&std::ffi::OsStr>,
) -> Result<()> {
    let frame = terminal.draw(|frame| view(state, frame, crate::store::now()))?;
    if let Some(path) = dump {
        let width = usize::from(frame.buffer.area.width).max(1);
        let rows = frame.buffer.content.chunks(width);
        let text: String = rows
            .flat_map(|row| row.iter().map(ratatui::buffer::Cell::symbol).chain(["\n"]))
            .collect();
        if let Err(err) = std::fs::write(path, text) {
            tracing::warn!(%err, "couldn't write the frame dump");
        }
    }
    Ok(())
}

impl Effects {
    /// Runs `cmd` in the background. The editor needs the terminal, so an
    /// edit comes back for the loop to run.
    fn run(&mut self, cmd: Cmd) -> Option<(EditPurpose, String)> {
        let replies = panic_replies(&cmd);
        match cmd {
            Cmd::Api(api) => spawn(api, replies, &self.gh, &self.local, &self.tx),
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
            Cmd::Work(work) => {
                let cx = crate::work::Cx {
                    gh: self.gh.clone(),
                    git: self.git.clone(),
                };
                spawn_guarded(&self.tx, replies, |tx| async move {
                    let done = work.run(&cx).await;
                    let _ = tx.send(Msg::Done(done));
                    told(&cx.gh, &tx);
                });
            }
            Cmd::Offline(cmd) => self.offline(cmd, replies),
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
                                crate::diff::Content::Text(t) => Some((*i, &**t)),
                                _ => None,
                            })
                            .collect();
                        crate::diff::moves::detect_moves(&inputs)
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
            Git::Code(ask) => {
                let (gh, index) = (self.gh.clone(), self.index.clone());
                spawn_guarded(&self.tx, replies, |tx| async move {
                    if let Some(answer) = crate::code_nav::answer(&gh, &index, &ask).await {
                        let _ = tx.send(Msg::Code(ask, answer));
                    }
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

    fn offline(&self, cmd: crate::offline::Cmd, replies: Vec<Msg>) {
        use crate::offline::{Cmd, Msg as Off};
        match cmd {
            Cmd::Probe => {
                let gh = self.gh.clone();
                spawn_guarded(&self.tx, replies, |tx| async move {
                    let back = gh.probe().await;
                    let _ = tx.send(Msg::Offline(Off::Probed(back)));
                });
            }
            Cmd::Queue(op) => {
                if self.queue.send(op).is_err() {
                    let text = "Writes can't be queued (internal error; see the log)";
                    let _ = self.tx.send(Msg::Notice(Notice::Error(text.into())));
                }
            }
            Cmd::Keep(op) => {
                let _ = self.keep.send(op);
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

    fn job_git(&self, of: &DiffOf) -> Option<RepoGit> {
        self.jobs.get(of).and_then(|(control, _)| control.git())
    }
}

/// Runs a GitHub request and sends its answer, then the rate limits (and
/// the token's state, when it changed).
fn spawn(
    api: Api,
    replies: Vec<Msg>,
    gh: &GitHub,
    local: &Arc<crate::source::Local>,
    tx: &mpsc::UnboundedSender<Msg>,
) {
    let (gh, local) = (gh.clone(), local.clone());
    spawn_guarded(tx, replies, |tx| async move {
        let msg = match api {
            Api::FetchViewer => Msg::Viewer(gh.viewer_login().await),
            Api::FetchUnread => Msg::Unread(gh.unread().await),
            Api::Fetch { key, cached } => {
                // Nothing on screen yet: the cache's copy, first or (for
                // what shows it only then) while GitHub can't be reached.
                let send_cached = |gh: GitHub, key: DataKey| async {
                    let lookup = key.clone();
                    if let Some(c) = blocking(move || data::load::cached(&gh, &lookup)).await {
                        let _ = tx.send(Msg::Fetched {
                            key,
                            result: Ok(c.value),
                            cached_at: Some(c.fetched_at),
                            source: None,
                        });
                    }
                };
                if cached && key.cache_first() {
                    send_cached(gh.clone(), key.clone()).await;
                }
                let (result, source) = data::load::load(&gh, &local, &key, None, Some(&tx)).await;
                if cached && !key.cache_first() && matches!(result, Err(ApiError::Offline(_))) {
                    send_cached(gh.clone(), key.clone()).await;
                }
                Msg::Fetched {
                    key,
                    result,
                    cached_at: None,
                    source,
                }
            }
            Api::FetchMore { key, after } => {
                let more = Some(after.clone());
                let (result, source) = data::load::load(&gh, &local, &key, more, None).await;
                Msg::FetchedMore(key, after, result, source)
            }
            Api::Suggest(q) => {
                let result = gh
                    .search(crate::api::browse::SearchKind::Repos, &q, None)
                    .await
                    .map(|r| match r {
                        crate::api::browse::SearchResults::Repos(r) => {
                            r.items.into_iter().take(6).collect()
                        }
                        _ => Vec::new(),
                    });
                Msg::Suggested(q, result)
            }
            Api::SaveVisits(visits) => {
                gh.remember(crate::store::VISITS, &visits);
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
            Api::FetchMentionable(repo) => {
                let result = gh.mentionable(&repo).await;
                Msg::Mentionable(repo, result)
            }
            Api::Poll { pr, asks } => {
                let polls = asks.into_iter().map(|ask| {
                    let (gh, path) = (&gh, ask.probe.path(&pr));
                    async move {
                        let answer = gh.poll(&path, ask.etag.as_deref()).await;
                        (ask.probe, answer)
                    }
                });
                let answers = futures::future::join_all(polls).await;
                Msg::Polled(pr, answers)
            }
            Api::Watch { what, asks } => {
                let polls = asks.into_iter().map(|ask| {
                    let gh = &gh;
                    async move {
                        let answer = crate::live::page::ask(gh, &ask).await;
                        (ask.probe, answer)
                    }
                });
                let answers = futures::future::join_all(polls).await;
                Msg::Watched(what, answers)
            }
        };
        let _ = tx.send(msg);
        told(&gh, &tx);
    });
}

/// After a request: the rate limits, and what GitHub's answers left out,
/// doubted or rejected.
fn told(gh: &GitHub, tx: &mpsc::UnboundedSender<Msg>) {
    let _ = tx.send(Msg::RateLimits(gh.rate_limits()));
    let left_out = gh.take_left_out();
    if !left_out.is_empty() {
        let text = format!("GitHub left some data out: {}", left_out.join("; "));
        let _ = tx.send(Msg::Notice(Notice::Error(text)));
    }
    for missing in gh.take_missing() {
        let _ = tx.send(Msg::Notice(Notice::Info(missing)));
    }
    let doubts = gh.take_doubts();
    if !doubts.is_empty() {
        let text = format!("GitHub's answer doesn't add up: {}", doubts.join("; "));
        let _ = tx.send(Msg::Notice(Notice::Error(text)));
    }
    if let Some(rejected) = gh.token_rejected_change() {
        let hint = crate::api::auth::sign_in_hint(crate::host::current());
        let text = format!("GitHub rejected the token: {hint}, then restart");
        let _ = tx.send(Msg::Problem(Problem::Auth, rejected.then_some(text)));
    }
}

/// Tells the page each time GitHub becomes unreachable, or reachable
/// again, whichever request found out.
fn follow_reach(gh: &GitHub, tx: &mpsc::UnboundedSender<Msg>) {
    let mut reach = gh.reach().watch();
    let tx = tx.clone();
    tokio::spawn(async move {
        while reach.changed().await.is_ok() {
            let since = *reach.borrow_and_update();
            let msg = crate::offline::Msg::Reach(since);
            if tx.send(Msg::Offline(msg)).is_err() {
                return;
            }
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
        Cmd::Api(Api::FetchUnread) => Msg::Unread(api()),
        Cmd::Api(Api::Fetch { key, .. }) => Msg::Fetched {
            key: key.clone(),
            result: api(),
            cached_at: None,
            source: None,
        },
        Cmd::Api(Api::FetchMore { key, after }) => {
            Msg::FetchedMore(key.clone(), after.clone(), api(), None)
        }
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
        Cmd::Work(work) => Msg::Done(work.failed()),
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
        Cmd::Api(Api::FetchMentionable(repo)) => Msg::Mentionable(repo.clone(), api()),
        Cmd::Git(Git::Code(ask)) => Msg::Code(ask.clone(), Err("ghtui hit a bug".into())),
        Cmd::Api(Api::Poll { pr, .. }) => Msg::Polled(pr.clone(), Vec::new()),
        Cmd::Api(Api::Watch { what, .. }) => Msg::Watched(what.clone(), Vec::new()),
        Cmd::Offline(crate::offline::Cmd::Probe) => {
            Msg::Offline(crate::offline::Msg::Probed(false))
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
        | Cmd::Offline(crate::offline::Cmd::Queue(_) | crate::offline::Cmd::Keep(_))
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
/// terminal (a plain `kill` would leave it in raw mode with mouse
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
fn save_tabs(gh: &GitHub, state: &State) {
    let urls = if state.tab_count() > 1 {
        state.tab_urls()
    } else {
        Vec::new()
    };
    gh.remember(crate::store::TABS, &urls);
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
    if crate::ui::markdown::scheme(url)
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
        let problem = match crate::git::version().await {
            Ok(v) if v >= crate::git::MIN_VERSION => {
                tracing::info!(version = %v, "git");
                return;
            }
            Ok(v) => format!(
                "git {v} is too old; ghtui needs {} or newer",
                crate::git::MIN_VERSION
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
    // GitHub can't be reached, and nothing was sent: to queue.
    let unsent = |review: &Review<'_>| {
        Some(Box::new(crate::offline::queue::Review {
            pr: pr.clone(),
            head: head.to_owned(),
            drafts: review.drafts.clone(),
            event: review.event,
            body: review.body.to_owned(),
            seen: review.seen,
        }))
    };
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
                if let ApiError::Offline(_) = err {
                    outcome.unsent = unsent(&review);
                }
                outcome.error = Some(format!("Couldn't start the review: {}", api_message(&err)));
                return outcome;
            }
        },
        Err(err) => {
            if let ApiError::Offline(_) = err {
                outcome.unsent = unsent(&review);
            }
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

/// Submits a queued review ([`crate::offline::queue`]). What GitHub
/// refuses stays queued as refused, without the comments it took (they
/// wait in your pending review there, counted as seen).
pub(crate) async fn send_review(
    gh: &GitHub,
    queued: crate::offline::queue::Review,
) -> crate::offline::queue::Sent {
    use crate::offline::queue::Sent;
    let review = Review {
        drafts: queued.drafts.clone(),
        event: queued.event,
        body: &queued.body,
        seen: queued.seen,
    };
    let outcome = submit_review(gh, &queued.pr, &queued.head, review).await;
    if outcome.unsent.is_some() {
        return Sent::Unsent;
    }
    if outcome.submitted {
        return Sent::Taken;
    }
    let why = match (&outcome.error, outcome.rejected.as_slice()) {
        (Some(err), _) => err.clone(),
        (None, rejected) => {
            let reasons: Vec<&str> = rejected.iter().map(|(_, r)| r.as_str()).collect();
            format!("GitHub refused comments: {}", reasons.join("; "))
        }
    };
    let seen = outcome.on_github.unwrap_or(queued.seen);
    let mut rest = queued;
    rest.drafts.retain(|d| !outcome.accepted.contains(&d.id));
    for draft in &mut rest.drafts {
        draft.error = (outcome.rejected.iter())
            .find(|(id, _)| *id == draft.id)
            .map(|(_, reason)| reason.clone());
    }
    rest.seen = seen;
    Sent::Partly(rest, why)
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
            crate::ui::text::short_sha(commit)
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

/// Clears the screen and has the next frame drawn whole. Not
/// [`ratatui::Terminal::clear`]: it asks the terminal where its cursor is
/// first, which right after an editor (input not yet read again) can time
/// out, leaving the screen blank but for what changes.
fn redraw_whole(terminal: &mut DefaultTerminal) -> std::io::Result<()> {
    use ratatui::backend::Backend;
    terminal.backend_mut().clear()?;
    // Each swap blanks the buffer it leaves behind: twice, both are.
    terminal.swap_buffers();
    terminal.swap_buffers();
    Ok(())
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
            .and_then(|()| redraw_whole(terminal));
        crate::capture_input(true);
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
    use crate::api::model::NodeId;

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
