//! Actions jobs and their logs.

use super::*;
use crate::route::{code, runs};

/// On a wide screen, output (a job's log) takes the whole width, lists
/// grow to 200 columns, and reading (an issue) stays at 160 with a
/// wider sidebar, centered. A narrow screen keeps GitHub's sidebar.
#[test]
fn output_takes_the_screen_and_reading_stays_narrow() {
    let mut s = state();
    let mut width = |size, route| {
        s.size = (size, 30);
        let _ = s.push(route);
        s.sync_page();
        let p = page(&s).page();
        let aside = p.aside.as_ref().map(|a| a.width);
        (
            p.width + aside.map_or(0, |a| a + crate::ui::page::ASIDE_GAP),
            aside,
        )
    };
    let job = Route::Runs(runs::Job {
        repo: ghtui(),
        run: Some(7),
        job: 2,
        step: None,
        query: String::new(),
    });
    assert!(width(220, job).0 > 200);
    assert_eq!(width(220, issues(OPEN)), (200, None));
    assert_eq!(width(220, issue(1)), (160, Some(36)));
    assert_eq!(width(110, issue(1)), (105, Some(30)));
}

/// `/` on a job searches its log: the steps with a match open, the
/// page opens at the first match, every match is picked out, and `n`
/// `p` go through them. Nothing is hidden. Esc ends the search, back
/// on the same page as it was.
#[test]
fn a_jobs_log_search_goes_through_its_matches() {
    let mut s = with_repo();
    let job_route = Route::Runs(runs::Job {
        repo: ghtui(),
        run: Some(7),
        job: 2,
        step: Some((1, 1)),
        query: String::new(),
    });
    let _ = s.push(job_route.clone());
    let depth = s.screens.len();
    let (job, log) = crate::fixtures::job();
    fetched(&mut s, DataKey::Job(ghtui(), 2), Data::Job(Box::new(job)));
    fetched(
        &mut s,
        DataKey::JobLog(ghtui(), 2),
        Data::Log(Arc::new(log)),
    );
    press(&mut s, "/");
    press(&mut s, "TEST");
    press(&mut s, "<Enter>");
    assert!(
        matches!(route(&s), Route::Runs(runs::Job { query, step: Some(_), .. }) if query == "TEST")
    );
    let Screen::Page(p) = s.screen() else {
        panic!()
    };
    let page = p.page();
    let line = |i: usize| -> String { page.lines[i].text() };
    assert!(!page.marks.is_empty());
    for &m in &page.marks {
        assert!(line(m).to_lowercase().contains("test"), "{}", line(m));
        let picked = page.lines[m].segs.iter().any(|s| {
            s.text().eq_ignore_ascii_case("test") && s.role == crate::ui::page::Role::Match
        });
        assert!(picked, "{}", line(m));
    }
    // The failed step's other lines are still there.
    assert!((0..page.lines.len()).any(|i| line(i).contains("Compiling")));
    assert_eq!(page.jump, page.marks.first().copied());
    let n = page.marks.len();
    assert!(n > 1, "{n}");
    says(&s, &format!("Match 1 of {n}"));
    press(&mut s, "n");
    says(&s, &format!("Match 2 of {n}"));
    let Screen::Page(p) = s.screen() else {
        panic!()
    };
    assert_eq!(p.current_match(), p.page().marks.get(1).copied());
    press(&mut s, "N");
    says(&s, &format!("Match 1 of {n}"));
    press(&mut s, "<Esc>");
    assert_eq!(route(&s), job_route);
    assert_eq!(s.screens.len(), depth);
    press(&mut s, "/");
    press(&mut s, "no such text");
    press(&mut s, "<Enter>");
    says(&s, "No matches for “no such text”");
    press(&mut s, "n");
    says(&s, "No matches for “no such text”");
}

/// A failed job opens at its failing step's first error, that line
/// selected.
#[test]
fn a_failed_job_opens_at_its_first_error() {
    let mut s = with_repo();
    let _ = s.push(Route::Runs(runs::Job {
        repo: ghtui(),
        run: Some(7),
        job: 2,
        step: None,
        query: String::new(),
    }));
    let (job, log) = crate::fixtures::job();
    fetched(&mut s, DataKey::Job(ghtui(), 2), Data::Job(Box::new(job)));
    fetched(
        &mut s,
        DataKey::JobLog(ghtui(), 2),
        Data::Log(Arc::new(log)),
    );
    let Screen::Page(p) = s.screen() else {
        panic!()
    };
    let page = p.page();
    let jump = page.jump.expect("it opens at a line");
    let text: String = page.lines[jump]
        .segs
        .iter()
        .map(crate::ui::page::Seg::text)
        .collect();
    assert!(text.contains("exit code 101"), "{text}");
    let link = p.selected_link().map(|l| format!("{l:?}"));
    assert!(
        link.as_ref().is_some_and(|l| l.contains("#step:3:")),
        "{link:?}"
    );
}

/// A running job, and its log (which GitHub doesn't have until the
/// job ends), are fetched again when their page is back on screen.
#[test]
fn a_running_jobs_log_is_checked_again() {
    let mut s = with_repo();
    let job_route = Route::Runs(runs::Job {
        repo: ghtui(),
        run: Some(7),
        job: 2,
        step: None,
        query: String::new(),
    });
    let _ = s.push(job_route);
    let (mut job, _) = crate::fixtures::job();
    job.outcome = crate::api::browse::CheckOutcome::Pending;
    let running = crate::api::browse::JobLog {
        running: true,
        ..Default::default()
    };
    fetched(&mut s, DataKey::Job(ghtui(), 2), Data::Job(Box::new(job)));
    fetched(
        &mut s,
        DataKey::JobLog(ghtui(), 2),
        Data::Log(Arc::new(running)),
    );
    let _ = s.push(Route::Code(code::Repo(ghtui())));
    let cmds = s.back();
    let keys = crate::fixtures::asked(&cmds);
    assert!(keys.contains(&DataKey::JobLog(ghtui(), 2)), "{keys:?}");
    assert!(keys.contains(&DataKey::Job(ghtui(), 2)), "{keys:?}");
    // A finished one isn't.
    let (job, log) = crate::fixtures::job();
    fetched(&mut s, DataKey::Job(ghtui(), 2), Data::Job(Box::new(job)));
    fetched(
        &mut s,
        DataKey::JobLog(ghtui(), 2),
        Data::Log(Arc::new(log)),
    );
    let _ = s.push(Route::Code(code::Repo(ghtui())));
    assert_eq!(crate::fixtures::asked(&s.back()), Vec::new());
}
