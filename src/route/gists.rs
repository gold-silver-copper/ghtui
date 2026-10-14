//! Gists: someone's, one gist, and the forms that make and change one.

use super::{Family, Route, Target, valid_login};
use crate::browse::{Build, Width};
use crate::chrome::Chrome;
use crate::data::{Data, DataKey};
use crate::state::State;
use crate::ui::pages::{self, url as links};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Page {
    /// A gist, by its ID (its owner, when the URL names them).
    Gist { owner: Option<String>, id: String },
    /// Someone's gists.
    Gists(String),
    /// A new gist's form.
    NewGist,
    /// The form changing one of your gists.
    EditGist { owner: String, id: String },
}

pub use Page::*;

impl Family for Page {
    fn url(&self) -> String {
        match self {
            Gist { owner, id } => match owner {
                Some(owner) => format!("{}/{owner}/{id}", links::gists()),
                None => format!("{}/{id}", links::gists()),
            },
            Gists(login) => format!("{}/{login}", links::gists()),
            NewGist => format!("{}/", links::gists()),
            EditGist { owner, id } => format!("{}/{owner}/{id}/edit", links::gists()),
        }
    }

    fn title(&self) -> String {
        match self {
            Gist { id, .. } => format!("Gist {}", id.get(..7).unwrap_or(id)),
            Gists(login) => format!("@{login} · Gists"),
            NewGist => "New gist".to_owned(),
            EditGist { id, .. } => format!("Editing gist {}", id.get(..7).unwrap_or(id)),
        }
    }

    fn up(&self, state: &State) -> Option<Route> {
        Some(Route::Gists(match self {
            Gist { owner, .. } => Gists(owner.clone()?),
            NewGist => Gists(state.viewer.clone()?),
            EditGist { owner, id } => Gist {
                owner: Some(owner.clone()),
                id: id.clone(),
            },
            Gists(login) => return Some(Route::user(login)),
        }))
    }

    fn needs(&self) -> Vec<DataKey> {
        match self {
            Gist { id, .. } | EditGist { id, .. } => vec![DataKey::Gist(id.clone())],
            Gists(_) => self.paged().into_iter().collect(),
            NewGist => Vec::new(),
        }
    }

    fn paged(&self) -> Option<DataKey> {
        match self {
            Gists(login) => Some(DataKey::Gists(login.to_lowercase())),
            _ => None,
        }
    }

    fn width(&self) -> Width {
        match self {
            Gist { .. } | Gists(_) => Width::Lists,
            NewGist | EditGist { .. } => Width::Reading,
        }
    }

    fn chrome(&self, state: &State, c: &mut Chrome) {
        let user = |login: &str| Some(Target::Page(Route::user(login)));
        match self {
            Gist { owner, id } => {
                // Its owner, from the gist when the URL doesn't say.
                let gist = match state.get(&DataKey::Gist(id.clone())) {
                    Some(Data::Gist(g)) => g.owner.clone(),
                    _ => None,
                };
                if let Some(owner) = owner.clone().or(gist) {
                    c.crumb(&owner, user(&owner));
                    c.crumb("Gists", Some(Target::Page(Route::Gists(Gists(owner)))));
                }
                c.crumb("Gist", None);
            }
            Gists(login) => {
                c.crumb(login, user(login));
                c.crumb("Gists", None);
            }
            NewGist | EditGist { .. } => {}
        }
    }

    fn build(&self, b: Build<'_>, page: &mut crate::ui::page::Page) {
        let now = b.cx.now;
        let route = || Route::Gists(self.clone());
        match self {
            Gist { id, .. } => {
                if let Some(g) = b.get(&DataKey::Gist(id.clone())).show(page, &self.title()) {
                    pages::gist(page, g, now);
                }
            }
            Gists(login) => pages::gists(page, login, b.list(self.paged()), now),
            NewGist => crate::gist::build(b.state, page, &route()),
            EditGist { id, .. } => {
                let gist = b.get::<crate::api::browse::Gist>(&DataKey::Gist(id.clone()));
                if gist.show(page, &self.title()).is_some() {
                    crate::gist::build(b.state, page, &route());
                }
            }
        }
    }
}

/// The gist website's own pages, not people's gists.
const GIST_PAGES: &[&str] = &[
    "discover", "starred", "search", "auth", "login", "join", "mine",
];

/// Whether a gist path segment after an owner is a gist's ID
/// (numbers for old gists, 20 or 32 hex digits for newer).
fn is_gist_id(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_digit())
        || (matches!(s.len(), 20 | 32) && s.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Whether a lone gist segment is a gist rather than someone's
/// gists. GitHub tries a login first; ghtui can't ask, so it takes short
/// numbers for logins (`/1`, `/2000`) and long ones for old gists
/// (`/1162032`, which GitHub redirects to its owner's).
fn is_lone_gist_id(s: &str) -> bool {
    is_gist_id(s) && !(s.len() < 5 && s.bytes().all(|b| b.is_ascii_digit()))
}

/// A gist website's page, from its path: a gist (`/<id>`, `/<owner>/<id>`)
/// or someone's gists (`/<owner>`). Raw files and embeds are downloads.
pub(super) fn parse<'a>(segments: impl Iterator<Item = &'a str>) -> Option<Target> {
    let segments: Vec<&str> = segments.filter(|s| !s.is_empty()).collect();
    let page = match segments.as_slice() {
        // The gist website's front page is the form for a new gist.
        [] => NewGist,
        [first, ..] if GIST_PAGES.contains(first) => return None,
        [owner, id, "edit"] if is_gist_id(id) && valid_login(owner) => EditGist {
            owner: (*owner).to_owned(),
            id: (*id).to_owned(),
        },
        [id] if is_lone_gist_id(id) => Gist {
            owner: None,
            id: (*id).to_owned(),
        },
        [login] if valid_login(login) => Gists((*login).to_owned()),
        // Revisions, forks, stars: the gist.
        [owner, id] | [owner, id, "revisions" | "forks" | "stargazers"]
            if is_gist_id(id) && valid_login(owner) =>
        {
            Gist {
                owner: Some((*owner).to_owned()),
                id: (*id).to_owned(),
            }
        }
        _ => return None,
    };
    Some(Target::Page(Route::Gists(page)))
}

#[cfg(test)]
pub(crate) fn arb() -> impl proptest::strategy::Strategy<Value = Route> {
    use crate::route::arb::login;
    use proptest::prelude::*;
    let page = prop_oneof![
        Just(NewGist),
        (login(), "[0-9a-f]{20}")
            .prop_filter("an owner isn't a gist ID or a gist page", |(o, _)| {
                !is_gist_id(o) && !GIST_PAGES.contains(&o.as_str())
            })
            .prop_map(|(owner, id)| EditGist { owner, id }),
        (prop::option::of(login()), "[0-9a-f]{20}|[0-9]{1,8}")
            .prop_filter("an owner isn't a gist ID or a gist page", |(o, _)| {
                o.as_deref()
                    .is_none_or(|o| !is_gist_id(o) && !GIST_PAGES.contains(&o))
            })
            .prop_filter("a lone ID is a gist's", |(o, id)| {
                o.is_some() || is_lone_gist_id(id)
            })
            .prop_map(|(owner, id)| Gist { owner, id }),
        login()
            .prop_filter("not a gist ID or a gist page", |l| {
                !is_lone_gist_id(l) && !GIST_PAGES.contains(&l.as_str())
            })
            .prop_map(Gists),
    ];
    page.prop_map(Route::Gists)
}
