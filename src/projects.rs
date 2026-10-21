//! Working a project from its pages: going between a board's columns,
//! moving an item to another column and setting its fields, adding an
//! item (an issue or pull request, or a draft), archiving and restoring
//! one, and editing a draft's title and body. A picker or the composer
//! takes what's chosen; it goes to GitHub as a [`Change`], and the page
//! follows GitHub until it shows it ([`crate::act::follow`], which asks
//! [`shows`]). An issue's or pull request's fields in its projects are
//! set from its own page's sidebar too ([`sidebar`]), with the same
//! pickers.

pub mod sidebar;

use crate::api::change::Change;
use crate::api::model::NodeId;
use crate::api::project::{
    FieldKind, FieldValue, ItemContent, Layout, Project, ProjectChange, ProjectField, ProjectItem,
    ProjectRef, grouped,
};
use crate::compose;
use crate::route::{Route, issues, projects, pulls};
use crate::ui::overlays::PaletteItem;
use crate::ui::page::Link;
use crate::ui::pages::item_link;

use crate::act::{By, Step, Subject, awaiting_here};
use crate::data::DataKey;
use crate::keymap::Action;
use crate::nav::Doable;
use crate::picker::{Choice, Kind, Rows, entry, fuzzy_score};
use crate::state::{Cmds, Screen, State};

/// A project's own actions.
const ACTIONS: &[Action] = &[
    Action::PrevColumn,
    Action::NextColumn,
    Action::MoveItem,
    Action::SetField,
    Action::AddItem,
    Action::ArchiveItem,
];

/// What projects add: their own actions, and editing a draft on a
/// project's pages.
pub const FEATURE: crate::features::Feature = crate::features::Feature {
    owns: ACTIONS,
    takes,
    act,
    offers: doables,
    ..crate::features::Feature::NONE
};

/// Whether `action` is the project's on screen to do: its own, and
/// editing (a draft's text); on an issue or pull request, setting its
/// fields in its projects.
fn takes(state: &State, action: Action) -> bool {
    let on_page =
        ACTIONS.contains(&action) || matches!(action, Action::EditComment | Action::EditTitle);
    (on_page && here(state).is_some())
        || (action == Action::SetField && sidebar::here(state).is_some())
}

/// A draft being written in the composer: a new one, or new text for
/// one (`draft`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drafting {
    pub project: ProjectRef,
    pub view: Option<u64>,
    /// The project's node ID, which adding names it by.
    pub project_id: NodeId,
    pub draft: Option<NodeId>,
    /// The composer's title.
    pub title: String,
}

impl Drafting {
    /// Sends what the composer wrote: the title on the first line, then
    /// the body.
    pub fn send(&self, text: &str) -> Result<Cmds, String> {
        let (title, body) = text.split_once('\n').unwrap_or((text, ""));
        let (title, body) = (title.trim().to_owned(), body.trim().to_owned());
        if title.is_empty() {
            return Err("Write its title on the first line".into());
        }
        let change = match &self.draft {
            Some(draft) => ProjectChange::EditDraft {
                draft: draft.clone(),
                title,
                body,
            },
            None => ProjectChange::AddDraft {
                project: self.project_id.clone(),
                title,
                body,
            },
        };
        let about = Subject::Project(self.project.clone(), self.view);
        Ok(crate::act::send(Change::Project(change), By::Edit(about)).into())
    }
}

/// An item of a project, where it's changed from: what a change is
/// about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct At {
    pub on: On,
    pub item: NodeId,
}

/// Where an item's fields are set from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum On {
    /// A view of its project.
    View(ProjectRef, Option<u64>),
    /// The page of the issue or pull request it is, and the project (by
    /// its node ID).
    Sidebar(Subject, NodeId),
}

impl At {
    /// What following the change watches.
    fn about(&self) -> Subject {
        match &self.on {
            On::View(project, view) => Subject::Project(project.clone(), *view),
            On::Sidebar(subject, _) => subject.clone(),
        }
    }
}

/// What setting an item's fields reads: its project's ID and fields, its
/// values, and (on a board) the field its columns are.
struct Target<'a> {
    project: &'a NodeId,
    fields: &'a [ProjectField],
    values: &'a [(NodeId, FieldValue)],
    column: Option<&'a ProjectField>,
}

impl Target<'_> {
    fn value(&self, field: &NodeId) -> Option<&FieldValue> {
        self.values.iter().find(|(f, _)| f == field).map(|(_, v)| v)
    }

    fn field(&self, id: &NodeId) -> Option<&ProjectField> {
        self.fields.iter().find(|f| f.id == *id)
    }
}

/// The item `at` names, as loaded.
fn target<'a>(state: &'a State, at: &At) -> Option<Target<'a>> {
    match &at.on {
        On::View(project, view) => {
            let p = state.picked::<Project>(&DataKey::Project(project.clone(), *view))?;
            let it = p.loaded().find(|it| it.id == at.item)?;
            Some(Target {
                project: &p.id,
                fields: &p.fields.items,
                values: &it.values,
                column: p.column_field(),
            })
        }
        On::Sidebar(subject, project) => {
            let p = sidebar::project(state, subject, project)?;
            (p.item == at.item).then_some(Target {
                project: &p.id,
                fields: &p.fields.items,
                values: &p.values,
                column: None,
            })
        }
    }
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
    /// What to add to the project at a view.
    Add(ProjectRef, Option<u64>),
    /// Which of an issue's or pull request's projects to set a field in.
    Project(Subject),
}

impl Pick {
    pub fn title(&self) -> &'static str {
        match self {
            Pick::Column(_) => "Move to column",
            Pick::Field(_) => "Set a field",
            Pick::Value(..) => "Set the field",
            Pick::Add(..) => "Add an item",
            Pick::Project(_) => "Set a field in a project",
        }
    }

    pub fn placeholder(&self) -> &'static str {
        match self {
            Pick::Column(_) => "Find a column",
            Pick::Field(_) => "Find a field",
            Pick::Value(..) => "Find a value, or type one",
            Pick::Add(..) => "owner/repo#123, a link, or a new draft's title",
            Pick::Project(_) => "Find a project",
        }
    }
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
        Route::Projects(projects::Project { project, view }) => (project, *view, None),
        Route::Projects(projects::ProjectItem {
            project,
            view,
            item,
        }) => (project, *view, Some(*item)),
        _ => return None,
    };
    let data = state.picked::<Project>(&DataKey::Project(project.clone(), view));
    let item = data.and_then(|d| match number {
        Some(n) => d.item(n),
        None => {
            let url = p.selected_link()?.url()?;
            d.loaded()
                .find(|it| item_link(project, view, it).as_deref() == Some(url))
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
    let p = changeable(state, h)?;
    let Some(item) = h.item else {
        let down = state.first_key(Action::Down);
        return Err(format!("No item is selected: {down} selects one"));
    };
    if matches!(item.content, crate::api::project::ItemContent::Redacted) {
        return Err("You can't see this item, so you can't change it".into());
    }
    Ok((p, item))
}

/// The project, if GitHub lets you change it and it shows the last
/// change.
fn changeable<'a>(state: &State, h: &Here<'a>) -> Result<&'a Project, String> {
    let p = h.data.ok_or("The project hasn't loaded yet")?;
    if !p.can_update {
        return Err(
            "GitHub doesn't let you change this project: that takes write access to it".into(),
        );
    }
    if let Some(waiting) = awaiting_here(state)
        && matches!(
            waiting.change,
            Change::SetProjectField { .. } | Change::Project(_)
        )
    {
        return Err("GitHub hasn't shown the last change yet: wait a moment".into());
    }
    Ok(p)
}

/// The draft here to edit, or why there's none.
fn draft<'a>(state: &State, h: &Here<'a>) -> Result<&'a crate::api::project::Draft, String> {
    let (_, item) = editable(state, h)?;
    match &item.content {
        ItemContent::Draft(d) => Ok(d),
        _ => Err(format!(
            "Only a draft's text is edited here: {} opens the issue or pull request, to edit it there",
            state.first_key(Action::Open)
        )),
    }
}

/// Why `action` can't be done here, if it can't.
fn why_not(state: &State, h: &Here<'_>, action: Action) -> Option<String> {
    let archived = h.item.is_some_and(|it| it.archived);
    match action {
        Action::PrevColumn | Action::NextColumn if !h.on_view => {
            Some("Columns are a board's: open the project's view".into())
        }
        Action::PrevColumn | Action::NextColumn => board(h).err(),
        Action::MoveItem if archived => Some(format!(
            "It's archived: {} restores it",
            state.first_key(Action::ArchiveItem)
        )),
        Action::MoveItem => board(h).and_then(|_| editable(state, h).map(drop)).err(),
        Action::AddItem => changeable(state, h).err(),
        Action::ArchiveItem => editable(state, h).err(),
        Action::EditComment | Action::EditTitle => draft(state, h).err(),
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
        out.extend(sidebar::doable(state));
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
        (
            Action::AddItem,
            "Change",
            "Add an issue, a pull request or a draft",
            "add",
        ),
        (
            Action::ArchiveItem,
            "Change",
            if h.item.is_some_and(|it| it.archived) {
                "Restore the item"
            } else {
                "Archive the item"
            },
            "",
        ),
        (Action::EditComment, "Edit", "Edit the draft", ""),
        (Action::EditTitle, "Edit", "Edit the draft's title", ""),
    ];
    let is_draft = h
        .item
        .is_some_and(|it| matches!(it.content, ItemContent::Draft(_)));
    for (action, section, label, short) in rows {
        // An item's page has no columns to go between.
        if !h.on_view && matches!(action, Action::PrevColumn | Action::NextColumn) {
            continue;
        }
        if !is_draft && matches!(action, Action::EditComment | Action::EditTitle) {
            continue;
        }
        let why = why_not(state, &h, action);
        out.push(crate::nav::row(section, action, label, short, why));
    }
}

/// Runs one of [`ACTIONS`], or an edit, on a project's page.
pub fn act(state: &mut State, action: Action) -> Cmds {
    let Some(h) = here(state) else {
        if action == Action::SetField && sidebar::here(state).is_some() {
            return sidebar::act(state);
        }
        return state.just_say(action.not_here());
    };
    if let Some(why) = why_not(state, &h, action) {
        return state.just_say(why);
    }
    let (project, view) = (h.project.clone(), h.view);
    let item = h.item.map(|it| it.id.clone());
    let project_id = h.data.map(|p| p.id.clone()).unwrap_or_default();
    match (action, item) {
        (Action::PrevColumn | Action::NextColumn, _) => {
            column_step(state, action == Action::NextColumn);
            Cmds::new()
        }
        (Action::AddItem, _) => state.open_picker(Kind::Project(Pick::Add(project, view))),
        (Action::ArchiveItem, Some(id)) => {
            let Some(it) = h.item else {
                return Cmds::new();
            };
            let archived = !it.archived;
            let change = Change::Project(ProjectChange::Archive {
                project: project_id,
                item: id,
                archived,
            });
            let about = Subject::Project(project, view);
            if !archived {
                state.info("Restoring it…");
                return crate::act::send(change, By::Act(about)).into();
            }
            // GitHub asks first.
            let title = crate::ui::text::truncate(it.title(), 50);
            let restore = state.first_key(Action::ArchiveItem);
            let facts = vec![(
                format!(
                    "It leaves the project's views; {restore} on it under Archived restores it"
                ),
                false,
            )];
            let title = format!("Archive “{title}”?");
            let choices = vec![("Archive".into(), change)];
            let confirm = crate::act::confirm(action, about, title, facts, choices);
            state.overlay = Some(crate::state::Overlay::Ask(Box::new(confirm)));
            Cmds::new()
        }
        (Action::EditComment | Action::EditTitle, _) => {
            let Ok(d) = draft(state, &h) else {
                return Cmds::new();
            };
            let text = format!("{}\n\n{}", d.title, d.body);
            let drafting = Drafting {
                project,
                view,
                project_id,
                draft: Some(d.id.clone()),
                title: "Edit the draft: its title on the first line, then its text".into(),
            };
            state.compose(compose::Target::ProjectDraft(drafting), &text)
        }
        (Action::MoveItem, Some(item)) => {
            let at = At {
                on: On::View(project, view),
                item,
            };
            state.open_picker(Kind::Project(Pick::Column(at)))
        }
        (_, Some(item)) => {
            let at = At {
                on: On::View(project, view),
                item,
            };
            state.open_picker(Kind::Project(Pick::Field(at)))
        }
        (_, None) => Cmds::new(),
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
    // With swimlanes, the columns are the lane's.
    let lanes: Vec<Vec<&ProjectItem>> = match p.lanes() {
        Some((_, lanes)) => lanes.into_iter().map(|l| l.items).collect(),
        None => vec![p.sorted()],
    };
    let link = |it: &ProjectItem| item_link(h.project, h.view, it);
    let selected = h.item.and_then(link);
    let lane = (lanes.iter())
        .find(|l| {
            l.iter()
                .any(|it| link(it) == selected && selected.is_some())
        })
        .or_else(|| lanes.iter().find(|l| !l.is_empty()));
    let columns: Vec<Vec<String>> = grouped(field, lane.map_or(&[][..], Vec::as_slice))
        .iter()
        .map(|c| c.items.iter().filter_map(|it| link(it)).collect())
        .filter(|c: &Vec<String>| !c.is_empty())
        .collect();
    let at = selected.and_then(|url| {
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
pub fn rows(state: &State, pick: &Pick, q: &str) -> Rows {
    let at = match pick {
        Pick::Column(at) | Pick::Field(at) | Pick::Value(at, _) => at,
        Pick::Add(project, view) => return adding(state, project, *view, q),
        Pick::Project(subject) => return sidebar::project_rows(state, subject, q),
    };
    let Some(p) = target(state, at) else {
        let gone = match at.on {
            On::View(..) => "The item is no longer in the view",
            On::Sidebar(..) => "It's no longer in the project",
        };
        return vec![entry(gone, "", None)];
    };
    let set = |field: &ProjectField, value: Option<FieldValue>| {
        let said = match &value {
            Some(v) => format!("{} to {}", field.name, v.text()),
            None => field.name.clone(),
        };
        let change = Change::SetProjectField {
            project: p.project.clone(),
            item: at.item.clone(),
            field: field.id.clone(),
            value,
            said,
        };
        Some(Choice::Step(Step::Send(change, at.about())))
    };
    let field = match pick {
        Pick::Column(_) => p.column,
        Pick::Value(_, id) => p.field(id),
        Pick::Add(..) | Pick::Project(_) => None,
        Pick::Field(_) => {
            return (p.fields.iter())
                .filter(|f| f.kind.settable() && fuzzy_score(q, &f.name).is_some())
                .map(|f| {
                    let hint = p.value(&f.id).map_or("", FieldValue::text);
                    let values = Pick::Value(at.clone(), f.id.clone());
                    entry(f.name.clone(), hint, Some(listing(values)))
                })
                .collect();
        }
    };
    let Some(field) = field else {
        return vec![entry("The field is no longer in the project", "", None)];
    };
    let current = p.value(&field.id);
    let is_current = |v: &FieldValue| current.is_some_and(|c| c.same(v));
    let mut rows = Vec::new();
    let mut choice = |value: FieldValue, hint: &str| {
        if fuzzy_score(q, value.text()).is_some() {
            let hint = if is_current(&value) { "current" } else { hint };
            rows.push(entry(
                value.text().to_owned(),
                hint,
                set(field, Some(value)),
            ));
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
            rows.push(entry(label, "", set(field, None)));
        }
    }
    rows
}

/// The rows adding an item offers for what's typed: the issue or pull
/// request it names (`owner/repo#123`, or its link), and a new draft
/// titled with it.
fn adding(state: &State, project: &ProjectRef, view: Option<u64>, q: &str) -> Rows {
    let q = q.trim();
    let key = DataKey::Project(project.clone(), view);
    let Some(p) = state.picked::<Project>(&key) else {
        return vec![entry("The project hasn't loaded yet", "", None)];
    };
    if q.is_empty() {
        let what = "Type owner/repo#123 or a link to add it, or a new draft's title";
        return vec![entry(what, "", None)];
    }
    let mut rows = Vec::new();
    let named = crate::route::parse_input(q, None).and_then(|dest| match dest.target {
        crate::route::Target::Page(Route::Issues(issues::Issue { repo, number })) => {
            Some((repo, number))
        }
        crate::route::Target::Page(Route::Pulls(pulls::Pr { pr, .. })) => {
            Some((pr.repo, pr.number))
        }
        _ => None,
    });
    if let Some((repo, number)) = named {
        let here = p.loaded().find(|it| match &it.content {
            ItemContent::Issue(l, _) | ItemContent::Pr(l, _) => {
                l.repo == repo && l.number == number
            }
            _ => false,
        });
        let label = format!("Add {repo}#{number}");
        match here {
            Some(it) if it.archived => {
                rows.push(entry(label, "archived: restore it", None));
            }
            Some(_) => rows.push(entry(label, "already in the project", None)),
            None => {
                let change = Change::Project(ProjectChange::Add {
                    project: p.id.clone(),
                    repo,
                    number,
                });
                let add = Step::Send(change, Subject::Project(project.clone(), view));
                rows.push(entry(label, "↵", Some(Choice::Step(add))));
            }
        }
    }
    let drafting = Drafting {
        project: project.clone(),
        view,
        project_id: p.id.clone(),
        draft: None,
        title: "New draft: its title on the first line, then its text".into(),
    };
    rows.push(entry(
        format!("New draft “{q}”"),
        "↵ writes it",
        Some(Choice::Step(Step::Compose(
            compose::Target::ProjectDraft(drafting),
            q.to_owned(),
        ))),
    ));
    rows
}

/// The row a typed value makes: setting it, or why it can't be.
fn typed(
    field: &ProjectField,
    q: &str,
    set: impl Fn(&ProjectField, Option<FieldValue>) -> Option<Choice>,
) -> (PaletteItem, Option<Choice>) {
    let row = |label: String, chosen: Option<Choice>| {
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

/// The row that lists `pick`'s rows.
fn listing(pick: Pick) -> Choice {
    Choice::Step(Step::Pick(Kind::Project(pick)))
}

/// Whether the project at `view`, as loaded, shows `change`: the item has
/// the value now, has come or gone, or is archived or back; or it may be
/// past what's loaded, or left out by the view's filter, where it can't
/// be told.
pub fn shows(state: &State, change: &Change, project: &ProjectRef, view: Option<u64>) -> bool {
    let key = DataKey::Project(project.clone(), view);
    let Some(p) = state.picked::<Project>(&key) else {
        return false;
    };
    // An item that's not loaded may be further down, or filtered out.
    let unseen = p.items.next.is_some() || p.shown().is_some_and(|v| !v.filter.is_empty());
    match change {
        Change::SetProjectField {
            item, field, value, ..
        } => {
            let Some(it) = p.loaded().find(|it| it.id == *item) else {
                return true;
            };
            match value {
                Some(value) => it.value(field).is_some_and(|v| v.same(value)),
                None => it.value(field).is_none(),
            }
        }
        Change::Project(ProjectChange::Add { repo, number, .. }) => {
            unseen
                || p.loaded().any(|it| match &it.content {
                    ItemContent::Issue(l, _) | ItemContent::Pr(l, _) => {
                        l.repo == *repo && l.number == *number
                    }
                    _ => false,
                })
        }
        Change::Project(ProjectChange::AddDraft { title, .. }) => {
            unseen
                || p.loaded()
                    .any(|it| matches!(&it.content, ItemContent::Draft(d) if d.title == *title))
        }
        Change::Project(ProjectChange::Archive { item, archived, .. }) => {
            let viewed = p.items.items.iter().any(|it| it.id == *item);
            let put_away = p.archived.items.iter().any(|it| it.id == *item);
            if *archived {
                !viewed && (put_away || p.archived.next.is_some())
            } else {
                !put_away && (viewed || unseen)
            }
        }
        Change::Project(ProjectChange::EditDraft { draft, title, body }) => {
            let found = p.loaded().find_map(|it| match &it.content {
                ItemContent::Draft(d) if d.id == *draft => Some(d),
                _ => None,
            });
            found.is_none_or(|d| d.title == *title && d.body.trim() == body.trim())
        }
        _ => true,
    }
}

/// What a change is doing while it's sent.
pub fn doing(change: &ProjectChange) -> &'static str {
    match change {
        ProjectChange::Add { .. } | ProjectChange::AddDraft { .. } => "Adding it to the project",
        ProjectChange::Archive { archived: true, .. } => "Archiving it",
        ProjectChange::Archive { .. } => "Restoring it",
        ProjectChange::EditDraft { .. } => "Saving the draft",
    }
}

/// What a change did, once GitHub said so.
pub fn done(change: &ProjectChange) -> &'static str {
    match change {
        ProjectChange::Add { .. } => "Added to the project",
        ProjectChange::AddDraft { .. } => "Draft added",
        ProjectChange::Archive { archived: true, .. } => "Archived",
        ProjectChange::Archive { .. } => "Restored",
        ProjectChange::EditDraft { .. } => "Draft saved",
    }
}

/// What was refused, for "GitHub refused …".
pub fn what(change: &ProjectChange) -> &'static str {
    match change {
        ProjectChange::Add { .. } => "adding it",
        ProjectChange::AddDraft { .. } => "the draft",
        ProjectChange::Archive { archived: true, .. } => "archiving it",
        ProjectChange::Archive { .. } => "restoring it",
        ProjectChange::EditDraft { .. } => "the draft's edit",
    }
}
