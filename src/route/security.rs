//! Security: a repository's advisories and alerts (Dependabot's, code
//! scanning's, secret scanning's), and GitHub's advisory database.

use super::{Family, OPEN, Route, Target, Url};
use crate::api::model::RepoId;
use crate::api::security::{AlertKind, AlertRef};
use crate::browse::{Build, Width};
use crate::chrome::{Chrome, Section};
use crate::data::DataKey;
use crate::state::State;
use crate::ui::pages::{self, url as links};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Page {
    /// A repository's security advisories, or GitHub's newest.
    Advisories(Option<RepoId>),
    /// A security advisory: a repository's, or from GitHub's database.
    Advisory { repo: Option<RepoId>, ghsa: String },
    /// A repository's Dependabot, code scanning or secret scanning
    /// alerts, filtered as GitHub's lists are (`is:open severity:high`).
    Alerts {
        repo: RepoId,
        kind: AlertKind,
        query: String,
    },
    /// One security alert.
    Alert {
        repo: RepoId,
        kind: AlertKind,
        number: u64,
    },
}

pub use Page::*;

impl Family for Page {
    fn url(&self) -> String {
        match self {
            Advisories(Some(repo)) => format!("{}/security/advisories", links::repo(repo)),
            Advisories(None) => format!("{}/advisories", links::base()),
            Advisory {
                repo: Some(repo),
                ghsa,
            } => format!("{}/security/advisories/{ghsa}", links::repo(repo)),
            Advisory { repo: None, ghsa } => format!("{}/advisories/{ghsa}", links::base()),
            Alerts { repo, kind, query } => {
                let base = format!("{}/security/{}", links::repo(repo), kind.slug());
                if query == OPEN {
                    base
                } else {
                    format!("{base}?{}={}", kind.query_param(), links::encode(query))
                }
            }
            Alert { repo, kind, number } => {
                format!("{}/security/{}/{number}", links::repo(repo), kind.slug())
            }
        }
    }

    fn title(&self) -> String {
        match self {
            Advisories(Some(repo)) => format!("{repo} · Security"),
            Advisories(None) => "Advisories".to_owned(),
            Advisory { ghsa, .. } => ghsa.clone(),
            Alerts { repo, kind, .. } => format!("{repo} · {}", kind.name()),
            Alert { repo, kind, number } => format!("{repo} · {} alert {number}", kind.name()),
        }
    }

    fn repo(&self) -> Option<&RepoId> {
        match self {
            Advisories(repo) | Advisory { repo, .. } => repo.as_ref(),
            Alerts { repo, .. } | Alert { repo, .. } => Some(repo),
        }
    }

    fn up(&self, _: &State) -> Option<Route> {
        Some(match self {
            Advisories(repo) => repo.clone().map_or(Route::App(super::app::Home), |repo| {
                Route::Code(super::code::Repo(repo))
            }),
            Advisory { repo, .. } => Route::Security(Advisories(repo.clone())),
            Alert { repo, kind, .. } => Route::Security(Alerts {
                repo: repo.clone(),
                kind: *kind,
                query: OPEN.into(),
            }),
            Alerts { repo, .. } => Route::Security(Advisories(Some(repo.clone()))),
        })
    }

    fn needs(&self) -> Vec<DataKey> {
        let header = self.repo().map(|repo| DataKey::Repo(repo.clone()));
        let own = match self {
            Advisory { repo, ghsa } => Some(DataKey::Advisory(repo.clone(), ghsa.clone())),
            Alert { repo, kind, number } => Some(DataKey::Alert(AlertRef {
                repo: repo.clone(),
                kind: *kind,
                number: *number,
            })),
            Advisories(_) | Alerts { .. } => self.paged(),
        };
        header.into_iter().chain(own).collect()
    }

    fn paged(&self) -> Option<DataKey> {
        match self {
            Advisories(repo) => Some(DataKey::Advisories(repo.clone())),
            Alerts { repo, kind, query } => {
                Some(DataKey::Alerts(repo.clone(), *kind, query.clone()))
            }
            Advisory { .. } | Alert { .. } => None,
        }
    }

    fn width(&self) -> Width {
        match self {
            Advisories(_) | Alerts { .. } => Width::Lists,
            Advisory { .. } | Alert { .. } => Width::Reading,
        }
    }

    fn query(&self) -> Option<&str> {
        match self {
            Alerts { query, .. } => Some(query),
            _ => None,
        }
    }

    fn with_query(&self, query: String) -> Option<Route> {
        match self {
            Alerts { repo, kind, .. } => Some(Route::Security(Alerts {
                repo: repo.clone(),
                kind: *kind,
                query,
            })),
            _ => None,
        }
    }

    fn section(&self) -> Section {
        Section::Security
    }

    fn chrome(&self, state: &State, c: &mut Chrome) {
        match self {
            Advisories(None) => c.crumb("Advisories", None),
            Advisory { repo: None, ghsa } => {
                let list = Route::Security(Advisories(None));
                c.crumb("Advisories", Some(Target::Page(list)));
                c.crumb(ghsa, None);
            }
            _ => {
                if let Some(repo) = self.repo() {
                    state.repo_chrome(repo, self.section(), c);
                }
            }
        }
    }

    fn build(&self, b: Build<'_>, page: &mut crate::ui::page::Page) {
        let (cx, now) = (b.cx, b.cx.now);
        match self {
            Advisories(repo) => {
                if let Some(repo) = repo {
                    pages::security_nav(page, repo, None);
                }
                pages::advisories(page, repo.as_ref(), b.list(self.paged()), now);
            }
            Advisory { repo, ghsa } => {
                let key = DataKey::Advisory(repo.clone(), ghsa.clone());
                if let Some(a) = b.get(&key).show(page, ghsa) {
                    pages::advisory(page, a, now);
                }
            }
            Alerts { repo, kind, query } => {
                pages::security_nav(page, repo, Some(*kind));
                pages::alerts(page, repo, *kind, query, b.list(self.paged()), cx);
            }
            Alert { repo, kind, number } => {
                let key = DataKey::Alert(AlertRef {
                    repo: repo.clone(),
                    kind: *kind,
                    number: *number,
                });
                if let Some(a) = b.get(&key).show(page, &self.title()) {
                    pages::alert(page, repo, a, cx);
                }
            }
        }
    }
}

/// A security advisory's ID: `GHSA-xxxx-xxxx-xxxx`.
pub(super) fn valid_ghsa(s: &str) -> bool {
    s.strip_prefix("GHSA-").is_some_and(|rest| {
        !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    })
}

/// GitHub's advisory database.
pub(super) fn parse(url: &Url<'_>) -> Option<Target> {
    let page = match url.segments {
        ["advisories"] => Advisories(None),
        ["advisories", ghsa] if valid_ghsa(ghsa) => Advisory {
            repo: None,
            ghsa: (*ghsa).to_owned(),
        },
        _ => return None,
    };
    Some(Target::Page(Route::Security(page)))
}

pub(super) fn parse_repo(repo: &RepoId, rest: &[&str], url: &Url<'_>) -> Option<Target> {
    let repo = repo.clone();
    let page = match rest {
        // The security overview's part ghtui can show: advisories.
        ["security"] | ["security", "advisories"] => Advisories(Some(repo)),
        ["security", "advisories", ghsa] if valid_ghsa(ghsa) => Advisory {
            repo: Some(repo),
            ghsa: (*ghsa).to_owned(),
        },
        // Dependabot's, code scanning's and secret scanning's alerts.
        ["security", slug] => {
            let kind = AlertKind::from_slug(slug)?;
            let query = (url.param(kind.query_param()))
                .or_else(|| url.param("q").or_else(|| url.param("query")))
                .map(|q| q.trim().to_owned())
                .filter(|q| !q.is_empty())
                .unwrap_or_else(|| OPEN.to_owned());
            Alerts { repo, kind, query }
        }
        ["security", slug, n] => Alert {
            repo,
            kind: AlertKind::from_slug(slug)?,
            number: n.parse().ok().filter(|&n| n > 0)?,
        },
        _ => return None,
    };
    Some(Target::Page(Route::Security(page)))
}

#[cfg(test)]
pub(crate) fn arb() -> impl proptest::strategy::Strategy<Value = Route> {
    use crate::route::arb::{query, repo};
    use proptest::prelude::*;
    let kind = || prop::sample::select(AlertKind::ALL.to_vec());
    let page = prop_oneof![
        prop::option::of(repo()).prop_map(Advisories),
        (prop::option::of(repo()), "GHSA(-[2-9a-z]{4}){3}")
            .prop_map(|(repo, ghsa)| Advisory { repo, ghsa }),
        (repo(), kind(), prop_oneof![Just(OPEN.to_owned()), query()])
            .prop_map(|(repo, kind, query)| Alerts { repo, kind, query }),
        (repo(), kind(), 1..u64::MAX).prop_map(|(repo, kind, number)| Alert { repo, kind, number }),
    ];
    page.prop_map(Route::Security)
}
