use crate::api::change::Change;
use crate::api::model::NodeId;
use crate::api::project::{FieldValue, Project, ProjectOwner, ProjectRef, ProjectsOf};
use crate::theme::{ColorDepth, Mode};

use super::{fixtures, open, page_text, render, sized, state, with_profile, with_repo};
use crate::act::{By, Subject};
use crate::browse::{Data, DataKey};
use crate::fixtures::{answer, fetched, press, says};
use crate::route::Route;
use crate::state::{Api, Cmd, Overlay, Screen, State};
use crate::ui::page::Link;
use crate::ui::pages::ProfileTab;

fn roadmap() -> ProjectRef {
    ProjectRef {
        owner: ProjectOwner::Org("gold-silver-copper".into()),
        number: 1,
    }
}

/// The Roadmap project at `view`: its board (the first view), or its
/// table (view 2), which leaves out what's done.
pub(super) fn with_project(mode: Mode, view: Option<u64>) -> State {
    let mut s = sized(state(mode, ColorDepth::TrueColor), 100, 40);
    let mut p = fixtures::project();
    if view == Some(2) {
        p.view = 2;
        p.items.items.retain(|it| it.number != 1004);
        p.items.total = 4;
    }
    let route = Route::Project {
        project: roadmap(),
        view,
    };
    open(&mut s, route, Data::Project(Box::new(p)));
    s
}

/// The selected row's link.
fn selected(s: &State) -> String {
    let Screen::Page(p) = s.screen() else {
        panic!("not a page");
    };
    match p.selected_link() {
        Some(Link::Url(url)) => url.clone(),
        other => panic!("{other:?}"),
    }
}

const ISSUE: &str = "https://github.com/gold-silver-copper/ghtui/issues/14";

#[test]
fn project_board_dark() {
    assert_snapshot!(render(&with_project(Mode::Dark, None)));
}

#[test]
fn project_table_light() {
    assert_snapshot!(render(&with_project(Mode::Light, Some(2))));
}

/// A profile and a repository each have a Projects tab, which lists
/// their projects; a project opens from the list.
#[test]
fn projects_are_reached_from_profiles_and_repositories() {
    let projects = |s: &State| {
        s.chrome()
            .tabs
            .into_iter()
            .find(|(t, _)| t.label == "Projects")
            .map(|(_, target)| target)
    };
    let s = with_profile(Mode::Dark, ProfileTab::Overview);
    let user = ProjectsOf::Owner(ProjectOwner::User("octocat".into()));
    assert_eq!(
        projects(&s),
        Some(crate::route::Target::Page(Route::Projects(user)))
    );
    let mut s = with_repo(Mode::Dark, ColorDepth::TrueColor);
    let linked = Route::Projects(ProjectsOf::Repo(super::ghtui()));
    assert_eq!(
        projects(&s),
        Some(crate::route::Target::Page(linked.clone()))
    );
    open(
        &mut s,
        linked,
        Data::Projects(Box::new(fixtures::projects())),
    );
    let active = s
        .chrome()
        .active
        .and_then(|i| s.chrome().tabs.get(i).cloned());
    assert_eq!(active.map(|(t, _)| t.label), Some("Projects".to_owned()));
    let text = page_text(&s);
    assert!(
        text.contains("Roadmap") && text.contains("Launch"),
        "{text}"
    );
    press(&mut s, "<Enter>");
    let expect = Route::Project {
        project: roadmap(),
        view: None,
    };
    assert_eq!(s.route(), Some(&expect));
}

/// `h` and `l` go to the card at the same place in the column before or
/// after, past columns without cards, and say so at the board's edge.
#[test]
fn h_and_l_go_between_columns() {
    let mut s = with_project(Mode::Dark, None);
    // The first column is what has no Status.
    let someday = selected(&s);
    assert!(someday.contains("itemId=1005"), "{someday}");
    press(&mut s, "l");
    assert_eq!(selected(&s), ISSUE);
    press(&mut s, "jl");
    assert_eq!(
        selected(&s),
        "https://github.com/gold-silver-copper/ghtui/pull/12"
    );
    press(&mut s, "l");
    assert_eq!(
        selected(&s),
        "https://github.com/gold-silver-copper/ghtui/issues/9"
    );
    press(&mut s, "l");
    says(&s, "No column to the right has cards");
    press(&mut s, "hhh");
    assert_eq!(selected(&s), someday);

    let mut s = with_project(Mode::Dark, Some(2));
    press(&mut s, "l");
    says(&s, "Columns are a board's: this view is a table");
}

/// A draft opens in the view's side pane, as on GitHub: its fields and
/// its body.
#[test]
fn a_draft_shows_its_body() {
    let mut s = with_project(Mode::Dark, None);
    press(&mut s, "lj<Enter>");
    let expect = Route::ProjectItem {
        project: roadmap(),
        view: None,
        item: 1003,
    };
    assert_eq!(s.route(), Some(&expect));
    let text = page_text(&s);
    assert!(text.contains("Write the board's docs"), "{text}");
    assert!(
        text.contains("Explain h and l, and moving cards."),
        "{text}"
    );
    assert!(text.contains("Status") && text.contains("Todo"), "{text}");
}

/// `,c` lists the board's columns, the card's own marked; taking one
/// sets the card's Status on GitHub, and the board follows GitHub until
/// it shows the card in its new column.
#[test]
fn moving_a_card_sets_its_column_and_the_board_follows() {
    let mut s = with_project(Mode::Dark, None);
    press(&mut s, "l");
    assert!(press(&mut s, ",c").is_empty());
    let Some(Overlay::Picker(picker)) = &s.overlay else {
        panic!("no picker");
    };
    let rows = s.picker_rows(picker);
    let labels: Vec<(&str, &str)> = (rows.iter())
        .map(|(r, _)| (r.label.as_str(), r.hint.as_str()))
        .collect();
    assert_eq!(
        labels,
        [
            ("Todo", "current"),
            ("In Progress", ""),
            ("Done", "Shipped"),
            ("No Status", "")
        ]
    );
    let sent = press(&mut s, "Done<Enter>");
    let done = FieldValue::Option {
        id: "O_done".into(),
        name: "Done".into(),
    };
    let change = Change::SetProjectField {
        project: NodeId::new("PVT_1"),
        item: NodeId::new("PVTI_1"),
        field: NodeId::new("F_status"),
        value: Some(done.clone()),
        said: "Status to Done".into(),
    };
    let about = Subject::Project(roadmap(), None);
    assert_eq!(sent, vec![Cmd::Api(Api::Change(change, By::Act(about)))]);
    answer(&mut s, &sent, Ok(()));
    says(&s, "Set Status to Done");
    // Until GitHub shows it moved, the board is fetched again.
    let key = DataKey::Project(roadmap(), None);
    let mut p: Project = fixtures::project();
    fetched(&mut s, key.clone(), Data::Project(Box::new(p.clone())));
    assert!(s.awaiting.is_some());
    press(&mut s, ",c");
    says(&s, "GitHub hasn't shown the last change yet");
    if let Some(it) = p.items.items.first_mut() {
        it.values.retain(|(f, _)| f.as_str() != "F_status");
        it.values.push((NodeId::new("F_status"), done));
    }
    fetched(&mut s, key, Data::Project(Box::new(p)));
    assert!(s.awaiting.is_none());
    let text = page_text(&s);
    let (done_at, issue_at) = (text.find(" Done "), text.find("Projects aren't there yet"));
    assert!(done_at < issue_at, "{text}");
}

/// `,f` lists the fields that can be set, then a field's values; a
/// number is typed, and what isn't one can't be sent.
#[test]
fn setting_a_field_sends_its_value() {
    let mut s = with_project(Mode::Dark, Some(2));
    press(&mut s, ",f");
    let Some(Overlay::Picker(picker)) = &s.overlay else {
        panic!("no picker");
    };
    let fields: Vec<String> = (s.picker_rows(picker).into_iter())
        .map(|(r, _)| r.label)
        .collect();
    assert_eq!(fields, ["Status", "Estimate", "Sprint", "Due"]);
    press(&mut s, "Esti<Enter>x");
    let Some(Overlay::Picker(picker)) = &s.overlay else {
        panic!("no picker");
    };
    let rows = s.picker_rows(picker);
    assert_eq!(
        rows.first().map(|(r, _)| r.label.as_str()),
        Some("“x” isn't a number")
    );
    assert!(rows.first().is_some_and(|(_, chosen)| chosen.is_none()));
    let sent = press(&mut s, "<BS>8<Enter>");
    let [Cmd::Api(Api::Change(Change::SetProjectField { field, value, .. }, _))] = sent.as_slice()
    else {
        panic!("{sent:?}");
    };
    assert_eq!(field.as_str(), "F_estimate");
    assert_eq!(value, &Some(FieldValue::Number("8".into())));
}

/// What GitHub won't let you change says so, and changes nothing.
#[test]
fn a_project_you_cant_change_says_so() {
    let mut s = state(Mode::Dark, ColorDepth::TrueColor);
    let mut p = fixtures::project();
    p.can_update = false;
    let route = Route::Project {
        project: roadmap(),
        view: None,
    };
    open(&mut s, route, Data::Project(Box::new(p)));
    assert!(press(&mut s, ",c").is_empty());
    says(&s, "GitHub doesn't let you change this project");
    assert!(s.overlay.is_none());
}
