use crate::act::{By, Subject};
use crate::api::browse::DiscussionsOf;
use crate::api::manage::{
    Categories, Category, DiscussionReason, HideReason, IssueAt, LockReason, Manage, Target,
};
use crate::api::model::Capped;
use crate::api::timeline::{Linked, LinkedState};
use crate::route::{discussions, issues};

use super::*;
use crate::fixtures::{self};
use crate::manage::{Pick, Revert, Start, Transfer};
use crate::ui::page::{Link, ReviewLink};

fn issue() -> Subject {
    Subject::Issue(ghtui(), 14)
}

fn at(number: u64) -> IssueAt {
    IssueAt {
        repo: ghtui(),
        number,
    }
}

fn linked(number: u64, title: &str) -> Linked {
    Linked {
        repo: ghtui(),
        number,
        title: title.into(),
        state: LinkedState::Issue(IssueState::Open),
    }
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
    act(&mut s, Action::Lock);
    assert!(matches!(pick(&s), Pick::Lock { .. }));
    press(&mut s, "heated");
    let cmds = press(&mut s, "<Enter>");
    let lock = Change::Manage(Manage::Lock {
        subject: NodeId::new("I_14"),
        reason: Some(LockReason::TooHeated),
    });
    assert_eq!(fixtures::sent_change(&cmds), (&lock, &By::Act(issue())));
    answer(&mut s, &cmds, Ok(()));
    assert!(s.following.is_some(), "it waits for GitHub to show it");
    issue_shows(&mut s, |d| {
        d.locked = Some(crate::api::manage::Locked {
            reason: Some(LockReason::TooHeated),
        });
    });
    assert!(s.following.is_none());
    assert!(screen_text(&s).contains("Locked as too heated"));

    act(&mut s, Action::Lock);
    assert_eq!(
        overlay!(s, Ask).title,
        "Unlock the conversation on this issue?"
    );
    let cmds = press(&mut s, "<Enter>");
    let unlock = Change::Manage(Manage::Unlock {
        subject: NodeId::new("I_14"),
    });
    assert_eq!(fixtures::sent_change(&cmds).0, &unlock);

    let mut s = issue_with(|d| d.sidebar.labels = false);
    act(&mut s, Action::Lock);
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
    act(&mut s, Action::DeleteBranch);
    says(&s, "It's still open");
    let mut s = pr_with(|d| {
        same(d);
        d.state = PrState::Merged;
    });
    let cmds = act(&mut s, Action::DeleteBranch);
    let delete = Change::Manage(Manage::DeleteBranch {
        repo: ghtui(),
        branch: "syntax-palette".into(),
    });
    assert_eq!(fixtures::sent_change(&cmds).0, &delete);
    answer(&mut s, &cmds, Ok(()));
    pr_shows(&mut s, |d| {
        same(d);
        d.state = PrState::Merged;
        d.head_gone = true;
    });
    assert!(s.following.is_none());
    assert!(screen_text(&s).contains("branch deleted"));
    let cmds = act(&mut s, Action::DeleteBranch);
    let restore = Change::Manage(Manage::RestoreBranch {
        repo: ghtui(),
        branch: "syntax-palette".into(),
        oid: pr_detail().head_oid,
    });
    assert_eq!(fixtures::sent_change(&cmds).0, &restore);

    let mut s = pr_with(|d| d.state = PrState::Merged);
    act(&mut s, Action::DeleteBranch);
    says(&s, "Its branch is in octocat/ghtui");
}

/// `,r` on a pull request that isn't a draft converts it back, after
/// asking.
#[test]
fn a_pull_request_converts_back_to_a_draft_after_asking() {
    let mut s = pr_with(|_| {});
    act(&mut s, Action::ReadyForReview);
    assert!(overlay!(s, Ask).title.ends_with("to a draft?"));
    let cmds = press(&mut s, "<Enter>");
    let draft = Change::Manage(Manage::ConvertToDraft {
        pr: NodeId::new("PR_12"),
    });
    assert_eq!(
        fixtures::sent_change(&cmds),
        (&draft, &By::Confirm(Subject::Pr(pr())))
    );
    answer(&mut s, &cmds, Ok(()));
    pr_shows(&mut s, |d| d.state = PrState::Draft);
    assert!(s.following.is_none());
}

/// A new base is chosen from the branches, then asked about.
#[test]
fn a_new_base_is_picked_from_the_branches() {
    let mut s = pr_with(|_| {});
    from_menu(&mut s, "change the base");
    assert!(matches!(pick(&s), Pick::Base { .. }));
    let refs = crate::api::browse::Refs {
        branches: vec!["main".into(), "next".into(), "syntax-palette".into()].into(),
        tags: Capped::default(),
    };
    fetched(&mut s, DataKey::Refs(ghtui()), Data::from(refs));
    press(&mut s, "next<Enter>");
    assert_eq!(overlay!(s, Ask).title, "Change #12's base to next?");
    let cmds = press(&mut s, "<Enter>");
    let base = Change::Manage(Manage::ChangeBase {
        pr: NodeId::new("PR_12"),
        base: "next".into(),
    });
    assert_eq!(fixtures::sent_change(&cmds).0, &base);
}

/// Reverting asks first; GitHub's new pull request opens.
#[test]
fn a_revert_opens_the_pull_request_github_makes() {
    let mut s = pr_with(|d| d.state = PrState::Merged);
    from_menu(&mut s, "revert");
    assert_eq!(overlay!(s, Ask).title, "Revert #12?");
    let cmds = press(&mut s, "<Enter>");
    let revert = Revert {
        pr: pr(),
        id: NodeId::new("PR_12"),
    };
    assert_eq!(fixtures::sent::<Revert>(&cmds), revert);
    assert!(overlay!(s, Ask).sent.sending);
    fixtures::done(&mut s, revert, Ok(20));
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
    press(&mut s, "x");
    let labels: Vec<&str> = overlay!(s, Ask)
        .choices
        .iter()
        .map(|(l, _)| l.as_str())
        .collect();
    assert_eq!(
        labels,
        ["Close as completed", "Close as not planned", "Duplicate…"]
    );
    let cmds = press(&mut s, "<Tab><Tab><Enter>");
    let key = DataKey::OpenIssues(ghtui());
    assert_eq!(fixtures::asked(&cmds), std::slice::from_ref(&key));
    let open = Capped::new(vec![(3, "Wrap long lines".into())], 1);
    fixtures::fetched(&mut s, key, Data::from(open));
    let cmds = press(&mut s, "wrap<Enter>");
    let close = Change::Manage(Manage::CloseAsDuplicate {
        issue: NodeId::new("I_14"),
        of: Some(at(3)),
    });
    assert_eq!(fixtures::sent_change(&cmds), (&close, &By::Act(issue())));
    answer(&mut s, &cmds, Ok(()));
    issue_shows(&mut s, |d| d.state = IssueState::NotPlanned);
    assert!(s.following.is_none());
}

/// A hidden comment is folded, saying why; `za` shows it, and `,H`
/// unhides it. A shown one is hidden with one of GitHub's reasons.
#[test]
fn a_comment_is_hidden_with_a_reason_and_shows_folded() {
    let mut s = issue_with(|_| {});
    fixtures::select_by(
        &mut s,
        |l| matches!(l, Link::Quote { author, .. } if author == "hubot"),
    );
    act(&mut s, Action::HideComment);
    press(&mut s, "topic");
    let cmds = press(&mut s, "<Enter>");
    let hide = Change::Manage(Manage::Hide {
        comment: NodeId::new("IC_1"),
        reason: HideReason::OffTopic,
    });
    assert_eq!(fixtures::sent_change(&cmds), (&hide, &By::Act(issue())));
    answer(&mut s, &cmds, Ok(()));
    issue_shows(&mut s, |d| {
        if let Some(c) = d.first_comment() {
            c.hiding.reason = Some("off-topic".into());
        }
    });
    assert!(s.following.is_none());
    let shown = screen_text(&s);
    assert!(shown.contains("Hidden as off-topic"), "{shown}");
    assert!(!shown.contains("120 columns, centered?"), "{shown}");
    fixtures::select_by(&mut s, |l| matches!(l, Link::Review(ReviewLink::Fold(_))));
    act(&mut s, Action::FoldToggle);
    assert!(screen_text(&s).contains("120 columns, centered?"));
    let cmds = act(&mut s, Action::HideComment);
    let unhide = Change::Manage(Manage::Unhide {
        comment: NodeId::new("IC_1"),
    });
    assert_eq!(fixtures::sent_change(&cmds).0, &unhide);
}

/// Sub-issues: one typed is added, one there is removed, and alt-↓ moves
/// the one chosen below the next; what blocks an issue likewise.
#[test]
fn sub_issues_and_what_blocks_an_issue_change_in_their_pickers() {
    let mut s = issue_with(|d| {
        d.sub_issues = vec![linked(2, "First"), linked(3, "Second")].into();
    });
    act(&mut s, Action::SubIssues);
    let none = Data::OpenIssues(Box::default());
    fixtures::fetched(&mut s, DataKey::OpenIssues(ghtui()), none);
    let cmds = press(&mut s, "<A-Down>");
    let moved = Change::Manage(Manage::MoveSubIssue {
        parent: NodeId::new("I_14"),
        child: at(2),
        sibling: at(3),
        before: false,
    });
    assert_eq!(fixtures::sent_change(&cmds).0, &moved);
    assert!(matches!(pick(&s), Pick::SubIssues { .. }), "it stays open");
    answer(&mut s, &cmds, Ok(()));
    issue_shows(&mut s, |d| {
        d.sub_issues = vec![linked(3, "Second"), linked(2, "First")].into();
    });
    assert!(s.following.is_none());
    let cmds = press(&mut s, "#5<Enter>");
    let add = Change::Manage(Manage::SubIssue {
        parent: NodeId::new("I_14"),
        child: at(5),
        add: true,
    });
    assert_eq!(fixtures::sent_change(&cmds).0, &add);
    answer(&mut s, &cmds, Ok(()));
    issue_shows(&mut s, |d| {
        d.sub_issues = vec![linked(3, "Second"), linked(2, "First"), linked(5, "Third")].into();
    });
    assert!(s.following.is_none());
    s.overlay = None;
    act(&mut s, Action::SubIssues);
    let cmds = press(&mut s, "<Enter>");
    let remove = Change::Manage(Manage::SubIssue {
        parent: NodeId::new("I_14"),
        child: at(3),
        add: false,
    });
    assert_eq!(fixtures::sent_change(&cmds).0, &remove);

    let mut s = issue_with(|_| {});
    act(&mut s, Action::BlockedBy);
    let cmds = press(&mut s, "#4<Enter>");
    let blocked = Change::Manage(Manage::BlockedBy {
        issue: NodeId::new("I_14"),
        by: at(4),
        add: true,
    });
    assert_eq!(fixtures::sent_change(&cmds).0, &blocked);
    answer(&mut s, &cmds, Ok(()));
    issue_shows(&mut s, |d| d.blocked_by = vec![linked(4, "Blocker")].into());
    assert!(s.following.is_none());
    assert!(screen_text(&s).contains("Blocked by"));
}

/// `,c` on an issue makes its branch, named as GitHub names it, then
/// offers to check it out.
#[test]
fn a_branch_for_an_issue_is_made_then_checked_out() {
    let mut s = issue_with(|_| {});
    act(&mut s, Action::Checkout);
    let cmds = press(&mut s, "<Enter>");
    let branch = Change::Manage(Manage::LinkBranch {
        issue: NodeId::new("I_14"),
        repo: ghtui(),
        name: "14-pages-wrap-at-the-terminal-width".into(),
    });
    assert_eq!(fixtures::sent_change(&cmds), (&branch, &By::Act(issue())));
    answer(&mut s, &cmds, Ok(()));
    assert!(matches!(pick(&s), Pick::Checkout { .. }));
    let cmds = press(&mut s, "<Enter>");
    let branch = "14-pages-wrap-at-the-terminal-width".to_owned();
    assert_eq!(
        fixtures::sent::<crate::clone::Checkout>(&cmds),
        crate::clone::Checkout {
            repo: ghtui(),
            pr: None,
            name: branch.clone(),
            track: Some(branch),
        }
    );
    issue_shows(&mut s, |d| {
        d.branches = vec!["14-pages-wrap-at-the-terminal-width".to_owned()].into();
    });
    assert!(s.following.is_none());
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
    assert_eq!(fixtures::sent_change(&cmds).0, &pin);
    answer(&mut s, &cmds, Ok(()));
    issue_shows(&mut s, |d| d.pinned = true);
    assert!(s.following.is_none());
    assert!(screen_text(&s).contains("Pinned"));

    let cmds = from_menu(&mut s, "transfer");
    let key = DataKey::MoveTargets("gold-silver-copper".into());
    assert_eq!(fixtures::asked(&cmds), std::slice::from_ref(&key));
    let to = RepoId::new("gold-silver-copper", "elsewhere");
    let targets = vec![Target {
        repo: to.clone(),
        id: NodeId::new("R_2"),
    }];
    fixtures::fetched(&mut s, key, Data::from(Capped::from(targets)));
    press(&mut s, "<Enter>");
    assert_eq!(
        overlay!(s, Ask).title,
        "Transfer #14 to gold-silver-copper/elsewhere?"
    );
    let cmds = press(&mut s, "<Enter>");
    let transfer = fixtures::sent::<Transfer>(&cmds);
    assert_eq!(transfer.from, at(14));
    let moved = IssueAt {
        repo: to.clone(),
        number: 1,
    };
    fixtures::done(&mut s, transfer, Ok(moved));
    assert_eq!(
        s.route(),
        Some(&Route::Issues(issues::Issue {
            repo: to,
            number: 1
        }))
    );

    let s = issue_with(|d| d.can_write = false);
    let why = s.availability(crate::keymap::Action::Pin).unwrap_err();
    assert!(why.contains("takes write access"), "{why}");
}

/// A discussion closes with one of GitHub's reasons, and says so.
#[test]
fn a_discussion_closes_with_a_reason() {
    let mut s = with_discussion();
    press(&mut s, "x");
    let labels: Vec<&str> = overlay!(s, Ask)
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
        fixtures::sent_change(&cmds),
        (&close, &By::Confirm(Subject::Discussion(of.clone(), 14603)))
    );
    answer(&mut s, &cmds, Ok(()));
    let mut d = fixtures::discussion();
    d.closed = Some(Some(DiscussionReason::Resolved));
    fetched(&mut s, DataKey::Discussion(of, 14603), Data::from(d));
    assert!(s.following.is_none());
    assert!(screen_text(&s).contains("Closed as resolved"));
}

/// A new discussion: its category, then the composer with the category's
/// form; once GitHub has it, it opens.
#[test]
fn a_new_discussion_starts_from_its_categorys_form() {
    let mut s = with_discussions();
    let cmds = from_menu(&mut s, "new discussion");
    let key = DataKey::Categories(ghtui());
    assert_eq!(fixtures::asked(&cmds), std::slice::from_ref(&key));
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
    fixtures::fetched(&mut s, key, Data::from(found));
    press(&mut s, "<Enter>");
    let Some(Overlay::Compose(c)) = &s.overlay else {
        panic!("no composer");
    };
    let text = c.text();
    assert!(text.starts_with("[Idea]"), "{text}");
    assert!(text.contains("### What would it do"), "{text}");
    let cmds = press(&mut s, "<C-s>");
    let start = fixtures::sent::<Start>(&cmds);
    assert_eq!(
        (&start.repo_id, &start.category),
        (&NodeId::new("R_1"), &NodeId::new("DIC_1"))
    );
    assert_eq!(start.title, "[Idea]");
    assert!(
        start.body.starts_with("### What would it do"),
        "{}",
        start.body
    );
    fixtures::done(&mut s, start, Ok(7));
    assert!(s.overlay.is_none());
    let of = DiscussionsOf::Repo(ghtui());
    assert_eq!(
        s.route(),
        Some(&Route::Discussions(discussions::Discussion {
            of,
            number: 7
        }))
    );
}
