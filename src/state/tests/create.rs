//! New issues and pull requests, and checking one out: what each asks
//! first, what it sends, and the page following GitHub after.

use crate::route::{issues, pulls};
use std::path::PathBuf;

use crate::api::create::{NewIssue, NewPr};
use crate::git::local::{Branch, Checkout, Clone, CommitMessage};
use ratatui_textarea::CursorMove;

use super::*;
use crate::create::{HeadState, Push, Submission, Submit};
use crate::ui::form::Row;

/// The repository's page, then `keys` (`,i` or `,p`), with what the form
/// can have delivered.
fn form(action: Action) -> (State, Vec<Cmd>) {
    let mut s = with_repo();
    let cmds = act(&mut s, action);
    let form = Data::from(crate::fixtures::new_form());
    let mut more = fetched(&mut s, DataKey::NewForm(ghtui()), form);
    more.extend(cmds);
    (s, more)
}

/// The new issue's form, from the bug report template, its composer open.
fn bug_report() -> State {
    let (mut s, _) = form(Action::NewIssue);
    // The templates come first; the bug report (by file name) is selected.
    press(&mut s, "<Enter>");
    s
}

/// `,i` offers the templates first; the one chosen fills in the title,
/// the text (a form's fields as headings), the labels and the assignees.
/// `ctrl-s` sends them, and the new issue opens in place of the form.
#[test]
fn a_new_issue_starts_from_its_template_and_opens_once_made() {
    let (mut s, cmds) = form(Action::NewIssue);
    assert_eq!(s.route(), Some(&Route::Issues(issues::NewIssue(ghtui()))));
    let form = DataKey::NewForm(ghtui());
    assert!(crate::fixtures::asks(&cmds, &form), "{cmds:?}");
    let text = page_text(&s);
    for starter in ["Bug report", "Feature request", "Blank issue"] {
        assert!(text.contains(starter), "{text}");
    }
    press(&mut s, "<Enter>");
    let c = fixtures::composer(&mut s);
    assert_eq!(c.text(), "[Bug]: \n\n### What happened?\n\n### Version");
    c.input.move_cursor(CursorMove::Top);
    c.input.move_cursor(CursorMove::End);
    c.input.insert_str("it crashes");
    let sent = press(&mut s, "<C-s>");
    let route = Route::Issues(issues::NewIssue(ghtui()));
    let new = NewIssue {
        repo: NodeId::new("R_ghtui"),
        title: "[Bug]: it crashes".into(),
        body: "### What happened?\n\n### Version".into(),
        labels: vec![NodeId::new("LA_bug")],
        assignees: vec![NodeId::new("U_octo")],
        milestone: None,
    };
    let submit = Submit {
        route: route.clone(),
        new: Submission::Issue(new),
    };
    assert_eq!(sent, [fixtures::start(submit.clone())]);
    assert!(fixtures::composer(&mut s).sent.sending);

    let cmds = fixtures::done(&mut s, submit, Ok(42));
    let issue = Route::Issues(issues::Issue {
        repo: ghtui(),
        number: 42,
    });
    assert_eq!(s.route(), Some(&issue));
    assert!(s.overlay.is_none());
    says(&s, "Opened issue #42");
    let issue = DataKey::Issue(ghtui(), 42);
    assert!(crate::fixtures::asks(&cmds, &issue), "{cmds:?}");
    assert!(
        s.forms.get::<crate::create::Form>(&route).is_none(),
        "the form is done"
    );
}

/// GitHub's refusal stays in the composer, with the text, to try again.
#[test]
fn a_refused_issue_says_why_in_the_composer() {
    let mut s = bug_report();
    let submit = fixtures::sent::<Submit>(&press(&mut s, "<C-s>"));
    let refused = ApiError::GraphQl(vec!["Title can't be blank".into()]);
    fixtures::done(&mut s, submit, Err(refused.into()));
    let c = fixtures::composer(&mut s);
    assert!(!c.sent.sending);
    assert_eq!(
        c.sent.error.as_deref(),
        Some("GitHub didn't create the issue: Title can't be blank")
    );
    assert_eq!(s.route(), Some(&Route::Issues(issues::NewIssue(ghtui()))));
}

/// Labels are picked and unpicked in a picker that stays open; esc in the
/// composer keeps what's written, on the page.
#[test]
fn labels_are_picked_and_text_is_kept() {
    let mut s = bug_report();
    fixtures::composer(&mut s).input.insert_str("\nMore.");
    press(&mut s, "<Esc>");
    assert!(s.overlay.is_none());
    assert!(page_text(&s).contains("More."), "{}", page_text(&s));
    let _ = crate::forms::row(&mut s, &Row::Labels);
    // bug is on (from the template): off; then documentation on.
    press(&mut s, "<Enter><Down><Enter>");
    assert!(
        matches!(s.overlay, Some(Overlay::Picker(_))),
        "still picking"
    );
    press(&mut s, "<Esc>");
    let form = (s
        .forms
        .get::<crate::create::Form>(&Route::Issues(issues::NewIssue(ghtui()))))
    .unwrap();
    assert_eq!(form.labels, [NodeId::new("LA_docs")]);
}

fn commit(oid: &str, message: &str) -> CommitMessage {
    CommitMessage {
        oid: oid.into(),
        message: message.into(),
    }
}

/// Branch `fix-parser` of the clone, and where GitHub has it.
fn local_branch(on_github: Option<&str>) -> Branch {
    let oid = |s: &str| Oid::parse(&s.repeat(40)).unwrap();
    Branch {
        name: "fix-parser".into(),
        head: oid("a"),
        remote: "origin".into(),
        owner: Some("gold-silver-copper".into()),
        on_github: on_github.map(oid),
        ahead: on_github.map(|_| 0),
        behind: on_github.map(|_| 0),
        tracked: on_github.is_some(),
    }
}

/// Where `fix-parser` stands against main.
fn head(s: &mut State, on_github: Option<&str>) -> Vec<Cmd> {
    let hs = HeadState {
        local: Some(local_branch(on_github)),
        commits: vec![commit(
            &"a".repeat(40),
            "Fix the parser\n\nIt dropped the last line.",
        )]
        .into(),
    };
    fixtures::fetched(s, compare(), Data::from(hs))
}

/// Where `fix-parser` stands against main, as it's asked for.
fn compare() -> DataKey {
    DataKey::Head {
        repo: ghtui(),
        base: "main".into(),
        head: "fix-parser".into(),
    }
}

/// A new pull request inside a clone, on branch `fix-parser`.
fn in_clone() -> (State, Vec<Cmd>) {
    let (mut s, cmds) = form(Action::NewPr);
    assert!(fixtures::asks(&cmds, &DataKey::Clone(ghtui())), "{cmds:?}");
    let clone = Clone {
        top: PathBuf::from("/src/ghtui"),
        remote: "origin".into(),
        branch: Some("fix-parser".into()),
    };
    let cmds = fixtures::cloned(&mut s, &ghtui(), Some(clone));
    (s, cmds)
}

/// Inside a clone, `,p` proposes its branch into the default one, filled
/// in from its one commit and the template. A branch GitHub doesn't have
/// is pushed (asking first) before `,s` asks whether it's a draft; then
/// the new pull request opens.
#[test]
fn a_pull_request_from_the_clones_branch_is_pushed_then_made() {
    let (mut s, cmds) = in_clone();
    assert_eq!(fixtures::asked(&cmds), [compare()]);
    head(&mut s, None);
    let route = Route::Pulls(pulls::NewPr(ghtui()));
    let text = "Fix the parser\n\nIt dropped the last line.\n\n## Checklist\n\n- [ ] Tests";
    assert_eq!(
        s.forms.get::<crate::create::Form>(&route).unwrap().text,
        text
    );

    assert!(press(&mut s, "<C-s>").is_empty());
    fixtures::warns(
        &s,
        "fix-parser isn't on GitHub: :push branch pushes it to origin",
    );
    assert!(act(&mut s, Action::PushBranch).is_empty());
    assert_eq!(overlay!(s, Ask).title, "Push fix-parser to origin?");
    let push = press(&mut s, "<Enter>");
    let pushed = Push {
        repo: ghtui(),
        branch: "fix-parser".into(),
    };
    assert_eq!(push, [fixtures::start(pushed.clone())]);
    let again = fixtures::done(&mut s, pushed, Ok(()));
    assert!(s.overlay.is_none());
    says(&s, "Pushed fix-parser");
    assert_eq!(fixtures::asked(&again), [compare()], "compared again");
    head(&mut s, Some("a"));

    press(&mut s, "<C-s>");
    let choices: Vec<&str> = overlay!(s, Ask)
        .choices
        .iter()
        .map(|(l, _)| l.as_str())
        .collect();
    assert_eq!(
        choices,
        ["Create pull request", "Create draft pull request"]
    );
    let sent = press(&mut s, "<Tab><Enter>");
    let new = NewPr {
        repo: NodeId::new("R_ghtui"),
        base: "main".into(),
        head: "fix-parser".into(),
        title: "Fix the parser".into(),
        body: "It dropped the last line.\n\n## Checklist\n\n- [ ] Tests".into(),
        draft: true,
    };
    let submit = Submit {
        route,
        new: Submission::Pr(new),
    };
    assert_eq!(sent, [fixtures::start(submit.clone())]);

    let cmds = fixtures::done(&mut s, submit, Ok(7));
    let pr = PrRef {
        repo: ghtui(),
        number: 7,
    };
    assert_eq!(s.route(), Some(&Route::pr(pr.clone())));
    assert!(s.overlay.is_none());
    assert!(crate::fixtures::asks_pr(&cmds, &pr), "{cmds:?}");
}

/// `,c` checks the pull request out on a branch of its own: a fork's is
/// named for its owner. Uncommitted changes stop it, saying which.
#[test]
fn checkout_says_what_branch_or_why_not() {
    let mut s = with_pr();
    let pr = PrRef::parse("gold-silver-copper/ghtui#12").unwrap();
    let cmds = act(&mut s, Action::Checkout);
    let work = crate::clone::Checkout {
        repo: pr.repo,
        pr: Some(pr.number),
        name: "octocat/syntax-palette".into(),
        track: None,
    };
    assert_eq!(cmds, [fixtures::start(work.clone())]);
    let dirty = Checkout::Uncommitted(vec!["src/a.rs".into(), "b.rs".into()]);
    fixtures::done(&mut s, work.clone(), Ok(dirty));
    says(&s, "Not checked out: src/a.rs and 1 other has changes");
    let made = Checkout::Created("octocat/syntax-palette".into());
    fixtures::done(&mut s, work, Ok(made));
    says(&s, "you're on octocat/syntax-palette");
    // Outside a clone of it, it says so.
    fixtures::cloned(&mut s, &ghtui(), None);
    assert!(act(&mut s, Action::Checkout).is_empty());
    says(&s, "isn't running in a clone of gold-silver-copper/ghtui");
}

/// On a list, `,c` waits for the row's pull request to load, then
/// checks it out.
#[test]
fn checkout_on_a_row_waits_for_its_pull_request() {
    let mut s = with_home();
    s.data.retain(|key, _| !matches!(key, DataKey::Pr(_)));
    press(&mut s, "<Down><Down>");
    let pr = PrRef::parse("gold-silver-copper/ghtui#12").unwrap();
    assert_eq!(act(&mut s, Action::Checkout), [fixtures::fetch_pr(&pr)]);
    says(&s, "Loading gold-silver-copper/ghtui#12");
    let cmds = fixtures::pr_fetched(&mut s, &pr, Ok(fixtures::pr_detail()));
    assert_eq!(fixtures::sent::<crate::clone::Checkout>(&cmds).pr, Some(12));
}

/// A new issue's form, written, its composer closed.
pub(super) fn with_new_issue() -> State {
    let mut s = bug_report();
    press(&mut s, "<Esc>");
    s
}

/// A new pull request's form, its branch on GitHub.
pub(super) fn with_new_pr() -> State {
    let (mut s, _) = in_clone();
    head(&mut s, Some("a"));
    s
}
