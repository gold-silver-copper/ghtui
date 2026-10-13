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
mod housekeeping;
mod inbox;
mod issue;
mod local_reads;
mod manage;
mod merge;
mod narrow;
mod offline;
mod project_views;
mod projects;
mod pull;
mod releases;
mod review;
mod reviews;
mod security;
mod settings;
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
    releases::CHECK,
    writes::GIST,
    writes::WATCH,
    writes::SUBSCRIBE,
    projects::CHECK,
    projects::ITEMS,
    project_views::CHECK,
    discussion::CHECK,
    // Managing an issue, a pull request and a discussion.
    manage::ISSUE,
    manage::PULL,
    manage::DISCUSSION,
    manage::TRANSFER,
    // A repository's settings.
    settings::BRANCHES,
    settings::ACCESS,
    settings::HOOKS,
    settings::KEYS,
    settings::ARCHIVE,
    local_reads::CHECK,
    wiki::CHECK,
    // Repository housekeeping.
    housekeeping::LABELS,
    housekeeping::MILESTONES,
    housekeeping::REFS,
    housekeeping::REFS_GIT,
    housekeeping::FILES,
    housekeeping::FILES_GIT,
    housekeeping::FORK,
    housekeeping::NEW_REPO,
    enterprise::CHECK,
    enterprise::OLDER,
    // Reading offline: GitHub goes away and comes back.
    offline::CHECK,
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
