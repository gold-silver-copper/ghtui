//! A repository's wiki: a page, and the form that writes one.

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
}

pub use Page::*;

impl Page {
    fn parts(&self) -> (&RepoId, Option<&String>) {
        match self {
            Wiki { repo, page } | WikiEdit { repo, page } => (repo, page.as_ref()),
        }
    }
}

impl Family for Page {
    fn url(&self) -> String {
        let (repo, page) = self.parts();
        let wiki = format!("{}/wiki", links::repo(repo));
        match (self, page) {
            (Wiki { .. }, Some(page)) => format!("{wiki}/{}", links::encode_path(page)),
            (Wiki { .. }, None) => wiki,
            (WikiEdit { .. }, Some(page)) => format!("{wiki}/{}/_edit", links::encode_path(page)),
            (WikiEdit { .. }, None) => format!("{wiki}/_new"),
        }
    }

    fn title(&self) -> String {
        let (repo, page) = self.parts();
        match (self, page) {
            (Wiki { .. }, Some(page)) => format!("{repo} · {}", page.replace('-', " ")),
            (Wiki { .. }, None) => format!("{repo} · Wiki"),
            (WikiEdit { .. }, Some(page)) => format!("{repo} · Editing {page}"),
            (WikiEdit { .. }, None) => format!("{repo} · New wiki page"),
        }
    }

    fn repo(&self) -> Option<&RepoId> {
        Some(self.parts().0)
    }

    fn up(&self, _: &State) -> Option<Route> {
        let (repo, page) = self.parts();
        let repo = repo.clone();
        Some(match (self, page) {
            (Wiki { .. }, None) => Route::Code(super::code::Repo(repo)),
            (Wiki { .. }, Some(_)) => Route::Wiki(Wiki { repo, page: None }),
            (WikiEdit { .. }, page) => Route::Wiki(Wiki {
                repo,
                page: page.cloned(),
            }),
        })
    }

    fn needs(&self) -> Vec<DataKey> {
        let (repo, page) = self.parts();
        let own = match self {
            Wiki { .. } => DataKey::Wiki(repo.clone(), page.cloned()),
            WikiEdit { .. } => crate::wiki::form::key(repo, page),
        };
        vec![DataKey::Repo(repo.clone()), own]
    }

    fn width(&self) -> Width {
        Width::Reading
    }

    fn section(&self) -> Section {
        Section::Wiki
    }

    fn build(&self, b: Build<'_>, page: &mut crate::ui::page::Page) {
        let (repo, name) = self.parts();
        let title = self.title();
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
        }
    }
}

pub(super) fn parse_repo(repo: &RepoId, rest: &[&str], _: &Url<'_>) -> Option<Target> {
    let repo = repo.clone();
    let page = match rest {
        ["wiki", "_new"] => WikiEdit { repo, page: None },
        ["wiki", page, "_edit"] if !page.contains(char::is_control) => WikiEdit {
            repo,
            page: Some((*page).to_owned()),
        },
        // A wiki's comparisons are on GitHub.
        ["wiki", .., "_compare"] => return None,
        // A wiki's own pages but its list (history, comparisons).
        ["wiki", special, ..] if special.starts_with('_') && *special != "_pages" => return None,
        // A page's revisions and history: the page.
        ["wiki", rest @ ..] if !rest.iter().any(|p| p.contains(char::is_control)) => Wiki {
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
    let page = prop_oneof![
        (repo(), name()).prop_map(|(repo, page)| WikiEdit { repo, page }),
        (repo(), name())
            .prop_filter("not a writing page", |(_, p)| {
                !matches!(p.as_deref(), Some("_new" | "_edit" | "_compare"))
            })
            .prop_map(|(repo, page)| Wiki { repo, page }),
    ];
    page.prop_map(Route::Wiki)
}
