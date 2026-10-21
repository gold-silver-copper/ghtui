//! Discussions: a repository's or an organization's, in a category, and
//! one discussion.

use std::collections::HashSet;

use super::{Family, Route, Target, Url, valid_slug};
use crate::api::browse::DiscussionsOf;
use crate::api::model::RepoId;
use crate::browse::{Build, Width};
use crate::chrome::{Chrome, Section};
use crate::data::DataKey;
use crate::state::State;
use crate::ui::pages::{self, url as links};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Page {
    /// Discussions, in a category (by its slug) if given.
    Discussions {
        of: DiscussionsOf,
        category: Option<String>,
    },
    Discussion {
        of: DiscussionsOf,
        number: u64,
    },
}

pub use Page::*;

impl Page {
    fn of(&self) -> &DiscussionsOf {
        match self {
            Discussions { of, .. } | Discussion { of, .. } => of,
        }
    }
}

fn list_url(of: &DiscussionsOf) -> String {
    match of {
        DiscussionsOf::Repo(repo) => format!("{}/discussions", links::repo(repo)),
        DiscussionsOf::Org(org) => format!("{}/orgs/{org}/discussions", links::base()),
    }
}

impl Family for Page {
    fn url(&self) -> String {
        match self {
            Discussions {
                of,
                category: Some(slug),
            } => format!("{}/categories/{}", list_url(of), links::encode_path(slug)),
            Discussions { of, category: None } => list_url(of),
            Discussion { of, number } => format!("{}/{number}", list_url(of)),
        }
    }

    fn title(&self) -> String {
        let of = match self.of() {
            DiscussionsOf::Repo(repo) => repo.to_string(),
            DiscussionsOf::Org(org) => format!("@{org}"),
        };
        match self {
            Discussions { .. } => format!("{of} · Discussions"),
            Discussion { number, .. } => format!("{of} · Discussion {number}"),
        }
    }

    fn repo(&self) -> Option<&RepoId> {
        match self.of() {
            DiscussionsOf::Repo(repo) => Some(repo),
            DiscussionsOf::Org(_) => None,
        }
    }

    fn up(&self, _: &State) -> Option<Route> {
        Some(match (self, self.of()) {
            (Discussion { of, .. }, _) => Route::Discussions(Discussions {
                of: of.clone(),
                category: None,
            }),
            (Discussions { .. }, DiscussionsOf::Repo(repo)) => Route::code(repo.clone()),
            (Discussions { .. }, DiscussionsOf::Org(org)) => Route::user(org),
        })
    }

    fn needs(&self) -> Vec<DataKey> {
        let header = self.repo().map(|repo| DataKey::Repo(repo.clone()));
        let own = match self {
            Discussion { of, number } => Some(DataKey::Discussion(of.clone(), *number)),
            Discussions { .. } => self.paged(),
        };
        header.into_iter().chain(own).collect()
    }

    fn paged(&self) -> Option<DataKey> {
        match self {
            Discussions { of, category } => {
                Some(DataKey::Discussions(of.clone(), category.clone()))
            }
            Discussion { .. } => None,
        }
    }

    fn width(&self) -> Width {
        match self {
            Discussions { .. } => Width::Lists,
            Discussion { .. } => Width::Reading,
        }
    }

    fn starts_on_row(&self) -> bool {
        matches!(self, Discussions { .. })
    }

    fn section(&self) -> Section {
        Section::Discussions
    }

    fn chrome(&self, state: &State, c: &mut Chrome) {
        match self.of() {
            DiscussionsOf::Repo(repo) => state.repo_chrome(repo, self.section(), c),
            DiscussionsOf::Org(org) => {
                c.crumb(org, Some(Target::Page(Route::user(org))));
                c.crumb("Discussions", None);
            }
        }
    }

    fn build(&self, b: Build<'_>, page: &mut crate::ui::page::Page) {
        let now = b.cx.now;
        match self {
            Discussions { of, category } => {
                let list = b.list(self.paged());
                pages::discussions(page, &list_url(of), list, category.as_deref(), now);
            }
            Discussion { of, number } => {
                let key = DataKey::Discussion(of.clone(), *number);
                let none = HashSet::new();
                let route = Route::Discussions(self.clone());
                let fresh = crate::live::page::fresh(b.state, &route).unwrap_or(&none);
                if let Some(d) = b.get(&key).show(page, &self.title()) {
                    pages::discussion(page, d, fresh, &b.state.talk.unfolded, b.cx);
                }
            }
        }
    }
}

/// A discussions page from what follows `/discussions` in its URL.
fn discussions(of: DiscussionsOf, rest: &[&str]) -> Option<Target> {
    let page = match rest {
        [] => Discussions { of, category: None },
        ["categories", slug] if valid_slug(slug) => Discussions {
            of,
            category: Some((*slug).to_owned()),
        },
        [n] => Discussion {
            of,
            number: n.parse().ok()?,
        },
        _ => return None,
    };
    Some(Target::Page(Route::Discussions(page)))
}

/// An organization's discussions.
pub(super) fn parse(url: &Url<'_>) -> Option<Target> {
    match url.segments {
        ["orgs", org, "discussions", rest @ ..] => {
            discussions(DiscussionsOf::Org((*org).to_owned()), rest)
        }
        _ => None,
    }
}

pub(super) fn parse_repo(repo: &RepoId, rest: &[&str], _: &Url<'_>) -> Option<Target> {
    match rest {
        ["discussions", rest @ ..] => discussions(DiscussionsOf::Repo(repo.clone()), rest),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) fn arb() -> impl proptest::strategy::Strategy<Value = Route> {
    use crate::route::arb::{login, repo};
    use proptest::prelude::*;
    let of = || {
        prop_oneof![
            repo().prop_map(DiscussionsOf::Repo),
            login().prop_map(DiscussionsOf::Org),
        ]
    };
    let page = prop_oneof![
        (of(), prop::option::of("[a-z0-9][a-z0-9-]{0,11}"))
            .prop_map(|(of, category)| Discussions { of, category }),
        (of(), 1..u64::MAX).prop_map(|(of, number)| Discussion { of, number }),
    ];
    page.prop_map(Route::Discussions)
}
