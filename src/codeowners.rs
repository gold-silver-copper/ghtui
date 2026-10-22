//! Who owns each file of a pull request: its base branch's CODEOWNERS
//! (what GitHub asks reviews of), read once its Files changed is on
//! screen, and drawn in the file tree. A file's own page reads its
//! revision's ([`crate::route::code`]).

use crate::api::codeowners::Codeowners;
use crate::data::DataKey;
use crate::state::{Cmds, State};

/// What code owners add: the pull request's, kept up to date.
pub const FEATURE: crate::features::Feature = crate::features::Feature {
    follow,
    ..crate::features::Feature::NONE
};

/// The code owners of the pull request whose files are on screen: its
/// base branch's, once the pull request is loaded.
fn of_diff(state: &State) -> Option<DataKey> {
    let pr = state.screen().diff()?.of.pr()?;
    let base = &state.pr(pr)?.base_ref;
    Some(DataKey::Codeowners(pr.repo.clone(), base.clone()))
}

/// The pull request's code owners on screen, as loaded.
pub fn on_screen(state: &State) -> Option<&Codeowners> {
    state.picked::<Codeowners>(&of_diff(state)?)
}

/// Once a pull request's files are on screen, its code owners are read.
fn follow(state: &mut State) -> Cmds {
    match of_diff(state) {
        Some(key) if state.remote(&key).is_none() => state.ensure(key, false),
        _ => Cmds::new(),
    }
}
