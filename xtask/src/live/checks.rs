//! The checks, in the order they run. A check is a [`Check`]: a name,
//! what it shows, and a function that drives ghtui ([`Cx::ghtui`]), asks
//! GitHub ([`super::gh`]) and asks [`Cx::defer`] to undo what it made.
//!
//! [`Cx::ghtui`]: super::Cx::ghtui
//! [`Cx::defer`]: super::Cx::defer

use super::Check;

mod actions;
mod discussion;
mod enterprise;
mod inbox;
mod issue;
mod local_reads;
mod manage;
mod merge;
mod narrow;
mod projects;
mod pull;
mod review;
mod reviews;
mod security;
mod site;
mod timeline;
mod wiki;
mod writes;

pub const ALL: &[Check] = &[
    issue::CHECK,
    pull::CREATE,
    pull::CHECKOUT,
    merge::CHECK,
    review::CHECK,
    reviews::CHECK,
    timeline::CHECK,
    actions::CHECK,
    security::CHECK,
    writes::RELEASE,
    writes::GIST,
    writes::WATCH,
    writes::SUBSCRIBE,
    projects::CHECK,
    projects::ITEMS,
    discussion::CHECK,
    // Managing an issue, a pull request and a discussion.
    manage::ISSUE,
    manage::PULL,
    manage::DISCUSSION,
    manage::TRANSFER,
    local_reads::CHECK,
    wiki::CHECK,
    enterprise::CHECK,
    enterprise::OLDER,
    // Insights, Explore, starred lists, packages, saved replies.
    site::INSIGHTS,
    site::EXPLORE,
    site::LISTS,
    site::PACKAGES,
    site::REPLIES,
    narrow::CHECK,
    // Last: the notification it asks for when the run starts can take
    // GitHub minutes.
    inbox::CHECK,
];
