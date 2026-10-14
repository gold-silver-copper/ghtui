//! Reviews and timelines: a review's threads under it, the sidebar's
//! reviewers, the list of reviews and a review's page, editing a review's
//! summary, folding, and the events between comments.

use crate::act::{By, Subject};
use crate::api::edit::{Edit, Written};
use crate::api::model::ReviewState;
use crate::api::timeline::{Item, What};
use crate::route::pulls;

use super::*;
use crate::fixtures::{self};
use crate::ui::page::{Link, ReviewLink};

fn pr() -> PrRef {
    PrRef::parse("gold-silver-copper/ghtui#12").unwrap()
}

/// The page on screen, as text.
fn text(s: &State) -> String {
    let Screen::Page(p) = s.screen() else {
        panic!("not a page");
    };
    // Each line with what's on its right (a box's chips).
    let lines = p.page().lines.iter().map(|l| {
        let right: String = l.right.iter().map(crate::ui::page::Seg::text).collect();
        format!("{}  {right}", l.text())
    });
    lines.collect::<Vec<_>>().join("\n")
}

/// The pull request's conversation in a tall, wide terminal.
fn conversation() -> State {
    sized(with_pr(), 200, 80)
}

/// Selects the first item whose link `pick` likes, going down with `}`.
fn select(s: &mut State, pick: impl Fn(&Link) -> bool) {
    for _ in 0..40 {
        if let Screen::Page(p) = s.screen()
            && p.selected_link().is_some_and(&pick)
        {
            return;
        }
        press(s, "}");
    }
    panic!("nothing selectable like that:\n{}", text(s));
}

/// A review shows its state, its summary, and the threads it started
/// with the lines they're on and their replies; Enter on a thread opens
/// it in Files changed.
#[test]
fn a_review_shows_its_threads_and_opens_them() {
    let mut s = conversation();
    let page = text(&s);
    for part in [
        "hubot reviewed",
        "Yes: every role is checked at both depths.",
        "src/theme.rs:42",
        "+        Syntax::from_seed(seed)",
        "Is the seed's tone checked against the diff backgrounds?",
        "octocat replied",
        "monalisa approved these changes",
        "octocat requested a review from defunkt",
    ] {
        assert!(page.contains(part), "{part:?} in\n{page}");
    }
    select(
        &mut s,
        |l| matches!(l, Link::Url(u) if u.contains("discussion_r")),
    );
    press(&mut s, "<Enter>");
    let Screen::Diff(d) = s.screen() else {
        panic!("Enter opens Files changed")
    };
    assert_eq!(d.anchor.as_deref(), Some("discussion_r3000001"));
}

/// The sidebar says each reviewer's latest review, an approval of an
/// older head as such, who is asked (a code owner as one), and what it
/// closes.
#[test]
fn the_sidebar_says_where_each_reviewer_stands() {
    let s = conversation();
    let Screen::Page(p) = s.screen() else {
        panic!("not a page")
    };
    let aside = p.page().aside.as_ref().expect("a sidebar");
    let aside: Vec<String> = aside
        .lines
        .iter()
        .map(crate::ui::page::PageLine::text)
        .collect();
    let aside = aside.join("\n");
    for part in [
        "✓ monalisa approved · before the",
        "for octo-org/design",
        "◉ hubot commented",
        "● defunkt asked · code owner",
        "● octo-org/core asked",
        "#14 Pages wrap at the",
    ] {
        assert!(aside.contains(part), "{part:?} in\n{aside}");
    }
    // hubot reviewed the first commit, but only approvals go stale.
    assert!(!aside.contains("hubot commented · before"), "{aside}");
}

/// The Review tab lists every review, newest first; one opens its page,
/// which leads to the changes it reviewed and what changed since.
#[test]
fn the_review_tab_lists_reviews_and_one_opens_its_page() {
    let mut s = conversation();
    press(&mut s, "gr");
    fetched(
        &mut s,
        DataKey::Reviews(pr()),
        Data::Reviews(Box::new(fixtures::reviews())),
    );
    let page = text(&s);
    let (monalisa, hubot) = (page.find("monalisa approved"), page.rfind("hubot reviewed"));
    assert!(
        monalisa.zip(hubot).is_some_and(|(m, h)| m < h),
        "newest first:\n{page}"
    );
    select(&mut s, |l| {
        matches!(l, Link::Review(ReviewLink::Open { id: 2_000_002, .. }))
    });
    press(&mut s, "<Enter>");
    assert_eq!(
        s.route(),
        Some(&Route::Pulls(pulls::Pr {
            pr: pr(),
            tab: PrTab::Reviewed(2_000_002)
        }))
    );
    let page = text(&s);
    assert!(page.contains("src/theme.rs:42"), "{page}");
    assert!(
        page.contains("The changes it reviewed, as they were at 1b2c3d4"),
        "{page}"
    );
    assert!(
        page.contains("What changed since, from 1b2c3d4 to fedcba9"),
        "{page}"
    );
    select(&mut s, |l| {
        matches!(l, Link::Review(ReviewLink::AsReviewed { .. }))
    });
    let cmds = press(&mut s, "<Enter>");
    assert!(matches!(s.screen(), Screen::Diff(_)));
    let range = cmds.iter().find_map(|c| match c {
        Cmd::Git(crate::state::Git::LoadDiff { range, .. }) => range.clone(),
        _ => None,
    });
    assert_eq!(
        range,
        Some((
            String::new(),
            "1b2c3d4e5f60718293a4b5c6d7e8f9012345678a".into()
        ))
    );
}

/// `,e` on a review's summary edits it with GitHub's
/// `updatePullRequestReview`.
#[test]
fn a_reviews_summary_is_edited() {
    let mut s = conversation();
    select(
        &mut s,
        |l| matches!(l, Link::Quote { author, body, .. } if author == "hubot" && body.starts_with("Yes")),
    );
    press(&mut s, ",e");
    let Some(Overlay::Compose(c)) = &s.overlay else {
        panic!(",e opens the composer")
    };
    assert_eq!(c.text(), "Yes: every role is checked at both depths.");
    press(&mut s, "!");
    let cmds = press(&mut s, "<C-s>");
    let edit = Edit::Body {
        of: Written::Review,
        id: NodeId::new("PRR_2000002"),
        body: "Yes: every role is checked at both depths.!".into(),
    };
    let [Cmd::Api(Api::Change(change, by))] = cmds.as_slice() else {
        panic!("{cmds:?}")
    };
    assert_eq!(
        (change, by),
        (&Change::Edit(edit), &By::Edit(Subject::Pr(pr())))
    );
}

/// A review with many threads shows its first few; `za` there, or Enter
/// on its fold row, shows the rest, and folds them again.
#[test]
fn a_long_review_folds() {
    let mut activity = fixtures::activity();
    let thread = activity.threads[0].clone();
    activity.threads = (0..6)
        .map(|n| {
            let mut t = thread.clone();
            t.id = NodeId::new(format!("PRRT_{n}"));
            t.line = Some(10 + n);
            t
        })
        .collect();
    let mut s = conversation();
    fetched(
        &mut s,
        DataKey::PrActivity(pr()),
        Data::PrActivity(Box::new(activity)),
    );
    let count = |s: &State| text(s).matches("src/theme.rs:").count();
    assert_eq!(count(&s), 2, "{}", text(&s));
    assert!(
        text(&s).contains("▸ 4 more threads in this review"),
        "{}",
        text(&s)
    );
    select(
        &mut s,
        |l| matches!(l, Link::Quote { body, .. } if body.starts_with("Yes")),
    );
    press(&mut s, "za");
    assert_eq!(count(&s), 6, "{}", text(&s));
    select(&mut s, |l| matches!(l, Link::Review(ReviewLink::Fold(_))));
    press(&mut s, "<Enter>");
    assert_eq!(count(&s), 2);
}

/// An issue's timeline has its events between its comments, one line
/// each, a run of one person's labels as one; its sidebar has its type
/// and the pull requests that will close it.
#[test]
fn an_issues_timeline_and_development() {
    let s = sized(with_issue(), 200, 60);
    let page = text(&s);
    assert!(page.contains("octocat added  bug   ui "), "{page}");
    assert!(
        page.contains("monalisa linked a pull request that will close this issue"),
        "{page}"
    );
    let Screen::Page(p) = s.screen() else {
        panic!()
    };
    let aside = p.page().aside.as_ref().expect("a sidebar");
    let aside: Vec<String> = aside
        .lines
        .iter()
        .map(crate::ui::page::PageLine::text)
        .collect();
    let aside = aside.join("\n");
    assert!(aside.contains("Development"), "{aside}");
    assert!(aside.contains("#12 Cap the reading width"), "{aside}");
    assert!(aside.contains("Bug"), "{aside}");
    assert!(aside.contains("▦ Roadmap · In progress"), "{aside}");
}

/// A closed pull request says who closed it and when.
#[test]
fn a_closed_pr_says_who_closed_it() {
    let mut activity = fixtures::activity();
    activity.timeline.items.push(fixtures::event(
        "monalisa",
        "2026-10-02T10:00:00Z",
        What::Closed {
            by: None,
            reason: None,
            duplicate: false,
        },
    ));
    let mut s = conversation();
    fetched(
        &mut s,
        DataKey::PrActivity(pr()),
        Data::PrActivity(Box::new(activity)),
    );
    assert!(
        text(&s).contains("monalisa closed this 1d ago"),
        "{}",
        text(&s)
    );
}

/// Another's review that comes in while you look is marked New and named
/// in the status bar; yours isn't.
#[test]
fn a_new_review_is_marked_new() {
    let mut s = conversation();
    s.viewer = Some("octocat".into());
    let _ = crate::live::schedule(&mut s);
    let mut activity = fixtures::activity();
    activity.timeline.items.push(Item::Review(fixtures::review(
        2_000_009,
        "defunkt",
        ReviewState::ChangesRequested,
        "Please split this.",
        "2026-10-03T11:00:00Z",
    )));
    activity.timeline.items.push(Item::Review(fixtures::review(
        2_000_010,
        "octocat",
        ReviewState::Commented,
        "Done.",
        "2026-10-03T11:30:00Z",
    )));
    fetched(
        &mut s,
        DataKey::PrActivity(pr()),
        Data::PrActivity(Box::new(activity)),
    );
    fixtures::says(&s, "defunkt requested changes");
    let page = text(&s);
    let defunkt = page
        .lines()
        .find(|l| l.contains("defunkt requested changes"));
    assert!(defunkt.is_some_and(|l| l.contains("New")), "{page}");
    let mine = page.lines().find(|l| l.contains("octocat reviewed"));
    assert!(mine.is_some_and(|l| !l.contains("New")), "{page}");
}

/// Once git has read the file at the thread's commit, the thread shows
/// those lines, numbered, rather than GitHub's diff hunk.
#[test]
fn a_thread_shows_its_lines_from_git() {
    let mut s = conversation();
    let file: Vec<String> = (1..=42).map(|n| format!("line {n}")).collect();
    fetched(
        &mut s,
        DataKey::Lines(pr().repo, "abc".into(), "src/theme.rs".into()),
        Data::Lines(std::sync::Arc::new(file)),
    );
    let page = text(&s);
    assert!(page.contains("42 │ line 42"), "{page}");
    assert!(page.contains("39 │ line 39"), "{page}");
    assert!(!page.contains("Syntax::from_seed"), "{page}");
}
