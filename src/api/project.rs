//! GitHub Projects (v2): an owner's or a repository's projects, a project
//! with its views, fields and items, and setting an item's field.

use serde::{Deserialize, Serialize};

use crate::api::browse::{Page, Results, wire};
use crate::api::client::{ApiError, GitHub};
use crate::api::model::{Capped, IssueState, Label, NodeId, PrState, RepoId, Status};
use crate::api::raw;

/// Who owns a project, as its URL says: `/orgs/o/projects/1` or
/// `/users/u/projects/1`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProjectOwner {
    Org(String),
    User(String),
}

impl ProjectOwner {
    pub fn login(&self) -> &str {
        match self {
            ProjectOwner::Org(login) | ProjectOwner::User(login) => login,
        }
    }

    /// Where its projects' URLs start: `https://github.com/orgs/o`.
    pub fn url(&self) -> String {
        match self {
            ProjectOwner::Org(org) => format!("{}/orgs/{org}", crate::ui::pages::url::base()),
            ProjectOwner::User(user) => format!("{}/users/{user}", crate::ui::pages::url::base()),
        }
    }
}

/// A project, by its owner and number.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProjectRef {
    pub owner: ProjectOwner,
    pub number: u64,
}

impl ProjectRef {
    pub fn url(&self) -> String {
        format!("{}/projects/{}", self.owner.url(), self.number)
    }
}

impl std::fmt::Display for ProjectRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "@{}'s project {}", self.owner.login(), self.number)
    }
}

/// Whose projects a list is: an owner's, or those linked to a repository.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProjectsOf {
    Owner(ProjectOwner),
    Repo(RepoId),
}

/// A project in a list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSummary {
    pub number: u64,
    pub title: String,
    pub description: String,
    pub url: String,
    pub closed: bool,
    pub public: bool,
    pub updated_at: String,
    /// How many items it has.
    pub items: u64,
}

/// A project at one of its views: what it is, its views and fields, and a
/// page of the items the view's filter finds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    /// What changes name it by.
    pub id: NodeId,
    pub title: String,
    pub description: String,
    pub url: String,
    pub closed: bool,
    pub public: bool,
    /// GitHub lets you change it: move its items, set their fields.
    pub can_update: bool,
    pub views: Capped<ProjectView>,
    pub fields: Capped<ProjectField>,
    /// The view shown, by its number.
    pub view: u64,
    /// The items the view's filter finds: all of them (up to
    /// [`ALL_ITEMS`]) where the view sorts or groups them, as a board
    /// does, else a page.
    pub items: Results<ProjectItem>,
    /// The items archived, the newest 50 (whatever the view's filter):
    /// what restoring one picks from.
    pub archived: Results<ProjectItem>,
}

/// How a view lays out its items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Layout {
    Board,
    Table,
    Roadmap,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectView {
    pub number: u64,
    pub name: String,
    pub layout: Layout,
    /// Which items it shows, in GitHub's filter syntax (`status:Todo`).
    pub filter: String,
    /// The fields it shows, in order.
    pub fields: Vec<NodeId>,
    /// A board's columns are this field's values.
    pub column: Option<NodeId>,
    /// Its items are grouped by this field's values.
    pub group: Option<NodeId>,
    /// What its items are sorted by, first to last, and whether
    /// descending.
    pub sort: Vec<(NodeId, bool)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectField {
    pub id: NodeId,
    pub name: String,
    pub kind: FieldKind,
}

/// A field's kind: GitHub's own (an item's title, assignees …), or one of
/// the project's, which can be set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldKind {
    Title,
    Assignees,
    Labels,
    Repository,
    Milestone,
    Text,
    Number,
    Date,
    SingleSelect(Vec<SelectOption>),
    Iteration(Vec<Iteration>),
    /// What ghtui doesn't show: linked pull requests, reviewers, …
    Other,
}

impl FieldKind {
    /// Whether an item's value of it can be set.
    pub fn settable(&self) -> bool {
        matches!(
            self,
            FieldKind::Text
                | FieldKind::Number
                | FieldKind::Date
                | FieldKind::SingleSelect(_)
                | FieldKind::Iteration(_)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectOption {
    pub id: String,
    pub name: String,
    /// GitHub's name for its color: `GREEN`, `GRAY` …
    pub color: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Iteration {
    pub id: String,
    pub title: String,
    /// `2026-10-01`.
    pub start: String,
    pub days: u64,
    pub completed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectItem {
    /// What changes name it by.
    pub id: NodeId,
    /// What its URL names it by (`?itemId=`).
    pub number: u64,
    /// Out of the project's views, until it's restored.
    pub archived: bool,
    pub content: ItemContent,
    /// Its values of the project's own fields, by field.
    pub values: Vec<(NodeId, FieldValue)>,
}

impl ProjectItem {
    pub fn value(&self, field: &NodeId) -> Option<&FieldValue> {
        self.values.iter().find(|(f, _)| f == field).map(|(_, v)| v)
    }

    pub fn title(&self) -> &str {
        match &self.content {
            ItemContent::Issue(l, _) | ItemContent::Pr(l, _) => &l.title,
            ItemContent::Draft(d) => &d.title,
            ItemContent::Redacted => "You don't have access to this item",
        }
    }
}

/// What an item is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemContent {
    Issue(Linked, IssueState),
    Pr(Linked, PrState),
    Draft(Draft),
    /// One you can't see (a private repository's).
    Redacted,
}

impl ItemContent {
    pub fn status(&self) -> Option<Status> {
        match self {
            ItemContent::Issue(_, state) => Some(Status::Issue(*state)),
            ItemContent::Pr(_, state) => Some(Status::Pr(*state)),
            ItemContent::Draft(_) | ItemContent::Redacted => None,
        }
    }
}

/// An issue or pull request in a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Linked {
    pub repo: RepoId,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub assignees: Capped<String>,
    pub labels: Capped<Label>,
    pub milestone: Option<String>,
}

/// A draft issue: only in the project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    /// What editing it names it by (not the item's).
    pub id: NodeId,
    pub title: String,
    pub body: String,
    pub assignees: Capped<String>,
}

/// An item's value of one of the project's own fields; what setting one
/// sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldValue {
    Text(String),
    /// As GitHub writes it (`3`, `2.5`).
    Number(String),
    /// `2026-10-01`.
    Date(String),
    Option {
        id: String,
        name: String,
    },
    Iteration {
        id: String,
        title: String,
    },
}

impl FieldValue {
    /// It as GitHub shows it.
    pub fn text(&self) -> &str {
        match self {
            FieldValue::Text(s) | FieldValue::Number(s) | FieldValue::Date(s) => s,
            FieldValue::Option { name, .. } => name,
            FieldValue::Iteration { title, .. } => title,
        }
    }

    /// Whether it's the same value (an option or iteration by its ID, a
    /// number by its value).
    pub fn same(&self, other: &FieldValue) -> bool {
        match (self, other) {
            (FieldValue::Option { id: a, .. }, FieldValue::Option { id: b, .. })
            | (FieldValue::Iteration { id: a, .. }, FieldValue::Iteration { id: b, .. }) => a == b,
            (FieldValue::Number(a), FieldValue::Number(b)) => {
                a.parse::<f64>().ok() == b.parse::<f64>().ok()
            }
            _ => self == other,
        }
    }
}

impl Project {
    /// The view shown.
    pub fn shown(&self) -> Option<&ProjectView> {
        self.views.iter().find(|v| v.number == self.view)
    }

    pub fn field(&self, id: &NodeId) -> Option<&ProjectField> {
        self.fields.iter().find(|f| f.id == *id)
    }

    /// The item named `number` in URLs, if it's loaded (in the view, or
    /// archived).
    pub fn item(&self, number: u64) -> Option<&ProjectItem> {
        self.loaded().find(|i| i.number == number)
    }

    /// Every item loaded: the view's, then the archived.
    pub fn loaded(&self) -> impl Iterator<Item = &ProjectItem> {
        self.items.items.iter().chain(&self.archived.items)
    }

    /// Whether the view sorts or groups its items (a board's columns
    /// group them): then they're all loaded, so the order and the groups
    /// are the whole view's.
    pub fn arranges(view: &ProjectView) -> bool {
        view.layout == Layout::Board || view.group.is_some() || !view.sort.is_empty()
    }

    /// The field a board's columns are: the view's, or else Status (as
    /// GitHub's boards start).
    pub fn column_field(&self) -> Option<&ProjectField> {
        let groups = |f: &&ProjectField| {
            matches!(f.kind, FieldKind::SingleSelect(_) | FieldKind::Iteration(_))
        };
        let view = self.shown()?;
        match &view.column {
            Some(id) => self.field(id).filter(groups),
            None => self
                .fields
                .iter()
                .filter(groups)
                .find(|f| f.name == "Status"),
        }
    }

    /// The items in the view's order: by its sort, then as the project
    /// orders them.
    pub fn sorted(&self) -> Vec<&ProjectItem> {
        let mut items: Vec<&ProjectItem> = self.items.items.iter().collect();
        let Some(view) = self.shown() else {
            return items;
        };
        for (field, descending) in view.sort.iter().rev() {
            let Some(field) = self.field(field) else {
                continue;
            };
            // Items without a value go last, either way.
            items.sort_by(|a, b| {
                use std::cmp::Ordering;
                match (sort_key(field, a), sort_key(field, b)) {
                    (Some(a), Some(b)) => {
                        let order = a.partial_cmp(&b).unwrap_or(Ordering::Equal);
                        if *descending { order.reverse() } else { order }
                    }
                    (Some(_), None) => Ordering::Less,
                    (None, Some(_)) => Ordering::Greater,
                    (None, None) => Ordering::Equal,
                }
            });
        }
        items
    }
}

/// A project an issue or pull request is in: its fields and the item's
/// values of them, which its sidebar shows and sets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InProject {
    pub title: String,
    pub url: String,
    /// The project's node ID, which setting a field names it by.
    pub id: NodeId,
    /// The item's.
    pub item: NodeId,
    /// GitHub lets you change it.
    pub can_update: bool,
    pub fields: Capped<ProjectField>,
    pub values: Vec<(NodeId, FieldValue)>,
}

impl InProject {
    pub fn value(&self, field: &NodeId) -> Option<&FieldValue> {
        self.values.iter().find(|(f, _)| f == field).map(|(_, v)| v)
    }

    pub fn field(&self, id: &NodeId) -> Option<&ProjectField> {
        self.fields.iter().find(|f| f.id == *id)
    }

    /// Its value of the project's Status field, if it has one.
    pub fn status(&self) -> Option<&str> {
        let status = self
            .fields
            .iter()
            .find(|f| f.name == "Status" && matches!(f.kind, FieldKind::SingleSelect(_)))?;
        self.value(&status.id).map(FieldValue::text)
    }

    /// Its values of the project's other fields that can be set, by
    /// their names, in the project's order.
    pub fn others(&self) -> impl Iterator<Item = (&str, &FieldValue)> {
        (self.fields.iter())
            .filter(|f| f.kind.settable() && f.name != "Status")
            .filter_map(|f| Some((f.name.as_str(), self.value(&f.id)?)))
    }
}

/// Items grouped by `field`'s values, in its order: those without one first
/// ("No Status"), then each option or iteration, empty ones too. By any
/// other field, its values as they come, in order (an item with two
/// assignees or labels is in each one's group).
pub fn grouped<'a>(field: &'a ProjectField, items: &[&'a ProjectItem]) -> Vec<Group<'a>> {
    if !matches!(
        field.kind,
        FieldKind::SingleSelect(_) | FieldKind::Iteration(_)
    ) {
        return grouped_by_text(field, items);
    }
    let values: Vec<(String, String, Option<&str>)> = match &field.kind {
        FieldKind::SingleSelect(options) => options
            .iter()
            .map(|o| (o.id.clone(), o.name.clone(), Some(o.color.as_str())))
            .collect(),
        FieldKind::Iteration(iterations) => iterations
            .iter()
            .filter(|i| !i.completed)
            .map(|i| (i.id.clone(), i.title.clone(), None))
            .collect(),
        _ => Vec::new(),
    };
    let id_of = |item: &ProjectItem| match item.value(&field.id) {
        Some(FieldValue::Option { id, .. } | FieldValue::Iteration { id, .. }) => Some(id.clone()),
        _ => None,
    };
    let mut groups = vec![Group {
        value: None,
        name: format!("No {}", field.name),
        color: None,
        items: Vec::new(),
    }];
    for (id, name, color) in values {
        groups.push(Group {
            value: Some(id),
            name,
            color,
            items: Vec::new(),
        });
    }
    for item in items {
        let id = id_of(item);
        match groups.iter_mut().find(|g| g.value == id) {
            Some(group) => group.items.push(item),
            // A value the field no longer offers (a completed
            // iteration): a group of its own.
            None => groups.push(Group {
                name: item
                    .value(&field.id)
                    .map_or_else(String::new, |v| v.text().to_owned()),
                value: id,
                color: None,
                items: vec![item],
            }),
        }
    }
    // "No Status" only when something has none, as on GitHub.
    groups.retain(|g| g.value.is_some() || !g.items.is_empty());
    groups
}

/// Items grouped by what they show of `field`: by each value, in order
/// (numbers by their value), those without one first.
fn grouped_by_text<'a>(field: &'a ProjectField, items: &[&'a ProjectItem]) -> Vec<Group<'a>> {
    let mut groups = vec![Group {
        value: None,
        name: format!("No {}", field.name),
        color: None,
        items: Vec::new(),
    }];
    for item in items {
        let keys = group_keys(field, item);
        if keys.is_empty()
            && let Some(none) = groups.first_mut()
        {
            none.items.push(item);
        }
        for key in keys {
            match groups.iter_mut().find(|g| g.value.as_ref() == Some(&key)) {
                Some(group) => group.items.push(item),
                None => groups.push(Group {
                    name: key.clone(),
                    value: Some(key),
                    color: None,
                    items: vec![item],
                }),
            }
        }
    }
    groups.retain(|g| g.value.is_some() || !g.items.is_empty());
    let number = |g: &Group<'_>| g.value.as_deref().and_then(|v| v.parse::<f64>().ok());
    groups.sort_by(|a, b| match (a.value.is_some(), b.value.is_some()) {
        (false, true) => std::cmp::Ordering::Less,
        (true, false) => std::cmp::Ordering::Greater,
        _ => match (number(a), number(b)) {
            (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        },
    });
    groups
}

/// The values an item is grouped under for `field`: its assignees or
/// labels, each; its milestone or repository; its own value.
fn group_keys(field: &ProjectField, item: &ProjectItem) -> Vec<String> {
    let linked = match &item.content {
        ItemContent::Issue(l, _) | ItemContent::Pr(l, _) => Some(l),
        _ => None,
    };
    match &field.kind {
        FieldKind::Assignees => match &item.content {
            ItemContent::Issue(l, _) | ItemContent::Pr(l, _) => l.assignees.items.clone(),
            ItemContent::Draft(d) => d.assignees.items.clone(),
            ItemContent::Redacted => Vec::new(),
        },
        FieldKind::Labels => (linked.iter())
            .flat_map(|l| l.labels.iter().map(|l| l.name.clone()))
            .collect(),
        FieldKind::Milestone => linked
            .and_then(|l| l.milestone.clone())
            .into_iter()
            .collect(),
        FieldKind::Repository => linked.map(|l| l.repo.to_string()).into_iter().collect(),
        _ => (item.value(&field.id).map(|v| v.text().to_owned()))
            .into_iter()
            .collect(),
    }
}

/// What a roadmap places items by: a start and an end date field (or
/// the start alone), or an iteration field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dates<'a> {
    Fields {
        start: &'a ProjectField,
        end: Option<&'a ProjectField>,
    },
    Iteration(&'a ProjectField),
}

/// `2026-10-31` as days since 1970-01-01.
pub fn day(date: &str) -> Option<i64> {
    crate::ui::time::day_of(date)
}

impl Project {
    /// A board's swimlanes: its items grouped by the view's grouping
    /// field (not its columns'), each lane drawn with the columns.
    pub fn lanes(&self) -> Option<(&ProjectField, Vec<Group<'_>>)> {
        let view = self.shown()?;
        if view.layout != Layout::Board {
            return None;
        }
        let field = self.field(view.group.as_ref()?)?;
        if self.column_field().is_some_and(|c| c.id == field.id) {
            return None;
        }
        Some((field, grouped(field, &self.sorted())))
    }

    /// The fields a roadmap places items by. GitHub's API doesn't say
    /// which the view uses, so as GitHub starts one: a date field named
    /// for a start and one for a target (or end, or due), else the first
    /// two date fields; without dates, the first iteration field.
    pub fn roadmap_dates(&self) -> Option<Dates<'_>> {
        let dates: Vec<&ProjectField> = (self.fields.iter())
            .filter(|f| f.kind == FieldKind::Date)
            .collect();
        let named = |words: &[&str]| {
            dates.iter().copied().find(|f| {
                let name = f.name.to_lowercase();
                words.iter().any(|w| name.contains(w))
            })
        };
        let start = named(&["start", "begin"]);
        let end = named(&["target", "end", "due", "finish"]).filter(|e| Some(*e) != start);
        match (start, end) {
            (Some(start), end) => Some(Dates::Fields { start, end }),
            (None, Some(end)) => Some(Dates::Fields {
                start: end,
                end: None,
            }),
            (None, None) if !dates.is_empty() => Some(Dates::Fields {
                start: dates.first().copied()?,
                end: dates.get(1).copied(),
            }),
            (None, None) => (self.fields.iter())
                .find(|f| matches!(f.kind, FieldKind::Iteration(_)))
                .map(Dates::Iteration),
        }
    }

    /// Where `it` is on a roadmap: its first and last day (days since
    /// 1970-01-01), if it has a date.
    pub fn span(dates: Dates<'_>, it: &ProjectItem) -> Option<(i64, i64)> {
        match dates {
            Dates::Fields { start, end } => {
                let at = |f: &ProjectField| it.value(&f.id).and_then(|v| day(v.text()));
                let from = at(start);
                let to = end.and_then(at);
                let (a, b) = match (from, to) {
                    (Some(a), Some(b)) => (a.min(b), a.max(b)),
                    (Some(a), None) | (None, Some(a)) => (a, a),
                    (None, None) => return None,
                };
                Some((a, b))
            }
            Dates::Iteration(field) => {
                let FieldKind::Iteration(iterations) = &field.kind else {
                    return None;
                };
                let Some(FieldValue::Iteration { id, .. }) = it.value(&field.id) else {
                    return None;
                };
                let i = iterations.iter().find(|i| i.id == *id)?;
                let start = day(&i.start)?;
                let days = i64::try_from(i.days).ok()?;
                Some((start, start + days.max(1) - 1))
            }
        }
    }
}

/// Items with one value of a field: a board's column, a table's group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group<'a> {
    /// The option's or iteration's ID; `None` for the items without one.
    pub value: Option<String>,
    pub name: String,
    pub color: Option<&'a str>,
    pub items: Vec<&'a ProjectItem>,
}

/// What an item sorts by for `field`.
fn sort_key(field: &ProjectField, item: &ProjectItem) -> Option<SortKey> {
    let value = item.value(&field.id);
    Some(match (&field.kind, value) {
        (FieldKind::Title, _) => SortKey::Text(item.title().to_lowercase()),
        (FieldKind::Number, Some(v)) => SortKey::Number(v.text().parse::<f64>().ok()?),
        (FieldKind::SingleSelect(options), Some(FieldValue::Option { id, .. })) => {
            SortKey::Place(options.iter().position(|o| o.id == *id)?)
        }
        (FieldKind::Iteration(iterations), Some(FieldValue::Iteration { id, .. })) => {
            SortKey::Text(iterations.iter().find(|i| i.id == *id)?.start.clone())
        }
        (_, Some(v)) => SortKey::Text(v.text().to_lowercase()),
        (_, None) => return None,
    })
}

#[derive(PartialEq, PartialOrd)]
enum SortKey {
    Text(String),
    Number(f64),
    Place(usize),
}

impl Page for Project {
    fn counts(&self) -> (usize, u64) {
        self.items.counts()
    }
}

/// Cache keys.
pub mod keys {
    use super::{ProjectRef, ProjectsOf};

    pub fn projects(of: &ProjectsOf) -> String {
        format!("projects:{of:?}")
    }

    pub fn project(project: &ProjectRef, view: Option<u64>) -> String {
        format!("project:{project}:{view:?}")
    }
}

/// How many items a page of a project has.
const ITEMS: usize = 50;

/// How many items a view that sorts or groups them loads at most.
pub const ALL_ITEMS: usize = 1000;

/// A change to a project's items, other than setting a field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectChange {
    /// Add an issue or pull request, by its repository and number.
    Add {
        project: NodeId,
        repo: RepoId,
        number: u64,
    },
    /// Add a draft issue.
    AddDraft {
        project: NodeId,
        title: String,
        body: String,
    },
    /// Archive an item, or restore it (`archived: false`).
    Archive {
        project: NodeId,
        item: NodeId,
        archived: bool,
    },
    /// A draft issue's new title and body.
    EditDraft {
        draft: NodeId,
        title: String,
        body: String,
    },
}

/// GitHub's word that the token lacks a scope projects need, said as what
/// to do about it.
fn scoped(err: ApiError) -> ApiError {
    let ApiError::GraphQl(messages) = &err else {
        return err;
    };
    let read = messages.iter().any(|m| m.contains("'read:project'"));
    let write = messages.iter().any(|m| m.contains("'project'"));
    if write {
        ApiError::GraphQl(vec![
            "changing a project takes the project scope, which your token lacks: `gh auth refresh -s project` adds it".into(),
        ])
    } else if read {
        ApiError::GraphQl(vec![
            "projects take the read:project scope, which your token lacks: `gh auth refresh -s read:project` adds it".into(),
        ])
    } else {
        err
    }
}

impl GitHub {
    /// An owner's projects, or a repository's, most recently updated
    /// first, 30 at a time from `after`.
    pub async fn projects(
        &self,
        of: &ProjectsOf,
        after: Option<String>,
    ) -> Result<Results<ProjectSummary>, ApiError> {
        let first = after.is_none();
        let (query, vars, at, what) = match of {
            ProjectsOf::Owner(owner) => (
                raw::OWNER_PROJECTS,
                serde_json::json!({ "login": owner.login(), "after": after }),
                "/repositoryOwner/projectsV2",
                format!("@{}'s projects", owner.login()),
            ),
            ProjectsOf::Repo(repo) => (
                raw::REPO_PROJECTS,
                serde_json::json!({ "owner": repo.owner, "name": repo.name, "after": after }),
                "/repository/projectsV2",
                format!("{repo}'s projects"),
            ),
        };
        let wire: wire::Connection<WireSummary> = (self.graphql_json(query, vars, &what, at))
            .await
            .map_err(scoped)?;
        self.check_connection(&wire, 30, &what);
        let list = wire.into_results(WireSummary::into_summary);
        Ok(self.kept_page(first, &keys::projects(of), list))
    }

    /// A project at view `view` (its first without one): the project, its
    /// views and fields, and the items the view's filter finds, 50 at a
    /// time from `after`.
    pub async fn project(
        &self,
        project: &ProjectRef,
        view: Option<u64>,
        after: Option<String>,
    ) -> Result<Project, ApiError> {
        let first = after.is_none();
        let login = project.owner.login();
        let vars = serde_json::json!({ "login": login, "number": project.number });
        let wire: WireProject =
            (self.graphql_json(&raw::project(), vars, project, "/repositoryOwner/projectV2"))
                .await
                .map_err(scoped)?;
        let fields = wire.fields.into_capped(WireField::into_field);
        let views = wire.views.into_capped(WireView::into_view);
        let shown = match view {
            Some(n) => views.iter().find(|v| v.number == n),
            None => views.first(),
        };
        let Some(shown) = shown else {
            let which = view.map_or_else(|| "views".to_owned(), |n| format!("view {n}"));
            return Err(ApiError::NotFound(format!("{project}'s {which}")));
        };
        let (number, filter) = (shown.number, shown.filter.clone());
        // A view that sorts or groups its items does it over all of them,
        // as GitHub does: they're all fetched, up to a limit.
        let pages = if first && Project::arranges(shown) {
            ALL_ITEMS / ITEMS
        } else {
            1
        };
        let mut items = self.project_items(project, &filter, after, false).await?;
        for _ in 1..pages {
            let Some(next) = items.next.take() else {
                break;
            };
            let more = self
                .project_items(project, &filter, Some(next), false)
                .await?;
            items.items.extend(more.items);
            items.next = more.next;
        }
        let archived = if first {
            self.project_items(project, "", None, true).await?
        } else {
            Results {
                total: 0,
                items: Vec::new(),
                next: None,
            }
        };
        let key = keys::project(project, view);
        let found = Project {
            id: wire.id,
            title: wire.title,
            description: wire.short_description.unwrap_or_default(),
            url: wire.url,
            closed: wire.closed,
            public: wire.public,
            can_update: wire.viewer_can_update,
            views,
            fields,
            view: number,
            items,
            archived,
        };
        Ok(self.kept_page(first, &key, found))
    }

    /// The projects issue or pull request `number` is in (the first 10),
    /// with its values of their fields: reading them takes the
    /// read:project scope.
    pub async fn projects_of(
        &self,
        repo: &RepoId,
        number: u64,
    ) -> Result<Capped<InProject>, ApiError> {
        let vars = serde_json::json!({ "owner": repo.owner, "name": repo.name, "number": number });
        let what = format!("{repo}#{number}'s projects");
        let items: wire::Counted<WireInProject> = (self.graphql_json(
            &raw::projects_of(),
            vars,
            what,
            "/repository/issueOrPullRequest/projectItems",
        ))
        .await
        .map_err(scoped)?;
        Ok(items.into_capped(WireInProject::into_in_project))
    }

    /// A page of a project's items that `filter` finds, from `after`: those
    /// archived, or those not.
    async fn project_items(
        &self,
        project: &ProjectRef,
        filter: &str,
        after: Option<String>,
        archived: bool,
    ) -> Result<Results<ProjectItem>, ApiError> {
        let state = if archived { "ARCHIVED" } else { "NOT_ARCHIVED" };
        let vars = serde_json::json!({
            "login": project.owner.login(),
            "number": project.number,
            "after": after,
            "query": filter,
            "archived": [state],
        });
        let what = if archived {
            format!("{project}'s archived items")
        } else {
            format!("{project}'s items")
        };
        let items: wire::Connection<WireItem> = (self.graphql_json(
            &raw::project_items(),
            vars,
            &what,
            "/repositoryOwner/projectV2/items",
        ))
        .await
        .map_err(scoped)?;
        self.check_connection(&items, ITEMS, &what);
        let mut items = items.filter_results(WireItem::into_item);
        // An Enterprise Server older than 3.22 can't be asked for the
        // archived apart: each item says whether it is.
        items.items.retain(|it| it.archived == archived);
        Ok(items)
    }

    /// Makes a change to a project's items.
    pub(crate) async fn change_project(&self, change: &ProjectChange) -> Result<(), ApiError> {
        let (query, vars) = match change {
            ProjectChange::Add {
                project,
                repo,
                number,
            } => {
                #[derive(Deserialize)]
                struct Found {
                    id: NodeId,
                }
                let vars =
                    serde_json::json!({ "owner": repo.owner, "name": repo.name, "number": number });
                let found: Found = (self.graphql_json(
                    raw::ISSUE_OR_PR_ID,
                    vars,
                    format!("{repo}#{number}"),
                    "/repository/issueOrPullRequest",
                ))
                .await?;
                let vars = serde_json::json!({ "project": project.as_str(), "content": found.id.as_str() });
                (raw::ADD_PROJECT_ITEM, vars)
            }
            ProjectChange::AddDraft {
                project,
                title,
                body,
            } => (
                raw::ADD_PROJECT_DRAFT,
                serde_json::json!({ "project": project.as_str(), "title": title, "body": body }),
            ),
            ProjectChange::Archive {
                project,
                item,
                archived,
            } => (
                if *archived {
                    raw::ARCHIVE_PROJECT_ITEM
                } else {
                    raw::UNARCHIVE_PROJECT_ITEM
                },
                serde_json::json!({ "project": project.as_str(), "item": item.as_str() }),
            ),
            ProjectChange::EditDraft { draft, title, body } => (
                raw::UPDATE_PROJECT_DRAFT,
                serde_json::json!({ "draft": draft.as_str(), "title": title, "body": body }),
            ),
        };
        self.mutate_json(query, vars).await.map_err(scoped)
    }

    /// Sets an item's field to `value`, or clears it (`None`).
    pub(crate) async fn set_project_field(
        &self,
        project: &NodeId,
        item: &NodeId,
        field: &NodeId,
        value: Option<&FieldValue>,
    ) -> Result<(), ApiError> {
        let ids = serde_json::json!({
            "project": project.as_str(),
            "item": item.as_str(),
            "field": field.as_str(),
        });
        let sent = match value {
            None => self.mutate_json(raw::CLEAR_PROJECT_FIELD, ids).await,
            Some(value) => {
                let value = match value {
                    FieldValue::Text(text) => serde_json::json!({ "text": text }),
                    FieldValue::Number(n) => {
                        let n: f64 = n.trim().parse().map_err(|_| {
                            ApiError::Internal(format!("{n:?} isn't a number to send"))
                        })?;
                        serde_json::json!({ "number": n })
                    }
                    FieldValue::Date(date) => serde_json::json!({ "date": date }),
                    FieldValue::Option { id, .. } => {
                        serde_json::json!({ "singleSelectOptionId": id })
                    }
                    FieldValue::Iteration { id, .. } => serde_json::json!({ "iterationId": id }),
                };
                let mut vars = ids;
                if let Some(vars) = vars.as_object_mut() {
                    vars.insert("value".into(), value);
                }
                self.mutate_json(raw::SET_PROJECT_FIELD, vars).await
            }
        };
        sent.map_err(scoped)
    }
}

// ---- wire ---------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireSummary {
    number: u64,
    title: String,
    short_description: Option<String>,
    url: String,
    closed: bool,
    public: bool,
    updated_at: String,
    items: wire::Count,
}

impl WireSummary {
    fn into_summary(self) -> ProjectSummary {
        ProjectSummary {
            number: self.number,
            title: self.title,
            description: self.short_description.unwrap_or_default(),
            url: self.url,
            closed: self.closed,
            public: self.public,
            updated_at: self.updated_at,
            items: self.items.total_count,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireProject {
    id: NodeId,
    title: String,
    short_description: Option<String>,
    url: String,
    closed: bool,
    public: bool,
    viewer_can_update: bool,
    fields: wire::Counted<WireField>,
    views: wire::Counted<WireView>,
}

/// An item of a project an issue or pull request is in.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireInProject {
    id: NodeId,
    project: WireOfProject,
    field_values: wire::Counted<WireValue>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireOfProject {
    id: NodeId,
    title: String,
    url: String,
    viewer_can_update: bool,
    fields: wire::Counted<WireField>,
}

impl WireInProject {
    fn into_in_project(self) -> Option<InProject> {
        let values = self.field_values.nodes.into_iter().flatten();
        Some(InProject {
            title: self.project.title,
            url: self.project.url,
            id: self.project.id,
            item: self.id,
            can_update: self.project.viewer_can_update,
            fields: self.project.fields.into_capped(WireField::into_field),
            values: values.filter_map(WireValue::into_value).collect(),
        })
    }
}

/// A field, or what names one (a view's, a value's).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireField {
    id: Option<NodeId>,
    name: Option<String>,
    data_type: Option<String>,
    options: Option<Vec<WireOption>>,
    configuration: Option<WireIterations>,
}

#[derive(Deserialize)]
struct WireOption {
    id: String,
    name: String,
    color: String,
    description: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireIterations {
    iterations: Vec<WireIteration>,
    completed_iterations: Vec<WireIteration>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireIteration {
    id: String,
    title: String,
    start_date: String,
    duration: u64,
}

impl WireIteration {
    fn into_iteration(self, completed: bool) -> Iteration {
        Iteration {
            id: self.id,
            title: self.title,
            start: self.start_date,
            days: self.duration,
            completed,
        }
    }
}

impl WireField {
    fn into_field(self) -> Option<ProjectField> {
        let kind = match self.data_type.as_deref() {
            Some("TITLE") => FieldKind::Title,
            Some("ASSIGNEES") => FieldKind::Assignees,
            Some("LABELS") => FieldKind::Labels,
            Some("REPOSITORY") => FieldKind::Repository,
            Some("MILESTONE") => FieldKind::Milestone,
            Some("TEXT") => FieldKind::Text,
            Some("NUMBER") => FieldKind::Number,
            Some("DATE") => FieldKind::Date,
            Some("SINGLE_SELECT") => FieldKind::SingleSelect(
                (self.options.unwrap_or_default().into_iter())
                    .map(|o| SelectOption {
                        id: o.id,
                        name: o.name,
                        color: o.color,
                        description: o.description,
                    })
                    .collect(),
            ),
            Some("ITERATION") => {
                let (now, done) = self
                    .configuration
                    .map_or_else(Default::default, |c| (c.iterations, c.completed_iterations));
                let now = now.into_iter().map(|i| i.into_iteration(false));
                let done = done.into_iter().map(|i| i.into_iteration(true));
                FieldKind::Iteration(now.chain(done).collect())
            }
            _ => FieldKind::Other,
        };
        Some(ProjectField {
            id: self.id?,
            name: self.name?,
            kind,
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireView {
    number: u64,
    name: String,
    layout: String,
    filter: Option<String>,
    fields: Option<wire::Counted<WireField>>,
    vertical_group_by_fields: Option<wire::Nodes<WireField>>,
    group_by_fields: Option<wire::Nodes<WireField>>,
    sort_by_fields: Option<wire::Counted<WireSort>>,
}

#[derive(Deserialize)]
struct WireSort {
    direction: String,
    field: WireField,
}

impl WireView {
    fn into_view(self) -> Option<ProjectView> {
        let first = |n: Option<wire::Nodes<WireField>>| n.into_iter().flatten().find_map(|f| f.id);
        Some(ProjectView {
            number: self.number,
            name: self.name,
            layout: match self.layout.as_str() {
                "BOARD_LAYOUT" => Layout::Board,
                "ROADMAP_LAYOUT" => Layout::Roadmap,
                _ => Layout::Table,
            },
            filter: self.filter.unwrap_or_default(),
            fields: (self.fields.into_iter())
                .flat_map(|f| f.nodes.into_iter().flatten())
                .filter_map(|f| f.id)
                .collect(),
            column: first(self.vertical_group_by_fields),
            group: first(self.group_by_fields),
            sort: (self.sort_by_fields.into_iter())
                .flat_map(|s| s.nodes.into_iter().flatten())
                .filter_map(|s| Some((s.field.id?, s.direction == "DESC")))
                .collect(),
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireItem {
    id: NodeId,
    /// A `BigInt`, which GitHub sends as a string.
    full_database_id: Option<serde_json::Value>,
    #[serde(default)]
    is_archived: bool,
    content: Option<WireContent>,
    field_values: wire::Counted<WireValue>,
}

#[derive(Deserialize)]
#[serde(tag = "__typename")]
enum WireContent {
    DraftIssue {
        id: NodeId,
        title: String,
        body: String,
        assignees: wire::Counted<Login>,
    },
    Issue(WireLinked),
    PullRequest(WireLinked),
}

#[derive(Deserialize)]
struct Login {
    login: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireLinked {
    number: u64,
    title: String,
    url: String,
    state: String,
    state_reason: Option<String>,
    is_draft: Option<bool>,
    repository: wire::RepoName,
    assignees: wire::Counted<Login>,
    labels: Option<wire::Counted<WireLabel>>,
    milestone: Option<WireTitle>,
}

#[derive(Deserialize)]
struct WireLabel {
    name: String,
    color: String,
}

#[derive(Deserialize)]
struct WireTitle {
    title: String,
}

impl WireLinked {
    fn into_linked(self) -> Option<Linked> {
        let label = |l: WireLabel| {
            Some(Label {
                name: l.name,
                color: l.color,
            })
        };
        Some(Linked {
            repo: RepoId::parse(&self.repository.name_with_owner)?,
            number: self.number,
            title: self.title,
            url: self.url,
            assignees: self.assignees.into_capped(|u| Some(u.login)),
            labels: (self.labels)
                .map_or_else(|| Capped::new(Vec::new(), 0), |l| l.into_capped(label)),
            milestone: self.milestone.map(|m| m.title),
        })
    }
}

/// A value, by its type; the ones ghtui doesn't read are `Other`.
#[derive(Deserialize)]
#[serde(tag = "__typename")]
enum WireValue {
    #[serde(rename = "ProjectV2ItemFieldTextValue")]
    Text {
        text: Option<String>,
        field: WireField,
    },
    #[serde(rename = "ProjectV2ItemFieldNumberValue")]
    Number {
        number: Option<serde_json::Number>,
        field: WireField,
    },
    #[serde(rename = "ProjectV2ItemFieldDateValue")]
    Date {
        date: Option<String>,
        field: WireField,
    },
    #[serde(rename = "ProjectV2ItemFieldSingleSelectValue")]
    Option {
        #[serde(rename = "optionId")]
        option_id: Option<String>,
        name: Option<String>,
        field: WireField,
    },
    #[serde(rename = "ProjectV2ItemFieldIterationValue")]
    Iteration {
        #[serde(rename = "iterationId")]
        iteration_id: String,
        title: String,
        field: WireField,
    },
    #[serde(other)]
    Other,
}

impl WireValue {
    fn into_value(self) -> Option<(NodeId, FieldValue)> {
        Some(match self {
            WireValue::Text { text, field } => (field.id?, FieldValue::Text(text?)),
            WireValue::Number { number, field } => {
                let n = number?.to_string();
                let n = n
                    .strip_suffix(".0")
                    .map_or_else(|| n.clone(), str::to_owned);
                (field.id?, FieldValue::Number(n))
            }
            WireValue::Date { date, field } => (field.id?, FieldValue::Date(date?)),
            WireValue::Option {
                option_id,
                name,
                field,
            } => (
                field.id?,
                FieldValue::Option {
                    id: option_id?,
                    name: name.unwrap_or_default(),
                },
            ),
            WireValue::Iteration {
                iteration_id,
                title,
                field,
            } => (
                field.id?,
                FieldValue::Iteration {
                    id: iteration_id,
                    title,
                },
            ),
            WireValue::Other => return None,
        })
    }
}

impl WireItem {
    /// An item; one without a number for its URL can't be shown.
    fn into_item(self) -> Option<ProjectItem> {
        let number = match self.full_database_id? {
            serde_json::Value::String(s) => s.parse().ok()?,
            serde_json::Value::Number(n) => n.as_u64()?,
            _ => return None,
        };
        let content = match self.content {
            None => ItemContent::Redacted,
            Some(WireContent::DraftIssue {
                id,
                title,
                body,
                assignees,
            }) => ItemContent::Draft(Draft {
                id,
                title,
                body,
                assignees: assignees.into_capped(|u| Some(u.login)),
            }),
            Some(WireContent::Issue(issue)) => {
                let state = match (issue.state.as_str(), issue.state_reason.as_deref()) {
                    ("OPEN", _) => IssueState::Open,
                    (_, Some("NOT_PLANNED" | "DUPLICATE")) => IssueState::NotPlanned,
                    _ => IssueState::Closed,
                };
                ItemContent::Issue(issue.into_linked()?, state)
            }
            Some(WireContent::PullRequest(pr)) => {
                let state = match (pr.state.as_str(), pr.is_draft) {
                    ("MERGED", _) => PrState::Merged,
                    ("CLOSED", _) => PrState::Closed,
                    (_, Some(true)) => PrState::Draft,
                    _ => PrState::Open,
                };
                ItemContent::Pr(pr.into_linked()?, state)
            }
        };
        let values = self.field_values.nodes.into_iter().flatten();
        Some(ProjectItem {
            id: self.id,
            number,
            archived: self.is_archived,
            content,
            values: values.filter_map(WireValue::into_value).collect(),
        })
    }
}
