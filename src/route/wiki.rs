//! A repository's wiki: a page, the form that writes one, and its
//! history and changes, read with git.

use super::{Family, Route, Target, Url};
use crate::api::browse::WikiPage;
use crate::api::model::RepoId;
use crate::browse::{Build, Width};
use crate::chrome::Section;
use crate::data::DataKey;
use crate::state::State;
use crate::ui::pages::{self, url as links};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Page {
    /// A wiki page (its Home without one; its pages for `_pages`).
    Wiki { repo: RepoId, page: Option<String> },
    /// The form writing a wiki page (a new one without `page`).
    WikiEdit { repo: RepoId, page: Option<String> },
    /// The wiki's history, or a page's.
    WikiHistory { repo: RepoId, page: Option<String> },
    /// What a revision changed (`sha`), or what changed between two
    /// (`a...b`): of a page, or of the wiki.
    WikiCompare {
        repo: RepoId,
        page: Option<String>,
        spec: String,
    },
}

pub use Page::*;

impl Page {
    fn parts(&self) -> (&RepoId, Option<&String>) {
        match self {
            Wiki { repo, page }
            | WikiEdit { repo, page }
            | WikiHistory { repo, page }
            | WikiCompare { repo, page, .. } => (repo, page.as_ref()),
        }
    }
}

impl Family for Page {
    fn url(&self) -> String {
        let (repo, page) = self.parts();
        let wiki = format!("{}/wiki", links::repo(repo));
        let of = page.map_or_else(
            || wiki.clone(),
            |p| format!("{wiki}/{}", links::encode_path(p)),
        );
        match (self, page) {
            (Wiki { .. }, _) => of,
            (WikiEdit { .. }, Some(_)) => format!("{of}/_edit"),
            (WikiEdit { .. }, None) => format!("{wiki}/_new"),
            (WikiHistory { .. }, _) => format!("{of}/_history"),
            (WikiCompare { spec, .. }, _) => format!("{of}/_compare/{spec}"),
        }
    }

    fn title(&self) -> String {
        let (repo, page) = self.parts();
        let page = page.map(|p| p.replace('-', " "));
        match (self, page) {
            (Wiki { .. }, Some(page)) => format!("{repo} · {page}"),
            (Wiki { .. }, None) => format!("{repo} · Wiki"),
            (WikiEdit { .. }, Some(page)) => format!("{repo} · Editing {page}"),
            (WikiEdit { .. }, None) => format!("{repo} · New wiki page"),
            (WikiHistory { .. }, Some(page)) => format!("{repo} · History of {page}"),
            (WikiHistory { .. }, None) => format!("{repo} · Wiki history"),
            (WikiCompare { spec, .. }, page) => {
                use crate::ui::text::short_sha;
                let spec = match crate::wiki::compared(spec) {
                    Some((Some(a), b)) => format!("{}...{}", short_sha(a), short_sha(b)),
                    _ => short_sha(spec).to_owned(),
                };
                match page {
                    Some(page) => format!("{repo} · {page} at {spec}"),
                    None => format!("{repo} · Wiki at {spec}"),
                }
            }
        }
    }

    fn repo(&self) -> Option<&RepoId> {
        Some(self.parts().0)
    }

    fn up(&self, _: &State) -> Option<Route> {
        let (repo, page) = self.parts();
        let (repo, page) = (repo.clone(), page.cloned());
        Some(match (self, page) {
            (Wiki { .. }, None) => Route::code(repo),
            (Wiki { .. }, Some(_)) => Route::Wiki(Wiki { repo, page: None }),
            (WikiEdit { .. } | WikiHistory { .. }, page) => Route::Wiki(Wiki { repo, page }),
            (WikiCompare { .. }, page) => Route::Wiki(WikiHistory { repo, page }),
        })
    }

    fn needs(&self) -> Vec<DataKey> {
        let (repo, page) = self.parts();
        let own = match self {
            Wiki { .. } => DataKey::Wiki(repo.clone(), page.cloned()),
            WikiEdit { .. } => crate::wiki::form::key(repo, page),
            WikiHistory { .. } => DataKey::WikiHistory(repo.clone(), page.cloned()),
            WikiCompare { spec, .. } => {
                DataKey::WikiDiff(repo.clone(), page.cloned(), spec.clone())
            }
        };
        vec![DataKey::Repo(repo.clone()), own]
    }

    fn width(&self) -> Width {
        match self {
            WikiHistory { .. } => Width::Lists,
            _ => Width::Reading,
        }
    }

    fn starts_on_row(&self) -> bool {
        matches!(self, WikiHistory { .. })
    }

    fn section(&self) -> Section {
        Section::Wiki
    }

    fn form(&self) -> Option<&'static crate::forms::Kind> {
        matches!(self, WikiEdit { .. }).then_some(&crate::wiki::form::FORM)
    }

    fn build(&self, b: Build<'_>, page: &mut crate::ui::page::Page) {
        let (repo, name) = self.parts();
        let title = self.title();
        let now = b.cx.now;
        match self {
            Wiki { .. } => {
                let key = DataKey::Wiki(repo.clone(), name.cloned());
                if let Some(w) = b.get(&key).show(page, &title) {
                    pages::wiki(page, repo, w);
                }
            }
            WikiEdit { .. } => {
                let key = crate::wiki::form::key(repo, name);
                if b.get::<WikiPage>(&key).show(page, &title).is_some() {
                    crate::wiki::form::build(b.state, page, &Route::Wiki(self.clone()));
                }
            }
            WikiHistory { .. } => {
                let key = DataKey::WikiHistory(repo.clone(), name.cloned());
                if let Some(h) = b.get(&key).show(page, "the wiki's history") {
                    pages::wiki_history(page, repo, h, now);
                }
            }
            WikiCompare { spec, .. } => {
                let key = DataKey::WikiDiff(repo.clone(), name.cloned(), spec.clone());
                if let Some(d) = b.get(&key).show(page, "the wiki's changes") {
                    pages::wiki_diff(page, repo, name.map(String::as_str), d, now);
                }
            }
        }
    }
}

pub(super) fn parse_repo(repo: &RepoId, rest: &[&str], _: &Url<'_>) -> Option<Target> {
    let repo = repo.clone();
    let named = |p: &str| !p.contains(char::is_control);
    let page = |p: &str| (!p.starts_with('_')).then(|| p.to_owned());
    let page = match rest {
        ["wiki", "_new"] => WikiEdit { repo, page: None },
        ["wiki", page, "_edit"] if named(page) => WikiEdit {
            repo,
            page: Some((*page).to_owned()),
        },
        ["wiki", "_history"] => WikiHistory { repo, page: None },
        ["wiki", p, "_history"] if named(p) => WikiHistory {
            repo,
            page: page(p),
        },
        ["wiki", "_compare", spec] if crate::wiki::compared(spec).is_some() => WikiCompare {
            repo,
            page: None,
            spec: (*spec).to_owned(),
        },
        ["wiki", p, "_compare", spec] if named(p) && crate::wiki::compared(spec).is_some() => {
            WikiCompare {
                repo,
                page: page(p),
                spec: (*spec).to_owned(),
            }
        }
        // A comparison GitHub's form hasn't named yet is on GitHub.
        ["wiki", .., "_compare"] => return None,
        // A wiki's own pages but its list.
        ["wiki", special, ..] if special.starts_with('_') && *special != "_pages" => return None,
        // A page's revision: the page.
        ["wiki", rest @ ..] if rest.iter().all(|p| named(p)) => Wiki {
            repo,
            page: rest.first().map(|p| (*p).to_owned()),
        },
        _ => return None,
    };
    Some(Target::Page(Route::Wiki(page)))
}

#[cfg(test)]
pub(crate) fn arb() -> impl proptest::strategy::Strategy<Value = Route> {
    use crate::route::arb::repo;
    use proptest::prelude::*;
    let name = || prop::option::of("[A-Za-z0-9][A-Za-z0-9 _-]{0,15}");
    let spec = prop_oneof![
        "[0-9a-f]{40}",
        ("[0-9a-f]{7}", "[0-9a-f]{7}").prop_map(|(a, b)| format!("{a}...{b}")),
    ];
    let page = prop_oneof![
        (repo(), name()).prop_map(|(repo, page)| WikiEdit { repo, page }),
        (repo(), name())
            .prop_filter("not a writing page", |(_, p)| {
                !matches!(p.as_deref(), Some("_new" | "_edit" | "_compare"))
            })
            .prop_map(|(repo, page)| Wiki { repo, page }),
        (repo(), name()).prop_map(|(repo, page)| WikiHistory { repo, page }),
        (repo(), name(), spec).prop_map(|(repo, page, spec)| WikiCompare { repo, page, spec }),
    ];
    page.prop_map(Route::Wiki)
}
