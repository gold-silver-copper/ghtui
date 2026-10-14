//! Behaviour through `update`: keys pressed, data delivered, and the
//! commands, pages and status line that follow. No I/O and no runtime.

use super::*;
use crate::api::browse::{IssueKind, IssueState, PrState, SearchKind, SearchResults, Status};
use crate::api::change::Change;
use crate::api::model::{ChecksState, PrDetail, RepoId, ReviewDecision};
use crate::api::rate_limit::{Bucket, RateLimits};
use crate::fixtures;
use crate::fixtures::{answer, diff_msg, fetched, pr_detail, press, says, thread, warns};
use crate::picker::{Choice, fuzzy_score};
use crate::route::{Dest, Family, OPEN, issues};
use crate::theme::{ColorDepth, DEFAULT_SEED, Mode, Theme};
use crate::ui::annotations::{AnnotationKey, Place};
use crate::ui::overlays::PALETTE_ROWS;
use crate::ui::pages::{PrTab, ProfileTab};

/// The overlay showing, which must be a `$kind`.
macro_rules! overlay {
    ($state:expr, $kind:ident) => {
        match &$state.overlay {
            Some(Overlay::$kind(overlay)) => overlay,
            _ => panic!("no {} overlay", stringify!($kind)),
        }
    };
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
            source: None,
        },
    );
}

/// Home with `n` of your pull requests and nothing else.
fn with_prs(n: u64) -> State {
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

fn issues(query: &str) -> Route {
    let (repo, query) = (ghtui(), query.to_owned());
    Route::Issues(issues::Issues { repo, query })
}

fn issue(number: u64) -> Route {
    let repo = ghtui();
    Route::Issues(issues::Issue { repo, number })
}

fn blob(rev: &str, path: &str) -> Route {
    Route::blob(ghtui(), rev.to_owned(), path.to_owned())
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

/// 2026-10-03T12:00:00Z
pub(crate) const NOW: u64 = 1_791_028_800;

/// The whole page's text, a line each, on screen or not.
pub(crate) fn page_text(state: &State) -> String {
    let lines = page(state).page().lines.iter();
    let lines = lines.map(crate::ui::page::PageLine::text);
    lines.collect::<Vec<_>>().join("\n")
}

/// The screen as drawn at the state's size.
pub(crate) fn draw(state: &State) -> ratatui::buffer::Buffer {
    let (w, h) = state.size;
    let backend = ratatui::backend::TestBackend::new(w, h);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| crate::view::view(state, frame, NOW))
        .unwrap();
    terminal.backend().buffer().clone()
}

/// What's on screen, as plain text: a line per row, wide characters
/// once.
pub(crate) fn screen_text(state: &State) -> String {
    let buf = draw(state);
    let area = buf.area;
    let row = |y| {
        let mut skip = 0;
        let mut text = String::new();
        for x in area.left()..area.right() {
            let symbol = buf[(x, y)].symbol();
            if skip == 0 {
                text.push_str(symbol);
                skip = crate::ui::text::width(symbol).saturating_sub(1);
            } else {
                skip -= 1;
            }
        }
        text
    };
    let rows = (area.top()..area.bottom()).map(row);
    rows.collect::<Vec<_>>().join("\n")
}

mod screens;
use diff::{commit_of, fixture_diff, screen_state};
pub(crate) use screens::*;

mod changes;
mod code_nav;
mod create;
mod discussions;
mod edits;
mod failures;
mod features;
mod housekeeping;
mod manage;
mod merging;
mod pages;
mod projects;
mod releases;
mod reviews;
mod security;
mod settings;
mod site;
mod smoke;
mod writes;

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
mod rebuilds;
mod repo;
mod search;
mod watch;
