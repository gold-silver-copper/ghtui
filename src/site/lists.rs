//! Putting a repository in your starred lists or taking it out (`,B`,
//! GitHub's star button's list menu), making a list on the way, and
//! deleting one of your lists (`,D` on its page). Each goes to GitHub in
//! the background ([`crate::work`]), and the page follows GitHub until
//! it shows the change.

use crate::api::lists::{ListPage, MyLists, StarList};
use crate::api::model::{Capped, NodeId, RepoId};
use crate::follow::{Expectation, Shown};
use crate::keymap::Action;
use crate::nav::{Doable, row};
use crate::picker::fuzzy_score;
use crate::picker::{Choice, entry};
use crate::route::Route;
use crate::state::{Cmds, State};
use crate::work::{self, Cx, Failure, Job};

use super::Page;
use crate::data::DataKey;

/// What's sent: the lists a repository is to be in, after making a new
/// one if asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetLists {
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
pub struct DeleteList {
    pub login: String,
    pub slug: String,
    pub id: NodeId,
    pub name: String,
}

impl SetLists {
    /// What it does, as the status bar says it.
    fn said(&self) -> String {
        match (&self.create, self.add) {
            (Some(name), _) => format!("Made the list “{name}” and added {} to it", self.repo),
            (None, true) => format!("Added {} to “{}”", self.repo, self.list),
            (None, false) => format!("Took {} out of “{}”", self.repo, self.list),
        }
    }

    /// The picker's row that sends it.
    fn chosen(self) -> Choice {
        Choice::Step(crate::act::Step::Start(
            format!("{}…", self.said()),
            self.into(),
        ))
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

impl Expectation for Expected {
    fn needs(&self, _: &State, _: &Route) -> Vec<DataKey> {
        match self {
            Expected::Membership { repo, .. } => {
                vec![DataKey::MyLists(repo.clone())]
            }
            Expected::Gone { login, .. } => {
                vec![DataKey::StarLists(login.to_lowercase())]
            }
        }
    }

    fn waiting(&self) -> &'static str {
        match self {
            Expected::Membership { .. } => "Waiting for GitHub to show the list",
            Expected::Gone { .. } => "Waiting for GitHub to show the list gone",
        }
    }

    fn shown(&self, state: &State, _: &Route) -> Shown {
        match self {
            Expected::Membership { repo, list, add } => state
                .picked::<MyLists>(&DataKey::MyLists(repo.clone()))
                .is_some_and(|m| {
                    m.lists
                        .iter()
                        .any(|l| l.name == *list && l.has == Some(*add))
                        || (!add && m.lists.iter().all(|l| l.name != *list))
                }),
            Expected::Gone { login, slug } => state
                .picked::<Capped<StarList>>(&DataKey::StarLists(login.to_lowercase()))
                .is_some_and(|lists| lists.iter().all(|l| l.slug != *slug)),
        }
        .into()
    }
}

/// The repository `,B` is about: the one on screen.
fn subject(state: &State) -> Option<RepoId> {
    state.route()?.repo().cloned()
}

/// `,B`: your lists, the ones with the repository checked. What GitHub
/// has now, not what was read before.
pub fn open(state: &mut State) -> Cmds {
    let Some(repo) = subject(state) else {
        return state.just_say(Action::Lists.not_here());
    };
    let mut cmds = state.ensure(DataKey::MyLists(repo.clone()), true);
    let pick = crate::picker::Kind::Site(super::Pick::Lists(repo));
    cmds.extend(state.open_picker(pick));
    cmds
}

/// The picker's rows: each of your lists that fits what's typed, checked
/// if the repository's in it; and, for a name no list has, making it.
pub fn rows(state: &State, repo: &RepoId, q: &str) -> crate::picker::Rows {
    let key = DataKey::MyLists(repo.clone());
    let mine = match crate::picker::listed::<MyLists, _>(state, &key, "your lists") {
        Ok(m) => m,
        Err(rows) => return rows,
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
                return entry(label, hint, None);
            };
            let mut lists = ids(Some(&l.id));
            if !has {
                lists.push(l.id.clone());
            }
            let work = SetLists {
                repo: repo.clone(),
                item: mine.repo.clone(),
                lists,
                create: None,
                list: l.name.clone(),
                add: !has,
            };
            entry(label, hint, Some(work.chosen()))
        })
        .collect();
    let name = q.trim();
    if !name.is_empty()
        && mine
            .lists
            .iter()
            .all(|l| !l.name.eq_ignore_ascii_case(name))
    {
        let work = SetLists {
            repo: repo.clone(),
            item: mine.repo.clone(),
            lists: ids(None),
            create: Some(name.to_owned()),
            list: name.to_owned(),
            add: true,
        };
        let label = format!("+ Make the list “{name}” and add it");
        rows.push(entry(label, "↵", Some(work.chosen())));
    }
    if mine.lists.left_out() > 0 {
        let n = mine.lists.left_out();
        rows.push(entry(
            format!("{n} more lists on GitHub, not listed"),
            "",
            None,
        ));
    }
    if rows.is_empty() {
        rows.push(entry(
            "You have no lists: type a name to make one",
            "",
            None,
        ));
    }
    rows
}

impl Job for SetLists {
    type Out = ();

    async fn run(&self, cx: &Cx) -> Result<(), Failure> {
        let gh = &cx.gh;
        let mut lists = self.lists.clone();
        if let Some(name) = &self.create {
            lists.push(gh.create_list(name).await?);
        }
        Ok(gh.set_lists(&self.item, &lists).await?)
    }

    /// GitHub's answer: your lists are read again until they show it.
    fn done(self, state: &mut State, result: Result<(), Failure>) -> Cmds {
        match result {
            Ok(()) => {
                state.info(self.said());
                if let Some(route) = state.route().cloned() {
                    let what = Expected::Membership {
                        repo: self.repo.clone(),
                        list: self.list,
                        add: self.add,
                    };
                    crate::follow::expect(state, route, what);
                }
                let mut cmds = state.ensure(DataKey::MyLists(self.repo), true);
                cmds.extend(state.load_visible(false));
                cmds
            }
            Err(why) => {
                state.error(refused("change your lists", &why));
                Cmds::new()
            }
        }
    }
}

/// What GitHub's refusal to `what` means: changing lists takes a
/// classic token's `user` scope, which gh doesn't ask for.
fn refused(what: &str, why: &Failure) -> String {
    let why = why.to_string();
    if why.contains("['user']") {
        crate::api::scope_missing("Changing your lists", "user")
    } else {
        format!("GitHub didn't {what}: {why}")
    }
}

/// The list on screen, if it's yours.
fn yours<'a>(state: &'a State, login: &str) -> Result<&'a ListPage, String> {
    let list = (state.route())
        .and_then(|r| r.family().paged())
        .and_then(|k| state.picked::<ListPage>(&k));
    let Some(list) = list else { return Err("it hasn't loaded yet".into()) };
    match &state.viewer {
        Some(me) if me.eq_ignore_ascii_case(login) => Ok(list),
        _ => Err(format!("it's @{login}'s list, not yours")),
    }
}

/// `,D` on one of your lists: asks first.
pub fn ask_delete(state: &mut State) -> Cmds {
    let Some(route @ Route::Site(Page::StarList { login, .. })) = state.route() else {
        return Cmds::new();
    };
    let list = yours(state, login).map(|l| l.list.clone());
    let route = route.clone();
    let list = match list {
        Ok(list) => list,
        Err(why) => return state.just_say(format!("Can't delete the list: {why}")),
    };
    let n = list.count;
    let facts = vec![(
        format!(
            "It has {n} {}; they stay starred. It can't be brought back",
            crate::ui::text::noun(n, "repository")
        ),
        true,
    )];
    crate::forms::ask(
        state,
        route,
        format!("Delete the list “{}”?", list.name),
        facts,
        vec![(
            "Delete list".into(),
            crate::forms::Answer::Site(super::Answer::DeleteList),
        )],
    )
}

/// Deletes the list on `route`'s page, as asked.
pub fn delete(state: &mut State, route: &Route) -> Cmds {
    let Route::Site(Page::StarList { login, slug }) = route else { return Cmds::new() };
    let Ok(list) = yours(state, login).map(|l| l.list.clone()) else {
        return Cmds::new();
    };
    crate::forms::sending(state);
    let delete = DeleteList {
        login: login.clone(),
        slug: slug.clone(),
        id: list.id,
        name: list.name,
    };
    work::start(delete)
}

impl Job for DeleteList {
    type Out = ();

    async fn run(&self, cx: &Cx) -> Result<(), Failure> {
        Ok(cx.gh.delete_list(&self.id).await?)
    }

    /// GitHub's answer to deleting a list: your Stars follow until it's gone.
    fn done(self, state: &mut State, result: Result<(), Failure>) -> Cmds {
        let d = self;
        let route = Route::Site(Page::StarList {
            login: d.login.clone(),
            slug: d.slug.clone(),
        });
        match result {
            Ok(()) => {
                crate::forms::close_sender(state, &route);
                state.info(format!("Deleted the list “{}”", d.name));
                if state.route() != Some(&route) {
                    return Cmds::new();
                }
                let stars = super::stars(&d.login);
                let gone = Expected::Gone {
                    login: d.login,
                    slug: d.slug,
                };
                crate::follow::replace(state, stars, gone)
            }
            Err(why) => crate::forms::say(state, refused("delete the list", &why)),
        }
    }
}

/// The menu's row for `,B`, on a repository's pages.
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
