//! The checks, in the order they run. A check is a [`Check`]: a name,
//! what it shows, and a function that drives ghtui ([`Cx::ghtui`]), asks
//! GitHub ([`super::gh`]) and asks [`Cx::defer`] to undo what it made.
//!
//! [`Cx::ghtui`]: super::Cx::ghtui
//! [`Cx::defer`]: super::Cx::defer

use super::Check;

mod actions;
mod issue;
mod labels;
mod offline;
mod orgs;
mod pages;
mod pull;
mod release;
mod settings;

pub const ALL: &[Check] = &[
    pages::CHECK,
    issue::CHECK,
    pull::CHECK,
    release::CHECK,
    labels::CHECK,
    settings::CHECK,
    actions::CHECK,
    orgs::ADVISORY,
    orgs::ORGS,
    offline::CHECK,
];
