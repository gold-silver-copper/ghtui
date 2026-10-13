use super::*;
use crate::api::browse::SearchResults;
use crate::api::browse::{IssueState, PrState, Status};
use crate::api::model::RepoId;
use crate::fixtures::{answer, diff_msg, fetched, press, says, thread, warns};
use crate::picker::{Choice, fuzzy_score};
use crate::route::{Dest, OPEN};
use crate::theme::{ColorDepth, DEFAULT_SEED, Mode};
use crate::ui::annotations::{AnnotationKey, Place};
use crate::ui::overlays::PALETTE_ROWS;
use crate::ui::pages::PrTab;

/// The overlay showing, which must be a `$kind`.
macro_rules! overlay {
    ($state:expr, $kind:ident) => {
        match &$state.overlay {
            Some(Overlay::$kind(overlay)) => overlay,
            _ => panic!("no {} overlay", stringify!($kind)),
        }
    };
}

pub(crate) fn state() -> State {
    let mut state = State::new(
        Theme::new(DEFAULT_SEED, Mode::Dark, ColorDepth::TrueColor),
        Icons::default(),
        Keymap::default(),
        (100, 30),
    );
    // Never written: changes to Home's sections are commands.
    state.config_path = Some("config.toml".into());
    state
}

/// Your `n` pull requests of `total`, as Home's second section finds them.
fn mine(n: u64, total: u64) -> SearchResults {
    let row = |n| crate::fixtures::found_pr(&format!("o/r#{n}"), &format!("PR {n}"), PrState::Open);
    crate::fixtures::found_prs((1..=n).map(row).collect(), total)
}

/// Home's second section: your pull requests.
fn mine_key(state: &State) -> DataKey {
    state.home[1].search.clone().unwrap().key()
}

/// Home's second section's fetch failed.
fn mine_failed(state: &mut State, err: ApiError) {
    let key = mine_key(state);
    update(
        state,
        Msg::Fetched {
            key,
            result: Err(err),
            cached_at: None,
        },
    );
}

fn with_home(n: u64) -> State {
    let mut state = state();
    crate::fixtures::section(&mut state, 1, mine(n, n));
    state
}

/// Runs an action that has no default key (it's in the menu).
fn act(state: &mut State, action: Action) -> Vec<Cmd> {
    let mut cmds = apply(state, action);
    state.sync_page();
    cmds.extend(state.settle_diff());
    cmds
}

fn page(state: &State) -> &PageScreen {
    match state.screen() {
        Screen::Page(p) => p,
        Screen::Diff(_) => panic!("not on a page"),
    }
}

fn route(state: &State) -> Route {
    page(state).route.clone()
}

/// The text of the selected row's first line.
fn selected_text(state: &State) -> String {
    let p = page(state);
    let item = p.page().items[p.selected.expect("a selection")];
    p.page().lines[item.start].text()
}

/// Fetching `key`, the cached copy first.
fn fetch(key: DataKey) -> Cmd {
    Cmd::Api(Api::Fetch { key, cached: true })
}

/// Commands other than saving visits.
#[must_use]
fn fetches(cmds: Vec<Cmd>) -> Vec<Cmd> {
    cmds.into_iter()
        .filter(|c| !matches!(c, Cmd::Api(Api::SaveVisits(_))))
        .collect()
}

fn repo() -> RepoId {
    RepoId::new("gold-silver-copper", "ghtui")
}

fn issues(query: &str) -> Route {
    let (repo, query) = (repo(), query.to_owned());
    Route::Issues { repo, query }
}

fn issue(number: u64) -> Route {
    let repo = repo();
    Route::Issue { repo, number }
}

fn blob(rev: &str, path: &str) -> Route {
    Route::blob(repo(), rev.to_owned(), path.to_owned())
}

fn with_repo() -> State {
    let mut state = state();
    let _ = state.push(Route::Repo(repo()));
    fetched(
        &mut state,
        DataKey::Repo(repo()),
        Data::Repo(Box::new(crate::fixtures::overview())),
    );
    let readme = Some(Box::new(crate::fixtures::readme()));
    fetched(&mut state, DataKey::Readme(repo()), Data::Readme(readme));
    state
}

#[must_use]
fn click(state: &mut State, x: u16, y: u16) -> Vec<Cmd> {
    update(
        state,
        Msg::Mouse(MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        }),
    )
}

/// Where `text` is drawn on the page.
fn find(state: &State, text: &str) -> (u16, u16) {
    let p = page(state);
    let area = state.page_area();
    let cols = crate::ui::page::columns(p.page(), area);
    for (row, line) in p.page().lines.iter().enumerate().skip(p.scroll) {
        let at = line.text().find(text);
        if let Some(at) = at {
            let x = cols.main_x + 2 + line.indent + u16::try_from(at).unwrap();
            return (x, area.y + u16::try_from(row - p.scroll).unwrap());
        }
    }
    panic!("{text} isn't on screen")
}

mod tabs;

pub(super) mod diff;

mod actions;
mod fetch;
mod home;
mod jobs;
mod keys;
mod links;
mod lists;
mod offline;
mod pr_diff;
mod repo;
mod search;
mod watch;
