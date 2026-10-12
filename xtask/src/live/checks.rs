//! The checks, in the order they run. A check is a [`Check`]: a name,
//! what it shows, and a function that drives ghtui ([`Cx::ghtui`]), asks
//! GitHub ([`super::gh`]) and asks [`Cx::defer`] to undo what it made.
//!
//! [`Cx::ghtui`]: super::Cx::ghtui
//! [`Cx::defer`]: super::Cx::defer

use super::Check;

mod enterprise;
mod inbox;
mod issue;
mod merge;
mod narrow;
mod projects;
mod pull;
mod review;

pub const ALL: &[Check] = &[
    issue::CHECK,
    pull::CREATE,
    pull::CHECKOUT,
    merge::CHECK,
    review::CHECK,
    projects::CHECK,
    enterprise::CHECK,
    enterprise::OLDER,
    narrow::CHECK,
    // Last: the notification it asks for when the run starts can take
    // GitHub minutes.
    inbox::CHECK,
];
