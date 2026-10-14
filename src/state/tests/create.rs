//! New issues and pull requests, and checking one out: what each asks
//! first, what it sends, and the page following GitHub after.

use crate::route::{issues, pulls};
use std::path::PathBuf;

use crate::api::create::{NewIssue, NewPr};
use crate::git::local::{Branch, Checkout, Clone, CommitMessage};
use ratatui_textarea::CursorMove;

use super::*;
use crate::create::{Done, HeadState, Submission, Work};
use crate::ui::page::FormLink;

/// The repository's page, then `keys` (`,i` or `,p`), with what the form
/// can have delivered.
fn form(keys: &str) -> (State, Vec<Cmd>) {
    let mut s = with_repo();
    let cmds = press(&mut s, keys);
    let form = Data::NewForm(Box::new(crate::fixtures::new_form()));
    let mut more = fetched(&mut s, DataKey::NewForm(ghtui()), form);
    more.extend(cmds);
    (s, more)
}

fn composer(s: &mut State) -> &mut crate::review::Compose {
    match &mut s.overlay {
        Some(Overlay::Compose(c)) => c,
        _ => panic!("no composer"),
    }
}

fn asked(s: &State) -> &crate::create::Ask {
    match &s.overlay {
        Some(Overlay::Ask(a)) => a,
        _ => panic!("nothing asked"),
    }
}

/// The new issue's form, from the bug report template, its composer open.
fn bug_report() -> State {
    let (mut s, _) = form(",i");
    // The templates come first; the bug report (by file name) is selected.
    press(&mut s, "<Enter>");
    s
}

/// `,i` offers the templates first; the one chosen fills in the title,
/// the text (a form's fields as headings), the labels and the assignees.
/// `ctrl-s` sends them, and the new issue opens in place of the form.
#[test]
fn a_new_issue_starts_from_its_template_and_opens_once_made() {
    let (mut s, cmds) = form(",i");
    assert_eq!(s.route(), Some(&Route::Issues(issues::NewIssue(ghtui()))));
    let form = DataKey::NewForm(ghtui());
    assert!(crate::fixtures::asks(&cmds, &form), "{cmds:?}");
    let text = page_text(&s);
    for starter in ["Bug report", "Feature request", "Blank issue"] {
        assert!(text.contains(starter), "{text}");
    }
    press(&mut s, "<Enter>");
    let c = composer(&mut s);
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
    let submit = Cmd::Create(Work::Submit(route.clone(), Submission::Issue(new)));
    assert_eq!(sent, [submit]);
    assert!(composer(&mut s).sending);

    let cmds = update(&mut s, Msg::Create(Done::Created(route.clone(), Ok(42))));
    let issue = Route::Issues(issues::Issue {
        repo: ghtui(),
        number: 42,
    });
    assert_eq!(s.route(), Some(&issue));
    assert!(s.overlay.is_none());
    says(&s, "Opened issue #42");
    let issue = DataKey::Issue(ghtui(), 42);
    assert!(crate::fixtures::asks(&cmds, &issue), "{cmds:?}");
    assert!(!s.creating.forms.contains_key(&route), "the form is done");
}

/// GitHub's refusal stays in the composer, with the text, to try again.
#[test]
fn a_refused_issue_says_why_in_the_composer() {
    let mut s = bug_report();
    press(&mut s, "<C-s>");
    let route = Route::Issues(issues::NewIssue(ghtui()));
    let refused = ApiError::GraphQl(vec!["Title can't be blank".into()]);
    update(&mut s, Msg::Create(Done::Created(route, Err(refused))));
    let c = composer(&mut s);
    assert!(!c.sending);
    assert_eq!(
        c.error.as_deref(),
        Some("GitHub didn't create the issue: Title can't be blank")
    );
    assert_eq!(s.route(), Some(&Route::Issues(issues::NewIssue(ghtui()))));
}

/// Labels are picked and unpicked in a picker that stays open; esc in the
/// composer keeps what's written, on the page.
#[test]
fn labels_are_picked_and_text_is_kept() {
    let mut s = bug_report();
    composer(&mut s).input.insert_str("\nMore.");
    press(&mut s, "<Esc>");
    assert!(s.overlay.is_none());
    assert!(page_text(&s).contains("More."), "{}", page_text(&s));
    let _ = crate::create::follow_link(&mut s, &FormLink::Labels);
    // bug is on (from the template): off; then documentation on.
    press(&mut s, "<Enter><Down><Enter>");
    assert!(
        matches!(s.overlay, Some(Overlay::Picker(_))),
        "still picking"
    );
    press(&mut s, "<Esc>");
    let form = &s.creating.forms[&Route::Issues(issues::NewIssue(ghtui()))];
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
    let done = Done::Head {
        repo: ghtui(),
        base: "main".into(),
        head: "fix-parser".into(),
        result: Ok(hs),
    };
    update(s, Msg::Create(done))
}

/// A new pull request inside a clone, on branch `fix-parser`.
fn in_clone() -> (State, Vec<Cmd>) {
    let (mut s, cmds) = form(",p");
    assert!(
        cmds.contains(&Cmd::Create(Work::Clone(ghtui()))),
        "{cmds:?}"
    );
    let clone = Clone {
        top: PathBuf::from("/src/ghtui"),
        remote: "origin".into(),
        branch: Some("fix-parser".into()),
    };
    let cmds = update(&mut s, Msg::Create(Done::Clone(ghtui(), Ok(Some(clone)))));
    (s, cmds)
}

/// Inside a clone, `,p` proposes its branch into the default one, filled
/// in from its one commit and the template. A branch GitHub doesn't have
/// is pushed (asking first) before `,s` asks whether it's a draft; then
/// the new pull request opens.
#[test]
fn a_pull_request_from_the_clones_branch_is_pushed_then_made() {
    let (mut s, cmds) = in_clone();
    let compare = Work::Head {
        repo: ghtui(),
        base: "main".into(),
        head: "fix-parser".into(),
    };
    assert_eq!(cmds, [Cmd::Create(compare.clone())]);
    head(&mut s, None);
    let route = Route::Pulls(pulls::NewPr(ghtui()));
    let text = "Fix the parser\n\nIt dropped the last line.\n\n## Checklist\n\n- [ ] Tests";
    assert_eq!(s.creating.forms[&route].text, text);

    assert!(press(&mut s, ",s").is_empty());
    says(&s, "fix-parser isn't on GitHub: ,P pushes it to origin");
    assert!(press(&mut s, ",P").is_empty());
    assert_eq!(asked(&s).title, "Push fix-parser to origin?");
    let push = press(&mut s, "<Enter>");
    let pushed = Work::Push {
        repo: ghtui(),
        branch: "fix-parser".into(),
    };
    assert_eq!(push, [Cmd::Create(pushed)]);
    let again = update(
        &mut s,
        Msg::Create(Done::Pushed(ghtui(), "fix-parser".into(), Ok(()))),
    );
    assert!(s.overlay.is_none());
    says(&s, "Pushed fix-parser");
    assert_eq!(again, [Cmd::Create(compare)], "compared again");
    head(&mut s, Some("a"));

    press(&mut s, ",s");
    let choices: Vec<&str> = asked(&s).choices.iter().map(|(l, _)| l.as_str()).collect();
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
    assert_eq!(
        sent,
        [Cmd::Create(Work::Submit(
            route.clone(),
            Submission::Pr(new)
        ))]
    );

    let cmds = update(&mut s, Msg::Create(Done::Created(route, Ok(7))));
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
    let cmds = press(&mut s, ",c");
    let work = Work::Checkout {
        pr: pr.clone(),
        name: "octocat/syntax-palette".into(),
        track: None,
    };
    assert_eq!(cmds, [Cmd::Create(work)]);
    let dirty = Checkout::Uncommitted(vec!["src/a.rs".into(), "b.rs".into()]);
    update(&mut s, Msg::Create(Done::CheckedOut(pr.clone(), Ok(dirty))));
    says(&s, "Not checked out: src/a.rs and 1 other has changes");
    let made = Checkout::Created("octocat/syntax-palette".into());
    update(&mut s, Msg::Create(Done::CheckedOut(pr, Ok(made))));
    says(&s, "you're on octocat/syntax-palette");
    // Outside a clone of it, it says so.
    update(&mut s, Msg::Create(Done::Clone(ghtui(), Ok(None))));
    assert!(press(&mut s, ",c").is_empty());
    says(&s, "isn't running in a clone of gold-silver-copper/ghtui");
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
