//! A repository's releases: the list, a release, and the forms that make
//! and edit one.

use super::{Family, Route, Target, Url};
use crate::api::browse::{Release as Data, RepoOverview};
use crate::api::model::RepoId;
use crate::browse::{Build, Width};
use crate::data::DataKey;
use crate::state::State;
use crate::ui::pages::{self, url as links};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Page {
    Releases(RepoId),
    Release {
        repo: RepoId,
        tag: String,
    },
    /// A new release's form.
    NewRelease(RepoId),
    /// The form editing a release, by its tag.
    EditRelease {
        repo: RepoId,
        tag: String,
    },
}

pub use Page::*;

impl Page {
    fn repo_id(&self) -> &RepoId {
        match self {
            Releases(repo) | Release { repo, .. } | NewRelease(repo) | EditRelease { repo, .. } => {
                repo
            }
        }
    }
}

impl Family for Page {
    fn url(&self) -> String {
        match self {
            Releases(repo) => format!("{}/releases", links::repo(repo)),
            Release { repo, tag } => links::release(repo, tag),
            NewRelease(repo) => format!("{}/releases/new", links::repo(repo)),
            EditRelease { repo, tag } => format!(
                "{}/releases/edit/{}",
                links::repo(repo),
                links::encode_path(tag)
            ),
        }
    }

    fn title(&self) -> String {
        match self {
            Releases(repo) => format!("{repo} · Releases"),
            Release { repo, tag } => format!("{repo} {tag}"),
            NewRelease(repo) => format!("{repo} · New release"),
            EditRelease { repo, tag } => format!("{repo} · Editing {tag}"),
        }
    }

    fn repo(&self) -> Option<&RepoId> {
        Some(self.repo_id())
    }

    fn up(&self, _: &State) -> Option<Route> {
        let repo = self.repo_id().clone();
        Some(match self {
            Releases(_) => Route::Code(super::code::Repo(repo)),
            Release { .. } | NewRelease(_) => Route::Releases(Releases(repo)),
            EditRelease { tag, .. } => Route::Releases(Release {
                repo,
                tag: tag.clone(),
            }),
        })
    }

    fn needs(&self) -> Vec<DataKey> {
        let repo = self.repo_id();
        let header = DataKey::Repo(repo.clone());
        // A release's form picks its tag and its branch.
        let refs = [DataKey::Tags(repo.clone()), DataKey::Branches(repo.clone())];
        match self {
            Releases(_) => std::iter::once(header).chain(self.paged()).collect(),
            Release { tag, .. } => vec![header, DataKey::Release(repo.clone(), tag.clone())],
            NewRelease(_) => std::iter::once(header).chain(refs).collect(),
            EditRelease { tag, .. } => [header, DataKey::Release(repo.clone(), tag.clone())]
                .into_iter()
                .chain(refs)
                .collect(),
        }
    }

    /// What GitHub has of the tag a release's form chose.
    fn also_reads(&self, key: &DataKey) -> bool {
        match (self, key) {
            (NewRelease(repo) | EditRelease { repo, .. }, DataKey::TagLook(of, _)) => of == repo,
            _ => false,
        }
    }

    fn paged(&self) -> Option<DataKey> {
        match self {
            Releases(repo) => Some(DataKey::Releases(repo.clone())),
            _ => None,
        }
    }

    fn width(&self) -> Width {
        match self {
            Releases(_) => Width::Lists,
            Release { .. } | NewRelease(_) | EditRelease { .. } => Width::Reading,
        }
    }

    fn starts_on_row(&self) -> bool {
        !matches!(self, Release { .. })
    }

    fn form(&self) -> Option<&'static crate::forms::Kind> {
        matches!(self, NewRelease(_) | EditRelease { .. }).then_some(&crate::release::FORM)
    }

    fn build(&self, b: Build<'_>, page: &mut crate::ui::page::Page) {
        let now = b.cx.now;
        let title = self.title();
        let form = |page: &mut crate::ui::page::Page| {
            crate::release::build(
                b.state,
                page,
                &Route::Releases(self.clone()),
                self.repo_id(),
            );
        };
        match self {
            Releases(repo) => pages::releases(page, repo, b.list(self.paged()), now),
            Release { repo, tag } => {
                let key = DataKey::Release(repo.clone(), tag.clone());
                if let Some(r) = b.get(&key).show(page, &title) {
                    pages::release(page, repo, r, now);
                }
            }
            NewRelease(repo) => {
                let header = b.get::<RepoOverview>(&DataKey::Repo(repo.clone()));
                if header.show(page, &title).is_some() {
                    form(page);
                }
            }
            EditRelease { repo, tag } => {
                let key = DataKey::Release(repo.clone(), tag.clone());
                if b.get::<Data>(&key).show(page, &title).is_some() {
                    form(page);
                }
            }
        }
    }
}

pub(super) fn parse_repo(repo: &RepoId, rest: &[&str], _: &Url<'_>) -> Option<Target> {
    let repo = repo.clone();
    let page = match rest {
        ["releases", "tag", tag @ ..] if !tag.is_empty() => Release {
            repo,
            tag: tag.join("/"),
        },
        ["releases", "new"] => NewRelease(repo),
        ["releases", "edit", tag @ ..] if !tag.is_empty() => EditRelease {
            repo,
            tag: tag.join("/"),
        },
        // Assets are downloads.
        ["releases", "download", ..] => return None,
        ["releases", ..] => Releases(repo),
        _ => return None,
    };
    Some(Target::Page(Route::Releases(page)))
}

#[cfg(test)]
pub(crate) fn arb() -> impl proptest::strategy::Strategy<Value = Route> {
    use crate::route::arb::repo;
    use proptest::prelude::*;
    let tag = || "[A-Za-z0-9._-]{1,12}".prop_filter("not . or ..", |t| t != "." && t != "..");
    let page = prop_oneof![
        repo().prop_map(Releases),
        repo().prop_map(NewRelease),
        (repo(), tag()).prop_map(|(repo, tag)| Release { repo, tag }),
        (repo(), tag()).prop_map(|(repo, tag)| EditRelease { repo, tag }),
    ];
    page.prop_map(Route::Releases)
}
