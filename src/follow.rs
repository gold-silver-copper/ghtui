//! Following GitHub until the page shows a change it accepted. GitHub
//! can take a while (a cancel, a minute or more): while the page the
//! change went to is on screen, what shows it is fetched again at the
//! live refresh's pace, until the page shows it or [`POLLS`] go by.

use std::any::Any;

use crate::data::DataKey;
use crate::keymap::Action;
use crate::route::Route;
use crate::state::{Cmds, State};

/// What a page shows once GitHub has a change.
pub trait Expectation: Any + std::fmt::Debug + Send + Sync {
    /// What's fetched again to see it: the page's needs.
    fn needs(&self, state: &State, route: &Route) -> Vec<DataKey> {
        state.needs(route)
    }

    /// What the status bar says meanwhile.
    fn waiting(&self) -> &'static str;

    /// Whether `route`'s page, as loaded, shows it.
    fn shown(&self, state: &State, route: &Route) -> Shown;

    /// Once it's shown: what follows.
    fn then(&self, _state: &mut State, _route: &Route) -> Cmds {
        Cmds::new()
    }
}

/// Whether the page shows a change.
pub enum Shown {
    No,
    Yes,
    /// It's on another page, followed there: a run's latest attempt, a
    /// job's re-run.
    There(Route, Box<dyn Expectation>),
}

impl From<bool> for Shown {
    fn from(shown: bool) -> Self {
        if shown { Shown::Yes } else { Shown::No }
    }
}

/// A change GitHub accepted that the page it went to doesn't show yet.
#[derive(Debug)]
pub struct Following {
    route: Route,
    what: Box<dyn Expectation>,
    polls: u32,
}

/// How many times a page is fetched again for a change before giving up:
/// a few minutes, at the live refresh's pace.
const POLLS: u32 = 40;

/// Waits, on `route`, for GitHub to show `what`.
pub fn expect(state: &mut State, route: Route, what: impl Expectation) {
    state.following = Some(Following {
        route,
        what: Box::new(what),
        polls: 0,
    });
}

/// Opens `route` in place of the page on screen, fetched again, to wait
/// there for GitHub to show `what`.
pub fn replace(state: &mut State, route: Route, what: impl Expectation) -> Cmds {
    expect(state, route.clone(), what);
    state.replace(route, true, None)
}

/// What the page on screen waits for GitHub to show, if anything.
fn following(state: &State) -> Option<&Following> {
    (state.following.as_ref()).filter(|f| state.route() == Some(&f.route))
}

/// What the page on screen waits for GitHub to show, if it's a `T`.
pub fn here<T: Expectation>(state: &State) -> Option<&T> {
    (following(state)?.what.as_ref() as &dyn Any).downcast_ref()
}

/// What the status bar says while GitHub hasn't shown a change.
pub fn waiting(state: &State) -> Option<&'static str> {
    following(state).map(|f| f.what.waiting())
}

/// What the live refresh fetches again for the change the page waits to
/// see. Past [`POLLS`], it stops waiting.
pub fn poll(state: &mut State) -> Vec<DataKey> {
    let Some(needs) = following(state).map(|f| f.what.needs(state, &f.route)) else {
        return Vec::new();
    };
    if let Some(f) = &mut state.following {
        f.polls += 1;
        if f.polls > POLLS {
            state.following = None;
            let key = state.first_key(Action::Refresh);
            state.info(format!(
                "GitHub doesn't show the change yet: {key} fetches the page again"
            ));
            return Vec::new();
        }
    }
    needs
}

/// After GitHub's data arrives: stops waiting for a change the page now
/// shows, or follows it to the page it's on.
pub fn follow(state: &mut State) -> Cmds {
    let Some(f) = following(state) else { return Cmds::new() };
    let polls = f.polls;
    match f.what.shown(state, &f.route) {
        Shown::No => Cmds::new(),
        Shown::Yes => {
            let Some(f) = state.following.take() else { return Cmds::new() };
            f.what.then(state, &f.route)
        }
        Shown::There(route, what) => {
            let followed = Following {
                route: route.clone(),
                what,
                polls,
            };
            state.following = Some(followed);
            state.replace(route, false, None)
        }
    }
}
