//! Checks, and Actions: workflows, runs, jobs and their logs.

use super::*;
use crate::api::workflows::{Annotation, Artifact, Input, InputKind, Level};
use crate::ui::form::Row;

pub(super) fn outcome_mark(o: CheckOutcome) -> (&'static str, Role) {
    match o {
        CheckOutcome::Success => ("✓", Role::Success),
        CheckOutcome::Failure => ("✗", Role::Error),
        CheckOutcome::Pending => ("●", Role::Accent),
        CheckOutcome::Cancelled | CheckOutcome::Skipped => ("⊘", Role::Meta),
        CheckOutcome::Neutral => ("◦", Role::Meta),
    }
}

/// The checks on a commit, grouped by workflow or app, failures first.
/// Each links to its details on GitHub (logs, re-runs).
pub fn checks(page: &mut Page, checks: Fetched<'_, Checks>, now: u64) {
    let Some(c) = checks.show(page, "checks") else {
        return;
    };
    let sha = crate::ui::text::short_sha(&c.oid);
    if c.items.is_empty() {
        let title = meta(format!("✓ Checks on {sha}"));
        empty_box(page, title, "No checks ran on this commit.");
        return;
    }
    let mut summary = Vec::new();
    for (outcome, word) in [
        (CheckOutcome::Failure, "failed"),
        (CheckOutcome::Pending, "running"),
        (CheckOutcome::Success, "passed"),
        (CheckOutcome::Cancelled, "cancelled"),
        (CheckOutcome::Skipped, "skipped"),
        (CheckOutcome::Neutral, "neutral"),
    ] {
        let n = c.items.iter().filter(|i| i.outcome == outcome).count();
        if n > 0 {
            sep(&mut summary, " · ");
            let (mark, role) = outcome_mark(outcome);
            summary.push(Seg::new(format!("{mark} {n} {word}"), role));
        }
    }
    summary.push(meta(format!("   on {sha}")));
    if c.items.left_out() > 0 {
        let shown = format!(" · {} shown", count_of(&c.items));
        summary.push(meta(shown));
    }
    page.wrapped(summary, 0, Frame::None);
    page.blank();
    // Groups in order of their worst check, then by name.
    let mut groups: Vec<(&str, Vec<&CheckItem>)> = Vec::new();
    for check in &c.items {
        match groups.iter_mut().find(|(g, _)| *g == check.group) {
            Some((_, checks)) => checks.push(check),
            None => groups.push((&check.group, vec![check])),
        }
    }
    for (_, checks) in &mut groups {
        checks.sort_by(|a, b| (a.outcome, &a.name).cmp(&(b.outcome, &b.name)));
    }
    groups.sort_by_key(|(g, checks)| (checks.first().map(|c| c.outcome), *g));
    for (group, checks) in groups {
        page.box_top(vec![meta(group.to_owned())], Vec::new());
        for (n, check) in checks.into_iter().enumerate() {
            if n > 0 {
                page.box_rule();
            }
            check_row(page, check, now);
        }
        page.box_bottom();
        page.blank();
    }
}

pub(super) fn check_row(page: &mut Page, check: &CheckItem, now: u64) {
    let took = took(
        check.started_at.as_ref(),
        check.completed_at.as_ref(),
        check.outcome,
        now,
    );
    maybe_item(page, check.url.clone(), |page, link| {
        let name = Seg::linked(check.name.clone(), Role::Strong, link);
        status_row(page, outcome_mark(check.outcome), name, took);
        if let Some(summary) = check.summary.as_ref().filter(|_| !page.compact) {
            body(page, vec![meta(format!("  {summary}"))]);
        }
    });
    annotation_rows(page, &check.annotations, check.url.as_deref(), 2);
}

/// How many of a check's annotations a list of checks shows; its own
/// page (a job's) shows all it has.
const CHECK_ANNOTATIONS: usize = 5;

/// A check's annotations, worst first, each a row that opens the file at
/// its lines (one about the run opens `run`, the check's page). Under a
/// check in a list (`indent` > 0), only the first few.
pub(super) fn annotation_rows(
    page: &mut Page,
    list: &Capped<Annotation>,
    run: Option<&str>,
    indent: u16,
) {
    let shown = if indent > 0 {
        CHECK_ANNOTATIONS
    } else {
        usize::MAX
    };
    for a in list.iter().take(shown) {
        let (mark, role) = match a.level {
            Level::Failure => ("✗", Role::Error),
            Level::Warning => ("!", Role::Accent),
            Level::Notice => ("i", Role::Meta),
        };
        let target = a.url.as_deref().or(run).map(str::to_owned);
        maybe_item(page, target, |page, link| {
            let linked = if link.is_some() {
                Role::Link
            } else {
                Role::Meta
            };
            let place = Seg::linked(a.place(), linked, link);
            let mut segs = vec![Seg::new(format!("{mark} "), role), place];
            if !a.title.is_empty() {
                segs.push(strong(format!("  {}", a.title)));
            }
            page.box_line(segs, Vec::new(), indent);
            // Under a check, its first line; on its own page, all of it.
            let lines = a
                .message
                .lines()
                .take(if indent > 0 { 1 } else { usize::MAX });
            for line in lines {
                let text = vec![Seg::new(line.to_owned(), Role::Body)];
                page.wrapped(text, indent + 2, Frame::Body);
            }
        });
    }
    let more = list.total().saturating_sub(list.len().min(shown) as u64);
    if more > 0 {
        let text = format!("… {more} more {}", noun(more, "annotation"));
        let segs = match run {
            Some(url) => vec![link_seg(page, text, url.to_owned(), Role::Link)],
            None => vec![meta(text)],
        };
        page.box_line(segs, Vec::new(), indent);
    }
}

/// How long something took, or how long ago it started while it runs.
pub(super) fn took(
    start: Option<&String>,
    end: Option<&String>,
    outcome: CheckOutcome,
    now: u64,
) -> Option<String> {
    match (start, end, outcome) {
        (Some(s), Some(e), _) => time::duration_iso(s, e),
        (Some(s), None, CheckOutcome::Pending) => {
            Some(format!("started {}", time::ago_iso(s, now)))
        }
        _ => None,
    }
}

/// A row of something that ran: its mark, its name, and how long it
/// took.
pub(super) fn status_row(
    page: &mut Page,
    (mark, role): (&str, Role),
    name: Seg,
    took: Option<String>,
) {
    let segs = vec![Seg::new(format!("{mark} "), role), name];
    page.box_line(segs, Vec::from_iter(took.map(meta)), 0);
}

pub(super) fn outcome_title(page: &mut Page, title: String, outcome: CheckOutcome) {
    let (mark, role) = outcome_mark(outcome);
    page.wrapped(
        vec![
            Seg::new(format!("{mark} "), role),
            Seg::new(title, Role::Title),
        ],
        0,
        Frame::None,
    );
}

/// A workflow run: what it ran for, and its jobs, failures first. Re-runs
/// and cancelling write, so they stay on GitHub.
pub fn workflow_run(page: &mut Page, repo: &RepoId, run: &WorkflowRun, now: u64) {
    outcome_title(page, format!("{} #{}", run.name, run.number), run.outcome);
    if !run.title.is_empty() {
        say(page, run.title.clone(), Role::Body);
    }
    let mut subline = vec![meta(format!("{} · ", run.event))];
    if let Some(branch) = &run.branch {
        subline.push(link_seg(
            page,
            branch.clone(),
            url::tree(repo, branch, ""),
            Role::Code,
        ));
        subline.push(meta(" · "));
    }
    let sha = crate::ui::text::short_sha(&run.sha);
    subline.push(link_seg(page, sha, url::commit(repo, &run.sha), Role::Code));
    if let Some(actor) = &run.actor {
        subline.push(meta(" · "));
        subline.push(login_seg(page, actor, Role::Strong));
    }
    if let Some(at) = &run.started_at {
        subline.push(meta(format!(" · started {}", time::ago_iso(at, now))));
    }
    if let Some(d) = took(
        run.started_at.as_ref(),
        run.updated_at.as_ref(),
        run.outcome,
        now,
    )
    .filter(|_| run.outcome != CheckOutcome::Pending)
    {
        subline.push(meta(format!(" · took {d}")));
    }
    page.wrapped(subline, 0, Frame::None);
    let file = run.path.rsplit('/').next().unwrap_or(&run.path).to_owned();
    let mut facts = Vec::new();
    if !file.is_empty() {
        let workflow = format!(
            "{}/actions/workflows/{}",
            url::repo(repo),
            url::encode_path(&file)
        );
        facts.push(meta("Workflow "));
        facts.push(link_seg(page, file, workflow, Role::Link));
        facts.push(meta("   "));
    }
    if run.attempt > 1 {
        facts.push(meta(format!("Attempt {}   ", run.attempt)));
    }
    page.wrapped(facts, 0, Frame::None);
    page.blank();
    let mut jobs = run.jobs.clone();
    jobs.sort_by(|a, b| (a.outcome, &a.name).cmp(&(b.outcome, &b.name)));
    let title = counted("Jobs", &run.jobs);
    // A run just started (or re-run) has no jobs until GitHub queues them.
    let none = if run.outcome == CheckOutcome::Pending {
        "Waiting for GitHub to queue its jobs…"
    } else {
        "No jobs."
    };
    // A failure is a key away: the page opens on the first failed job.
    let failed = jobs.iter().position(|j| j.outcome == CheckOutcome::Failure);
    if failed.is_some() {
        page.selects = Some(page.items.len());
    }
    list_box(page, title, Vec::new(), &jobs, none, |page, job| {
        let target = format!("{}/actions/runs/{}/job/{}", url::repo(repo), run.id, job.id);
        let took = took(
            job.started_at.as_ref(),
            job.completed_at.as_ref(),
            job.outcome,
            now,
        );
        item(page, target, |page, link| {
            let name = Seg::linked(job.name.clone(), Role::Strong, link);
            status_row(page, outcome_mark(job.outcome), name, took);
        });
    });
    artifacts(page, &run.artifacts, now);
}

/// A run's artifacts: each one's size and when it expires; opening one
/// saves it.
fn artifacts(page: &mut Page, list: &Capped<Artifact>, now: u64) {
    if list.is_empty() {
        return;
    }
    page.blank();
    let title = counted("Artifacts", list);
    let right = vec![meta("↵ saves one")];
    list_box(page, title, right, list, "", |page, a| {
        let when = match (&a.expires_at, a.expired) {
            (_, true) => "expired".to_owned(),
            (Some(at), false) => format!("expires {}", time::until_iso(at, now)),
            (None, false) => String::new(),
        };
        let right = vec![meta(format!("{}  {when}", crate::ui::text::size(a.size)))];
        let target = Link::Artifact {
            id: a.id,
            name: a.name.clone(),
        };
        item(page, target, |page, link| {
            let role = if a.expired { Role::Meta } else { Role::Strong };
            let segs = vec![meta("▤ "), Seg::linked(a.name.clone(), role, link)];
            page.box_line(segs, right, 0);
        });
    });
}

/// Which step each log line belongs to. GitHub's one log for a job
/// doesn't say, and its steps' times are to the second, so a step that
/// ran begins at the line that starts it (`##[group]Run …`, `Post job
/// cleanup.`, `Cleaning up orphan processes`) or once the step before
/// has ended, whichever comes first. The lines cut from a long log
/// (`cut`) only count: each step's first number is how many of them it has.
pub fn step_lines<'a>(job: &Job, cut: &'a str, log: &'a str) -> Vec<(u32, usize, Vec<&'a str>)> {
    fn second(at: Option<&String>) -> Option<&str> {
        at.map(|at| at.get(..19).unwrap_or(at))
    }
    let mut out: Vec<(u32, usize, Vec<&str>)> = job
        .steps
        .iter()
        .map(|s| (s.number, 0, Vec::new()))
        .collect();
    // Skipped steps log nothing.
    let ran: Vec<usize> = job
        .steps
        .iter()
        .enumerate()
        .filter(|(_, s)| s.outcome != CheckOutcome::Skipped)
        .map(|(i, _)| i)
        .collect();
    let mut at = 0;
    let lines = cut
        .lines()
        .map(|l| (true, l))
        .chain(log.lines().map(|l| (false, l)));
    for (was_cut, raw) in lines {
        let (time, text) = crate::api::browse::log::split(raw);
        if let Some(time) = time {
            let t = time.get(..19).unwrap_or(time);
            let run = text.strip_prefix("##[group]");
            let marks = crate::api::browse::log::starts_step(text);
            while let (Some(current), Some(next)) = (
                ran.get(at).and_then(|&i| job.steps.get(i)),
                ran.get(at + 1).and_then(|&i| job.steps.get(i)),
            ) {
                // A step named after what it runs starts at its own `Run`
                // line, not a composite action's inner ones. Each step's
                // log begins with its start line, so one that has none yet
                // is the one this line starts.
                let begun = ran
                    .get(at)
                    .and_then(|&i| out.get(i))
                    .is_some_and(|(_, cut, lines)| *cut > 0 || !lines.is_empty());
                let starts = marks
                    && begun
                    && run.is_none_or(|r| !next.name.starts_with("Run ") || r == next.name);
                let next_started = second(next.started_at.as_ref()).is_some_and(|s| s <= t);
                let ended = second(current.completed_at.as_ref()).is_some_and(|s| s < t);
                if !next_started || !(ended || starts) {
                    break;
                }
                at += 1;
            }
        }
        if let Some((_, cut, lines)) = ran.get(at).and_then(|&i| out.get_mut(i)) {
            if was_cut {
                *cut += 1;
            } else {
                lines.push(text);
            }
        }
    }
    out
}

/// `text` without terminal escape sequences (colors, mostly).
pub fn strip_escapes(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.next() {
            // CSI: parameters, then a final byte from @ to ~.
            Some('[') => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            // OSC: up to BEL or ESC \.
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\u{7}' || (c == '\u{1b}' && chars.next_if_eq(&'\\').is_some()) {
                        break;
                    }
                }
            }
            // Others: intermediate bytes, then a final byte (`ESC ( B`);
            // or one byte (`ESC 7`).
            Some(' '..='/') => {
                for c in chars.by_ref() {
                    if ('0'..='~').contains(&c) {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// A log line as shown, as GitHub shows it: its markers (`##[group]`,
/// a composite action's `##[start-action]`, `##[error]`, `##[warning]`,
/// `##[notice]`, `##[debug]`, `[command]`) as styles rather than text, and
/// its color codes gone (`text` comes without them).
pub fn log_seg(text: &str) -> Seg {
    if let Some(group) = text.strip_prefix("##[group]") {
        strong(format!("▾ {group}"))
    } else if text.starts_with("##[endgroup]") || text.starts_with("##[end-action ") {
        meta("")
    } else if let Some(action) = text.strip_prefix("##[start-action ") {
        // A composite action's own step: `display=Its name;id=…]`.
        let display = action
            .split(';')
            .find_map(|field| field.strip_prefix("display="))
            .unwrap_or(action)
            .trim_end_matches(']');
        strong(format!("▾ {display}"))
    } else if let Some(error) = text.strip_prefix("##[error]") {
        Seg::new(error.to_owned(), Role::Removed)
    } else if let Some(warning) = text.strip_prefix("##[warning]") {
        Seg::new(warning.to_owned(), Role::Accent)
    } else if let Some(notice) = text.strip_prefix("##[notice]") {
        strong(notice.to_owned())
    } else if let Some(debug) = text.strip_prefix("##[debug]") {
        meta(debug.to_owned())
    } else if let Some(command) = text
        .strip_prefix("##[command]")
        .or_else(|| text.strip_prefix("[command]"))
    {
        Seg::new(command.to_owned(), Role::Syntax(Syntax::Function))
    } else {
        Seg::new(text.to_owned(), Role::Body)
    }
}

/// A job: its steps, each collapsed to its outcome, failing ones (and the
/// one a link points at) expanded with their whole log. With `query`, the
/// steps whose log has it are expanded too, its matches picked out and
/// marked for `n`/`N`.
pub fn job(
    page: &mut Page,
    repo: &RepoId,
    job: &Job,
    log: Fetched<'_, JobLog>,
    at: JobAt<'_>,
    now: u64,
) {
    outcome_title(page, job.name.clone(), job.outcome);
    let mut subline = Vec::new();
    let run = format!("{}/actions/runs/{}", url::repo(repo), job.run_id);
    subline.push(link_seg(
        page,
        format!("{} #{}", job.run.name, job.run.number),
        run.clone(),
        Role::Link,
    ));
    if job.run.attempt > 1 {
        let of = format!(" · attempt {} of {}", job.attempt, job.run.attempt);
        subline.push(meta(of));
    }
    if let Some(rerun) = job.run.rerun {
        subline.push(meta(" · "));
        let target = format!("{run}/job/{rerun}");
        subline.push(link_seg(page, "its re-run", target, Role::Link));
    }
    if let Some(d) = took(
        job.started_at.as_ref(),
        job.completed_at.as_ref(),
        job.outcome,
        now,
    ) {
        subline.push(meta(format!(" · {d}")));
    }
    if job.outcome != CheckOutcome::Pending && job.run.outcome == CheckOutcome::Pending {
        subline.push(meta(" · its run is still going"));
    }
    page.wrapped(subline, 0, Frame::None);
    page.blank();
    if !job.annotations.is_empty() {
        let title = counted("Annotations", &job.annotations);
        page.box_top(title, Vec::new());
        annotation_rows(page, &job.annotations, None, 0);
        page.box_bottom();
        page.blank();
    }
    filter_field(page, at.query, at.keys, "search");
    page.blank();
    #[expect(
        clippy::disallowed_methods,
        reason = "said under each open step, or in place of the steps"
    )]
    let log = log.text("the log");
    let lines = match log {
        Ok(log) if !log.running => Some(step_lines(job, &log.cut, &log.text)),
        _ => None,
    };
    let title = vec![strong(format!("Steps  {}", job.steps.len()))];
    let right = vec![meta(format!("{} searches the log", at.keys.filter))];
    page.box_top(title, right);
    // GitHub drops an old job's steps; its log may be gone too.
    if job.steps.is_empty() {
        let text = match &log {
            Err(why) => why.clone(),
            Ok(l) if l.running => RUNNING_LOG.to_owned(),
            Ok(_) => "GitHub lists no steps for this job: open it on GitHub.".to_owned(),
        };
        empty_row(page, &text);
    }
    let q = at.query.to_lowercase();
    // Whether a line, without its escapes or as logged, has the search.
    let hit = |line: &str| !q.is_empty() && crate::ui::text::has(&line.to_lowercase(), &q);
    let matches = |line: &str| !q.is_empty() && hit(&strip_escapes(line));
    // With a match anywhere, the search decides where the page opens.
    let searched = lines
        .iter()
        .flatten()
        .any(|(_, _, l)| l.iter().any(|t| matches(t)));
    // It opens on the step a link points at, else the first that failed;
    // followed, on the step running, or once it's done, the last that
    // logged; else the first, so Enter opens it.
    let has_lines =
        |n: u32| (lines.iter().flatten()).any(|(s, cut, l)| *s == n && (*cut > 0 || !l.is_empty()));
    let followed = (at.follows && at.step.is_none() && at.query.is_empty())
        .then(|| {
            let running = job
                .steps
                .iter()
                .find(|s| s.outcome == CheckOutcome::Pending && s.started_at.is_some());
            running.or_else(|| job.steps.iter().rev().find(|s| has_lines(s.number)))
        })
        .flatten();
    let opens = (job.steps.iter())
        .find(|s| at.step.is_some_and(|(n, _)| n == s.number))
        .or_else(|| (job.steps.iter()).find(|s| s.outcome == CheckOutcome::Failure))
        .or(followed)
        .or(job.steps.first())
        .map(|s| s.number);
    let follow = followed.map(|s| s.number).filter(|n| opens == Some(*n));
    let job_url = format!(
        "{}/actions/runs/{}/job/{}",
        url::repo(repo),
        job.run_id,
        job.id
    );
    for (n, step) in job.steps.iter().enumerate() {
        if n > 0 {
            page.box_rule();
        }
        if opens == Some(step.number) {
            page.selects = Some(page.items.len());
            // Followed while it runs: the page stays on the step running.
            if follow == Some(step.number) && lines.is_none() {
                page.jump = Some(page.lines.len());
            }
        }
        // Each line is a row, as a file's are: `y` copies its link.
        let step_url = format!("{job_url}#step:{}:", step.number);
        let mark = match step.outcome {
            // Queued: not running yet.
            CheckOutcome::Pending if step.started_at.is_none() => ("○", Role::Meta),
            outcome => outcome_mark(outcome),
        };
        let took = took(
            step.started_at.as_ref(),
            step.completed_at.as_ref(),
            step.outcome,
            now,
        );
        item(page, format!("{step_url}1"), |page, link| {
            let name = Seg::linked(step.name.clone(), Role::Strong, link);
            status_row(page, mark, name, took);
        });
        let pointed = at.step.map(|(s, _)| s) == Some(step.number);
        let failed = step.outcome == CheckOutcome::Failure;
        // Numbered from the step's first line, cut ones included.
        let (cut, raw) = lines
            .as_ref()
            .and_then(|lines| lines.iter().find(|(n, _, _)| *n == step.number))
            .map_or((0, &[][..]), |(_, cut, l)| (*cut, l.as_slice()));
        let found = raw.iter().any(|l| matches(l));
        let followed = follow == Some(step.number);
        if !(pointed || failed || found || followed) {
            continue;
        }
        if lines.is_none() {
            let text = match &log {
                Err(why) => why.clone(),
                Ok(_) => RUNNING_LOG.to_owned(),
            };
            body(page, vec![meta(text)]);
            continue;
        }
        // Each shown line without its escapes, numbered from the step's
        // first line, cut ones included.
        let shown: Vec<(usize, String)> = (raw.iter().enumerate())
            .map(|(i, l)| (cut + i, strip_escapes(l)))
            .collect();
        // Where the page opens: the first search match; else the line a
        // link points at; else a failed step's first error (or its end).
        let at_line = |l: u32| shown.iter().position(|(i, _)| i + 1 == l as usize);
        let first_error = || {
            let error = (shown.iter()).position(|(_, l)| l.starts_with("##[error]"));
            error.or(shown.len().checked_sub(1))
        };
        let focus = match at.step {
            _ if searched => shown.iter().position(|(_, l)| hit(l)),
            Some((_, l)) if pointed => at_line(l),
            _ if failed => first_error(),
            // Followed to its end, as `tail -f` leaves it.
            _ if followed => shown.len().checked_sub(1),
            _ => None,
        };
        let pointed_cut = at
            .step
            .map(|(_, l)| l as usize)
            .filter(|&l| pointed && !searched && l >= 1 && l <= cut);
        if let Some(l) = pointed_cut {
            page.jump.get_or_insert(page.lines.len());
            let text =
                format!("Line {l} is in the part of the log too long to load: open it on GitHub.");
            body(page, vec![meta(text)]);
        }
        if cut > 0 {
            let lines = format!("{cut} earlier {}", noun(cut as u64, "line"));
            let text = if q.is_empty() {
                format!("… {lines} (all of them on GitHub)")
            } else {
                format!("… {lines} weren't loaded, so aren't searched (they're on GitHub)")
            };
            body(page, vec![meta(text)]);
        }
        let width = shown.last().map_or(1, |(i, _)| (i + 1).to_string().len());
        for (n, (i, text)) in shown.iter().enumerate() {
            // The page opens there, with that line selected.
            if focus == Some(n) && page.jump.is_none() {
                page.jump = Some(page.lines.len());
                page.selects = Some(page.items.len());
            }
            if hit(text) {
                page.marks.push(page.lines.len());
            }
            let mut segs = vec![line_number(i + 1, width)];
            segs.extend(crate::ui::page::highlight(log_seg(text), &q));
            let start = page.lines.len();
            page.push(PageLine {
                segs,
                frame: Frame::Body,
                tone: Tone::Code,
                fixed: 1,
                ..PageLine::default()
            });
            let link = page.link(format!("{step_url}{}", i + 1));
            page.item(start, link);
        }
        if pointed && !searched {
            page.jump.get_or_insert(page.lines.len().saturating_sub(1));
        }
    }
    page.box_bottom();
}

/// What a running job's page says in place of its log.
const RUNNING_LOG: &str = "GitHub writes the log when the job ends: it shows here then, and the steps follow the job until it does";

/// Where a job page is: the step a link points at, the log's filter, and
/// whether it follows the job (it was running when you started looking).
#[derive(Clone, Copy)]
pub struct JobAt<'a> {
    pub step: Option<(u32, u32)>,
    pub query: &'a str,
    pub keys: Keys<'a>,
    pub follows: bool,
}

/// A workflow and its runs, newest first.
pub fn workflow(
    page: &mut Page,
    repo: &RepoId,
    wf: Fetched<'_, Workflow>,
    runs: Fetched<'_, Results<RunSummary>>,
    now: u64,
) {
    if let Some(wf) = wf.show(page, "the workflow") {
        let mut title = vec![Seg::new(wf.name.clone(), Role::Title)];
        if let Some(why) = &wf.disabled {
            tag(&mut title, why.clone(), Bg::SecondaryContainer);
        }
        page.wrapped(title, 0, Frame::None);
        let file = link_seg(
            page,
            wf.path.clone(),
            url::blob(repo, "HEAD", &wf.path),
            Role::Code,
        );
        page.wrapped(vec![file], 0, Frame::None);
        page.blank();
    }
    run_list(page, repo, runs, "No runs yet.", now);
}

/// Workflow runs, each with its workflow, what started it, when, and how
/// long it took.
pub(super) fn run_list(
    page: &mut Page,
    repo: &RepoId,
    runs: Fetched<'_, Results<RunSummary>>,
    empty: &str,
    now: u64,
) {
    paged_list(page, "Runs", runs, empty, |page, run| {
        item(
            page,
            format!("{}/actions/runs/{}", url::repo(repo), run.id),
            |page, link| {
                let title = if run.title.is_empty() {
                    format!("Run #{}", run.number)
                } else {
                    run.title.clone()
                };
                let start = run.started_at.as_ref();
                let end = run
                    .updated_at
                    .as_ref()
                    .filter(|_| run.outcome != CheckOutcome::Pending);
                let took = took(start, end, run.outcome, now);
                let name = Seg::linked(title, Role::Strong, link);
                status_row(page, outcome_mark(run.outcome), name, took);
                let mut subline = format!("{} #{} · {}", run.workflow, run.number, run.event);
                if let Some(b) = &run.branch {
                    subline.push_str(&format!(" · {b}"));
                }
                if let Some(a) = &run.actor {
                    subline.push_str(&format!(" · {a}"));
                }
                if let Some(at) = &run.created_at {
                    subline.push_str(&format!(" · {}", time::ago_iso(at, now)));
                }
                body(page, vec![meta(subline)]);
            },
        );
    });
}

/// The form that runs a workflow by hand, as GitHub's "Run workflow"
/// menu has it.
pub struct DispatchView<'a> {
    pub name: &'a str,
    pub path: &'a str,
    /// The branch it runs on.
    pub git_ref: &'a str,
    /// Its inputs, or why there are none to show.
    pub inputs: Result<&'a [Input], String>,
    /// The values chosen, by input.
    pub values: &'a std::collections::HashMap<String, String>,
    pub sending: bool,
    pub error: Option<&'a str>,
    /// The key that runs it.
    pub run_key: &'a str,
}

pub fn dispatch_form(page: &mut Page, v: &DispatchView<'_>) {
    page.wrapped(
        vec![
            Seg::new("▶ ", Role::Accent),
            Seg::new(format!("Run {}", v.name), Role::Title),
        ],
        0,
        Frame::None,
    );
    say(page, v.path.to_owned(), Role::Code);
    page.blank();
    page.box_top(vec![strong("Use workflow from")], Vec::new());
    item(page, Link::Form(Row::Ref), |page, link| {
        let segs = vec![
            meta("Branch  "),
            Seg::linked(format!("{} ▾", v.git_ref), Role::Code, link),
        ];
        page.box_line(segs, Vec::new(), 0);
    });
    page.box_bottom();
    page.blank();
    let inputs = match &v.inputs {
        Ok(inputs) => *inputs,
        Err(why) => {
            say(page, why.clone(), Role::Meta);
            return;
        }
    };
    if !inputs.is_empty() {
        let title = vec![strong(format!("Inputs  {}", inputs.len()))];
        let right = vec![meta("↵ sets one")];
        page.box_top(title, right);
        for (n, input) in inputs.iter().enumerate() {
            if n > 0 {
                page.box_rule();
            }
            let value = v
                .values
                .get(&input.name)
                .cloned()
                .unwrap_or_else(|| input.start());
            item(
                page,
                Link::Form(Row::Input(input.name.clone())),
                |page, link| {
                    let mut segs = vec![Seg::linked(input.name.clone(), Role::Strong, link)];
                    if input.required {
                        segs.push(Seg::new(" *", Role::Error));
                    }
                    let shown = match input.kind {
                        InputKind::Choice(_) => format!("{value} ▾"),
                        InputKind::Boolean => {
                            if value == "true" {
                                "☑ true".to_owned()
                            } else {
                                "☐ false".to_owned()
                            }
                        }
                        InputKind::Text if value.is_empty() => "(empty)".to_owned(),
                        InputKind::Text => format!("“{value}”"),
                    };
                    page.box_line(segs, vec![Seg::new(shown, Role::Code)], 0);
                    if !input.description.is_empty() {
                        let text = meta(input.description.clone());
                        page.box_line(vec![text], Vec::new(), 2);
                    }
                },
            );
        }
        page.box_bottom();
        page.blank();
    }
    if let Some(error) = v.error {
        say(page, error.to_owned(), Role::Error);
    }
    item(page, Link::Form(Row::Save), |page, link| {
        let label = if v.sending {
            "Starting the run…"
        } else {
            "▶ Run workflow"
        };
        let segs = vec![
            Seg::linked(label, Role::Link, link),
            meta(format!("   ↵ or {} runs it on {}", v.run_key, v.git_ref)),
        ];
        page.line(segs);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The run a test job is in: one attempt, finished.
    fn finished_run() -> crate::api::browse::JobRun {
        crate::api::browse::JobRun {
            name: "CI".into(),
            number: 1,
            outcome: CheckOutcome::Success,
            attempt: 1,
            rerun: None,
        }
    }

    /// A finished job of `steps`.
    fn job_of(steps: Vec<crate::api::browse::Step>) -> Job {
        Job {
            id: 1,
            run_id: 1,
            attempt: 1,
            run: finished_run(),
            annotations: Default::default(),
            name: String::new(),
            outcome: CheckOutcome::Success,
            started_at: None,
            completed_at: None,
            steps,
        }
    }

    /// A job page at `step`, searched for `query`.
    fn at(step: Option<(u32, u32)>, query: &str) -> JobAt<'_> {
        JobAt {
            step,
            query,
            keys: Keys::default(),
            follows: false,
        }
    }

    /// Step `n`'s lines.
    fn lines_of<'a>(lines: &[(u32, usize, Vec<&'a str>)], n: u32) -> Vec<&'a str> {
        let step = lines.iter().find(|(s, _, _)| *s == n);
        step.map(|(_, _, l)| l.clone()).unwrap_or_default()
    }

    /// Steps that start in the second the one before ends still get their
    /// own lines, by the lines that start them.
    #[test]
    fn log_lines_go_to_their_steps() {
        let step = |number, outcome, start: &str, end: &str| crate::api::browse::Step {
            number,
            name: String::new(),
            outcome,
            started_at: Some(format!("2026-10-05T17:42:{start}Z")),
            completed_at: Some(format!("2026-10-05T17:42:{end}Z")),
        };
        let job = job_of(vec![
            step(1, CheckOutcome::Success, "04", "04"),
            step(2, CheckOutcome::Success, "04", "16"),
            step(3, CheckOutcome::Skipped, "16", "16"),
            step(4, CheckOutcome::Success, "16", "37"),
            step(8, CheckOutcome::Success, "37", "37"),
            step(9, CheckOutcome::Success, "37", "37"),
        ]);
        let log = [
            "\u{feff}2026-10-05T17:42:04.31Z Current runner version",
            "2026-10-05T17:42:04.32Z ##[group]Runner Image",
            "2026-10-05T17:42:04.90Z ##[group]Run actions/checkout@v4",
            "2026-10-05T17:42:16.01Z HEAD is now at 984a54e",
            "2026-10-05T17:42:16.07Z ##[group]Run cd src/ci/citool",
            "2026-10-05T17:42:37.10Z done",
            "2026-10-05T17:42:37.20Z Post job cleanup.",
            "2026-10-05T17:42:37.49Z Cleaning up orphan processes",
        ]
        .join("\n");
        let lines = step_lines(&job, "", &log);
        let of = |n| lines_of(&lines, n);
        assert_eq!(of(1), ["Current runner version", "##[group]Runner Image"]);
        assert_eq!(
            of(2),
            ["##[group]Run actions/checkout@v4", "HEAD is now at 984a54e"]
        );
        assert!(of(3).is_empty());
        assert_eq!(of(4), ["##[group]Run cd src/ci/citool", "done"]);
        assert_eq!(of(8), ["Post job cleanup."]);
        assert_eq!(of(9), ["Cleaning up orphan processes"]);
        // A start line just after the step before ended starts one step,
        // not every step that started in that second.
        let job = job_of(vec![
            step(1, CheckOutcome::Success, "48", "48"),
            step(2, CheckOutcome::Success, "48", "49"),
            step(3, CheckOutcome::Failure, "49", "49"),
        ]);
        let log = [
            "2026-10-05T17:42:48.10Z Current runner version",
            "2026-10-05T17:42:49.10Z ##[group]Run ./prepare",
            "2026-10-05T17:42:49.11Z prepared",
            "2026-10-05T17:42:49.20Z ##[group]Run ./build",
            "2026-10-05T17:42:49.21Z ##[error]boom",
        ]
        .join("\n");
        let lines = step_lines(&job, "", &log);
        assert_eq!(lines_of(&lines, 2), ["##[group]Run ./prepare", "prepared"]);
        assert_eq!(
            lines_of(&lines, 3),
            ["##[group]Run ./build", "##[error]boom"]
        );
    }

    /// A long step's log shows whole: a link to a line far up opens there,
    /// and a search sees (and opens at) matches anywhere in it.
    #[test]
    fn a_long_steps_log_is_all_there_to_link_and_search() {
        let step = crate::api::browse::Step {
            number: 1,
            name: "Build".into(),
            outcome: CheckOutcome::Success,
            started_at: Some("2026-10-05T17:42:00Z".into()),
            completed_at: Some("2026-10-05T17:43:00Z".into()),
        };
        let job = job_of(vec![step]);
        let log: String = (1..=3000)
            .map(|n| {
                let needle = if n % 1000 == 0 { " needle" } else { "" };
                format!("2026-10-05T17:42:01.0Z line {n}{needle}\n")
            })
            .collect();
        let shown = |query: &str| {
            let mut page = Page::new(100);
            job_page(&mut page, &job, &log, at(Some((1, 10)), query));
            let lines: Vec<String> = page.lines.iter().map(PageLine::text).collect();
            let at = |i: Option<usize>| i.and_then(|i| lines.get(i)).cloned().unwrap_or_default();
            let marks: Vec<String> = page.marks.iter().map(|&m| at(Some(m))).collect();
            (at(page.jump), marks)
        };
        let (jump, marks) = shown("");
        assert!(jump.ends_with("line 10"), "{jump}");
        assert!(marks.is_empty());
        let (jump, marks) = shown("NEEDLE");
        assert_eq!(marks.len(), 3, "{marks:?}");
        assert!(jump.ends_with("line 1000 needle"), "{jump}");
        assert!(marks[2].ends_with("line 3000 needle"));
    }

    /// The log's filter matches what shows, not the color codes around it.
    #[test]
    fn the_log_filter_ignores_color_codes() {
        let job = job_of(vec![crate::api::browse::Step {
            number: 1,
            name: "Build".into(),
            outcome: CheckOutcome::Success,
            started_at: None,
            completed_at: None,
        }]);
        let log = "2026-10-05T17:42:01.0Z \u{1b}[36;1mcd\u{1b}[0m src\n";
        let mut page = Page::new(100);
        job_page(&mut page, &job, log, at(None, "cd src"));
        assert!(page.lines.iter().any(|l| l.text().ends_with("cd src")));
    }

    /// A run's jobs past what's fetched read like any capped list's.
    #[test]
    fn a_runs_jobs_left_out_are_more_on_github() {
        let job = crate::api::browse::JobSummary {
            id: 1,
            name: "build".into(),
            outcome: CheckOutcome::Success,
            started_at: None,
            completed_at: None,
        };
        let run = WorkflowRun {
            id: 1,
            name: "CI".into(),
            title: "t".into(),
            number: 1,
            attempt: 1,
            event: "push".into(),
            branch: None,
            sha: "abc".into(),
            outcome: CheckOutcome::Success,
            actor: None,
            started_at: None,
            updated_at: None,
            path: ".github/workflows/ci.yml".into(),
            jobs: Capped::new(vec![job], 51),
            artifacts: Capped::default(),
        };
        let mut page = Page::new(100);
        workflow_run(&mut page, &RepoId::new("o", "r"), &run, 0);
        let texts: Vec<String> = page.lines.iter().map(PageLine::text).collect();
        assert!(
            texts.iter().any(|t| t.contains("… 50 more on GitHub")),
            "{texts:#?}"
        );
    }

    fn job_page(page: &mut Page, job: &Job, log: &str, at: JobAt<'_>) {
        let log = JobLog {
            text: log.into(),
            ..JobLog::default()
        };
        log_page(page, job, Fetched::ready(&log), at);
    }

    fn log_page(page: &mut Page, job: &Job, log: Fetched<'_, JobLog>, at: JobAt<'_>) {
        super::job(page, &RepoId::new("o", "r"), job, log, at, 0);
    }

    fn one_step_job(outcome: CheckOutcome) -> Job {
        let job = job_of(vec![crate::api::browse::Step {
            number: 1,
            name: "Build".into(),
            outcome,
            started_at: Some("2026-10-05T17:42:00Z".into()),
            completed_at: None,
        }]);
        Job { outcome, ..job }
    }

    /// A job GitHub keeps no steps for (an old one) still says what became
    /// of its log.
    #[test]
    fn a_job_without_steps_says_what_became_of_its_log() {
        let mut job = one_step_job(CheckOutcome::Success);
        job.steps.clear();
        let mut page = Page::new(120);
        let expired = "this log has expired";
        log_page(&mut page, &job, Fetched::failed(expired), at(None, ""));
        let text: Vec<String> = page.lines.iter().map(PageLine::text).collect();
        assert!(
            text.iter()
                .any(|l| l.contains("Couldn't load the log: this log has expired. r tries again.")),
            "{text:#?}"
        );
    }

    /// A log's markers show as GitHub shows them, and text that only looks
    /// like an escape (a literal `^[`) stays as it is.
    #[test]
    fn log_markers_show_as_styles() {
        let seg = |line: &str| {
            let s = log_seg(&strip_escapes(line));
            (s.text().to_owned(), s.role)
        };
        assert_eq!(seg("##[notice]Deployed"), ("Deployed".into(), Role::Strong));
        assert_eq!(
            seg("##[debug]Loading env"),
            ("Loading env".into(), Role::Meta)
        );
        assert_eq!(
            seg("[command]/usr/bin/git version"),
            (
                "/usr/bin/git version".into(),
                Role::Syntax(Syntax::Function)
            )
        );
        assert_eq!(
            seg("##[start-action display=Parse toolchain version;id=__x.parse]"),
            ("▾ Parse toolchain version".into(), Role::Strong)
        );
        assert_eq!(
            seg("##[end-action id=__x.parse;outcome=success]"),
            (String::new(), Role::Meta)
        );
        assert_eq!(
            seg("echo ^[[31m red"),
            ("echo ^[[31m red".into(), Role::Body)
        );
        assert_eq!(seg("\u{1b}[31mred\u{1b}[0m"), ("red".into(), Role::Body));
    }

    /// A log cut to its last 2 MB keeps GitHub's line numbers: a link to
    /// a line shows that line, and one into the cut part says so.
    #[test]
    fn a_cut_log_keeps_its_line_numbers() {
        let job = one_step_job(CheckOutcome::Failure);
        let full: String = (1..=300_000)
            .map(|n| format!("2026-10-05T17:42:01.0000000Z line {n}\n"))
            .collect();
        let (cut, text) = crate::api::browse::log::cut(&full);
        assert!(!cut.is_empty());
        let log = JobLog {
            cut: cut.clone(),
            text: text.into(),
            running: false,
        };
        let first_kept = cut.lines().count() + 1;
        let shown = |line: usize| {
            let mut page = Page::new(100);
            let at = at(Some((1, u32::try_from(line).unwrap())), "");
            log_page(&mut page, &job, Fetched::ready(&log), at);
            let jump = page
                .jump
                .and_then(|j| page.lines.get(j))
                .map(PageLine::text);
            jump.unwrap_or_default()
        };
        let line = first_kept + 10;
        let at = shown(line);
        assert!(
            at.starts_with(&format!("{line}  ")) && at.ends_with(&format!("line {line}")),
            "{at}"
        );
        assert!(shown(5).contains("Line 5 is in the part of the log too long to load"));
    }

    /// A running job's log, and one that couldn't load, say so rather
    /// than loading forever.
    #[test]
    fn a_log_that_isnt_there_says_why() {
        let text = |job: &Job, log: Fetched<'_, JobLog>| {
            let mut page = Page::new(120);
            log_page(&mut page, job, log, at(Some((1, 1)), ""));
            page.lines
                .iter()
                .map(PageLine::text)
                .collect::<Vec<_>>()
                .join("\n")
        };
        let running = JobLog {
            running: true,
            ..JobLog::default()
        };
        let page = text(
            &one_step_job(CheckOutcome::Pending),
            Fetched::ready(&running),
        );
        assert!(page.contains("when the job ends"), "{page}");
        let page = text(
            &one_step_job(CheckOutcome::Failure),
            Fetched::failed("this log has expired"),
        );
        assert!(
            page.contains("Couldn't load the log: this log has expired"),
            "{page}"
        );
    }

    /// A composite action's inner `Run` lines stay in its step, even in
    /// the second the next step starts.
    #[test]
    fn inner_run_lines_stay_in_their_step() {
        let step = |number, name: &str, start: &str, end: &str| crate::api::browse::Step {
            number,
            name: name.into(),
            outcome: CheckOutcome::Success,
            started_at: Some(format!("2026-10-05T17:42:{start}Z")),
            completed_at: Some(format!("2026-10-05T17:42:{end}Z")),
        };
        let job = job_of(vec![
            step(1, "Run ./build", "00", "05"),
            step(2, "Run ./test", "05", "09"),
        ]);
        let log = [
            "2026-10-05T17:42:00.1Z ##[group]Run ./build",
            "2026-10-05T17:42:05.1Z ##[group]Run inner",
            "2026-10-05T17:42:05.2Z still building",
            "2026-10-05T17:42:05.5Z ##[group]Run ./test",
        ]
        .join("\n");
        let lines = step_lines(&job, "", &log);
        let of = |n| lines_of(&lines, n).len();
        assert_eq!((of(1), of(2)), (3, 1));
    }

    #[test]
    fn escapes_are_stripped_from_logs() {
        assert_eq!(strip_escapes("\u{1b}[36;1mcd src\u{1b}[0m"), "cd src");
        assert_eq!(strip_escapes("a\u{1b}]8;;https://x\u{7}b"), "ab");
        assert_eq!(strip_escapes("plain [36m"), "plain [36m");
        assert_eq!(strip_escapes("a\u{1b}(Bb\u{1b}7c"), "abc");
    }
}
