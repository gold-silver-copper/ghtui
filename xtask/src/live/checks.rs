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
mod members;
mod merging;
mod offline;
mod orgs;
mod pages;
mod people;
mod properties;
mod pull;
mod release;
mod secrets;
mod settings;

pub const ALL: &[Check] = &[
    pages::CHECK,
    issue::CHECK,
    pull::CHECK,
    release::CHECK,
    labels::CHECK,
    settings::CHECK,
    secrets::CHECK,
    actions::CHECK,
    orgs::ADVISORY,
    orgs::ORGS,
    members::INVITATIONS,
    members::MEMBERS,
    properties::PROPERTIES,
    properties::ISSUE_TYPES,
    merging::SUGGESTION,
    merging::CONFLICTS,
    people::CHECK,
    offline::CHECK,
];
