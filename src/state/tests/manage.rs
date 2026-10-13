use crate::act::{By, Subject};
use crate::api::browse::{DiscussionsOf, IssueDetail};
use crate::api::manage::{
    Categories, Category, DiscussionReason, HideReason, IssueAt, LockReason, Manage, Target,
};
use crate::api::model::Capped;
use crate::api::timeline::{Linked, LinkedState};

use super::*;
use crate::fixtures::{self};
use crate::manage::{Done, Pick, Work};
use crate::ui::page::{Link, ReviewLink};

fn issue() -> Subject {
    Subject::Issue(ghtui(), 14)
}

fn pr() -> PrRef {
    PrRef::parse("gold-silver-copper/ghtui#12").unwrap()
}

fn at(number: u64) -> IssueAt {
    IssueAt {
        repo: ghtui(),
        number,
    }
}

/// The issue as GitHub has it, changed by `f`.
fn deliver(s: &mut State, f: impl FnOnce(&mut IssueDetail)) -> Vec<Cmd> {
    let mut d = fixtures::issue();
    f(&mut d);
    fetched(
        s,
        DataKey::Issue(ghtui(), 14),
        Data::Issue(Some(Box::new(d))),
    )
}

fn issue_with(f: impl FnOnce(&mut IssueDetail)) -> State {
    let mut s = sized(with_issue(), 100, 40);
    deliver(&mut s, f);
    s
}

/// The pull request as GitHub has it, changed by `f`.
fn pr_with(f: impl FnOnce(&mut PrDetail)) -> State {
    let mut s = with_pr();
    deliver_pr(&mut s, f);
    s
}

fn deliver_pr(s: &mut State, f: impl FnOnce(&mut PrDetail)) -> Vec<Cmd> {
    let mut d = pr_detail();
    f(&mut d);
    crate::fixtures::pr_fetched(s, &pr(), Ok(d))
}

fn linked(number: u64, title: &str) -> Linked {
    Linked {
        repo: ghtui(),
        number,
        title: title.into(),
        state: LinkedState::Issue(IssueState::Open),
    }
}

/// The one change `cmds` sends.
fn sent(cmds: &[Cmd]) -> (&Change, &By) {
    match cmds {
        [Cmd::Api(Api::Change(change, by))] => (change, by),
        _ => panic!("{cmds:?}"),
    }
}

/// The work `cmds` starts.
fn work(cmds: &[Cmd]) -> Work {
    let found = cmds.iter().find_map(|c| match c {
        Cmd::Write(crate::writes::Work::Manage(w)) => Some(w.clone()),
        _ => None,
    });
    found.unwrap_or_else(|| panic!("no work in {cmds:?}"))
}

/// Work's answer.
fn done(s: &mut State, done: Done) -> Vec<Cmd> {
    update(s, Msg::Write(crate::writes::Done::Manage(done)))
}

fn pick(s: &State) -> &Pick {
    match &s.overlay {
        Some(Overlay::Picker(p)) => match &p.kind {
            crate::picker::Kind::Manage(pick) => pick,
            _ => panic!("another picker"),
        },
        _ => panic!("no picker"),
    }
}

fn confirm(s: &State) -> &crate::act::Confirm {
    match &s.overlay {
        Some(Overlay::Confirm(c)) => c,
        _ => panic!("no confirmation"),
    }
}

/// Moves the selection, comment by comment, to the first where `is`.
fn select(s: &mut State, is: impl Fn(&Link) -> bool) {
    let selected = |s: &State| match s.screen() {
        Screen::Page(p) => p.selected_link().is_some_and(&is),
        Screen::Diff(_) => false,
    };
    press(s, "gg");
    for _ in 0..12 {
        if selected(s) {
            return;
        }
        press(s, "}");
    }
    assert!(selected(s), "nothing selected is what's wanted");
}

/// Runs the menu's row that `filter` finds.
fn from_menu(s: &mut State, filter: &str) -> Vec<Cmd> {
    press(s, "<Space>/");
    for c in filter.chars() {
        press(s, &c.to_string());
    }
    press(s, "<Enter>")
}

/// Locking asks GitHub's reason (or none); the page follows GitHub until
/// it says it's locked, and unlocking then asks first.
#[test]
fn a_conversation_is_locked_with_a_reason_and_unlocked_after_asking() {
    let mut s = issue_with(|_| {});
    press(&mut s, ",L");
    assert!(matches!(pick(&s), Pick::Lock { .. }));
    press(&mut s, "heated");
    let cmds = press(&mut s, "<Enter>");
    let lock = Change::Manage(Manage::Lock {
        subject: NodeId::new("I_14"),
        reason: Some(LockReason::TooHeated),
    });
    assert_eq!(sent(&cmds), (&lock, &By::Act(issue())));
    answer(&mut s, &cmds, Ok(()));
    assert!(s.awaiting.is_some(), "it waits for GitHub to show it");
    deliver(&mut s, |d| {
        d.locked = Some(crate::api::manage::Locked {
            reason: Some(LockReason::TooHeated),
        });
    });
    assert!(s.awaiting.is_none());
    assert!(screen_text(&s).contains("Locked as too heated"));

    press(&mut s, ",L");
    assert_eq!(confirm(&s).title, "Unlock the conversation on this issue?");
    let cmds = press(&mut s, "<Enter>");
    let unlock = Change::Manage(Manage::Unlock {
        subject: NodeId::new("I_14"),
    });
    assert_eq!(sent(&cmds).0, &unlock);

    let mut s = issue_with(|d| d.sidebar.labels = false);
    press(&mut s, ",L");
    says(&s, "that takes triage access");
}

/// A merged pull request's branch is deleted, then restored at its head;
/// an open one's isn't, and a fork's is its owner's.
#[test]
fn a_merged_pull_requests_branch_is_deleted_and_restored() {
    let same = |d: &mut PrDetail| {
        d.head_repo = Some("gold-silver-copper/ghtui".into());
        d.may.delete_head = true;
    };
    let mut s = pr_with(same);
    press(&mut s, ",b");
    says(&s, "It's still open");
    let mut s = pr_with(|d| {
        same(d);
        d.state = PrState::Merged;
    });
    let cmds = press(&mut s, ",b");
    let delete = Change::Manage(Manage::DeleteBranch {
        repo: ghtui(),
        branch: "syntax-palette".into(),
    });
    assert_eq!(sent(&cmds).0, &delete);
    answer(&mut s, &cmds, Ok(()));
    deliver_pr(&mut s, |d| {
        same(d);
        d.state = PrState::Merged;
        d.head_gone = true;
    });
    assert!(s.awaiting.is_none());
    assert!(screen_text(&s).contains("branch deleted"));
    let cmds = press(&mut s, ",b");
    let restore = Change::Manage(Manage::RestoreBranch {
        repo: ghtui(),
        branch: "syntax-palette".into(),
        oid: pr_detail().head_oid,
    });
    assert_eq!(sent(&cmds).0, &restore);

    let mut s = pr_with(|d| d.state = PrState::Merged);
    press(&mut s, ",b");
    says(&s, "Its branch is in octocat/ghtui");
}

/// `,r` on a pull request that isn't a draft converts it back, after
/// asking.
#[test]
fn a_pull_request_converts_back_to_a_draft_after_asking() {
    let mut s = pr_with(|_| {});
    press(&mut s, ",r");
    assert!(confirm(&s).title.ends_with("to a draft?"));
    let cmds = press(&mut s, "<Enter>");
    let draft = Change::Manage(Manage::ConvertToDraft {
        pr: NodeId::new("PR_12"),
    });
    assert_eq!(sent(&cmds), (&draft, &By::Confirm(Subject::Pr(pr()))));
    answer(&mut s, &cmds, Ok(()));
    deliver_pr(&mut s, |d| d.state = PrState::Draft);
    assert!(s.awaiting.is_none());
}

/// A new base is chosen from the branches, then asked about.
#[test]
fn a_new_base_is_picked_from_the_branches() {
    let mut s = pr_with(|_| {});
    from_menu(&mut s, "base");
    assert!(matches!(pick(&s), Pick::Base { .. }));
    let refs = crate::api::browse::Refs {
        branches: vec!["main".into(), "next".into(), "syntax-palette".into()].into(),
        tags: Capped::default(),
    };
    fetched(&mut s, DataKey::Refs(ghtui()), Data::Refs(Box::new(refs)));
    press(&mut s, "next<Enter>");
    assert_eq!(confirm(&s).title, "Change #12's base to next?");
    let cmds = press(&mut s, "<Enter>");
    let base = Change::Manage(Manage::ChangeBase {
        pr: NodeId::new("PR_12"),
        base: "next".into(),
    });
    assert_eq!(sent(&cmds).0, &base);
}

/// Reverting asks first; GitHub's new pull request opens.
#[test]
fn a_revert_opens_the_pull_request_github_makes() {
    let mut s = pr_with(|d| d.state = PrState::Merged);
    from_menu(&mut s, "revert");
    assert_eq!(confirm(&s).title, "Revert #12?");
    let cmds = press(&mut s, "<Enter>");
    assert_eq!(
        work(&cmds),
        Work::Revert {
            pr: pr(),
            id: NodeId::new("PR_12"),
        }
    );
    assert!(confirm(&s).sending);
    done(&mut s, Done::Reverted(pr(), Ok(20)));
    assert!(s.overlay.is_none());
    assert_eq!(
        s.route(),
        Some(&Route::pr(PrRef {
            repo: ghtui(),
            number: 20
        }))
    );
}

/// Closing an issue offers a duplicate, which is chosen next: typed, or
/// from its repository's open issues.
#[test]
fn an_issue_closes_as_a_duplicate_of_one_chosen() {
    let mut s = issue_with(|_| {});
    press(&mut s, ",x");
    let labels: Vec<&str> = confirm(&s)
        .choices
        .iter()
        .map(|(l, _)| l.as_str())
        .collect();
    assert_eq!(
        labels,
        ["Close as completed", "Close as not planned", "Duplicate…"]
    );
    let cmds = press(&mut s, "<Tab><Tab><Enter>");
    assert_eq!(work(&cmds), Work::Issues(ghtui()));
    let open = Capped::new(vec![(3, "Wrap long lines".into())], 1);
    done(&mut s, Done::Issues(ghtui(), Ok(open)));
    let cmds = press(&mut s, "wrap<Enter>");
    let close = Change::Manage(Manage::CloseAsDuplicate {
        issue: NodeId::new("I_14"),
        of: Some(at(3)),
    });
    assert_eq!(sent(&cmds), (&close, &By::Act(issue())));
    answer(&mut s, &cmds, Ok(()));
    deliver(&mut s, |d| d.state = IssueState::NotPlanned);
    assert!(s.awaiting.is_none());
}

/// A hidden comment is folded, saying why; `za` shows it, and `,H`
/// unhides it. A shown one is hidden with one of GitHub's reasons.
#[test]
fn a_comment_is_hidden_with_a_reason_and_shows_folded() {
    let mut s = issue_with(|_| {});
    select(
        &mut s,
        |l| matches!(l, Link::Quote { author, .. } if author == "hubot"),
    );
    press(&mut s, ",H");
    press(&mut s, "topic");
    let cmds = press(&mut s, "<Enter>");
    let hide = Change::Manage(Manage::Hide {
        comment: NodeId::new("IC_1"),
        reason: HideReason::OffTopic,
    });
    assert_eq!(sent(&cmds), (&hide, &By::Act(issue())));
    answer(&mut s, &cmds, Ok(()));
    deliver(&mut s, |d| {
        if let Some(c) = d.first_comment() {
            c.hiding.reason = Some("off-topic".into());
        }
    });
    assert!(s.awaiting.is_none());
    let shown = screen_text(&s);
    assert!(shown.contains("Hidden as off-topic"), "{shown}");
    assert!(!shown.contains("120 columns, centered?"), "{shown}");
    select(&mut s, |l| matches!(l, Link::Review(ReviewLink::Fold(_))));
    press(&mut s, "za");
    assert!(screen_text(&s).contains("120 columns, centered?"));
    let cmds = press(&mut s, ",H");
    let unhide = Change::Manage(Manage::Unhide {
        comment: NodeId::new("IC_1"),
    });
    assert_eq!(sent(&cmds).0, &unhide);
}

/// Sub-issues: one typed is added, one there is removed, and alt-↓ moves
/// the one chosen below the next; what blocks an issue likewise.
#[test]
fn sub_issues_and_what_blocks_an_issue_change_in_their_pickers() {
    let mut s = issue_with(|d| {
        d.sub_issues = vec![linked(2, "First"), linked(3, "Second")].into();
    });
    press(&mut s, ",S");
    done(&mut s, Done::Issues(ghtui(), Ok(Capped::default())));
    let cmds = press(&mut s, "<A-Down>");
    let moved = Change::Manage(Manage::MoveSubIssue {
        parent: NodeId::new("I_14"),
        child: at(2),
        sibling: at(3),
        before: false,
    });
    assert_eq!(sent(&cmds).0, &moved);
    assert!(matches!(pick(&s), Pick::SubIssues { .. }), "it stays open");
    answer(&mut s, &cmds, Ok(()));
    deliver(&mut s, |d| {
        d.sub_issues = vec![linked(3, "Second"), linked(2, "First")].into();
    });
    assert!(s.awaiting.is_none());
    let cmds = press(&mut s, "#5<Enter>");
    let add = Change::Manage(Manage::SubIssue {
        parent: NodeId::new("I_14"),
        child: at(5),
        add: true,
    });
    assert_eq!(sent(&cmds).0, &add);
    answer(&mut s, &cmds, Ok(()));
    deliver(&mut s, |d| {
        d.sub_issues = vec![linked(3, "Second"), linked(2, "First"), linked(5, "Third")].into();
    });
    assert!(s.awaiting.is_none());
    s.overlay = None;
    press(&mut s, ",S");
    let cmds = press(&mut s, "<Enter>");
    let remove = Change::Manage(Manage::SubIssue {
        parent: NodeId::new("I_14"),
        child: at(3),
        add: false,
    });
    assert_eq!(sent(&cmds).0, &remove);

    let mut s = issue_with(|_| {});
    press(&mut s, ",K");
    let cmds = press(&mut s, "#4<Enter>");
    let blocked = Change::Manage(Manage::BlockedBy {
        issue: NodeId::new("I_14"),
        by: at(4),
        add: true,
    });
    assert_eq!(sent(&cmds).0, &blocked);
    answer(&mut s, &cmds, Ok(()));
    deliver(&mut s, |d| d.blocked_by = vec![linked(4, "Blocker")].into());
    assert!(s.awaiting.is_none());
    assert!(screen_text(&s).contains("Blocked by"));
}

/// `,c` on an issue makes its branch, named as GitHub names it, then
/// offers to check it out.
#[test]
fn a_branch_for_an_issue_is_made_then_checked_out() {
    let mut s = issue_with(|_| {});
    press(&mut s, ",c");
    let cmds = press(&mut s, "<Enter>");
    let branch = Change::Manage(Manage::LinkBranch {
        issue: NodeId::new("I_14"),
        repo: ghtui(),
        name: "14-pages-wrap-at-the-terminal-width".into(),
    });
    assert_eq!(sent(&cmds), (&branch, &By::Act(issue())));
    answer(&mut s, &cmds, Ok(()));
    assert!(matches!(pick(&s), Pick::Checkout { .. }));
    let cmds = press(&mut s, "<Enter>");
    assert_eq!(
        work(&cmds),
        Work::Checkout {
            repo: ghtui(),
            branch: "14-pages-wrap-at-the-terminal-width".into(),
        }
    );
    deliver(&mut s, |d| {
        d.branches = vec!["14-pages-wrap-at-the-terminal-width".to_owned()].into();
    });
    assert!(s.awaiting.is_none());
}

/// Pinning needs write access; transferring picks a repository of the
/// same owner, asks, and opens the issue where it went.
#[test]
fn an_issue_is_pinned_and_transferred() {
    let mut s = issue_with(|_| {});
    let cmds = from_menu(&mut s, "pin the");
    let pin = Change::Manage(Manage::Pin {
        issue: NodeId::new("I_14"),
        on: true,
    });
    assert_eq!(sent(&cmds).0, &pin);
    answer(&mut s, &cmds, Ok(()));
    deliver(&mut s, |d| d.pinned = true);
    assert!(s.awaiting.is_none());
    assert!(screen_text(&s).contains("Pinned"));

    let cmds = from_menu(&mut s, "transfer");
    assert_eq!(work(&cmds), Work::Targets("gold-silver-copper".into()));
    let to = RepoId::new("gold-silver-copper", "elsewhere");
    let targets = vec![Target {
        repo: to.clone(),
        id: NodeId::new("R_2"),
    }];
    done(
        &mut s,
        Done::Targets("gold-silver-copper".into(), Ok(targets.into())),
    );
    press(&mut s, "<Enter>");
    assert_eq!(
        confirm(&s).title,
        "Transfer #14 to gold-silver-copper/elsewhere?"
    );
    let cmds = press(&mut s, "<Enter>");
    assert!(matches!(work(&cmds), Work::Transfer { .. }));
    let moved = IssueAt {
        repo: to.clone(),
        number: 1,
    };
    done(&mut s, Done::Transferred(at(14), Ok(moved)));
    assert_eq!(
        s.route(),
        Some(&Route::Issue {
            repo: to,
            number: 1
        })
    );

    let s = issue_with(|d| d.can_write = false);
    let why = s.availability(crate::keymap::Action::Pin).unwrap_err();
    assert!(why.contains("takes write access"), "{why}");
}

/// A discussion closes with one of GitHub's reasons, and says so.
#[test]
fn a_discussion_closes_with_a_reason() {
    let mut s = with_discussion();
    press(&mut s, ",x");
    let labels: Vec<&str> = confirm(&s)
        .choices
        .iter()
        .map(|(l, _)| l.as_str())
        .collect();
    assert_eq!(
        labels,
        [
            "Close as resolved",
            "Close as outdated",
            "Close as duplicate"
        ]
    );
    let cmds = press(&mut s, "<Enter>");
    let close = Change::Manage(Manage::CloseDiscussion {
        discussion: NodeId::new("D_14603"),
        reason: DiscussionReason::Resolved,
    });
    let of = DiscussionsOf::Repo(ghtui());
    assert_eq!(
        sent(&cmds),
        (&close, &By::Confirm(Subject::Discussion(of.clone(), 14603)))
    );
    answer(&mut s, &cmds, Ok(()));
    let mut d = fixtures::discussion();
    d.closed = Some(Some(DiscussionReason::Resolved));
    fetched(
        &mut s,
        DataKey::Discussion(of, 14603),
        Data::Discussion(Box::new(d)),
    );
    assert!(s.awaiting.is_none());
    assert!(screen_text(&s).contains("Closed as resolved"));
}

/// A new discussion: its category, then the composer with the category's
/// form; once GitHub has it, it opens.
#[test]
fn a_new_discussion_starts_from_its_categorys_form() {
    let mut s = with_discussions();
    let cmds = from_menu(&mut s, "new discussion");
    assert_eq!(work(&cmds), Work::Categories(ghtui()));
    let form = "title: \"[Idea] \"\nbody:\n  - type: textarea\n    attributes:\n      label: What would it do\n";
    let ideas = Category {
        id: NodeId::new("DIC_1"),
        name: "Ideas".into(),
        slug: "ideas".into(),
        description: "Share ideas".into(),
        answerable: false,
        form: Some(form.into()),
    };
    let found = Categories {
        repo: NodeId::new("R_1"),
        categories: vec![ideas].into(),
    };
    done(&mut s, Done::Categories(ghtui(), Ok(found)));
    press(&mut s, "<Enter>");
    let Some(Overlay::Compose(c)) = &s.overlay else {
        panic!("no composer");
    };
    let text = c.text();
    assert!(text.starts_with("[Idea]"), "{text}");
    assert!(text.contains("### What would it do"), "{text}");
    let cmds = press(&mut s, "<C-s>");
    let Work::Start {
        repo_id,
        category,
        title,
        body,
        ..
    } = work(&cmds)
    else {
        panic!("not started");
    };
    assert_eq!(
        (repo_id, category),
        (NodeId::new("R_1"), NodeId::new("DIC_1"))
    );
    assert_eq!(title, "[Idea]");
    assert!(body.starts_with("### What would it do"), "{body}");
    done(&mut s, Done::Started(ghtui(), Ok(7)));
    assert!(s.overlay.is_none());
    let of = DiscussionsOf::Repo(ghtui());
    assert_eq!(s.route(), Some(&Route::Discussion { of, number: 7 }));
}
