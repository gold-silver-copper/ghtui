//! What each feature adds to the screens, in one table: the actions it
//! owns, the others it takes on its own pages, what it does with them,
//! the menu's rows it offers, and what it brings up to date as data
//! arrives or a page opens.
//!
//! An action is done by the feature that takes it on the screen as it is
//! (editing a release, closing a milestone), else by the one that owns it,
//! which does it or says where it works. No two features own one action,
//! and on no screen do two take one (tests check both), so the table's
//! order is only the order the menu offers their rows in.

use crate::keymap::Action;
use crate::nav::Doable;
use crate::state::{Cmds, State};

/// What a feature adds.
pub struct Feature {
    /// Its own actions: wherever they're pressed, it does them, or says
    /// where they work.
    pub owns: &'static [Action],
    /// Whether it takes `action` on the screen as it is: another's on its
    /// own pages.
    pub takes: fn(&State, Action) -> bool,
    /// Does an action it owns or takes.
    pub act: fn(&mut State, Action) -> Cmds,
    /// The menu's rows it adds on the screen.
    pub offers: fn(&State, &mut Vec<Doable>),
    /// After GitHub's data arrives or a page opens: brings what it keeps
    /// up to date with it.
    pub follow: fn(&mut State) -> Cmds,
}

impl Feature {
    /// A feature that adds nothing, to fill in what one leaves out.
    pub const NONE: Feature = Feature {
        owns: &[],
        takes: |_, _| false,
        act: |_, _| Cmds::new(),
        offers: |_, _| {},
        follow: |_| Cmds::new(),
    };
}

/// Every feature, in the order the menu offers their rows.
pub const FEATURES: &[Feature] = &[
    crate::home::FEATURE,
    crate::forms::FEATURE,
    crate::projects::FEATURE,
    crate::discuss::FEATURE,
    crate::create::FEATURE,
    crate::clone::FEATURE,
    crate::release::FEATURE,
    crate::gist::FEATURE,
    crate::wiki::form::FEATURE,
    crate::site::FEATURE,
    crate::act::FEATURE,
    crate::review::FEATURE,
    crate::suggest::FEATURE,
    crate::conflicts::FEATURE,
    crate::edit::FEATURE,
    crate::manage::FEATURE,
    crate::settings::FEATURE,
    crate::watch::FEATURE,
    crate::housekeeping::FEATURE,
    crate::workflows::FEATURE,
    crate::offline::FEATURE,
];

/// The feature that does `action` here: the one that takes it on this
/// screen, else the one that owns it.
pub fn of(state: &State, action: Action) -> Option<&'static Feature> {
    (FEATURES.iter().find(|f| (f.takes)(state, action)))
        .or_else(|| FEATURES.iter().find(|f| f.owns.contains(&action)))
}

/// The menu's rows the features add.
pub fn offers(state: &State, out: &mut Vec<Doable>) {
    for f in FEATURES {
        (f.offers)(state, out);
    }
}

/// After GitHub's data arrives or a page opens: brings every feature up
/// to date with it.
pub fn follow(state: &mut State) -> Cmds {
    FEATURES.iter().flat_map(|f| (f.follow)(state)).collect()
}
