//! Working a project from its pages: going between a board's columns,
//! moving an item to another column, and setting its fields. A picker
//! lists the choices; the one taken goes to GitHub as a [`Change`], and
//! the page follows GitHub until it shows it ([`crate::act::follow`],
//! which asks [`shows`]).

use crate::api::change::Change;
use crate::api::model::NodeId;
use crate::api::project::{
    FieldKind, FieldValue, Layout, Project, ProjectField, ProjectItem, ProjectRef, grouped,
};
use crate::ui::overlays::PaletteItem;
use crate::ui::page::Link;
use crate::ui::pages::item_link;

use crate::act::{By, Subject, awaiting_here};
use crate::browse::DataKey;
use crate::keymap::Action;
use crate::nav::Doable;
use crate::picker::{Kind, fuzzy_score};
use crate::route::Route;
use crate::state::{Api, Cmd, Screen, State};

/// The actions on a project's pages.
pub const ACTIONS: [Action; 4] = [
    Action::PrevColumn,
    Action::NextColumn,
    Action::MoveItem,
    Action::SetField,
];

/// An item of a project at a view: what a change is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct At {
    pub project: ProjectRef,
    pub view: Option<u64>,
    pub item: NodeId,
}

/// What a project's picker lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// The board's columns, to move the item to.
    Column(At),
    /// The item's fields, to set one.
    Field(At),
    /// The values of one of its fields.
    Value(At, NodeId),
}

impl Pick {
    pub fn title(&self) -> &'static str {
        match self {
            Pick::Column(_) => "Move to column",
            Pick::Field(_) => "Set a field",
            Pick::Value(..) => "Set the field",
        }
    }

    pub fn placeholder(&self) -> &'static str {
        match self {
            Pick::Column(_) => "Find a column",
            Pick::Field(_) => "Find a field",
            Pick::Value(..) => "Find a value, or type one",
        }
    }
}

/// What choosing a row does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Chosen {
    /// List this field's values.
    Field(At, NodeId),
    Set(At, Change),
}

/// The project on screen at its view, as loaded, and its item that's
/// selected (on a view) or shown (on an item's page).
struct Here<'a> {
    project: &'a ProjectRef,
    view: Option<u64>,
    data: Option<&'a Project>,
    item: Option<&'a ProjectItem>,
    /// On a view (not an item's page).
    on_view: bool,
}

fn here(state: &State) -> Option<Here<'_>> {
    let Screen::Page(p) = state.screen() else {
        return None;
    };
    let (project, view, number) = match &p.route {
        Route::Project { project, view } => (project, *view, None),
        Route::ProjectItem {
            project,
            view,
            item,
        } => (project, *view, Some(*item)),
        _ => return None,
    };
    let data = state.picked::<Project>(&DataKey::Project(project.clone(), view));
    let item = data.and_then(|d| match number {
        Some(n) => d.item(n),
        None => {
            let url = p.selected_link()?.url()?;
            (d.items.items.iter()).find(|it| item_link(project, view, it).as_deref() == Some(url))
        }
    });
    Some(Here {
        project,
        view,
        data,
        item,
        on_view: number.is_none(),
    })
}

/// The board on screen and its column field, or why it isn't one.
fn board<'a>(h: &Here<'a>) -> Result<(&'a Project, &'a ProjectField), String> {
    let p = h.data.ok_or("The project hasn't loaded yet")?;
    let layout = p.shown().map(|v| v.layout);
    match (layout, p.column_field()) {
        (Some(Layout::Board), Some(field)) => Ok((p, field)),
        (Some(Layout::Board), None) => Err("This board has no field to make columns of".into()),
        _ => Err("Columns are a board's: this view is a table".into()),
    }
}

/// What a change to the item here is about, or why there's none to make.
fn editable<'a>(state: &State, h: &Here<'a>) -> Result<(&'a Project, &'a ProjectItem), String> {
    let p = h.data.ok_or("The project hasn't loaded yet")?;
    if !p.can_update {
        return Err(
            "GitHub doesn't let you change this project: that takes write access to it".into(),
        );
    }
    if let Some(waiting) = awaiting_here(state)
        && matches!(waiting.change, Change::SetProjectField { .. })
    {
        return Err("GitHub hasn't shown the last change yet: wait a moment".into());
    }
    let Some(item) = h.item else {
        let down = state.first_key(Action::Down);
        return Err(format!("No item is selected: {down} selects one"));
    };
    if matches!(item.content, crate::api::project::ItemContent::Redacted) {
        return Err("You can't see this item, so you can't change it".into());
    }
    Ok((p, item))
}

/// Why `action` can't be done here, if it can't.
fn why_not(state: &State, h: &Here<'_>, action: Action) -> Option<String> {
    match action {
        Action::PrevColumn | Action::NextColumn if !h.on_view => {
            Some("Columns are a board's: open the project's view".into())
        }
        Action::PrevColumn | Action::NextColumn => board(h).err(),
        Action::MoveItem => board(h).and_then(|_| editable(state, h).map(drop)).err(),
        Action::SetField => {
            let (p, _) = match editable(state, h) {
                Ok(found) => found,
                Err(why) => return Some(why),
            };
            (!p.fields.iter().any(|f| f.kind.settable())).then(|| {
                "The project has no fields to set: only its own text, number, date, single-select and iteration fields can be"
                    .to_owned()
            })
        }
        _ => Some(action.not_here()),
    }
}

/// The menu's rows for a project's pages.
pub fn doables(state: &State, out: &mut Vec<Doable>) {
    let Some(h) = here(state) else {
        return;
    };
    let rows = [
        (Action::PrevColumn, "This page", "Previous column", ""),
        (Action::NextColumn, "This page", "Next column", "columns"),
        (
            Action::MoveItem,
            "Change",
            "Move the item to another column",
            "move",
        ),
        (
            Action::SetField,
            "Change",
            "Set a field of the item",
            "set field",
        ),
    ];
    for (action, section, label, short) in rows {
        // An item's page has no columns to go between.
        if !h.on_view && matches!(action, Action::PrevColumn | Action::NextColumn) {
            continue;
        }
        out.push(Doable {
            section,
            action,
            label: label.to_owned(),
            short,
            unavailable: why_not(state, &h, action),
        });
    }
}

/// Runs one of [`ACTIONS`] on a project's page.
#[must_use]
pub fn act(state: &mut State, action: Action) -> Vec<Cmd> {
    let Some(h) = here(state) else {
        state.info(action.not_here());
        return Vec::new();
    };
    if let Some(why) = why_not(state, &h, action) {
        state.info(why);
        return Vec::new();
    }
    let (project, view) = (h.project.clone(), h.view);
    let item = h.item.map(|it| it.id.clone());
    match (action, item) {
        (Action::PrevColumn | Action::NextColumn, _) => {
            column_step(state, action == Action::NextColumn);
            Vec::new()
        }
        (Action::MoveItem, Some(item)) => {
            let at = At {
                project,
                view,
                item,
            };
            state.open_picker(Kind::Project(Pick::Column(at)))
        }
        (_, Some(item)) => {
            let at = At {
                project,
                view,
                item,
            };
            state.open_picker(Kind::Project(Pick::Field(at)))
        }
        (_, None) => Vec::new(),
    }
}

/// `h` and `l`: the card at the same place in the column before or after
/// (the nearest that has cards), or the first card of the board.
fn column_step(state: &mut State, next: bool) {
    let height = state.page_height();
    let Some(h) = here(state) else {
        return;
    };
    let Ok((p, field)) = board(&h) else {
        return;
    };
    let items = p.sorted();
    let columns: Vec<Vec<String>> = grouped(field, &items)
        .iter()
        .map(|c| {
            (c.items.iter())
                .filter_map(|it| item_link(h.project, h.view, it))
                .collect()
        })
        .filter(|c: &Vec<String>| !c.is_empty())
        .collect();
    let at = h
        .item
        .and_then(|it| item_link(h.project, h.view, it))
        .and_then(|url| {
            columns
                .iter()
                .enumerate()
                .find_map(|(c, cards)| Some((c, cards.iter().position(|u| *u == url)?)))
        });
    let to = match at {
        Some((c, row)) => {
            let c = if next {
                c.checked_add(1)
            } else {
                c.checked_sub(1)
            };
            c.and_then(|c| columns.get(c)).map(|cards| {
                let row = row.min(cards.len().saturating_sub(1));
                cards.get(row)
            })
        }
        None => Some(columns.first().and_then(|cards| cards.first())),
    };
    let Some(Some(url)) = to else {
        state.info(if next {
            "No column to the right has cards"
        } else {
            "No column to the left has cards"
        });
        return;
    };
    let link = Link::Url(url.clone());
    if let Screen::Page(p) = state.screen_mut()
        && let Some(i) = p.page().find(&link, p.selected.unwrap_or(0))
    {
        p.select(i, height);
    }
}

/// A picker's rows: what each says, and what choosing it does.
pub fn rows(state: &State, pick: &Pick, q: &str) -> Vec<(PaletteItem, Option<Chosen>)> {
    let row = |label: String, hint: &str, chosen: Option<Chosen>| {
        let item = PaletteItem {
            label,
            hint: hint.to_owned(),
        };
        (item, chosen)
    };
    let at = match pick {
        Pick::Column(at) | Pick::Field(at) | Pick::Value(at, _) => at,
    };
    let key = DataKey::Project(at.project.clone(), at.view);
    let found = state.picked::<Project>(&key).and_then(|p| {
        let item = p.items.items.iter().find(|it| it.id == at.item)?;
        Some((p, item))
    });
    let Some((p, it)) = found else {
        return vec![row("The item is no longer in the view".into(), "", None)];
    };
    let set = |field: &ProjectField, value: Option<FieldValue>| {
        let said = match &value {
            Some(v) => format!("{} to {}", field.name, v.text()),
            None => field.name.clone(),
        };
        let change = Change::SetProjectField {
            project: p.id.clone(),
            item: it.id.clone(),
            field: field.id.clone(),
            value,
            said,
        };
        Some(Chosen::Set(at.clone(), change))
    };
    let field = match pick {
        Pick::Column(_) => p.column_field(),
        Pick::Value(_, id) => p.field(id),
        Pick::Field(_) => {
            return (p.fields.iter())
                .filter(|f| f.kind.settable() && fuzzy_score(q, &f.name).is_some())
                .map(|f| {
                    let hint = it.value(&f.id).map_or("", FieldValue::text);
                    let chosen = Chosen::Field(at.clone(), f.id.clone());
                    row(f.name.clone(), hint, Some(chosen))
                })
                .collect();
        }
    };
    let Some(field) = field else {
        return vec![row(
            "The field is no longer in the project".into(),
            "",
            None,
        )];
    };
    let current = it.value(&field.id);
    let is_current = |v: &FieldValue| current.is_some_and(|c| c.same(v));
    let mut rows = Vec::new();
    let mut choice = |value: FieldValue, hint: &str| {
        if fuzzy_score(q, value.text()).is_some() {
            let hint = if is_current(&value) { "current" } else { hint };
            rows.push(row(value.text().to_owned(), hint, set(field, Some(value))));
        }
    };
    match &field.kind {
        FieldKind::SingleSelect(options) => {
            for o in options {
                let value = FieldValue::Option {
                    id: o.id.clone(),
                    name: o.name.clone(),
                };
                choice(value, &o.description);
            }
        }
        FieldKind::Iteration(iterations) => {
            for i in iterations {
                let value = FieldValue::Iteration {
                    id: i.id.clone(),
                    title: i.title.clone(),
                };
                let hint = if i.completed {
                    format!("from {} · done", i.start)
                } else {
                    format!("from {}", i.start)
                };
                choice(value, &hint);
            }
        }
        FieldKind::Text | FieldKind::Number | FieldKind::Date => {
            rows.push(typed(field, q.trim(), set));
        }
        _ => {}
    }
    if current.is_some() {
        let label = match pick {
            Pick::Column(_) => format!("No {}", field.name),
            _ => format!("Clear {}", field.name),
        };
        if fuzzy_score(q, &label).is_some() {
            rows.push(row(label, "", set(field, None)));
        }
    }
    rows
}

/// The row a typed value makes: setting it, or why it can't be.
fn typed(
    field: &ProjectField,
    q: &str,
    set: impl Fn(&ProjectField, Option<FieldValue>) -> Option<Chosen>,
) -> (PaletteItem, Option<Chosen>) {
    let row = |label: String, chosen: Option<Chosen>| {
        let hint = if chosen.is_some() { "↵" } else { "" };
        let item = PaletteItem {
            label,
            hint: hint.to_owned(),
        };
        (item, chosen)
    };
    let value = match &field.kind {
        _ if q.is_empty() => return row(format!("Type the {}", field.name), None),
        FieldKind::Number if q.parse::<f64>().is_err() => {
            return row(format!("“{q}” isn't a number"), None);
        }
        FieldKind::Number => FieldValue::Number(q.to_owned()),
        FieldKind::Date if !is_date(q) => {
            return row("Type a date as 2026-10-31".into(), None);
        }
        FieldKind::Date => FieldValue::Date(q.to_owned()),
        _ => FieldValue::Text(q.to_owned()),
    };
    row(
        format!("Set {} to “{q}”", field.name),
        set(field, Some(value)),
    )
}

/// `2026-10-31`.
fn is_date(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    matches!(parts.as_slice(), [y, m, d]
        if y.len() == 4 && m.len() == 2 && d.len() == 2
            && parts.iter().all(|p| p.bytes().all(|b| b.is_ascii_digit()))
            && (1..=12).contains(&m.parse::<u8>().unwrap_or(0))
            && (1..=31).contains(&d.parse::<u8>().unwrap_or(0)))
}

/// What choosing a row does: list a field's values, or send the change.
#[must_use]
pub fn choose(state: &mut State, chosen: Chosen) -> Vec<Cmd> {
    match chosen {
        Chosen::Field(at, field) => state.open_picker(Kind::Project(Pick::Value(at, field))),
        Chosen::Set(at, change) => {
            if let Change::SetProjectField { said, value, .. } = &change {
                let doing = if value.is_some() {
                    "Setting"
                } else {
                    "Clearing"
                };
                state.info(format!("{doing} {said}…"));
            }
            let about = Subject::Project(at.project, at.view);
            vec![Cmd::Api(Api::Change(change, By::Act(about)))]
        }
    }
}

/// Whether the project at `view`, as loaded, shows `change`: the item has
/// the value now, or has left the view (its filter no longer finds it).
pub fn shows(state: &State, change: &Change, project: &ProjectRef, view: Option<u64>) -> bool {
    let Change::SetProjectField {
        item, field, value, ..
    } = change
    else {
        return true;
    };
    let key = DataKey::Project(project.clone(), view);
    let Some(p) = state.picked::<Project>(&key) else {
        return false;
    };
    let Some(it) = p.items.items.iter().find(|it| it.id == *item) else {
        return true;
    };
    match value {
        Some(value) => it.value(field).is_some_and(|v| v.same(value)),
        None => it.value(field).is_none(),
    }
}
