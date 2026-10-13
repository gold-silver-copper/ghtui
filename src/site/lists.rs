//! Putting a repository in your starred lists or taking it out (`,L`,
//! GitHub's star button's list menu), making a list on the way, and
//! deleting one of your lists (`,D` on its page). Each goes to GitHub in
//! the background ([`crate::writes`]), and the page follows GitHub until
//! it shows the change.

use crate::api::GitHub;
use crate::api::lists::{ListPage, MyLists, StarList};
use crate::api::model::{Capped, NodeId, RepoId};
use crate::browse::{Need, Picked};
use crate::keymap::Action;
use crate::nav::Doable;
use crate::picker::fuzzy_score;
use crate::route::Route;
use crate::state::{Cmd, Remote, State};
use crate::writes::{Chosen, Pick, item, row};

use super::{Key, Page, site};

/// What's sent: the lists a repository is to be in, after making a new
/// one if asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Work {
    pub repo: RepoId,
    pub item: NodeId,
    /// Every list of yours it's to be in.
    pub lists: Vec<NodeId>,
    /// A list to make first, and put it in.
    pub create: Option<String>,
    /// The list changed, and whether the repository goes in.
    pub list: String,
    pub add: bool,
}

/// Deleting one of your lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delete {
    pub login: String,
    pub slug: String,
    pub id: NodeId,
    pub name: String,
}

impl Work {
    pub async fn run(self, gh: &GitHub) -> Result<(), String> {
        let mut lists = self.lists.clone();
        if let Some(name) = &self.create {
            let id = gh
                .create_list(name)
                .await
                .map_err(|e| crate::writes::reason(&e))?;
            lists.push(id);
        }
        let set = gh.set_lists(&self.item, &lists).await;
        set.map_err(|e| crate::writes::reason(&e))
    }

    /// What it does, as the status bar says it.
    fn said(&self) -> String {
        match (&self.create, self.add) {
            (Some(name), _) => format!("Made the list “{name}” and added {} to it", self.repo),
            (None, true) => format!("Added {} to “{}”", self.repo, self.list),
            (None, false) => format!("Took {} out of “{}”", self.repo, self.list),
        }
    }
}

impl Delete {
    pub async fn run(self, gh: &GitHub) -> Result<(), String> {
        gh.delete_list(&self.id)
            .await
            .map_err(|e| crate::writes::reason(&e))
    }
}

/// What the page shows once GitHub has the change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expected {
    /// The repository in (or out of) the list of this name.
    Membership {
        repo: RepoId,
        list: String,
        add: bool,
    },
    /// The list no longer among its owner's.
    Gone { login: String, slug: String },
}

impl Expected {
    pub fn needs(&self) -> Vec<Need> {
        match self {
            Expected::Membership { repo, .. } => {
                vec![Need::Data(site(Key::MyLists(repo.clone())))]
            }
            Expected::Gone { login, .. } => {
                vec![Need::Data(site(Key::StarLists(login.to_lowercase())))]
            }
        }
    }

    pub fn waiting(&self) -> &'static str {
        match self {
            Expected::Membership { .. } => "Waiting for GitHub to show the list",
            Expected::Gone { .. } => "Waiting for GitHub to show the list gone",
        }
    }

    pub fn shown(&self, state: &State) -> bool {
        match self {
            Expected::Membership { repo, list, add } => state
                .picked::<MyLists>(&site(Key::MyLists(repo.clone())))
                .is_some_and(|m| {
                    m.lists
                        .iter()
                        .any(|l| l.name == *list && l.has == Some(*add))
                        || (!add && m.lists.iter().all(|l| l.name != *list))
                }),
            Expected::Gone { login, slug } => state
                .picked::<Capped<StarList>>(&site(Key::StarLists(login.to_lowercase())))
                .is_some_and(|lists| lists.iter().all(|l| l.slug != *slug)),
        }
    }
}

/// The repository `,L` is about: the one on screen.
fn subject(state: &State) -> Option<RepoId> {
    state.route()?.repo().cloned()
}

/// `,L`: your lists, the ones with the repository checked. What GitHub
/// has now, not what was read before.
#[must_use]
pub fn open(state: &mut State) -> Vec<Cmd> {
    let Some(repo) = subject(state) else {
        state.info(Action::Lists.not_here());
        return Vec::new();
    };
    let mut cmds = state.ensure(Need::Data(site(Key::MyLists(repo.clone()))), true);
    cmds.extend(crate::writes::open_picker(
        state,
        Pick::Site(super::Pick::Lists(repo)),
    ));
    cmds
}

/// The picker's rows: each of your lists that fits what's typed, checked
/// if the repository's in it; and, for a name no list has, making it.
pub fn rows(
    state: &State,
    repo: &RepoId,
    q: &str,
) -> Vec<(crate::ui::overlays::PaletteItem, Option<Chosen>)> {
    let key = site(Key::MyLists(repo.clone()));
    #[expect(clippy::disallowed_methods, reason = "said as the picker's row")]
    let fetched = Remote::fetched(state.data.get(&key), "")
        .pick(<MyLists as Picked>::pick)
        .text("your lists");
    let mine = match fetched {
        Ok(m) => m,
        Err(why) => return vec![item(why, "", None)],
    };
    let ids = |except: Option<&NodeId>| -> Vec<NodeId> {
        (mine.lists.iter())
            .filter(|l| l.has == Some(true) && Some(&l.id) != except)
            .map(|l| l.id.clone())
            .collect()
    };
    let mut rows: Vec<_> = (mine.lists.iter())
        .filter(|l| fuzzy_score(q, &l.name).is_some())
        .map(|l| {
            let mark = if l.has == Some(true) { "✓" } else { " " };
            let noun = crate::ui::text::noun(l.count, "repository");
            let mut hint = format!("{} {noun}", l.count);
            if l.private {
                hint.push_str(" · private");
            }
            let label = format!("{mark} {}", l.name);
            let Some(has) = l.has else {
                let hint = "too long for ghtui to say whether it's in it";
                return item(label, hint, None);
            };
            let mut lists = ids(Some(&l.id));
            if !has {
                lists.push(l.id.clone());
            }
            let work = Work {
                repo: repo.clone(),
                item: mine.repo.clone(),
                lists,
                create: None,
                list: l.name.clone(),
                add: !has,
            };
            item(label, hint, Some(Chosen::Site(super::Chosen::Lists(work))))
        })
        .collect();
    let name = q.trim();
    if !name.is_empty()
        && mine
            .lists
            .iter()
            .all(|l| !l.name.eq_ignore_ascii_case(name))
    {
        let work = Work {
            repo: repo.clone(),
            item: mine.repo.clone(),
            lists: ids(None),
            create: Some(name.to_owned()),
            list: name.to_owned(),
            add: true,
        };
        let label = format!("+ Make the list “{name}” and add it");
        rows.push(item(
            label,
            "↵",
            Some(Chosen::Site(super::Chosen::Lists(work))),
        ));
    }
    if mine.lists.left_out() > 0 {
        let n = mine.lists.left_out();
        rows.push(item(
            format!("{n} more lists on GitHub, not listed"),
            "",
            None,
        ));
    }
    if rows.is_empty() {
        rows.push(item("You have no lists: type a name to make one", "", None));
    }
    rows
}

/// Sends a list chosen in the picker.
#[must_use]
pub fn choose(state: &mut State, work: Work) -> Vec<Cmd> {
    state.info(format!("{}…", work.said()));
    vec![Cmd::Write(crate::writes::Work::Site(super::Work::Lists(
        work,
    )))]
}

/// GitHub's answer: your lists are read again until they show it.
#[must_use]
pub fn on_done(state: &mut State, work: Work, result: Result<(), String>) -> Vec<Cmd> {
    match result {
        Ok(()) => {
            state.info(work.said());
            if let Some(route) = state.route().cloned() {
                let what = Expected::Membership {
                    repo: work.repo.clone(),
                    list: work.list.clone(),
                    add: work.add,
                };
                crate::writes::expect(
                    state,
                    route,
                    crate::writes::Expected::Site(super::Expected::Lists(what)),
                );
            }
            let mut cmds = state.ensure(Need::Data(site(Key::MyLists(work.repo))), true);
            cmds.extend(state.load_visible(false));
            cmds
        }
        Err(why) => {
            state.error(format!("GitHub didn't change your lists: {why}"));
            Vec::new()
        }
    }
}

/// The list on screen, if it's yours.
fn yours<'a>(state: &'a State, login: &str) -> Result<&'a ListPage, String> {
    let route = state.route().cloned();
    let list = route
        .and_then(|r| crate::browse::paged(&r))
        .and_then(|k| state.picked::<ListPage>(&k));
    let Some(list) = list else {
        return Err("it hasn't loaded yet".into());
    };
    match &state.viewer {
        Some(me) if me.eq_ignore_ascii_case(login) => Ok(list),
        _ => Err(format!("it's @{login}'s list, not yours")),
    }
}

/// `,D` on one of your lists: asks first.
#[must_use]
pub fn ask_delete(state: &mut State) -> Vec<Cmd> {
    let Some(route @ Route::Site(Page::StarList { .. })) = state.route().cloned() else {
        return Vec::new();
    };
    let Route::Site(Page::StarList { login, .. }) = &route else {
        return Vec::new();
    };
    let list = match yours(state, login) {
        Ok(l) => l.list.clone(),
        Err(why) => {
            state.info(format!("Can't delete the list: {why}"));
            return Vec::new();
        }
    };
    let n = list.count;
    let facts = vec![(
        format!(
            "It has {n} {}; they stay starred. It can't be brought back",
            crate::ui::text::noun(n, "repository")
        ),
        true,
    )];
    crate::writes::ask(
        state,
        route,
        format!("Delete the list “{}”?", list.name),
        facts,
        vec![(
            "Delete list".into(),
            crate::writes::Answer::Site(super::Answer::DeleteList),
        )],
    );
    Vec::new()
}

/// Deletes the list on `route`'s page, as asked.
#[must_use]
pub fn delete(state: &mut State, route: &Route) -> Vec<Cmd> {
    let Route::Site(Page::StarList { login, slug }) = route else {
        return Vec::new();
    };
    let Ok(list) = yours(state, login).map(|l| l.list.clone()) else {
        return Vec::new();
    };
    crate::writes::sending(state);
    let delete = Delete {
        login: login.clone(),
        slug: slug.clone(),
        id: list.id,
        name: list.name,
    };
    vec![Cmd::Write(crate::writes::Work::Site(
        super::Work::DeleteList(delete),
    ))]
}

/// GitHub's answer to deleting a list: your Stars follow until it's gone.
#[must_use]
pub fn on_deleted(state: &mut State, d: Delete, result: Result<(), String>) -> Vec<Cmd> {
    let route = Route::Site(Page::StarList {
        login: d.login.clone(),
        slug: d.slug.clone(),
    });
    match result {
        Ok(()) => {
            crate::writes::close_sender(state, &route);
            state.info(format!("Deleted the list “{}”", d.name));
            if state.route() != Some(&route) {
                return Vec::new();
            }
            let stars = Route::User {
                login: d.login.clone(),
                tab: crate::ui::pages::ProfileTab::Stars,
            };
            let gone = super::Expected::Lists(Expected::Gone {
                login: d.login,
                slug: d.slug,
            });
            crate::writes::expect(state, stars.clone(), crate::writes::Expected::Site(gone));
            state.replace(stars, true, None)
        }
        Err(why) => {
            crate::writes::say(state, format!("GitHub didn't delete the list: {why}"));
            Vec::new()
        }
    }
}

/// The menu's row for `,L`, on a repository's pages.
pub fn doable(state: &State) -> Option<Doable> {
    subject(state)?;
    Some(row(
        "Change",
        Action::Lists,
        "Add the repository to your lists, or take it out",
        "",
        None,
    ))
}

/// The menu's rows on a list's page.
pub fn doables(state: &State, route: &Route, out: &mut Vec<Doable>) {
    if let Route::Site(Page::StarList { login, .. }) = route {
        let why = yours(state, login).err();
        out.push(row(
            "Edit",
            Action::DeleteComment,
            "Delete the list",
            "",
            why,
        ));
    }
}
