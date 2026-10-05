//! `ghtui`: a keyboard-driven terminal client for GitHub.

mod browse;
mod chrome;
mod config;
mod diff_job;
mod diff_screen;
#[cfg(test)]
mod fixtures;
mod keymap;
mod nav;
mod picker;
mod review;
mod route;
mod runtime;
#[cfg(test)]
mod snapshot_tests;
mod state;
mod view;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use ghtui_api::GitHub;
use ghtui_api::model::{PrRef, RepoId};
use ghtui_store::Store;
use ghtui_theme::{ColorDepth, Mode, Theme};
use ghtui_ui::Icons;

use crate::config::{Config, DepthSetting, ModeSetting};
use crate::diff_job::GitContext;
use crate::keymap::Keymap;
use crate::route::{Route, Target};
use crate::state::{Remote, State};

/// How long to wait for the terminal to report its background color.
const BACKGROUND_QUERY_TIMEOUT: Duration = Duration::from_millis(100);

#[derive(Parser)]
#[command(
    version,
    about = "A keyboard-driven terminal client for GitHub",
    args_conflicts_with_subcommands = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// What to open: owner/repo, owner/repo#123, @user, a github.com URL, or
    /// #123 inside a clone. Without it, ghtui opens your home page.
    target: Option<String>,
    /// Config file (default: ~/.config/ghtui/config.toml).
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    /// Color scheme; overrides the config file.
    #[arg(long, global = true, value_enum)]
    theme: Option<ModeSetting>,
}

#[derive(Subcommand)]
enum Command {
    /// Open a pull request.
    Pr {
        /// `owner/repo#123`, a pull request URL, or `123` inside a clone.
        target: String,
    },
}

fn main() -> std::process::ExitCode {
    // git runs this binary as its GIT_ASKPASS helper for the cache clone.
    if std::env::var_os(ghtui_git::credentials::ASKPASS_FLAG).is_some() {
        return askpass();
    }
    let started = Instant::now();
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("ghtui: cannot start the async runtime: {err}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let result = runtime.block_on(run(started));
    // Don't wait on background work (a big diff, a stalled fetch) to exit.
    runtime.shutdown_timeout(Duration::from_millis(100));
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("ghtui: {err:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

async fn run(started: Instant) -> Result<()> {
    let cli = Cli::parse();

    // Token resolution may run `gh` (~40ms); overlap it with the rest of
    // startup.
    let token = std::thread::spawn(ghtui_api::auth::resolve_token);

    let config_path = cli.config.clone().or_else(config::default_config_path);
    let config = match &config_path {
        Some(path) => Config::load(path)?,
        None => Config::default(),
    };
    let keymap = Keymap::with_overrides(&config.keys).map_err(anyhow::Error::msg)?;
    let cache_dir = config::cache_dir().context("cannot determine a cache directory")?;
    let _log_guard = init_logging(&cache_dir);
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "starting");

    let target = match (&cli.command, &cli.target) {
        (Some(Command::Pr { target }), _) => Some(Target::Page(Route::Pr {
            pr: resolve_target(target).await?,
            tab: ghtui_ui::pages::PrTab::Conversation,
        })),
        (None, Some(target)) => Some(resolve_open(target).await?),
        (None, None) => None,
    };

    let theme = build_theme(&config, cli.theme)?;
    let icons = Icons {
        nerd_font: config.ui.nerd_font,
    };
    let store = Store::open(&cache_dir.join("cache.redb"));
    let reviews = open_reviews(&store);

    let (token, source) = token
        .join()
        .map_err(|_| anyhow::anyhow!("token resolution panicked"))??;
    tracing::info!(%source, "authenticated");
    let git = GitContext {
        cache_root: cache_dir.clone(),
        credentials: git_credentials(&token, source),
        cwd: std::env::current_dir()?,
    };
    let gh = GitHub::new(token, store);

    let size = crossterm::terminal::size().unwrap_or((80, 24));
    let mut state = State::new(theme, icons, keymap, size);
    if let Err(err) = &reviews {
        state.problems.insert(
            state::Problem::Drafts,
            format!("Drafts are not being saved: {err}"),
        );
    }
    state.inbox = Remote::cached(gh.cached_inbox());
    let mut cmds = match target {
        Some(target) => {
            if let Target::Page(Route::Pr { pr, .. }) | Target::Files(pr) = &target {
                state
                    .prs
                    .insert(pr.clone(), Remote::cached(gh.cached_pull_request(pr)));
            }
            // The home page stays underneath (Esc goes there) and loads
            // when you get there.
            let mut cmds = vec![state::Cmd::FetchViewer];
            cmds.extend(state.go(target));
            cmds
        }
        None => state.load_visible(true),
    };
    state.visits = gh
        .cached::<Vec<nav::Visit>>(ghtui_api::browse::keys::VISITS)
        .map(|c| c.value)
        .unwrap_or_default();
    cmds.push(state::Cmd::Timer(state::Timer::Minute, 60_000));
    state.data_gen += 1;
    state.sync_page();

    install_panic_hook();
    let mut terminal = init_terminal().context("needs an interactive terminal")?;
    set_mouse(true);
    let result = runtime::run(&mut terminal, state, gh, git, reviews, cmds, started).await;
    set_mouse(false);
    ratatui::restore();
    if let Err(err) = &result {
        tracing::error!("{err:#}");
    }
    result
}

/// Review drafts' home, with any left in the cache by older versions
/// moved in.
fn open_reviews(store: &Store) -> Result<ghtui_store::Reviews, String> {
    let dir = config::data_dir().ok_or("cannot determine a data directory")?;
    let reviews = ghtui_store::Reviews::open(&dir.join("reviews")).map_err(|e| e.to_string())?;
    match store.migrate_reviews(&reviews) {
        Ok(0) => {}
        Ok(n) => tracing::info!(n, "moved review state out of the cache"),
        Err(err) => tracing::warn!(%err, "moving review state out of the cache failed"),
    }
    Ok(reviews)
}

/// How git authenticates for the cache clone: through `gh` when that's
/// where the token came from, otherwise through this binary as askpass.
fn git_credentials(
    token: &ghtui_api::auth::Token,
    source: ghtui_api::auth::TokenSource,
) -> ghtui_git::credentials::Credentials {
    use ghtui_git::credentials::Credentials;
    match (source, std::env::current_exe()) {
        (ghtui_api::auth::TokenSource::GhCli, _) => Credentials::GhHelper,
        (_, Ok(program)) => Credentials::AskPass {
            program,
            token: token.expose().to_owned(),
        },
        (_, Err(err)) => {
            tracing::warn!(%err, "cannot locate ghtui for GIT_ASKPASS; using git's own credentials");
            Credentials::Ambient
        }
    }
}

/// `GIT_ASKPASS` mode: answer git's prompt from the environment ghtui set
/// on the git process, then exit.
fn askpass() -> std::process::ExitCode {
    use ghtui_git::credentials::{ASKPASS_TOKEN, askpass_answer};
    let prompt = std::env::args().nth(1).unwrap_or_default();
    match std::env::var(ASKPASS_TOKEN) {
        Ok(token) if !token.is_empty() => {
            println!("{}", askpass_answer(&prompt, &token));
            std::process::ExitCode::SUCCESS
        }
        _ => std::process::ExitCode::FAILURE,
    }
}

/// `owner/repo#N`, a URL, or a bare number resolved against this clone's
/// GitHub remote (`upstream` first, for forks).
async fn resolve_target(target: &str) -> Result<PrRef> {
    if let Some(pr) = PrRef::parse(target) {
        return Ok(pr);
    }
    let Ok(number) = target.trim_start_matches('#').parse::<u64>() else {
        bail!(
            "`{target}` is not a pull request; use owner/repo#123, a PR URL, or a number inside a clone"
        );
    };
    let cwd = std::env::current_dir()?;
    let remotes = ghtui_git::remotes(&cwd).await?;
    let repo = ghtui_git::infer_github_repo(&remotes)
        .context("not inside a clone with a github.com remote; use owner/repo#123")?;
    Ok(PrRef {
        repo: RepoId::new(repo.owner, repo.name),
        number,
    })
}

/// The page to open from the command line.
async fn resolve_open(target: &str) -> Result<Target> {
    let number = target.trim_start_matches('#');
    if !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()) {
        let pr = resolve_target(number).await?;
        return Ok(Target::Page(Route::Issue {
            repo: pr.repo,
            number: pr.number,
        }));
    }
    match route::parse_input(target, None) {
        Some(Target::External(_)) | None => bail!(
            "can't open `{target}`; use owner/repo, owner/repo#123, @user, or a github.com URL"
        ),
        Some(target) => Ok(target),
    }
}

fn build_theme(config: &Config, cli_mode: Option<ModeSetting>) -> Result<Theme> {
    let seed = config.seed()?;
    let depth = match config.theme.color_depth {
        DepthSetting::Auto => ghtui_theme::detect_color_depth(|k| std::env::var(k).ok()),
        DepthSetting::TrueColor => ColorDepth::TrueColor,
        DepthSetting::Ansi256 => ColorDepth::Ansi256,
    };
    let mode = match cli_mode.unwrap_or(config.theme.mode) {
        ModeSetting::Light => Mode::Light,
        ModeSetting::Dark => Mode::Dark,
        // Must happen before the alternate screen and raw mode.
        ModeSetting::Auto => ghtui_theme::detect_background(BACKGROUND_QUERY_TIMEOUT)
            .map(ghtui_theme::mode_for_background)
            .unwrap_or(Mode::Dark),
    };
    tracing::info!(?mode, ?depth, %seed, "theme");
    Ok(Theme::new(seed, mode, depth))
}

/// Logs to `<cache>/ghtui.log`, never to the terminal. Level from
/// `GHTUI_LOG` (default `info`).
fn init_logging(
    cache_dir: &std::path::Path,
) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use tracing_subscriber::EnvFilter;
    std::fs::create_dir_all(cache_dir).ok()?;
    let appender = tracing_appender::rolling::never(cache_dir, "ghtui.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter = EnvFilter::try_from_env("GHTUI_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,hyper=warn,hyper_util=warn,octocrab=warn"));
    tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_env_filter(filter)
        .try_init()
        .ok()?;
    Some(guard)
}

/// Clicks and the wheel go to ghtui while it runs.
pub fn set_mouse(on: bool) {
    use crossterm::ExecutableCommand;
    use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
    let mut out = std::io::stdout();
    let result = if on {
        out.execute(EnableMouseCapture).map(drop)
    } else {
        out.execute(DisableMouseCapture).map(drop)
    };
    if let Err(err) = result {
        tracing::warn!(%err, "mouse capture");
    }
}

/// Raw mode and the alternate screen, like `ratatui::init` but without its
/// panic hook (see [`install_panic_hook`]).
fn init_terminal() -> std::io::Result<ratatui::DefaultTerminal> {
    use crossterm::ExecutableCommand;
    crossterm::terminal::enable_raw_mode()?;
    let result = std::io::stdout()
        .execute(crossterm::terminal::EnterAlternateScreen)
        .and_then(|_| {
            ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(std::io::stdout()))
        });
    if result.is_err() {
        ratatui::restore();
    }
    result
}

/// Logs every panic. Only a panic on the main thread (the UI loop) ends
/// ghtui, so only that one restores the terminal and prints. Background
/// tasks catch their own panics (`runtime::spawn_guarded`) and keep the
/// session running; printing those would scribble over the screen.
fn install_panic_hook() {
    let main = std::thread::current().id();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("panic: {info}");
        if std::thread::current().id() == main {
            set_mouse(false);
            ratatui::restore();
            previous(info);
        }
    }));
}
