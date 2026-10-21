//! GitHub Projects: an owner's or a repository's projects, a project's
//! view (a board's columns of cards, in swimlanes if it has them; a
//! table's rows of fields; a roadmap's items on a timeline), and an item
//! as a view's side pane shows it.

use super::*;
use crate::api::project::{
    Dates, FieldKind, FieldValue, Group, ItemContent, Layout, Project, ProjectField, ProjectItem,
    ProjectRef, ProjectSummary, ProjectView, ProjectsOf, grouped,
};

/// An owner's projects, or those linked to a repository.
pub fn projects(
    page: &mut Page,
    of: &ProjectsOf,
    list: Fetched<'_, Results<ProjectSummary>>,
    now: u64,
) {
    let empty = match of {
        ProjectsOf::Owner(_) => "No projects you can see.",
        ProjectsOf::Repo(_) => "No projects are linked to this repository.",
    };
    paged_list(page, "Projects", list, empty, |page, p| {
        item(page, p.url.clone(), |page, link| {
            let mut segs = vec![Seg::linked(p.title.clone(), Role::Strong, link)];
            closed_private(&mut segs, p.closed, p.public);
            let right = vec![meta(format!("#{}", p.number))];
            page.box_line(segs, right, 0);
            let items = plural(p.items, "item");
            let updated = time::ago_iso(&p.updated_at, now);
            let subline = match p.description.as_str() {
                "" => format!("{items} · updated {updated}"),
                about => format!("{about} · {items} · updated {updated}"),
            };
            body(page, vec![meta(subline)]);
        });
    });
}

/// The issue or pull request an item is, if it's one.
fn linked(it: &ProjectItem) -> Option<&crate::api::project::Linked> {
    match &it.content {
        ItemContent::Issue(l, _) | ItemContent::Pr(l, _) => Some(l),
        ItemContent::Draft(_) | ItemContent::Redacted => None,
    }
}

/// Where an item is: an issue's or pull request's `o/r#12`, or a draft's.
fn place(it: &ProjectItem) -> Option<String> {
    match &it.content {
        ItemContent::Draft(_) => Some("Draft".to_owned()),
        _ => linked(it).map(|l| format!("{}#{}", l.repo, l.number)),
    }
}

/// Where an item opens: an issue's or pull request's page, or (for a
/// draft) the view's side pane. One you can't see opens nothing.
pub fn item_link(project: &ProjectRef, view: Option<u64>, it: &ProjectItem) -> Option<String> {
    match &it.content {
        ItemContent::Issue(l, _) | ItemContent::Pr(l, _) => Some(l.url.clone()),
        ItemContent::Draft(_) => Some(url::project_item(project, view, it.number)),
        ItemContent::Redacted => None,
    }
}

/// A project at a view (`view` as its route names it): its title, what
/// the view shows, then a board's columns of cards or a table's rows.
pub fn project_view(
    page: &mut Page,
    project: &ProjectRef,
    view: Option<u64>,
    p: &Project,
    cx: PageCtx<'_>,
) {
    heading(page, p);
    let Some(shown) = p.shown() else { return };
    let column = p.column_field();
    page.wrapped(about(p, shown, column), 0, Frame::None);
    let at = Spot { project, view, p };
    match (shown.layout, column) {
        (Layout::Board, Some(field)) => board(page, &at, field, cx),
        (Layout::Board, None) => {
            let note = "This board has no single-select or iteration field for its columns: it shows as a table.";
            say(page, note, Role::Meta);
            table(page, &at, shown, cx);
        }
        (Layout::Roadmap, _) => match p.roadmap_dates() {
            Some(dates) => roadmap(page, &at, shown, dates, cx),
            None => {
                let note = "This roadmap has no date or iteration field to place its items by: it shows as a table.";
                say(page, note, Role::Meta);
                table(page, &at, shown, cx);
            }
        },
        (Layout::Table, _) => table(page, &at, shown, cx),
    }
    more(page, &p.items);
    archived(page, &at, cx);
}

/// What a page shows of a project's items.
struct Spot<'a> {
    project: &'a ProjectRef,
    view: Option<u64>,
    p: &'a Project,
}

/// Chips for a project that's closed, or private.
fn closed_private(segs: &mut Vec<Seg>, closed: bool, public: bool) {
    if closed {
        tag(segs, "Closed", Bg::TertiaryContainer);
    }
    if !public {
        tag(segs, "Private", Bg::SecondaryContainer);
    }
}

/// The project's title, with whether it's closed or private, and its
/// description.
fn heading(page: &mut Page, p: &Project) {
    let mut title = vec![Seg::new(p.title.clone(), Role::Title)];
    closed_private(&mut title, p.closed, p.public);
    page.wrapped(title, 0, Frame::None);
    if !p.description.is_empty() {
        say(page, p.description.clone(), Role::Body);
    }
}

/// What the view shows: its layout and name, how many items, and its
/// filter, columns, grouping and sort.
fn about(p: &Project, view: &ProjectView, column: Option<&ProjectField>) -> Vec<Seg> {
    let layout = match view.layout {
        Layout::Board => "Board",
        Layout::Table => "Table",
        Layout::Roadmap => "Roadmap",
    };
    let mut said = vec![format!("{layout} · {}", plural(p.items.total, "item"))];
    let name = |id: &crate::api::model::NodeId| p.field(id).map(|f| f.name.clone());
    if let (Layout::Board, Some(field)) = (view.layout, column) {
        said.push(format!("columns by {}", field.name));
    }
    if let Some(group) = view.group.as_ref().and_then(name) {
        if p.lanes().is_some() {
            said.push(format!("swimlanes by {group}"));
        } else {
            said.push(format!("grouped by {group}"));
        }
    }
    let sorts: Vec<String> = (view.sort.iter())
        .filter_map(|(id, descending)| {
            let field = name(id)?;
            Some(if *descending {
                format!("{field} ↓")
            } else {
                field
            })
        })
        .collect();
    if !sorts.is_empty() {
        said.push(format!("sorted by {}", sorts.join(", ")));
    }
    // GitHub's order and groups are the whole view's: so are these, but
    // past the most loaded at once.
    if Project::arranges(view) && p.items.next.is_some() {
        said.push(format!(
            "arranged over the first {} loaded",
            p.items.items.len()
        ));
    }
    let mut segs = vec![meta(said.join(" · "))];
    if !view.filter.is_empty() {
        segs.extend([
            meta(" · filtered by "),
            Seg::new(view.filter.clone(), Role::Code),
        ]);
    }
    segs
}

/// Narrowest a board's column gets beside others.
const COLUMN_MIN: u16 = 30;
/// Columns between a board's columns.
const COLUMN_GAP: u16 = 1;

/// A board: a box for each column (each value of `field`), its cards in
/// the view's order; with swimlanes, the columns again for each lane (each
/// value of the view's grouping), under its name.
fn board(page: &mut Page, at: &Spot<'_>, field: &ProjectField, cx: PageCtx<'_>) {
    let Some((lane_field, lanes)) = at.p.lanes() else {
        let shown = card_fields(at.p, &[&field.id]);
        return columns(page, at, field, &at.p.sorted(), &shown, cx);
    };
    let shown = card_fields(at.p, &[&field.id, &lane_field.id]);
    for lane in &lanes {
        page.blank();
        let count = plural(lane.items.len() as u64, "item");
        let title = vec![meta("▾ "), group_chip(lane), meta(format!("  {count}"))];
        page.wrapped(title, 0, Frame::None);
        columns(page, at, field, &lane.items, &shown, cx);
    }
}

/// A board's columns of `items`: side by side, as many as fit (then
/// the next ones below), or one above another when two don't fit.
fn columns(
    page: &mut Page,
    at: &Spot<'_>,
    field: &ProjectField,
    items: &[&ProjectItem],
    shown: &[&ProjectField],
    cx: PageCtx<'_>,
) {
    let columns = grouped(field, items);
    let fit = usize::from(page.width.saturating_add(COLUMN_GAP) / (COLUMN_MIN + COLUMN_GAP));
    if fit < 2 || columns.len() < 2 {
        for c in &columns {
            column(page, at, field, c, shown, cx);
        }
        return;
    }
    let per = fit.min(columns.len());
    let gaps = COLUMN_GAP.saturating_mul(u16::try_from(per - 1).unwrap_or(0));
    let width = page.width.saturating_sub(gaps) / u16::try_from(per).unwrap_or(1);
    for band in columns.chunks(per) {
        page.blank();
        page.side_by_side(&vec![width; band.len()], COLUMN_GAP, |i, page| {
            if let Some(c) = band.get(i) {
                column(page, at, field, c, shown, cx);
            }
        });
    }
}

/// A board's column: its value, how many cards, and the cards.
fn column(
    page: &mut Page,
    at: &Spot<'_>,
    field: &ProjectField,
    column: &Group<'_>,
    shown: &[&ProjectField],
    cx: PageCtx<'_>,
) {
    let title = group_title(column, false);
    // The description only where it fits beside the title and count,
    // which matter more.
    let room = usize::from(page.room(Frame::Top, 0));
    let taken = title.iter().map(|s| width(s.text())).sum::<usize>() + 6;
    let about = option_description(field, column).filter(|d| taken + width(d) <= room);
    page.box_top(title, about.map(|d| vec![meta(d)]).unwrap_or_default());
    if column.items.is_empty() {
        empty_row(page, "No items");
    }
    for (n, it) in column.items.iter().enumerate() {
        if n > 0 && !page.compact {
            page.box_rule();
        }
        card(page, at, it, shown, cx);
    }
    page.box_bottom();
}

/// The items archived, to open or restore: a box of rows, as a table's.
fn archived(page: &mut Page, at: &Spot<'_>, cx: PageCtx<'_>) {
    let a = &at.p.archived;
    if a.items.is_empty() {
        return;
    }
    let title = vec![strong(format!("Archived  {}", a.total))];
    let hint = format!("{} restores one", cx.keys.archive);
    page.box_top(title, vec![meta(hint)]);
    for it in &a.items {
        let (icon, role) = item_icon(it, cx.icons);
        let place = place(it).unwrap_or_default();
        maybe_item(page, item_link(at.project, at.view, it), |page, link| {
            let title = Seg::linked(it.title().to_owned(), title_role(link), link);
            let segs = vec![Seg::new(format!("{icon} "), role), title];
            page.box_line(segs, vec![meta(place)], 0);
        });
    }
    if a.items.len() < usize::try_from(a.total).unwrap_or(usize::MAX) {
        let more = left_out_text(
            a.total.saturating_sub(a.items.len() as u64),
            "more archived item",
            "more archived items",
        );
        page.box_line(vec![meta(more)], Vec::new(), 0);
    }
    page.box_bottom();
}

/// The fields a card shows: the view's, but its title and those its
/// place on the page says (its column's, its lane's).
fn card_fields<'a>(p: &'a Project, said: &[&crate::api::model::NodeId]) -> Vec<&'a ProjectField> {
    let view = p.shown();
    (view.iter().flat_map(|v| v.fields.iter()))
        .filter(|id| !said.contains(id))
        .filter_map(|id| p.field(id))
        .filter(|f| !matches!(f.kind, FieldKind::Title | FieldKind::Other))
        .collect()
}

/// An option's description, for its column's top edge.
fn option_description(field: &ProjectField, group: &Group<'_>) -> Option<String> {
    let FieldKind::SingleSelect(options) = &field.kind else { return None };
    let option = options
        .iter()
        .find(|o| Some(&o.id) == group.value.as_ref())?;
    (!option.description.is_empty()).then(|| option.description.clone())
}

/// A view's items in its groups, or in one group of them all.
fn view_groups<'a>(
    p: &'a Project,
    view: &ProjectView,
    items: Vec<&'a ProjectItem>,
) -> Vec<Group<'a>> {
    match view.group.as_ref().and_then(|id| p.field(id)) {
        Some(field) => grouped(field, &items),
        None => vec![Group {
            value: None,
            name: String::new(),
            color: None,
            items,
        }],
    }
}

/// A group's box title: `Items  n` when it's all of them (`plain`), else
/// its chip and count.
fn group_title(group: &Group<'_>, plain: bool) -> Vec<Seg> {
    let n = group.items.len();
    if plain {
        vec![strong(format!("Items  {n}"))]
    } else {
        vec![group_chip(group), meta(format!("  {n}"))]
    }
}

/// A group's box: `header` over its rows.
fn group_box(
    page: &mut Page,
    (title, right): (Vec<Seg>, Vec<Seg>),
    header: Vec<Seg>,
    group: &Group<'_>,
    mut row: impl FnMut(&mut Page, &ProjectItem),
) {
    page.box_top(title, right);
    page.box_line(header, Vec::new(), 0);
    page.box_rule();
    if group.items.is_empty() {
        empty_row(page, "No items");
    }
    for it in &group.items {
        row(page, it);
    }
    page.box_bottom();
}

/// A column's or group's name, in its option's color.
fn group_chip(group: &Group<'_>) -> Seg {
    Seg::new(format!(" {} ", group.name), colored(group.color))
}

/// An option's color, as a label's.
fn colored(color: Option<&str>) -> Role {
    Role::Label(hex(color).to_owned())
}

/// GitHub's colors for a project's options, as label colors.
fn hex(color: Option<&str>) -> &'static str {
    match color {
        Some("BLUE") => "0969da",
        Some("GREEN") => "1a7f37",
        Some("YELLOW") => "9a6700",
        Some("ORANGE") => "bc4c00",
        Some("RED") => "cf222e",
        Some("PINK") => "bf3989",
        Some("PURPLE") => "8250df",
        _ => "6e7781",
    }
}

/// An item's icon: its state's for an issue or pull request.
fn item_icon(it: &ProjectItem, icons: Icons) -> (&'static str, Role) {
    match it.content.status() {
        Some(status) => issue_icon(icons, status),
        None if matches!(it.content, ItemContent::Draft(_)) => ("◌", Role::Meta),
        None => ("⊘", Role::Meta),
    }
}

/// A card: its title, then where it is and its fields' values.
fn card(
    page: &mut Page,
    at: &Spot<'_>,
    it: &ProjectItem,
    fields: &[&ProjectField],
    cx: PageCtx<'_>,
) {
    let (icon, role) = item_icon(it, cx.icons);
    let mut subline = Vec::from_iter(place(it).map(meta));
    for field in fields {
        let value = value_segs(it, field);
        if !value.is_empty() {
            sep(&mut subline, " · ");
            subline.extend(value);
        }
    }
    let Some(link) = item_link(at.project, at.view, it) else {
        let segs = vec![meta(it.title().to_owned())];
        hanging(page, Seg::new(format!("{icon} "), role), segs, Frame::Body);
        return;
    };
    item(page, link, |page, link| {
        let segs = vec![Seg::linked(it.title().to_owned(), Role::Strong, link)];
        hanging(page, Seg::new(format!("{icon} "), role), segs, Frame::Body);
        if !subline.is_empty() && !page.compact {
            page.wrapped(subline, 2, Frame::Body);
        }
    });
}

/// An item's value of `field`, as GitHub shows it: options as chips in
/// their color, labels as labels.
fn value_segs(it: &ProjectItem, field: &ProjectField) -> Vec<Seg> {
    let linked = linked(it);
    let text = |s: String| vec![meta(s)];
    match &field.kind {
        FieldKind::Title | FieldKind::Other => Vec::new(),
        FieldKind::Assignees => {
            let who = match &it.content {
                ItemContent::Issue(l, _) | ItemContent::Pr(l, _) => &l.assignees,
                ItemContent::Draft(d) => &d.assignees,
                ItemContent::Redacted => return Vec::new(),
            };
            if who.is_empty() {
                return Vec::new();
            }
            let names = who.join(", ");
            text(match who.left_out() {
                0 => names,
                n => format!("{names} +{n}"),
            })
        }
        FieldKind::Labels => {
            let mut segs = Vec::new();
            if let Some(l) = linked.filter(|l| !l.labels.is_empty()) {
                labels(&mut segs, &l.labels);
                // `labels` spaces each chip from what's before it.
                segs.remove(0);
            }
            segs
        }
        FieldKind::Repository => linked.map(|l| text(l.repo.to_string())).unwrap_or_default(),
        FieldKind::Milestone => (linked.and_then(|l| l.milestone.clone()))
            .map(text)
            .unwrap_or_default(),
        FieldKind::SingleSelect(options) => {
            let Some(value) = it.value(&field.id) else { return Vec::new() };
            let color = options
                .iter()
                .find(|o| o.name == value.text())
                .map(|o| o.color.as_str());
            vec![Seg::new(format!(" {} ", value.text()), colored(color))]
        }
        FieldKind::Text | FieldKind::Number | FieldKind::Date | FieldKind::Iteration(_) => it
            .value(&field.id)
            .map(|v| text(format!("{}: {}", field.name, v.text())))
            .unwrap_or_default(),
    }
}

/// An item's value of `field` in a table's cell: the text alone.
fn cell_text(it: &ProjectItem, field: &ProjectField) -> String {
    match &field.kind {
        FieldKind::Labels => (linked(it).iter().flat_map(|l| &l.labels))
            .map(|l| l.name.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        FieldKind::Text
        | FieldKind::Number
        | FieldKind::Date
        | FieldKind::Iteration(_)
        | FieldKind::SingleSelect(_) => it
            .value(&field.id)
            .map_or_else(String::new, |v| v.text().to_owned()),
        _ => value_segs(it, field).iter().map(Seg::text).collect(),
    }
}

/// Columns a table's title gets at least, before its fields.
const TITLE_MIN: usize = 24;
/// Widest a table's field column gets.
const CELL_MAX: usize = 20;

/// A table: a box of rows (one per group, if the view groups them), the
/// title and the view's fields in columns, as many as fit.
fn table(page: &mut Page, at: &Spot<'_>, view: &ProjectView, cx: PageCtx<'_>) {
    let items = at.p.sorted();
    let fields = card_fields(at.p, &[]);
    let room = usize::from(page.room(Frame::Body, 0));
    // Each field as wide as its widest value or name, while the title
    // keeps its room.
    let mut columns: Vec<(&ProjectField, usize)> = Vec::new();
    let mut used = TITLE_MIN + 2;
    for field in &fields {
        let widest = (items.iter())
            .map(|it| width(&cell_text(it, field)))
            .chain([width(&field.name)])
            .max()
            .unwrap_or(0)
            .min(CELL_MAX);
        if used + widest + 2 > room {
            break;
        }
        used += widest + 2;
        columns.push((field, widest));
    }
    let hidden = fields.len() - columns.len();
    let title_width = room
        .saturating_sub(used - TITLE_MIN)
        .max(TITLE_MIN.min(room));
    let groups = view_groups(at.p, view, items);
    let one = groups.len() == 1 && view.group.is_none();
    for group in &groups {
        let title = group_title(group, one);
        let right = if hidden > 0 {
            vec![meta(format!(
                "{} more {} past the edge",
                hidden,
                noun(hidden as u64, "field")
            ))]
        } else {
            Vec::new()
        };
        let mut header = vec![meta(pad("Title", title_width + 2))];
        for (field, w) in &columns {
            header.push(meta(pad(&truncate(&field.name, *w), w + 2)));
        }
        group_box(page, (title, right), header, group, |page, it| {
            row(page, at, it, &columns, title_width, cx);
        });
    }
}

/// Columns a roadmap's titles get at most, and at least.
const ROADMAP_TITLE: (usize, usize) = (36, 16);

/// A roadmap: each item a row, its title, then a bar on a timeline from
/// its start to its end (one day: a diamond), in its Status's color;
/// grouped as the view groups them. The timeline spans the items' dates
/// and today, one column for as many days as fit, which the box says.
fn roadmap(page: &mut Page, at: &Spot<'_>, view: &ProjectView, dates: Dates<'_>, cx: PageCtx<'_>) {
    let mut items = at.p.sorted();
    let span = |it: &ProjectItem| Project::span(dates, it);
    if view.sort.is_empty() {
        // Without a sort of its own, by when each starts, the undated last.
        items.sort_by_key(|it| span(it).map_or(i64::MAX, |(start, _)| start));
    }
    let today = crate::ui::time::today(cx.now);
    let spans: Vec<(i64, i64)> = items.iter().filter_map(|it| span(it)).collect();
    let first = spans.iter().map(|s| s.0).fold(today, i64::min) - 3;
    let last = spans.iter().map(|s| s.1).fold(today, i64::max) + 3;
    let room = usize::from(page.room(Frame::Body, 0));
    let title_width = (room / 3).clamp(ROADMAP_TITLE.1, ROADMAP_TITLE.0).min(room);
    let bar_width = room.saturating_sub(title_width + 2).max(1);
    let days = usize::try_from(last - first + 1).unwrap_or(1);
    let per = days.div_ceil(bar_width).max(1);
    let line = Timeline {
        first,
        per: i64::try_from(per).unwrap_or(1),
        width: bar_width,
        today,
    };
    let by = match dates {
        Dates::Fields {
            start,
            end: Some(end),
        } => format!("{} → {}", start.name, end.name),
        Dates::Fields { start, end: None } => start.name.clone(),
        Dates::Iteration(field) => field.name.clone(),
    };
    let scale = if per == 1 {
        "a column a day".to_owned()
    } else {
        format!("a column for {per} days")
    };
    let note = format!(
        "On a timeline by {by} ({scale}); GitHub's API doesn't say which fields the view uses, so these are its first date fields."
    );
    say(page, note, Role::Meta);
    let groups = view_groups(at.p, view, items);
    let status = at.p.fields.iter().find(|f| f.name == "Status");
    for group in &groups {
        let title = group_title(group, view.group.is_none());
        let header = vec![meta(" ".repeat(title_width + 2)), meta(line.months())];
        group_box(page, (title, Vec::new()), header, group, |page, it| {
            let color = status.and_then(|f| option_color(f, it));
            let bar = line.bar(span(it), color);
            item_row(page, at, it, title_width, bar, cx);
        });
    }
}

/// A roadmap's timeline: which day its first column is, how many days a
/// column is, how many columns, and today.
struct Timeline {
    first: i64,
    per: i64,
    width: usize,
    today: i64,
}

impl Timeline {
    /// The column `day` falls in, if it's on the timeline.
    fn column(&self, day: i64) -> Option<usize> {
        let c = usize::try_from((day - self.first).div_euclid(self.per)).ok()?;
        (c < self.width).then_some(c)
    }

    /// The months' names where each starts (the year with the first and
    /// with January), as far as each has room, and today's column.
    fn months(&self) -> String {
        // Where each month starts: the first column's, then each change.
        let mut starts: Vec<(usize, i64, i64)> = Vec::new();
        for c in 0..self.width {
            let day = self.first + i64::try_from(c).unwrap_or(0) * self.per;
            let (year, m, _) = crate::ui::time::civil(day);
            if starts.last().is_none_or(|(_, _, last)| *last != m) {
                starts.push((c, year, m));
            }
        }
        let mut out = vec![' '; self.width];
        if let Some(c) = self.column(self.today)
            && let Some(slot) = out.get_mut(c)
        {
            *slot = '┊';
        }
        let mut free = 0;
        let mut named = false;
        for (i, (c, year, m)) in starts.iter().enumerate() {
            let name = month_of(*m);
            let label = if !named || *m == 1 {
                format!("{name} {year}")
            } else {
                name.to_owned()
            };
            let len = label.chars().count();
            // A month begun before the timeline gives way to the next.
            let next = starts.get(i + 1).map_or(self.width, |n| n.0);
            if *c < free || c + len > self.width || (i == 0 && c + len >= next) {
                continue;
            }
            for (k, ch) in label.chars().enumerate() {
                if let Some(slot) = out.get_mut(c + k) {
                    *slot = ch;
                }
            }
            free = c + len + 1;
            named = true;
        }
        out.into_iter().collect()
    }

    /// An item's bar from its first to its last day, its dates in it
    /// (or after it) where they fit, or a diamond for one day; today's
    /// column marked where it's free.
    fn bar(&self, span: Option<(i64, i64)>, color: Option<&str>) -> Vec<Seg> {
        let Some((start, end)) = span else { return vec![meta("no dates")] };
        let (Some(a), Some(b)) = (self.column(start), self.column(end)) else {
            return Vec::new();
        };
        let today = self.column(self.today);
        let gap = |from: usize, to: usize| {
            let text: String = (from..to)
                .map(|c| if Some(c) == today { '┊' } else { ' ' })
                .collect();
            meta(text)
        };
        let mut segs = vec![gap(0, a)];
        let said = dates(start, end);
        let len = width(&said);
        let mut end_col = b + 1;
        if a == b && start == end {
            segs.push(strong("◆"));
        } else if len + 2 <= b - a + 1 {
            segs.push(Seg::new(format!(" {}", pad(&said, b - a)), colored(color)));
            segs.push(gap(b + 1, self.width));
            return segs;
        } else {
            segs.push(Seg::new(" ".repeat(b - a + 1), colored(color)));
        }
        // The dates after the bar, where they fit.
        if b + 2 + len <= self.width {
            segs.push(gap(b + 1, b + 2));
            segs.push(meta(said));
            end_col = b + 2 + len;
        }
        segs.push(gap(end_col, self.width));
        segs
    }
}

/// `Oct 5 → 31`, `Oct 28 → Nov 3`, or one day's `Oct 5`.
fn dates(start: i64, end: i64) -> String {
    let name = month_of;
    let (_, m1, d1) = crate::ui::time::civil(start);
    let (_, m2, d2) = crate::ui::time::civil(end);
    match (start == end, m1 == m2) {
        (true, _) => format!("{} {d1}", name(m1)),
        (false, true) => format!("{} {d1} → {d2}", name(m1)),
        (false, false) => format!("{} {d1} → {} {d2}", name(m1), name(m2)),
    }
}

/// "Oct" for a civil date's month 10.
fn month_of(m: i64) -> &'static str {
    usize::try_from(m).ok().and_then(month_abbr).unwrap_or("")
}

/// An item's color by `field`'s option: GitHub's name for it.
fn option_color<'a>(field: &'a ProjectField, it: &ProjectItem) -> Option<&'a str> {
    let FieldKind::SingleSelect(options) = &field.kind else { return None };
    let Some(FieldValue::Option { id, .. }) = it.value(&field.id) else { return None };
    options
        .iter()
        .find(|o| o.id == *id)
        .map(|o| o.color.as_str())
}

/// A table's or roadmap's row: the item's icon and title (linked where
/// it opens), then `tail` (its cells, its bar).
fn item_row(
    page: &mut Page,
    at: &Spot<'_>,
    it: &ProjectItem,
    title_width: usize,
    tail: Vec<Seg>,
    cx: PageCtx<'_>,
) {
    let (icon, role) = item_icon(it, cx.icons);
    let title = truncate(it.title(), title_width.saturating_sub(2));
    let rest = title_width.saturating_sub(width(&title) + 2) + 2;
    maybe_item(page, item_link(at.project, at.view, it), |page, link| {
        let mut segs = vec![
            Seg::new(format!("{icon} "), role),
            Seg::linked(title, title_role(link), link),
            Seg::new(" ".repeat(rest), Role::Body),
        ];
        segs.extend(tail);
        page.box_line(segs, Vec::new(), 0);
    });
}

/// An item's title: strong where it opens, else gray.
fn title_role(link: Option<u32>) -> Role {
    link.map_or(Role::Meta, |_| Role::Strong)
}

/// `s` padded with spaces to `cols` columns.
fn pad(s: &str, cols: usize) -> String {
    format!("{s}{}", " ".repeat(cols.saturating_sub(width(s))))
}

/// A table's row: the item's icon and title, then a cell per field.
fn row(
    page: &mut Page,
    at: &Spot<'_>,
    it: &ProjectItem,
    columns: &[(&ProjectField, usize)],
    title_width: usize,
    cx: PageCtx<'_>,
) {
    let mut cells = Vec::new();
    for (field, w) in columns {
        let text = truncate(&cell_text(it, field), *w);
        let shown = pad(&text, w + 2);
        cells.extend(match &field.kind {
            FieldKind::SingleSelect(options) if !text.is_empty() => {
                let color = (it.value(&field.id))
                    .and_then(|v| options.iter().find(|o| o.name == v.text()))
                    .map(|o| o.color.as_str());
                vec![
                    Seg::new(text.clone(), colored(color)),
                    Seg::new(" ".repeat(w + 2 - width(&text)), Role::Body),
                ]
            }
            _ => vec![Seg::new(shown, Role::Body)],
        });
    }
    item_row(page, at, it, title_width, cells, cx);
}

/// "Load more" while the view has more items, or how many GitHub didn't
/// list.
fn more(page: &mut Page, items: &Results<ProjectItem>) {
    let shown = items.items.len();
    let (target, text) = match items.rest() {
        Rest::All | Rest::Uncounted => return,
        Rest::OnGitHub(n) | Rest::At(n, _) => {
            page.blank();
            let text = left_out_text(n, "more item", "more items");
            return page.line(vec![meta(text)]);
        }
        Rest::NextPage { total } => {
            let of = total.map_or_else(
                || format!("{shown} so far"),
                |total| format!("{shown} of {}", compact(total)),
            );
            (Link::More, format!("Load more  ({of})"))
        }
    };
    page.blank();
    item(page, target, |page, link| {
        page.line(vec![Seg::linked(text, Role::Link, link)]);
    });
}

/// An item, as a view's side pane shows it: its title, where it is, its
/// fields, and a draft's body.
pub fn project_item(
    page: &mut Page,
    project: &ProjectRef,
    view: Option<u64>,
    p: &Project,
    number: u64,
    cx: PageCtx<'_>,
) {
    let back = url::project_view(project, view);
    let Some(it) = p.item(number) else {
        let link = link_seg(page, p.title.clone(), back, Role::Link);
        let why = "This item isn't among those loaded from ";
        page.wrapped(vec![meta(why), link], 0, Frame::None);
        say(
            page,
            "It may be further down the view, past Load more, or no longer in it.",
            Role::Meta,
        );
        return;
    };
    let (icon, role) = item_icon(it, cx.icons);
    let mut title = vec![Seg::new(it.title().to_owned(), Role::Title)];
    match &it.content {
        ItemContent::Draft(_) => title.extend([space(), chip("Draft", Bg::SecondaryContainer)]),
        content => {
            if let Some(status) = content.status() {
                title.extend([space(), state_chip(status, cx.icons)]);
            }
        }
    }
    if it.archived {
        tag(&mut title, "Archived", Bg::TertiaryContainer);
    }
    hanging(page, Seg::new(format!("{icon} "), role), title, Frame::None);
    let mut place = vec![meta("In ")];
    place.push(link_seg(page, p.title.clone(), back, Role::Link));
    if let Some(l) = linked(it) {
        place.push(meta(" · "));
        let name = format!("{}#{}", l.repo, l.number);
        place.push(link_seg(page, name, l.url.clone(), Role::Link));
    }
    page.wrapped(place, 0, Frame::None);
    page.blank();
    // Every field with a value, as the pane lists them.
    let fields: Vec<(&ProjectField, Vec<Seg>)> = (p.fields.iter())
        .map(|f| (f, value_segs(it, f)))
        .filter(|(_, v)| !v.is_empty())
        .collect();
    let name_width = fields
        .iter()
        .map(|(f, _)| width(&f.name))
        .max()
        .unwrap_or(0);
    page.box_top(vec![strong("Fields")], Vec::new());
    if fields.is_empty() {
        empty_row(page, "No fields set.");
    }
    for (field, value) in fields {
        let mut segs = vec![meta(pad(&field.name, name_width + 2))];
        // The pane names a field once: its value alone.
        segs.extend(match &field.kind {
            FieldKind::Text | FieldKind::Number | FieldKind::Date | FieldKind::Iteration(_) => {
                vec![Seg::new(cell_text(it, field), Role::Body)]
            }
            _ => value,
        });
        page.box_line(segs, Vec::new(), 0);
    }
    page.box_bottom();
    if let ItemContent::Draft(d) = &it.content {
        page.blank();
        if d.body.trim().is_empty() {
            page.line(vec![meta("No description provided.")]);
        } else {
            markdown::render(page, &d.body, None, Frame::None);
        }
    }
}
