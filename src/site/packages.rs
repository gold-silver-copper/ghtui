//! Deleting a package's version (`,D` on the version selected, or on a
//! version's page), after asking, as GitHub's own page does. GitHub
//! lets a package's admins do it, with a token's `delete:packages`
//! scope; ghtui can't know beforehand who's an admin, so a refusal says
//! what GitHub said, and which scope is missing.

use crate::api::packages::{PackageDetail, PackageRef};
use crate::follow::{Expectation, Shown};
use crate::keymap::Action;
use crate::nav::Doable;
use crate::route::{Dest, Route, Target};
use crate::state::{Cmd, Screen, State};
use crate::work::{self, Cx, Failure, Job};
use crate::writes::row;

use super::Page;
use crate::data::DataKey;

/// Deleting one version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteVersion {
    pub at: PackageRef,
    pub id: u64,
    pub name: String,
}

/// The version `,D` is about: the row selected, else the version whose
/// page this is.
fn chosen(state: &State) -> Option<(PackageRef, u64)> {
    let Screen::Page(p) = state.screen() else {
        return None;
    };
    let Route::Site(Page::Package { at, version }) = &p.route else {
        return None;
    };
    let row = p.selected_link().and_then(crate::ui::page::Link::url);
    let row = row.and_then(|url| match Dest::from_url(url).target {
        Target::Page(Route::Site(Page::Package {
            at: there,
            version: Some(id),
        })) if there == *at => Some(id),
        _ => None,
    });
    Some((at.clone(), row.or(*version)?))
}

fn detail<'a>(state: &'a State, at: &PackageRef) -> Option<&'a PackageDetail> {
    state.picked::<PackageDetail>(&DataKey::Package(at.clone()))
}

/// `,D`: asks before deleting the version.
#[must_use]
pub fn ask_delete(state: &mut State) -> Vec<Cmd> {
    let Some((at, id)) = chosen(state) else {
        state.info("Select a version to delete");
        return Vec::new();
    };
    let Some(d) = detail(state, &at) else {
        state.info("The package hasn't loaded yet");
        return Vec::new();
    };
    let Some(v) = d.versions.items.iter().find(|v| v.id == id) else {
        state.info("That version isn't among those loaded");
        return Vec::new();
    };
    let mut facts = vec![(
        "It can't be brought back: pulling it by name or digest fails".to_owned(),
        true,
    )];
    if !v.tags.is_empty() {
        facts.push((format!("Its tags go with it: {}", v.tags.join(", ")), true));
    }
    if d.package.versions <= 1 {
        facts.push((
            "It's the package's only version: GitHub may refuse, and ask that the package be deleted on github.com".to_owned(),
            true,
        ));
    }
    let Some(route) = state.route().cloned() else {
        return Vec::new();
    };
    // A digest by its first twelve characters, as GitHub shows it.
    let name = match v.name.strip_prefix("sha256:") {
        Some(hex) => format!("sha256:{}", hex.get(..12).unwrap_or(hex)),
        None => v.name.clone(),
    };
    crate::writes::ask(
        state,
        route,
        format!("Delete version {name} of {}?", at.name),
        facts,
        vec![(
            "Delete version".into(),
            crate::writes::Answer::Site(super::Answer::DeleteVersion(id)),
        )],
    );
    Vec::new()
}

/// Deletes version `id` of the package on `route`'s page, as asked.
#[must_use]
pub fn delete(state: &mut State, route: &Route, id: u64) -> Vec<Cmd> {
    let Route::Site(Page::Package { at, .. }) = route else {
        return Vec::new();
    };
    let name = (detail(state, at).and_then(|d| d.versions.items.iter().find(|v| v.id == id)))
        .map_or_else(|| id.to_string(), |v| v.name.clone());
    crate::forms::sending(state);
    let delete = DeleteVersion {
        at: at.clone(),
        id,
        name,
    };
    work::start(delete)
}

impl Job for DeleteVersion {
    type Out = ();

    async fn run(&self, cx: &Cx) -> Result<(), Failure> {
        Ok(cx.gh.delete_version(&self.at, self.id).await?)
    }

    /// GitHub's answer: the package's page follows until the version is
    /// gone (from a version's own page, the package's).
    fn done(self, state: &mut State, result: Result<(), Failure>) -> Vec<Cmd> {
        let d = self;
        let Some(route) = state.route().cloned() else {
            return Vec::new();
        };
        match result {
            Ok(()) => {
                crate::forms::close_sender(state, &route);
                state.info(format!("Deleted version {} of {}", d.name, d.at.name));
                let page = Route::Site(Page::Package {
                    at: d.at.clone(),
                    version: None,
                });
                crate::follow::expect(state, page.clone(), VersionGone(d.at, d.id));
                if route == page {
                    state.ensure_route(&page, true)
                } else {
                    state.replace(page, true, None)
                }
            }
            Err(why) => {
                crate::forms::say(state, format!("GitHub didn't delete the version: {why}"));
                Vec::new()
            }
        }
    }
}

/// A package without this version.
#[derive(Debug)]
struct VersionGone(PackageRef, u64);

impl Expectation for VersionGone {
    fn needs(&self, _: &State, _: &Route) -> Vec<DataKey> {
        vec![DataKey::Package(self.0.clone())]
    }

    fn waiting(&self) -> &'static str {
        "Waiting for GitHub to show the version gone"
    }

    fn shown(&self, state: &State, _: &Route) -> Shown {
        let gone = |d: &PackageDetail| d.versions.items.iter().all(|v| v.id != self.1);
        detail(state, &self.0).is_some_and(gone).into()
    }
}

/// The menu's rows on a package's page.
pub fn doables(state: &State, route: &Route, out: &mut Vec<Doable>) {
    if let Route::Site(Page::Package { .. }) = route {
        let why = chosen(state)
            .is_none()
            .then(|| "select a version first".to_owned());
        let label = "Delete the version (needs the delete:packages scope)";
        out.push(row("Edit", Action::DeleteComment, label, "", why));
    }
}
