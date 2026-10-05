//! The message loop: terminal events and background results become [`Msg`]s;
//! [`Cmd`]s run as tokio tasks. The UI task never awaits network or git work.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{Event, EventStream, KeyEventKind, MouseEventKind};
use futures::StreamExt;
use ghtui_api::model::{PrRef, ReviewEvent};
use ghtui_api::{ApiError, GitHub};
use ghtui_store::{DraftComment, ReviewState, Reviews};
use ghtui_ui::bars::Notice;
use ratatui::DefaultTerminal;
use tokio::sync::mpsc;

use crate::browse::{Data, DataKey};
use crate::diff_job::{self, GitContext, JobControl, JobMsg, JobTx};
use crate::review::{self, SubmitOutcome};
use crate::state::{Cmd, Msg, Problem, State, apply_msg, timers};
use crate::view::view;

struct Effects {
    gh: GitHub,
    git: GitContext,
    tx: mpsc::UnboundedSender<Msg>,
    /// Running diff jobs by PR.
    jobs: HashMap<PrRef, (Arc<JobControl>, tokio::task::JoinHandle<()>)>,
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
        effects.run(cmd);
    }
    check_git(tx.clone());

    let mut last_notice = None;
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
        cmds.extend(timers(&mut state, &mut last_notice));
        for cmd in cmds {
            match cmd {
                // The editor needs the terminal: handled here, not spawned.
                Cmd::Edit { purpose, text } => {
                    let (stream, result) = edit_externally(terminal, events, &text).await;
                    events = stream;
                    let _ = tx.send(Msg::Edited(purpose, result));
                }
                // Clipboard writes go to the terminal: also handled here.
                Cmd::Copy(text) => copy_to_clipboard(&text),
                cmd => effects.run(cmd),
            }
        }
        if state.quit {
            return Ok(());
        }
        terminal.draw(|frame| view(&state, frame, ghtui_store::now()))?;
    }
}

impl Effects {
    fn run(&mut self, cmd: Cmd) {
        let replies = panic_replies(&cmd);
        match cmd {
            Cmd::LoadDiff {
                pr,
                job,
                base_ref,
                range,
            } => {
                if let Some((_, handle)) = self.jobs.remove(&pr) {
                    handle.abort();
                }
                let control = Arc::new(JobControl::default());
                let out = JobTx {
                    tx: self.tx.clone(),
                    pr: pr.clone(),
                    job,
                };
                let run = diff_job::run(self.git.clone(), base_ref, range, out, control.clone());
                let handle = spawn_guarded(&self.tx, replies, run);
                self.jobs.insert(pr, (control, handle));
            }
            Cmd::Prioritize(pr, files) => {
                if let Some((control, _)) = self.jobs.get(&pr) {
                    control.prioritize(&files);
                }
            }
            Cmd::DetectMoves(pr, job, files) => {
                let out = JobTx {
                    tx: self.tx.clone(),
                    pr,
                    job,
                };
                spawn_guarded(&self.tx, replies, async move {
                    let moves = blocking(move || {
                        let inputs: Vec<_> = files
                            .iter()
                            .filter_map(|(i, d)| match &d.content {
                                ghtui_diff::Content::Text(t) => {
                                    Some((*i, &**t, t.lines(ghtui_diff::Whitespace::Exact)))
                                }
                                _ => None,
                            })
                            .collect();
                        ghtui_diff::moves::detect_moves(&inputs)
                    })
                    .await;
                    out.send(JobMsg::Moves(moves));
                });
            }
            Cmd::SinceReview { pr, old_head } => {
                let tx = self.tx.clone();
                let Some(git) = self.job_git(&pr) else {
                    let _ = tx.send(Msg::SinceReady(
                        pr,
                        old_head,
                        Err("the diff isn't ready".into()),
                    ));
                    return;
                };
                spawn_guarded(&self.tx, replies, async move {
                    let (repo, reader) = git;
                    let result =
                        diff_job::since_review_hashes(&repo, &reader, pr.number, &old_head)
                            .await
                            .map_err(|e| e.to_string());
                    let _ = tx.send(Msg::SinceReady(pr, old_head, result));
                });
            }
            Cmd::ListCommits(pr) => {
                let tx = self.tx.clone();
                let Some(git) = self.job_git(&pr) else {
                    let _ = tx.send(Msg::CommitsListed(pr, Err("the diff isn't ready".into())));
                    return;
                };
                spawn_guarded(&self.tx, replies, async move {
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
                    .map_err(|e| e.to_string());
                    let _ = tx.send(Msg::CommitsListed(pr, result));
                });
            }
            Cmd::MapOutdated { pr, head, items } => {
                let Some(git) = self.job_git(&pr) else {
                    return;
                };
                let tx = self.tx.clone();
                spawn_guarded(&self.tx, replies, async move {
                    let (repo, reader) = git;
                    let mut mapped = Vec::new();
                    for (thread, path, commit, line) in items {
                        let to =
                            diff_job::map_outdated(&repo, &reader, &head, &path, &commit, line)
                                .await;
                        mapped.push((thread, to));
                    }
                    let _ = tx.send(Msg::OutdatedMapped(pr, mapped));
                });
            }
            Cmd::LoadReview(pr) => {
                let reviews = self.reviews.clone();
                let tx = self.tx.clone();
                spawn_guarded(&self.tx, replies, async move {
                    let review = match reviews {
                        Ok(reviews) => {
                            let key = (pr.repo.owner.clone(), pr.repo.name.clone(), pr.number);
                            blocking(move || reviews.get(&key.0, &key.1, key.2))
                                .await
                                .map_err(|e| e.to_string())
                        }
                        Err(err) => Err(err),
                    };
                    let _ = tx.send(Msg::ReviewLoaded(pr, review));
                });
            }
            Cmd::SaveReview(pr, review) => {
                let _ = self.save_review.send((pr, review));
            }
            cmd => spawn(cmd, replies, &self.gh, &self.tx),
        }
    }

    fn job_git(
        &self,
        pr: &PrRef,
    ) -> Option<(
        Arc<ghtui_git::repo::Repo>,
        Arc<ghtui_git::blobs::BlobReader>,
    )> {
        self.jobs.get(pr).and_then(|(control, _)| control.git())
    }
}

fn spawn(cmd: Cmd, replies: Vec<Msg>, gh: &GitHub, tx: &mpsc::UnboundedSender<Msg>) {
    let gh = gh.clone();
    let tx = tx.clone();
    spawn_guarded(&tx.clone(), replies, async move {
        let msg = match cmd {
            Cmd::FetchViewer => Msg::Viewer(gh.viewer_login().await),
            Cmd::FetchInbox => Msg::Inbox(gh.inbox().await),
            Cmd::FetchPr(pr) => {
                let result = gh.pull_request(&pr).await;
                Msg::Pr(pr, Box::new(result))
            }
            Cmd::Fetch { key, cached } => {
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
                let result = fetch(&gh, &key).await;
                Msg::Fetched {
                    key,
                    result,
                    cached_at: None,
                }
            }
            Cmd::FetchMore { key, after } => {
                let result = match &key {
                    DataKey::Search(kind, query) => gh
                        .search(*kind, query, Some(after))
                        .await
                        .map(|r| Data::Search(Box::new(r))),
                    other => Err(ApiError::NotFound(format!("more of {other:?}"))),
                };
                Msg::FetchedMore(key, result)
            }
            Cmd::AddComment {
                subject_id,
                body,
                refresh,
            } => Msg::Commented(refresh, gh.add_comment(&subject_id, &body).await),
            Cmd::SetStarred { repo, id, starred } => Msg::Starred {
                result: gh.set_starred(&id, starred).await,
                repo,
                starred,
            },
            Cmd::Timer(timer, ms) => {
                tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
                let _ = tx.send(Msg::Timer(timer));
                return;
            }
            Cmd::SuggestLater(q) => {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                Msg::SuggestDue(q)
            }
            Cmd::Suggest(q) => {
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
            Cmd::SaveVisits(visits) => {
                gh.remember(ghtui_api::browse::keys::VISITS, visits).await;
                return;
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
            Cmd::FetchThreads(pr) => {
                let result = gh.review_threads(&pr).await;
                Msg::ThreadsLoaded(pr, result)
            }
            Cmd::FetchPatches(pr) => {
                let result = gh.pr_patches(&pr).await;
                Msg::PatchesLoaded(pr, result)
            }
            Cmd::Reply {
                pr,
                thread_id,
                body,
            } => {
                let result = gh.reply(&thread_id, &body).await;
                Msg::Replied(pr, result)
            }
            Cmd::SetResolved {
                pr,
                thread_id,
                resolved,
            } => {
                let result = gh.set_resolved(&thread_id, resolved).await;
                Msg::ResolvedSet {
                    pr,
                    thread_id,
                    resolved,
                    result,
                }
            }
            Cmd::SubmitReview {
                pr,
                head,
                drafts,
                event,
                body,
            } => {
                let outcome = submit_review(&gh, &pr, &head, drafts, event, &body).await;
                Msg::ReviewSubmitted(pr, outcome)
            }
            Cmd::FetchLastReview { pr, login } => {
                let result = gh.last_review_commit(&pr, &login).await;
                Msg::LastReview(pr, result)
            }
            cmd @ (Cmd::LoadDiff { .. }
            | Cmd::Prioritize(..)
            | Cmd::MapOutdated { .. }
            | Cmd::Edit { .. }
            | Cmd::Copy(_)
            | Cmd::DetectMoves(..)
            | Cmd::SinceReview { .. }
            | Cmd::ListCommits(..)
            | Cmd::LoadReview(_)
            | Cmd::SaveReview(..)) => {
                // Effects::run and the loop handle these; nothing to send.
                tracing::error!(?cmd, "not a GitHub command");
                return;
            }
        };
        let _ = tx.send(msg);
        let _ = tx.send(Msg::RateLimits(gh.rate_limits()));
        if let Some(rejected) = gh.token_rejected_change() {
            let text =
                "GitHub rejected the token: run `gh auth login` or set GH_TOKEN, then restart";
            let _ = tx.send(Msg::Problem(Problem::Auth, rejected.then(|| text.into())));
        }
    });
}

/// Spawns `task`. If it panics, the panic is logged and reported, and
/// `replies` (what it would have answered, as errors) go out instead, so no
/// screen waits forever.
fn spawn_guarded(
    tx: &mpsc::UnboundedSender<Msg>,
    replies: Vec<Msg>,
    task: impl Future<Output = ()> + Send + 'static,
) -> tokio::task::JoinHandle<()> {
    use futures::FutureExt;
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
    fn git<T>() -> Result<T, String> {
        Err("ghtui hit a bug".into())
    }
    let msg = match cmd {
        Cmd::FetchViewer => Msg::Viewer(api()),
        Cmd::FetchInbox => Msg::Inbox(api()),
        Cmd::FetchPr(pr) => Msg::Pr(pr.clone(), Box::new(api())),
        Cmd::Fetch { key, .. } => Msg::Fetched {
            key: key.clone(),
            result: api(),
            cached_at: None,
        },
        Cmd::FetchMore { key, .. } => Msg::FetchedMore(key.clone(), api()),
        Cmd::AddComment { refresh, .. } => Msg::Commented(refresh.clone(), api()),
        Cmd::SetStarred { repo, starred, .. } => Msg::Starred {
            repo: repo.clone(),
            starred: *starred,
            result: api(),
        },
        Cmd::Suggest(q) => Msg::Suggested(q.clone(), api()),
        Cmd::FetchViewed(pr) => Msg::ViewedLoaded(pr.clone(), Box::new(api())),
        Cmd::SetViewed {
            pr, file, previous, ..
        } => Msg::ViewedSaved {
            pr: pr.clone(),
            file: *file,
            previous: *previous,
            result: api(),
        },
        Cmd::FetchThreads(pr) => Msg::ThreadsLoaded(pr.clone(), api()),
        Cmd::FetchPatches(pr) => Msg::PatchesLoaded(pr.clone(), api()),
        Cmd::MapOutdated { pr, items, .. } => Msg::OutdatedMapped(
            pr.clone(),
            items.iter().map(|(t, ..)| (t.clone(), None)).collect(),
        ),
        Cmd::Reply { pr, .. } => Msg::Replied(pr.clone(), api()),
        Cmd::SetResolved {
            pr,
            thread_id,
            resolved,
        } => Msg::ResolvedSet {
            pr: pr.clone(),
            thread_id: thread_id.clone(),
            resolved: *resolved,
            result: api(),
        },
        Cmd::LoadReview(pr) => Msg::ReviewLoaded(pr.clone(), git()),
        Cmd::SubmitReview { pr, .. } => Msg::ReviewSubmitted(
            pr.clone(),
            SubmitOutcome {
                error: Some("Couldn't submit: ghtui hit a bug".into()),
                ..SubmitOutcome::default()
            },
        ),
        Cmd::FetchLastReview { pr, .. } => Msg::LastReview(pr.clone(), api()),
        Cmd::LoadDiff { pr, job, .. } => {
            Msg::Job(pr.clone(), *job, JobMsg::Failed("ghtui hit a bug".into()))
        }
        Cmd::DetectMoves(pr, job, _) => Msg::Job(pr.clone(), *job, JobMsg::Moves(Vec::new())),
        Cmd::SinceReview { pr, old_head } => Msg::SinceReady(pr.clone(), old_head.clone(), git()),
        Cmd::ListCommits(pr) => Msg::CommitsListed(pr.clone(), git()),
        // Nothing waits on these.
        Cmd::OpenUrl(_)
        | Cmd::Copy(_)
        | Cmd::Timer(..)
        | Cmd::SuggestLater(_)
        | Cmd::SaveVisits(_)
        | Cmd::Prioritize(..)
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
    let out = tx.clone();
    let writer = spawn_guarded(tx, vec![gone], async move {
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

/// What the cache has for a page, and when it was fetched.
fn cached_data(gh: &GitHub, key: &DataKey) -> Option<(Data, u64)> {
    use ghtui_api::browse::keys;
    fn at<T>(c: ghtui_store::Cached<T>, wrap: impl FnOnce(T) -> Data) -> (Data, u64) {
        (wrap(c.value), c.fetched_at)
    }
    Some(match key {
        DataKey::Repo(repo) => at(gh.cached(&keys::repo(repo))?, |v| Data::Repo(Box::new(v))),
        DataKey::Tree(repo, rev, path) => at(gh.cached(&keys::tree(repo, rev, path))?, Data::Tree),
        DataKey::Search(kind, query) => at(gh.cached(&keys::search(*kind, query))?, |v| {
            Data::Search(Box::new(v))
        }),
        DataKey::Issue(repo, number) => at(gh.cached(&keys::issue(repo, *number))?, |v| {
            Data::Issue(Some(Box::new(v)))
        }),
        DataKey::PrActivity(pr) => at(gh.cached(&keys::pr_activity(pr))?, |v| {
            Data::PrActivity(Box::new(v))
        }),
        DataKey::Profile(login) => at(gh.cached(&keys::profile(login))?, |v| {
            Data::Profile(Box::new(v))
        }),
        DataKey::ViewerRepos => at(gh.cached(keys::VIEWER_REPOS)?, Data::Repos),
        DataKey::Refs(repo) => at(gh.cached(&keys::refs(repo))?, |v| Data::Refs(Box::new(v))),
        DataKey::LastCommits(repo, rev, path) => {
            at(gh.cached(&keys::last_commits(repo, rev, path))?, |v| {
                Data::LastCommits(std::sync::Arc::new(v))
            })
        }
        // Files aren't cached; they can be large.
        DataKey::Blob(..) | DataKey::Files(..) => return None,
    })
}

async fn fetch(gh: &GitHub, key: &DataKey) -> Result<Data, ApiError> {
    Ok(match key {
        DataKey::Repo(repo) => Data::Repo(Box::new(gh.repo(repo).await?)),
        DataKey::Tree(repo, rev, path) => Data::Tree(gh.tree(repo, rev, path).await?),
        DataKey::Blob(repo, rev, path) => Data::Blob(Box::new(gh.blob(repo, rev, path).await?)),
        DataKey::Search(kind, query) => {
            Data::Search(Box::new(gh.search(*kind, query, None).await?))
        }
        DataKey::Issue(repo, number) => Data::Issue(gh.issue(repo, *number).await?.map(Box::new)),
        DataKey::PrActivity(pr) => Data::PrActivity(Box::new(gh.pr_activity(pr).await?)),
        DataKey::Profile(login) => Data::Profile(Box::new(gh.profile(login).await?)),
        DataKey::ViewerRepos => Data::Repos(gh.viewer_repos().await?),
        DataKey::Files(repo, rev) => {
            let (files, truncated) = gh.file_list(repo, rev).await?;
            Data::Files(std::sync::Arc::new(files), truncated)
        }
        DataKey::Refs(repo) => Data::Refs(Box::new(gh.refs(repo).await?)),
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

/// Submits a review: reuses the viewer's pending review on GitHub (or
/// starts one on `head`), adds each draft as a thread, and submits only if
/// GitHub accepted every draft. Each draft's fate is reported, so rejected
/// ones keep their text.
async fn submit_review(
    gh: &GitHub,
    pr: &PrRef,
    head: &str,
    drafts: Vec<DraftComment>,
    event: ReviewEvent,
    body: &str,
) -> SubmitOutcome {
    let mut outcome = SubmitOutcome::default();
    let review_id = match gh.pending_review(pr).await {
        Ok((_, Some(existing))) => existing,
        Ok((pr_id, None)) => match gh.start_review(&pr_id, head).await {
            Ok(id) => id,
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
    for draft in drafts {
        match gh
            .add_review_thread(&review_id, &review::new_thread(&draft))
            .await
        {
            Ok(_) => outcome.accepted.push(draft.id),
            Err(err) => outcome.rejected.push((draft.id, api_message(&err))),
        }
    }
    if !outcome.rejected.is_empty() {
        return outcome;
    }
    match gh.submit_review(&review_id, event, body).await {
        Ok(()) => outcome.submitted = true,
        Err(err) => outcome.error = Some(format!("Couldn't submit: {}", api_message(&err))),
    }
    outcome
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
) -> (EventStream, Result<String, String>) {
    use crossterm::ExecutableCommand;
    // Stop reading keys so the editor gets them.
    drop(events);
    let result = async {
        let file = draft_file(text).map_err(|e| e.to_string())?;
        let path = file.path().to_owned();
        crate::set_mouse(false);
        ratatui::restore();
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
            Ok(status) if status.success() => {
                std::fs::read_to_string(&path).map_err(|e| e.to_string())
            }
            Ok(status) => Err(format!("{editor} exited with {status}")),
            Err(err) => Err(format!("couldn't start {editor}: {err}")),
        }
    }
    .await;
    (EventStream::new(), result)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A panicking task still answers: a notice, then the failure reply
    /// the screen is waiting for.
    #[tokio::test]
    async fn panicking_task_sends_its_replies() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let pr = PrRef::parse("o/r#1").unwrap();
        let cmd = Cmd::FetchThreads(pr.clone());
        let handle = spawn_guarded(&tx, panic_replies(&cmd), async { panic!("boom") });
        handle.await.unwrap();
        assert!(matches!(
            rx.recv().await,
            Some(Msg::Notice(Notice::Error(e))) if e.contains("boom")
        ));
        assert!(matches!(
            rx.recv().await,
            Some(Msg::ThreadsLoaded(p, Err(ApiError::Internal(_)))) if p == pr
        ));
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
        let handle = spawn_guarded(&tx, Vec::new(), async {
            blocking(|| panic!("deep")).await;
        });
        handle.await.unwrap();
        assert!(matches!(
            rx.recv().await,
            Some(Msg::Notice(Notice::Error(e))) if e.contains("deep")
        ));
    }
}
