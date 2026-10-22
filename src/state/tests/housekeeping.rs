//! Repository housekeeping: labels, milestones, branches and tags, forks,
//! new repositories and files. Each asks where GitHub would, sends one
//! change (to GitHub, or to git in your clone), and the page follows
//! GitHub until it shows it.

use crate::route::{code, issues, pulls};
use std::path::PathBuf;

use crate::api::housekeeping::{
    BranchRules, FileLook, LabelChange, MadeRepo, MilestoneChange, NewFork, NewRepo, RefKind,
};
use crate::housekeeping::{FileChange, FormOf, MilestoneWrite, RefWrite};
use crate::housekeeping::{files, labels, milestones, refs, repos};

use super::*;
use crate::fixtures::{self};

fn picker(s: &State) -> bool {
    matches!(&s.overlay, Some(Overlay::Picker(_)))
}

/// Selects the row whose line has `text`.
fn select(s: &mut State, text: &str) {
    fixtures::select_where(s, |p| {
        let item = p.selected.and_then(|i| p.page().items.get(i));
        item.and_then(|item| p.page().lines.get(item.start))
            .is_some_and(|line| line.text().contains(text))
    });
}

/// The labels page, with GitHub's labels.
fn with_labels() -> State {
    at(with_repo(), Route::Issues(issues::Labels(ghtui())))
}

fn labels_key() -> DataKey {
    DataKey::Labels(ghtui())
}

/// The labels page lists each label with its open issues; `,o` writes a
/// new one in its form (its name first), `,s` makes it, and the page
/// follows GitHub until it lists it.
#[test]
fn a_label_is_made_and_the_page_follows() {
    let mut s = with_labels();
    let screen = screen_text(&s);
    assert!(
        screen.contains("good first issue") && screen.contains("12 open"),
        "{screen}"
    );
    act(&mut s, Action::NewHere);
    let form = Route::Housekeeping(FormOf::Label {
        repo: ghtui(),
        name: None,
    });
    assert_eq!(s.route(), Some(&form));
    assert!(picker(&s), "its name first");
    press(&mut s, "triage<Enter>");
    // A colour: GitHub's, or one typed.
    select(&mut s, "Colour");
    press(&mut s, "<Enter><C-u>#ABCDEF<Enter>");
    let cmds = press(&mut s, "<C-s>");
    let work = fixtures::sent::<labels::Save>(&cmds);
    let made = LabelChange {
        name: "triage".into(),
        color: "abcdef".into(),
        description: String::new(),
    };
    assert_eq!((&work.was, &work.change), (&None, &Some(made)));
    fixtures::done(&mut s, work, Ok(()));
    assert_eq!(
        s.route(),
        Some(&Route::Issues(issues::Labels(ghtui()))),
        "back on the labels"
    );
    says(&s, "Created the label triage");
    assert!(s.busy().is_some_and(|b| b.contains("Waiting for GitHub")));
    let mut labels = crate::housekeeping::fixtures::labels();
    labels.items.push(crate::api::housekeeping::LabelInfo {
        name: "triage".into(),
        color: "ABCDEF".into(),
        description: String::new(),
        open: 0,
    });
    let data = Data::from(labels);
    fetched(&mut s, labels_key(), data);
    assert!(s.following.is_none(), "GitHub lists it");
}

/// `,e` edits the selected label in its form; `,D` deletes it after
/// asking, saying how many issues it comes off.
#[test]
fn a_label_is_edited_and_deleted() {
    let mut s = with_labels();
    select(&mut s, "good first issue");
    press(&mut s, "e");
    let cmds = press(&mut s, "<C-s>");
    let work = fixtures::sent::<labels::Save>(&cmds);
    assert_eq!(work.was.as_deref(), Some("good first issue"));
    let change = work.change.clone().unwrap();
    assert_eq!(change.color, "7057ff");
    assert_eq!(change.description, "Good for newcomers");
    fixtures::done(&mut s, work, Ok(()));
    assert_eq!(s.route(), Some(&Route::Issues(issues::Labels(ghtui()))));

    select(&mut s, "bug");
    press(&mut s, "D");
    let ask = overlay!(s, Ask);
    assert_eq!(ask.title, "Delete the label bug?");
    assert!(
        ask.facts
            .iter()
            .any(|(f, _)| f.contains("its 4 open issues and pull requests")),
        "{:?}",
        ask.facts
    );
    let cmds = press(&mut s, "<Enter>");
    assert_eq!(
        fixtures::sent::<labels::Save>(&cmds),
        labels::Save {
            repo: ghtui(),
            was: Some("bug".into()),
            change: None,
        }
    );
}

/// Milestones: `,x` closes the selected one (GitHub doesn't ask), `,e`
/// edits its title, due date and description, and `,D` deletes it after
/// asking; the list follows each.
#[test]
fn milestones_are_closed_edited_and_deleted() {
    let mut s = with_milestones();
    select(&mut s, "Pages");
    let cmds = press(&mut s, "x");
    let close = milestones::Save {
        repo: ghtui(),
        number: Some(5),
        write: MilestoneWrite::Close(true),
    };
    assert_eq!(fixtures::sent::<milestones::Save>(&cmds), close);
    fixtures::done(&mut s, close, Ok(Some(5)));
    says(&s, "Closed the milestone");
    let mut list = fixtures::milestones();
    list.results.items.retain(|m| m.number != 5);
    let key = DataKey::Milestones(ghtui(), false);
    fetched(&mut s, key, Data::from(list));
    assert!(s.following.is_none(), "gone from the open ones");

    select(&mut s, "Links");
    press(&mut s, "e");
    let form = Route::Housekeeping(FormOf::Milestone {
        repo: ghtui(),
        number: Some(3),
    });
    assert_eq!(s.route(), Some(&form));
    fetched(
        &mut s,
        DataKey::Milestone(ghtui(), 3),
        Data::from(fixtures::milestone()),
    );
    assert!(page_text(&s).contains("2026-11-05"), "{}", page_text(&s));
    select(&mut s, "Due date");
    press(&mut s, "<Enter>2026-13-01");
    assert!(page_text(&s).contains("Due date"));
    press(&mut s, "<C-u>2027-01-31<Enter>");
    let cmds = press(&mut s, "<C-s>");
    let work = fixtures::sent::<milestones::Save>(&cmds);
    let saved = MilestoneChange {
        title: "Links".into(),
        description: fixtures::milestone().info.description,
        due_on: Some("2027-01-31".into()),
    };
    assert_eq!(work.write, MilestoneWrite::Save(saved));
    fixtures::done(&mut s, work, Ok(Some(3)));
    assert_eq!(
        s.route(),
        Some(&Route::Issues(issues::Milestones {
            repo: ghtui(),
            closed: false
        }))
    );

    select(&mut s, "Links");
    press(&mut s, "D");
    assert_eq!(overlay!(s, Ask).title, "Delete the milestone Links?");
    let cmds = press(&mut s, "<Enter>");
    let delete = milestones::Save {
        repo: ghtui(),
        number: Some(3),
        write: MilestoneWrite::Delete,
    };
    assert_eq!(fixtures::sent::<milestones::Save>(&cmds), delete);
}

/// A branch is made from the branches list at the branch selected, by
/// GitHub's API without a clone; the default branch and a protected one
/// are never deleted, and say why.
#[test]
fn branches_are_made_and_protected_ones_kept() {
    let mut s = with_branches();
    let repo = ghtui();
    let cmds = fixtures::cloned(&mut s, &repo, None);
    assert!(cmds.is_empty(), "{cmds:?}");
    select(&mut s, "tabs");
    act(&mut s, Action::NewHere);
    assert!(picker(&s), "its name first");
    press(&mut s, "topic<Enter>");
    // Where it starts: the branch selected first.
    let cmds = press(&mut s, "<Enter>");
    assert_eq!(
        fixtures::sent::<refs::Save>(&cmds),
        refs::Save {
            repo: repo.clone(),
            kind: RefKind::Branch,
            name: "topic".into(),
            write: RefWrite::Create("tabs".into()),
            clone: None,
        }
    );

    select(&mut s, "main");
    press(&mut s, "D");
    says(&s, "main is the default branch");

    select(&mut s, "links");
    let cmds = press(&mut s, "D");
    let key = DataKey::BranchRules(repo.clone(), "links".into());
    assert_eq!(fixtures::asked(&cmds), std::slice::from_ref(&key));
    let rules = BranchRules {
        protected: false,
        undeletable: true,
    };
    fixtures::fetched(&mut s, key, Data::from(rules));
    says(
        &s,
        "a ruleset of the repository's keeps it from being deleted",
    );
    assert!(s.overlay.is_none());
}

/// In your clone, a tag is made with git and pushed, after asking; and a
/// branch deleted the same way, its pull request's closing said.
#[test]
fn in_a_clone_git_pushes_after_asking() {
    let mut s = with_branches();
    let repo = ghtui();
    let clone = crate::git::local::Clone {
        top: PathBuf::from("/work/ghtui"),
        remote: "origin".into(),
        branch: Some("main".into()),
    };
    fixtures::cloned(&mut s, &repo, Some(clone.clone()));
    select(&mut s, "tabs");
    let cmds = press(&mut s, "D");
    let rules = BranchRules {
        protected: false,
        undeletable: false,
    };
    let key = DataKey::BranchRules(repo.clone(), "tabs".into());
    fixtures::fetched(&mut s, key.clone(), Data::from(rules));
    assert_eq!(fixtures::asked(&cmds), [key]);
    let ask = overlay!(s, Ask);
    assert_eq!(ask.title, "Delete the branch tabs?");
    assert!(
        ask.facts
            .iter()
            .any(|(f, _)| f.contains("Your clone's own branch tabs stays")),
        "{:?}",
        ask.facts
    );
    let cmds = press(&mut s, "<Enter>");
    let delete = refs::Save {
        repo: repo.clone(),
        kind: RefKind::Branch,
        name: "tabs".into(),
        write: RefWrite::Delete,
        clone: Some(clone),
    };
    assert_eq!(fixtures::sent::<refs::Save>(&cmds), delete);
    fixtures::done(&mut s, delete, Ok(()));
    says(&s, "Deleted the branch tabs");
    let mut branches = fixtures::branches();
    branches.items.retain(|b| b.name != "tabs");
    fetched(&mut s, DataKey::Branches(repo), Data::from(branches));
    assert!(s.following.is_none());
}

/// `,F` forks into your account, the default branch only unless asked;
/// the fork's page then follows GitHub until it's there.
#[test]
fn a_repository_is_forked_and_opened() {
    let mut s = with_repo();
    act(&mut s, Action::Fork);
    let form = Route::Housekeeping(FormOf::Fork(ghtui()));
    assert_eq!(s.route(), Some(&form));
    let owners = Data::from(crate::housekeeping::fixtures::owners());
    fetched(&mut s, DataKey::Owners, owners);
    assert!(page_text(&s).contains("It will be octocat/ghtui"));
    let cmds = press(&mut s, "<C-s>");
    let new = NewFork {
        repo: ghtui(),
        org: None,
        name: "ghtui".into(),
        default_branch_only: true,
    };
    let fork = repos::Fork { route: form, new };
    assert_eq!(fixtures::sent::<repos::Fork>(&cmds), fork);
    let made = MadeRepo {
        full_name: "octocat/ghtui".into(),
        clone_url: "https://github.com/octocat/ghtui.git".into(),
    };
    fixtures::done(&mut s, fork, Ok(made));
    let fork = crate::api::model::RepoId::new("octocat", "ghtui");
    assert_eq!(s.route(), Some(&Route::code(fork)));
    assert!(s.busy().is_some_and(|b| b.contains("make the repository")));
}

/// `,C` makes a repository: its owner (an organization you may make
/// repositories in), name, visibility and templates; then it opens, and
/// cloning it is offered.
#[test]
fn a_new_repository_is_made_and_cloning_offered() {
    let mut s = with_repo();
    act(&mut s, Action::NewRepo);
    let form = Route::Housekeeping(FormOf::NewRepo);
    assert_eq!(s.route(), Some(&form));
    let owners = Data::from(crate::housekeeping::fixtures::owners());
    fetched(&mut s, DataKey::Owners, owners);
    let templates = Data::from(crate::housekeeping::fixtures::templates());
    fetched(&mut s, DataKey::Templates, templates);
    assert!(picker(&s), "its name first");
    press(&mut s, "tools<Enter>");
    select(&mut s, "Owner");
    press(&mut s, "<Enter>readers");
    let readers = crate::housekeeping::rows(&s, &picked(&s), "readers");
    assert!(
        readers.iter().all(|(_, chosen)| chosen.is_none()),
        "not yours to make in"
    );
    press(&mut s, "<C-u>ghtui-org<Enter>");
    select(&mut s, "Visibility");
    press(&mut s, "<Enter>");
    select(&mut s, "Licence");
    press(&mut s, "<Enter>MIT<Enter>");
    let cmds = press(&mut s, "<C-s>");
    let new = NewRepo {
        org: Some("ghtui-org".into()),
        name: "tools".into(),
        description: String::new(),
        private: true,
        readme: false,
        gitignore: None,
        license: Some("mit".into()),
    };
    let make = repos::Make { route: form, new };
    assert_eq!(fixtures::sent::<repos::Make>(&cmds), make);
    let made = MadeRepo {
        full_name: "ghtui-org/tools".into(),
        clone_url: "https://github.com/ghtui-org/tools.git".into(),
    };
    fixtures::done(&mut s, make, Ok(made));
    let repo = crate::api::model::RepoId::new("ghtui-org", "tools");
    assert_eq!(s.route(), Some(&Route::code(repo)));
    assert_eq!(overlay!(s, Ask).title, "Clone ghtui-org/tools?");
    let cmds = press(&mut s, "<Enter>");
    assert_eq!(
        fixtures::sent::<repos::CloneTo>(&cmds),
        repos::CloneTo {
            url: "https://github.com/ghtui-org/tools.git".into(),
            dir: PathBuf::from("tools"),
        }
    );
}

/// The picker open now.
fn picked(s: &State) -> crate::housekeeping::Pick {
    match &s.overlay {
        Some(Overlay::Picker(p)) => match &p.kind {
            crate::picker::Kind::Housekeeping(pick) => pick.clone(),
            _ => panic!("not a housekeeping picker"),
        },
        _ => panic!("no picker"),
    }
}

/// What GitHub has of `src/main.rs` on main.
fn look() -> FileLook {
    fixtures::file_look()
}

/// The form for `change` to the fixture file.
fn main_rs(change: FileChange) -> Route {
    let (repo, rev, path) = (ghtui(), "main".into(), "src/main.rs".into());
    Route::Housekeeping(FormOf::File {
        repo,
        rev,
        path,
        change,
    })
}

/// `,e` on a file reads it from GitHub and opens the composer on it; a
/// new branch is chosen, ctrl-s asks before committing, and the commit
/// names the file as it was read. The file then opens on the new branch,
/// and a pull request is offered.
#[test]
fn a_file_is_edited_and_committed_on_a_new_branch() {
    let mut s = with_file();
    fixtures::cloned(&mut s, &ghtui(), None);
    let cmds = press(&mut s, "e");
    let form = main_rs(FileChange::Edit);
    assert_eq!(s.route(), Some(&form));
    let file = DataKey::FileLook(ghtui(), "main".into(), "src/main.rs".into());
    assert!(fixtures::asks(&cmds, &file), "{cmds:?}");
    read(&mut s);
    let c = fixtures::composer(&mut s);
    assert!(c.text().contains("Says hello."));
    c.input.insert_str("// Edited.\n");
    press(&mut s, "<Esc>");
    select(&mut s, "Commit to");
    press(&mut s, "<Enter>octocat-patch-1<Enter>");
    press(&mut s, "<C-s>");
    let ask = overlay!(s, Ask);
    assert_eq!(ask.title, "Commit changes?");
    assert!(
        ask.facts
            .iter()
            .any(|(f, _)| f.contains("a new branch, octocat-patch-1"))
    );
    assert!(ask.facts.iter().any(|(f, _)| f == "On GitHub, by you"));
    let cmds = press(&mut s, "<Enter>");
    let commit = fixtures::sent::<files::Commit>(&cmds);
    let write = &commit.write;
    assert_eq!(commit.route, form);
    assert_eq!(write.commit.branch, "octocat-patch-1");
    assert_eq!(write.commit.from.as_deref(), Some(look().commit.as_str()));
    assert_eq!(
        write.commit.seen,
        [("src/main.rs".into(), Some("b10b".into()))]
    );
    assert_eq!(write.commit.message, "Update main.rs");
    let text = &write.commit.additions[0].1;
    assert!(text.starts_with("// Edited.\nuse std::io;"), "{text}");
    fixtures::done(&mut s, commit, Ok(()));
    let page = Route::blob(ghtui(), "octocat-patch-1".into(), "src/main.rs".into());
    assert_eq!(s.route(), Some(&page));
    assert_eq!(
        overlay!(s, Ask).title,
        "Open a pull request for octocat-patch-1?"
    );
    press(&mut s, "<Enter>");
    assert_eq!(s.route(), Some(&Route::Pulls(pulls::NewPr(ghtui()))));
    let pr = (s
        .forms
        .get::<crate::create::Form>(&Route::Pulls(pulls::NewPr(ghtui()))))
    .unwrap();
    assert_eq!(pr.head.as_deref(), Some("octocat-patch-1"));
}

/// A file changed on GitHub since it was opened isn't overwritten: the
/// commit says so, and the text stays in the form.
#[test]
fn a_file_changed_meanwhile_keeps_your_text() {
    let mut s = with_file();
    fixtures::cloned(&mut s, &ghtui(), None);
    press(&mut s, "e");
    let form = main_rs(FileChange::Edit);
    read(&mut s);
    let c = fixtures::composer(&mut s);
    c.input.insert_str("Mine. ");
    press(&mut s, "<C-s>");
    let cmds = press(&mut s, "<Enter>");
    let commit = fixtures::sent::<files::Commit>(&cmds);
    let write = &commit.write;
    assert_eq!(write.commit.branch, "main");
    assert_eq!(write.commit.from, None);
    let why = "src/main.rs changed on GitHub since you opened it: committing would overwrite that";
    fixtures::done(&mut s, commit, Err(fixtures::failure(why)));
    assert_eq!(s.route(), Some(&form));
    assert!(page_text(&s).contains("changed on GitHub since you opened it"));
    let kept = &s
        .forms
        .get::<crate::housekeeping::Form>(&form)
        .unwrap()
        .text;
    assert!(kept.starts_with("Mine. "), "{kept}");
}

/// Deleting a file is its own form: `,D` on it, then the commit, asked.
#[test]
fn a_file_is_deleted_with_a_commit() {
    let mut s = with_file();
    fixtures::cloned(&mut s, &ghtui(), None);
    press(&mut s, "D");
    let form = main_rs(FileChange::Delete);
    assert_eq!(s.route(), Some(&form));
    read(&mut s);
    press(&mut s, "<C-s>");
    assert_eq!(overlay!(s, Ask).title, "Delete src/main.rs?");
    let cmds = press(&mut s, "<Enter>");
    let commit = fixtures::sent::<files::Commit>(&cmds);
    let write = &commit.write;
    assert_eq!(write.commit.deletions, ["src/main.rs"]);
    assert!(write.commit.additions.is_empty());
    assert_eq!(write.commit.message, "Delete main.rs");
    fixtures::done(&mut s, commit, Ok(()));
    let folder = Route::Code(code::Tree {
        repo: ghtui(),
        rev: "main".into(),
        path: "src".into(),
    });
    assert_eq!(s.route(), Some(&folder));
}

/// Without write access, each says why, and how GitHub would have you
/// change the files.
#[test]
fn without_write_access_each_says_why() {
    let mut s = with_file();
    let mut overview = fixtures::overview();
    overview.can_write = false;
    fetched(&mut s, DataKey::Repo(ghtui()), Data::from(overview));
    press(&mut s, "e");
    says(
        &s,
        "Changing its files takes write access to gold-silver-copper/ghtui: fork it (:fork)",
    );
    let route = Route::Issues(issues::Labels(ghtui()));
    let _ = s.push(route);
    act(&mut s, Action::NewHere);
    says(&s, "Changing labels takes write access");
}

/// A file's form opened from within ghtui (by `g/`, with nothing new to
/// fetch) still reads the file from GitHub.
#[test]
fn a_file_form_opened_from_a_page_reads_the_file() {
    let mut s = with_repo();
    let cmds = press(
        &mut s,
        ":gold-silver-copper/ghtui/edit/main/README.md<Enter>",
    );
    let file = DataKey::FileLook(ghtui(), "main".into(), "README.md".into());
    assert!(fixtures::asks(&cmds, &file), "{cmds:?}");
}

/// GitHub's src/main.rs on main, read for its form.
fn read(s: &mut State) -> Cmds {
    let file = DataKey::FileLook(ghtui(), "main".into(), "src/main.rs".into());
    fixtures::fetched(s, file, Data::from(look()))
}
