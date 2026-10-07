//! Random sequences of what can happen (keys, resizes, navigation, data
//! arriving fresh, cached, failed, twice, late, a next page) driven through
//! `update`, checking after every step that:
//! - nothing panics, and the screen renders at its size, however small;
//! - the selection is on an item, or there's none;
//! - no list holds the same item twice (a next page that overlaps the one
//!   before, as when the list moved between them);
//! - a cached copy never replaces data fetched since.
//!
//! gh-dash's history is full of these: crashes on empty or missing data,
//! duplicate rows after a quick refresh, the wrong row acted on.

use std::collections::{HashMap, HashSet};

use proptest::prelude::*;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use ghtui_api::ApiError;
use ghtui_api::browse::{BranchInfo, IssueState, Results, SearchKind, SearchResults};
use ghtui_api::model::RepoId;
use ghtui_theme::{ColorDepth, DEFAULT_SEED, Mode, Theme};
use ghtui_ui::Icons;

use crate::browse::{Data, DataKey};
use crate::fixtures::{self, press};
use crate::keymap::Keymap;
use crate::route::Route;
use crate::state::{Msg, Screen, State, update};

/// Totals at or above this mark fresh data; below, cached copies.
const FRESH: u64 = 1_000_000;

fn repo() -> RepoId {
    RepoId::new("o", "r")
}

/// The list pages the sequences visit.
fn routes() -> [Route; 3] {
    [
        Route::Search {
            kind: SearchKind::Issues,
            query: "repo:o/r".into(),
        },
        Route::Branches(repo()),
        Route::Repo(repo()),
    ]
}

fn key_of(route: &Route) -> DataKey {
    match route {
        Route::Search { kind, query } => DataKey::Search(*kind, query.clone()),
        Route::Branches(repo) => DataKey::Branches(repo.clone()),
        _ => DataKey::Repo(repo()),
    }
}

/// A page of `route`'s list: items `start..start + len`, with `total`
/// (the version marker) and a next page if `next`.
fn page(route: &Route, start: u64, len: u8, total: u64, next: bool) -> Data {
    let next = next.then(|| format!("c{start}"));
    match route {
        Route::Search { .. } => {
            let items = (start..start + u64::from(len))
                .map(|n| fixtures::issue_summary(n + 1, n % 3 == 0, IssueState::Open))
                .collect();
            Data::Search(Box::new(SearchResults::Issues(Results {
                total,
                items,
                next,
            })))
        }
        Route::Branches(_) => {
            let base = fixtures::branches().items.into_iter().next();
            let items = (start..start + u64::from(len))
                .filter_map(|n| {
                    base.clone().map(|b| BranchInfo {
                        name: format!("branch-{n:03}"),
                        ..b
                    })
                })
                .collect();
            Data::Branches(Box::new(Results { total, items, next }))
        }
        _ => {
            let mut overview = fixtures::overview();
            overview.commits = total;
            Data::Repo(Box::new(overview))
        }
    }
}

/// A list's items' identities, to find doubles.
fn identities(data: &Data) -> Vec<String> {
    match data {
        Data::Search(results) => match &**results {
            SearchResults::Issues(r) => r
                .items
                .iter()
                .map(|i| format!("{}#{}", i.repo, i.number))
                .collect(),
            _ => Vec::new(),
        },
        Data::Branches(r) => r.items.iter().map(|b| b.name.clone()).collect(),
        _ => Vec::new(),
    }
}

/// The version marker a list or page carries.
fn version(data: &Data) -> Option<u64> {
    match data {
        Data::Search(results) => match &**results {
            SearchResults::Issues(r) => Some(r.total),
            _ => None,
        },
        Data::Branches(r) => Some(r.total),
        Data::Repo(o) => Some(o.commits),
        _ => None,
    }
}

#[derive(Debug, Clone)]
enum Event {
    Keys(&'static str),
    Resize(u16, u16),
    Go(usize),
    Back,
    Refresh,
    /// Data for route `route`, fresh or a cached copy.
    Deliver {
        route: usize,
        cached: bool,
        start: u64,
        len: u8,
        next: bool,
    },
    /// The next page, overlapping the last by `overlap` items.
    More {
        route: usize,
        overlap: u8,
        len: u8,
        next: bool,
    },
    Fail(usize),
}

const KEYS: &[&str] = &[
    "j",
    "k",
    "g",
    "G",
    "<Enter>",
    "<Esc>",
    "<Tab>",
    "<S-Tab>",
    "<C-d>",
    "<C-u>",
    "]",
    "[",
    "1",
    "2",
    "3",
    "/",
    "n",
    "?",
    "<BS>",
    "x",
    "y",
    "<Down>",
    "<Up>",
    "<PageDown>",
    "h",
    "l",
    "T",
    "<C-w>",
    "<Right>",
    "<Left>",
];

fn event() -> impl Strategy<Value = Event> {
    prop_oneof![
        4 => proptest::sample::select(KEYS).prop_map(Event::Keys),
        1 => prop_oneof![
            Just((1u16, 1u16)),
            Just((20, 5)),
            Just((80, 24)),
            (1u16..200, 1u16..60),
        ]
        .prop_map(|(w, h)| Event::Resize(w, h)),
        2 => (0usize..3).prop_map(Event::Go),
        1 => Just(Event::Back),
        1 => Just(Event::Refresh),
        3 => (0usize..3, any::<bool>(), 0u64..50, 0u8..40, any::<bool>()).prop_map(
            |(route, cached, start, len, next)| Event::Deliver {
                route,
                cached,
                start,
                len,
                next,
            }
        ),
        2 => (0usize..3, 0u8..5, 0u8..40, any::<bool>())
            .prop_map(|(route, overlap, len, next)| Event::More { route, overlap, len, next }),
        1 => (0usize..3).prop_map(Event::Fail),
    ]
}

/// What the sequence did, to check against.
#[derive(Default)]
struct Model {
    counter: u64,
    /// Keys fresh data arrived for.
    fresh: HashSet<DataKey>,
    /// Where each list's last page ended.
    ends: HashMap<DataKey, u64>,
}

fn apply(state: &mut State, model: &mut Model, event: &Event) {
    let routes = routes();
    model.counter += 1;
    match event {
        Event::Keys(keys) => {
            press(state, keys);
        }
        Event::Resize(w, h) => {
            update(state, Msg::Resize(*w, *h));
        }
        Event::Go(i) => {
            let _ = state.push(routes[*i].clone());
        }
        Event::Back => {
            let _ = state.back();
        }
        Event::Refresh => {
            let _ = state.load_visible(true);
        }
        Event::Deliver {
            route,
            cached,
            start,
            len,
            next,
        } => {
            let route = &routes[*route];
            let key = key_of(route);
            let total = if *cached {
                model.counter
            } else {
                FRESH + model.counter
            };
            let data = page(route, *start, *len, total, *next);
            if !cached {
                model.fresh.insert(key.clone());
                model.ends.insert(key.clone(), start + u64::from(*len));
            }
            let cached_at = cached.then_some(1);
            update(
                state,
                Msg::Fetched {
                    key,
                    result: Ok(data),
                    cached_at,
                },
            );
        }
        Event::More {
            route,
            overlap,
            len,
            next,
        } => {
            let route = &routes[*route];
            let key = key_of(route);
            // Asking for more first, as scrolling to the end does.
            if let Some(remote) = state.data.get_mut(&key)
                && remote
                    .data
                    .as_ref()
                    .is_some_and(|d| d.next_cursor().is_some())
            {
                remote.loading_more = true;
            }
            let end = model.ends.get(&key).copied().unwrap_or(0);
            let start = end.saturating_sub(u64::from(*overlap));
            let data = page(route, start, *len, FRESH + model.counter, *next);
            model.ends.insert(key.clone(), start + u64::from(*len));
            update(state, Msg::FetchedMore(key, Ok(data)));
        }
        Event::Fail(route) => {
            let key = key_of(&routes[*route]);
            update(
                state,
                Msg::Fetched {
                    key,
                    result: Err(ApiError::Network("down".into())),
                    cached_at: None,
                },
            );
        }
    }
}

fn fail(e: impl std::fmt::Display) -> TestCaseError {
    TestCaseError::fail(e.to_string())
}

fn check(state: &State, model: &Model) -> Result<(), TestCaseError> {
    // Renders at its size.
    let (w, h) = state.size;
    let mut terminal = Terminal::new(TestBackend::new(w, h)).map_err(fail)?;
    terminal
        .draw(|frame| crate::view::view(state, frame, 0))
        .map_err(fail)?;
    // The selection is on an item.
    if let Screen::Page(p) = state.screen() {
        prop_assert!(
            p.selected.is_none_or(|s| s < p.page.items.len()),
            "selected {:?} of {} items",
            p.selected,
            p.page.items.len()
        );
    }
    for (key, remote) in &state.data {
        let Some(data) = &remote.data else { continue };
        // No item twice.
        let ids = identities(data);
        let unique: HashSet<&String> = ids.iter().collect();
        prop_assert_eq!(
            unique.len(),
            ids.len(),
            "{:?} holds an item twice: {:?}",
            key,
            ids
        );
        // A cached copy never replaces fresh data.
        if model.fresh.contains(key) {
            let v = version(data);
            prop_assert!(
                v.is_none_or(|v| v >= FRESH),
                "{:?}: a cached copy ({:?}) over fresh data",
                key,
                v
            );
        }
    }
    Ok(())
}

fn new_state() -> State {
    let mut state = State::new(
        Theme::new(DEFAULT_SEED, Mode::Dark, ColorDepth::TrueColor),
        Icons::default(),
        Keymap::default(),
        (100, 30),
    );
    state.viewer = Some("octocat".into());
    state.clock = || 1_700_000_000;
    state
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn any_sequence_of_events_keeps_the_app_sound(events in proptest::collection::vec(event(), 1..60)) {
        let mut state = new_state();
        let mut model = Model::default();
        for event in &events {
            apply(&mut state, &mut model, event);
            check(&state, &model)?;
        }
    }
}
