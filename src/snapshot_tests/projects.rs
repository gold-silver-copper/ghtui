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

fn change_sent(cmds: &[Cmd]) -> &Change {
    match cmds {
        [Cmd::Api(Api::Change(change, _))] => change,
        _ => panic!("{cmds:?}"),
    }
}

/// `,i` adds what's typed: an issue or pull request by its reference
/// (not one already in the project), or a new draft, written in the
/// composer with its title on the first line. The board follows GitHub
/// until the item is in it.
#[test]
fn items_are_added_by_reference_or_as_drafts() {
    use crate::api::project::{ItemContent, ProjectChange};
    let mut s = with_project(Mode::Dark, None);
    press(&mut s, ",i");
    press(&mut s, "gold-silver-copper/ghtui#14");
    let Some(Overlay::Picker(picker)) = &s.overlay else {
        panic!("no picker");
    };
    let rows = s.picker_rows(picker);
    assert_eq!(rows[0].0.hint, "already in the project");
    press(&mut s, "<C-u>gold-silver-copper/ghtui#20");
    let sent = press(&mut s, "<Enter>");
    let add = Change::Project(ProjectChange::Add {
        project: NodeId::new("PVT_1"),
        repo: crate::api::model::RepoId::new("gold-silver-copper", "ghtui"),
        number: 20,
    });
    assert_eq!(change_sent(&sent), &add);
    answer(&mut s, &sent, Ok(()));
    says(&s, "Added to the project");
    assert!(s.awaiting.is_some());
    let key = DataKey::Project(roadmap(), None);
    let mut p = fixtures::project();
    let mut added = p.items.items[0].clone();
    if let ItemContent::Issue(l, _) = &mut added.content {
        l.number = 20;
    }
    p.items.items.push(added);
    fetched(&mut s, key, Data::Project(Box::new(p)));
    assert!(s.awaiting.is_none());

    // A draft: its title typed, its text written in the composer.
    press(&mut s, ",i");
    press(&mut s, "Plan the release<Enter>");
    assert!(matches!(s.overlay, Some(Overlay::Compose(_))));
    let sent = press(&mut s, "Dates and notes<C-s>");
    let draft = Change::Project(ProjectChange::AddDraft {
        project: NodeId::new("PVT_1"),
        title: "Plan the release".into(),
        body: "Dates and notes".into(),
    });
    assert_eq!(change_sent(&sent), &draft);
}

/// `,X` archives the selected item, asking first, as GitHub does; it's
/// then under Archived, where `,X` restores it.
#[test]
fn items_are_archived_and_restored() {
    use crate::api::project::ProjectChange;
    let mut s = with_project(Mode::Dark, None);
    press(&mut s, "l");
    assert!(press(&mut s, ",X").is_empty());
    let Some(Overlay::Confirm(c)) = &s.overlay else {
        panic!("not asked");
    };
    assert_eq!(c.title, "Archive “Projects aren't there yet”?");
    let sent = press(&mut s, "<Enter>");
    let archive = Change::Project(ProjectChange::Archive {
        project: NodeId::new("PVT_1"),
        item: NodeId::new("PVTI_1"),
        archived: true,
    });
    assert_eq!(change_sent(&sent), &archive);
    answer(&mut s, &sent, Ok(()));
    let key = DataKey::Project(roadmap(), None);
    let mut p = fixtures::project();
    let mut gone = p.items.items.remove(0);
    gone.archived = true;
    p.archived.items.push(gone);
    fetched(&mut s, key, Data::Project(Box::new(p)));
    assert!(s.awaiting.is_none());

    // Restoring asks nothing.
    press(&mut s, "G");
    while !selected(&s).contains("itemId=1006") {
        press(&mut s, "k");
    }
    let sent = press(&mut s, ",X");
    let restore = Change::Project(ProjectChange::Archive {
        project: NodeId::new("PVT_1"),
        item: NodeId::new("PVTI_6"),
        archived: false,
    });
    assert_eq!(change_sent(&sent), &restore);
}

/// `,e` on a draft edits its title (the first line) and its text; on an
/// issue, it says where that's edited.
#[test]
fn a_drafts_title_and_text_are_edited() {
    use crate::api::project::ProjectChange;
    let mut s = with_project(Mode::Dark, None);
    press(&mut s, "lj");
    press(&mut s, ",e");
    let Some(Overlay::Compose(c)) = &s.overlay else {
        panic!("no composer");
    };
    assert_eq!(
        c.text(),
        "Write the board's docs\n\nExplain **h** and **l**, and moving cards."
    );
    let sent = press(&mut s, "<C-s>");
    let edit = Change::Project(ProjectChange::EditDraft {
        draft: NodeId::new("DI_3"),
        title: "Write the board's docs".into(),
        body: "Explain **h** and **l**, and moving cards.".into(),
    });
    assert_eq!(change_sent(&sent), &edit);
    s.overlay = None;
    press(&mut s, "k,e");
    says(&s, "Only a draft's text is edited here");
}

/// `,f` on an issue sets its field in the project it's in, from its
/// sidebar, with the board's pickers; the page follows GitHub until
/// the sidebar shows the value.
#[test]
fn an_issues_field_is_set_from_its_sidebar() {
    let mut s = super::with_issue(Mode::Dark);
    let said = "Roadmap (In progress · Estimate: 3)";
    assert!(page_text(&s).contains(said), "{}", page_text(&s));
    press(&mut s, ",f");
    let Some(Overlay::Picker(picker)) = &s.overlay else {
        panic!("no picker");
    };
    let fields: Vec<(String, String)> = (s.picker_rows(picker).into_iter())
        .map(|(r, _)| (r.label, r.hint))
        .collect();
    let want =
        [("Status", "In progress"), ("Estimate", "3")].map(|(f, v)| (f.to_owned(), v.to_owned()));
    assert_eq!(fields, want);
    press(&mut s, "Status<Enter>");
    let sent = press(&mut s, "Done<Enter>");
    let done = FieldValue::Option {
        id: "done".into(),
        name: "Done".into(),
    };
    let change = Change::SetProjectField {
        project: NodeId::new("PVT_1"),
        item: NodeId::new("PVTI_14"),
        field: NodeId::new("F_status"),
        value: Some(done.clone()),
        said: "Status to Done".into(),
    };
    let about = Subject::Issue(super::ghtui(), 14);
    assert_eq!(sent, vec![Cmd::Api(Api::Change(change, By::Act(about)))]);
    answer(&mut s, &sent, Ok(()));
    let key = DataKey::InProjects(super::ghtui(), 14);
    let polled = crate::act::poll(&mut s);
    assert!(
        polled.contains(&crate::browse::Need::Data(key.clone())),
        "{polled:?}"
    );
    let mut projects = fixtures::in_projects();
    let unchanged = Data::InProjects(Box::new(projects.clone()));
    fetched(&mut s, key.clone(), unchanged);
    assert!(s.awaiting.is_some());
    projects.items[0].values[0].1 = done;
    fetched(&mut s, key, Data::InProjects(Box::new(projects)));
    assert!(s.awaiting.is_none());
    let said = "Roadmap (Done · Estimate: 3)";
    assert!(page_text(&s).contains(said), "{}", page_text(&s));
}
