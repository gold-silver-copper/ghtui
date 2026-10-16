//! Searches, and your notifications.

use super::{Family, Route, Target, Url};
use crate::api::browse::{SearchKind, SearchResults};
use crate::browse::Build;
use crate::chrome::{Chrome, new_tab};
use crate::data::DataKey;
use crate::state::State;
use crate::ui::pages::{self, url as links};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Page {
    Search {
        kind: SearchKind,
        query: String,
    },
    /// Your inbox, filtered as GitHub's is (`is:unread reason:mention`).
    Notifications {
        query: String,
    },
}

pub use Page::*;

impl Family for Page {
    fn url(&self) -> String {
        match self {
            Search { kind, query } => links::search(*kind, query),
            Notifications { query } if query.is_empty() => {
                format!("{}/notifications", links::base())
            }
            Notifications { query } => format!(
                "{}/notifications?query={}",
                links::base(),
                links::encode(query)
            ),
        }
    }

    fn title(&self) -> String {
        match self {
            Search { query, .. } => format!("Search “{query}”"),
            Notifications { .. } => "Notifications".to_owned(),
        }
    }

    fn up(&self, _: &State) -> Option<Route> {
        Some(Route::App(super::app::Home))
    }

    fn needs(&self) -> Vec<DataKey> {
        self.paged().into_iter().collect()
    }

    fn paged(&self) -> Option<DataKey> {
        Some(match self {
            Search { kind, query } => DataKey::Search(*kind, query.clone()),
            Notifications { query } => DataKey::Notifications(query.clone()),
        })
    }

    fn shown_source(&self) -> Option<DataKey> {
        match self {
            Search {
                kind: SearchKind::Code,
                ..
            } => self.paged(),
            _ => None,
        }
    }

    fn query(&self) -> Option<&str> {
        match self {
            Search { query, .. } | Notifications { query } => Some(query),
        }
    }

    fn with_query(&self, query: String) -> Option<Route> {
        Some(Route::Search(match self {
            Search { kind, .. } => Search { kind: *kind, query },
            Notifications { .. } => Notifications { query },
        }))
    }

    fn chrome(&self, state: &State, c: &mut Chrome) {
        let Search { kind, query } = self else {
            return c.crumb("Notifications", None);
        };
        c.crumb("Search", None);
        let results = (self.paged()).and_then(|key| state.picked::<SearchResults>(&key));
        let total = results.map(|r| r.counts().0);
        let kinds = [
            // Short, so all seven fit.
            (SearchKind::Repos, "▤", "Repos"),
            (SearchKind::Issues, "◉", "Issues"),
            (SearchKind::Pulls, "⇄", "PRs"),
            (SearchKind::Users, "⚇", "Users"),
            (SearchKind::Discussions, "◈", "Discussions"),
            (SearchKind::Commits, "◷", "Commits"),
            (SearchKind::Code, "<>", "Code"),
        ];
        for (i, (k, icon, label)) in kinds.into_iter().enumerate() {
            if k == *kind {
                c.active = Some(i);
            }
            let count = (k == *kind).then_some(total).flatten();
            let search = Route::Search(Search {
                kind: k,
                query: query.clone(),
            });
            c.tabs
                .push((new_tab(icon, label, count), Target::Page(search)));
        }
    }

    fn build(&self, b: Build<'_>, page: &mut crate::ui::page::Page) {
        let (cx, now) = (b.cx, b.cx.now);
        match self {
            Search { kind, query } => {
                let results = b.list(self.paged());
                pages::search(page, *kind, query, results, cx.icons, cx.keys, now);
            }
            Notifications { query } => pages::notifications(page, query, b.list(self.paged()), cx),
        }
    }
}

/// Searches, the dashboards of your pull requests and issues (searches
/// too), and your inbox.
pub(super) fn parse(url: &Url<'_>) -> Option<Target> {
    let query = url.param("q");
    let page = match url.segments {
        ["search"] => Search {
            kind: match url.param("type").map(|k| k.to_ascii_lowercase()).as_deref() {
                None | Some("repositories") => SearchKind::Repos,
                Some("issues") => SearchKind::Issues,
                Some("pullrequests") => SearchKind::Pulls,
                Some("users") => SearchKind::Users,
                Some("discussions") => SearchKind::Discussions,
                Some("commits") => SearchKind::Commits,
                Some("code") => SearchKind::Code,
                // Topics: the repositories tagged with one.
                Some("topics") => {
                    let topic = query.unwrap_or_default();
                    let topic = topic.trim();
                    let query = if topic.is_empty() {
                        String::new()
                    } else {
                        format!("topic:{topic}")
                    };
                    let kind = SearchKind::Repos;
                    return Some(Target::Page(Route::Search(Search { kind, query })));
                }
                // Wikis, packages, the marketplace…
                Some(_) => return None,
            },
            query: query.unwrap_or_default(),
        },
        // The dashboards of your pull requests and issues: searches.
        [list @ ("pulls" | "issues"), rest @ ..] => {
            let (kind, is) = if *list == "pulls" {
                (SearchKind::Pulls, "pr")
            } else {
                (SearchKind::Issues, "issue")
            };
            let whose = match rest {
                [] => "author:@me",
                ["assigned"] => "assignee:@me",
                ["mentioned"] => "mentions:@me",
                ["review-requested"] if *list == "pulls" => "review-requested:@me",
                _ => return None,
            };
            Search {
                kind,
                query: query.unwrap_or_else(|| format!("is:open is:{is} {whose} archived:false")),
            }
        }
        ["notifications"] => Notifications {
            query: (url.param("query"))
                .map(|q| q.trim().to_owned())
                .unwrap_or_default(),
        },
        _ => return None,
    };
    Some(Target::Page(Route::Search(page)))
}

#[cfg(test)]
pub(crate) fn arb() -> impl proptest::strategy::Strategy<Value = Route> {
    use crate::route::arb::query;
    use proptest::prelude::*;
    let kind = prop::sample::select(vec![
        SearchKind::Repos,
        SearchKind::Issues,
        SearchKind::Pulls,
        SearchKind::Users,
        SearchKind::Discussions,
        SearchKind::Commits,
        SearchKind::Code,
    ]);
    let page = prop_oneof![
        prop_oneof![Just(String::new()), query()].prop_map(|query| Notifications { query }),
        (kind, any::<String>()).prop_map(|(kind, query)| Search { kind, query }),
    ];
    page.prop_map(Route::Search)
}
