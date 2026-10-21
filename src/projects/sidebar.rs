//! Setting an issue's or pull request's fields in its projects from its
//! own page, as its sidebar on GitHub does, without opening the board:
//! `,f` picks the project (when it's in several you may change), then
//! the field and its value, with the pickers a board's `,f` uses. The
//! page follows GitHub until its sidebar shows the value.

use crate::act::{Subject, awaiting_here};
use crate::api::change::Change;
use crate::api::model::{Capped, NodeId};
use crate::api::project::InProject;
use crate::data::DataKey;
use crate::keymap::Action;
use crate::nav::Doable;
use crate::picker::{Kind, fuzzy_score};
use crate::route::{Route, issues, pulls};
use crate::state::{Cmds, State};

use super::{At, On, Pick};

/// The issue or pull request on screen, and where its projects are
/// loaded.
pub fn here(state: &State) -> Option<(Subject, DataKey)> {
    let subject = match state.route()? {
        Route::Issues(issues::Issue { repo, number }) => Subject::Issue(repo.clone(), *number),
        Route::Pulls(pulls::Pr { pr, .. }) => Subject::Pr(pr.clone()),
        _ => return None,
    };
    let key = key(&subject)?;
    Some((subject, key))
}

/// Where `subject`'s projects are loaded.
fn key(subject: &Subject) -> Option<DataKey> {
    match subject {
        Subject::Issue(repo, number) => Some(DataKey::InProjects(repo.clone(), *number)),
        Subject::Pr(pr) => Some(DataKey::InProjects(pr.repo.clone(), pr.number)),
        _ => None,
    }
}

/// `subject`'s item in `project`, as loaded.
pub(super) fn project<'a>(
    state: &'a State,
    subject: &Subject,
    project: &NodeId,
) -> Option<&'a InProject> {
    let list = state.picked::<Capped<InProject>>(&key(subject)?)?;
    list.iter().find(|p| p.id == *project)
}

/// The projects it's in that you may change, or why there are none.
fn changeable(state: &State) -> Result<(Subject, Vec<&InProject>), String> {
    let Some((subject, key)) = here(state) else {
        return Err(Action::SetField.not_here());
    };
    let Some(list) = state.picked::<Capped<InProject>>(&key) else {
        let failed = state.remote(&key).and_then(|r| r.error.clone());
        return Err(failed.unwrap_or_else(|| "Its projects haven't loaded yet".into()));
    };
    if list.is_empty() {
        return Err("It's in no project yet: a project's board adds it (Add an item)".into());
    }
    let mine: Vec<&InProject> = (list.iter())
        .filter(|p| p.can_update && p.fields.iter().any(|f| f.kind.settable()))
        .collect();
    if mine.is_empty() {
        return Err(if list.iter().any(|p| p.can_update) {
            "Its projects have no fields to set: only text, number, date, single-select and iteration fields can be".into()
        } else {
            "GitHub doesn't let you change its projects: that takes write access to them".into()
        });
    }
    if let Some(waiting) = awaiting_here(state)
        && matches!(waiting.change, Change::SetProjectField { .. })
    {
        return Err("GitHub hasn't shown the last change yet: wait a moment".into());
    }
    Ok((subject, mine))
}

/// The menu's row on an issue's or pull request's page.
pub fn doable(state: &State) -> Option<Doable> {
    here(state)?;
    let label = "Set a field in its project";
    let why = changeable(state).err();
    Some(crate::nav::row(
        "Change",
        Action::SetField,
        label,
        "set field",
        why,
    ))
}

/// `,f` on an issue or pull request: its field to set, in the one
/// project it's in, or first which project.
pub fn act(state: &mut State) -> Cmds {
    let (subject, projects) = match changeable(state) {
        Ok(found) => found,
        Err(why) => return state.just_say(why),
    };
    let pick = match projects.as_slice() {
        [one] => Pick::Field(At {
            on: On::Sidebar(subject, one.id.clone()),
            item: one.item.clone(),
        }),
        _ => Pick::Project(subject),
    };
    state.open_picker(Kind::Project(pick))
}

/// The projects to choose from.
pub fn project_rows(state: &State, subject: &Subject, q: &str) -> crate::picker::Rows {
    let list = key(subject).and_then(|k| state.picked::<Capped<InProject>>(&k));
    let Some(list) = list else {
        let item = crate::picker::item("Its projects haven't loaded yet", "");
        return vec![(item, None)];
    };
    (list.iter())
        .filter(|p| fuzzy_score(q, &p.title).is_some())
        .map(|p| {
            let hint = if p.can_update {
                p.status().unwrap_or_default()
            } else {
                "you can't change it"
            };
            let item = crate::picker::item(p.title.clone(), hint);
            let at = At {
                on: On::Sidebar(subject.clone(), p.id.clone()),
                item: p.item.clone(),
            };
            (item, p.can_update.then(|| super::listing(Pick::Field(at))))
        })
        .collect()
}

/// What following a change to `about`'s fields fetches again: its
/// projects, which its sidebar shows.
pub fn polls(change: &Change, about: &Subject) -> Option<DataKey> {
    matches!(change, Change::SetProjectField { .. }).then(|| key(about))?
}

/// Whether `about`'s sidebar shows the field set (or cleared).
pub fn shows(state: &State, change: &Change, about: &Subject) -> bool {
    let Change::SetProjectField {
        project,
        field,
        value,
        ..
    } = change
    else {
        return true;
    };
    // Out of the project meanwhile, there's nothing to show.
    let Some(p) = self::project(state, about, project) else {
        return key(about).is_some_and(|k| state.picked::<Capped<InProject>>(&k).is_some());
    };
    match value {
        Some(value) => p.value(field).is_some_and(|v| v.same(value)),
        None => p.value(field).is_none(),
    }
}
