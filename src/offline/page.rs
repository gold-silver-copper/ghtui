//! The Offline page, and the queued writes shown on the pages they were
//! written on.

use crate::keymap::Action;
use crate::route::Route;
use crate::state::State;
use crate::theme::Bg;
use crate::ui::page::{Frame, Link, Page, Role, Seg};

use super::keep::Kept;
use super::queue::{Entry, Status};

/// How many lines of a queued write's text are shown.
const TEXT_LINES: usize = 4;

/// The Offline page: whether GitHub can be reached, the queued writes,
/// and the repositories kept to read offline, with the limits.
pub fn build(state: &State, page: &mut Page, now: u64) {
    let off = &state.offline;
    let refresh = state.first_key(Action::Refresh);
    let said = match off.since {
        Some(since) => format!(
            "GitHub can't be reached, since {}. Pages show what ghtui kept: the cache's copy, and code from the clones. Writes that can wait are queued below, and sent in order when it's back. ghtui asks every few seconds; {refresh} asks now.",
            crate::ui::time::ago(since, now)
        ),
        None => "GitHub can be reached. Writes it couldn't be reached for wait here, and are sent in order once it's back.".to_owned(),
    };
    page.wrapped(vec![Seg::new(said, Role::Body)], 0, Frame::None);

    let title = vec![Seg::new(
        format!("Queued for GitHub  {}", off.queue.len()),
        Role::Strong,
    )];
    page.box_top(title, Vec::new());
    if off.queue.is_empty() {
        let none = "Nothing's waiting. A comment, a review, a reaction, labels or closing, written while GitHub can't be reached, waits here.";
        page.wrapped(vec![Seg::new(none, Role::Meta)], 0, Frame::Body);
    }
    for (i, entry) in off.queue.iter().enumerate() {
        if i > 0 {
            page.box_rule();
        }
        entry_rows(page, entry, now, true);
    }
    page.box_bottom();
    if !off.queue.is_empty() {
        let keys = format!(
            "On a write: {} sends a refused one again · {} copies its text · {} throws it away · ↵ opens its page",
            state.first_key(Action::Rerun),
            state.first_key(Action::Copy),
            state.first_key(Action::Delete),
        );
        page.wrapped(vec![Seg::new(keys, Role::Meta)], 0, Frame::None);
    }

    let title = vec![Seg::new(
        format!("Kept to read offline  {}", off.kept.len()),
        Role::Strong,
    )];
    page.box_top(title, Vec::new());
    if off.kept.is_empty() {
        let keep = state.first_key(Action::KeepOffline);
        let none = format!(
            "None yet. On a repository's page, {keep} keeps its issues, pull requests and discussions fetched ahead, and its code in a clone."
        );
        page.wrapped(vec![Seg::new(none, Role::Meta)], 0, Frame::Body);
    }
    for (i, kept) in off.kept.iter().enumerate() {
        if i > 0 {
            page.box_rule();
        }
        kept_rows(page, kept, now);
    }
    page.box_bottom();
    let cache = off
        .cache
        .map(|c| format!(" The cache takes {} now.", crate::ui::text::size(c)))
        .unwrap_or_default();
    let limits = format!(
        "Kept: {}, refreshed every half hour while GitHub can be reached ([offline] in config.toml sets items, days and disk).{cache}",
        off.limits.said()
    );
    page.wrapped(vec![Seg::new(limits, Role::Meta)], 0, Frame::None);
}

/// The writes queued on `route`'s page, in a box at its top.
pub fn queued_here(state: &State, route: &Route, page: &mut Page, now: u64) {
    if *route == Route::Offline {
        return;
    }
    let url = route.url();
    let here: Vec<&Entry> = (state.offline.queue.iter())
        .filter(|e| e.page == url)
        .collect();
    if here.is_empty() {
        return;
    }
    let right = match state.offline.since {
        Some(_) => "sent when GitHub is back",
        None => "",
    };
    page.box_top(
        vec![Seg::new(
            format!("Queued for GitHub  {}", here.len()),
            Role::Strong,
        )],
        vec![Seg::new(right, Role::Meta)],
    );
    for (i, entry) in here.into_iter().enumerate() {
        if i > 0 {
            page.box_rule();
        }
        entry_rows(page, entry, now, false);
    }
    page.box_bottom();
}

/// A queued write's rows, as one item: what it is and where it stands,
/// GitHub's reason when it refused it, and the start of its text.
fn entry_rows(page: &mut Page, entry: &Entry, now: u64, whole: bool) {
    let start = page.lines.len();
    let link = page.link(Link::Queued(entry.id));
    let (chip, bg) = match &entry.status {
        Status::Queued => ("Queued", Bg::SecondaryContainer),
        Status::Sending => ("Sending", Bg::PrimaryContainer),
        Status::Refused(_) => ("Refused", Bg::ErrorContainer),
        Status::Unsure(_) => ("Not known if sent", Bg::TertiaryContainer),
    };
    let mut head = vec![
        Seg::linked(format!(" {chip} "), Role::Chip(bg), link),
        Seg::new(" ", Role::Body),
        Seg::linked(entry.write.what(), Role::Strong, link),
    ];
    if whole {
        head.push(Seg::new(format!(" on {}", entry.title), Role::Meta));
    }
    head.push(Seg::new(
        format!(" · {}", crate::ui::time::ago(entry.at, now)),
        Role::Meta,
    ));
    page.wrapped(head, 0, Frame::Body);
    match &entry.status {
        Status::Refused(why) => {
            let why = format!("GitHub said: {why}");
            page.wrapped(vec![Seg::new(why, Role::Error)], 2, Frame::Body);
        }
        Status::Unsure(why) => {
            let why = format!(
                "It may have reached GitHub ({why}): check, then send it again or throw it away."
            );
            page.wrapped(vec![Seg::new(why, Role::Error)], 2, Frame::Body);
        }
        Status::Queued | Status::Sending => {}
    }
    if let Some(text) = entry.write.text() {
        let lines: Vec<&str> = text.lines().collect();
        for line in lines.iter().take(TEXT_LINES) {
            page.wrapped(vec![Seg::new(*line, Role::Body)], 2, Frame::Body);
        }
        if lines.len() > TEXT_LINES {
            let more = format!("… {} more lines", lines.len() - TEXT_LINES);
            page.wrapped(vec![Seg::new(more, Role::Meta)], 2, Frame::Body);
        }
    }
    page.item(start, link);
}

/// A kept repository's rows: what's kept, how big its clone is, when it
/// was refreshed, and what stopped it short.
fn kept_rows(page: &mut Page, kept: &Kept, now: u64) {
    let start = page.lines.len();
    let link = page.link(crate::ui::pages::url::repo(&kept.repo));
    let mut counts = vec![
        format!("{} issues", kept.issues),
        format!("{} pull requests", kept.pulls),
        format!("{} discussions", kept.discussions),
    ];
    if let Some(clone) = kept.clone {
        counts.push(format!("clone {}", crate::ui::text::size(clone)));
    }
    let when = match (kept.busy, kept.refreshed) {
        (true, _) => "fetching now".to_owned(),
        (false, Some(at)) => format!("refreshed {}", crate::ui::time::ago(at, now)),
        (false, None) => "not fetched yet".to_owned(),
    };
    page.wrapped(
        vec![
            Seg::linked(kept.repo.to_string(), Role::Link, link),
            Seg::new(format!("  {} · {when}", counts.join(" · ")), Role::Meta),
        ],
        0,
        Frame::Body,
    );
    if let Some(problem) = &kept.problem {
        page.wrapped(vec![Seg::new(problem.clone(), Role::Error)], 2, Frame::Body);
    }
    page.item(start, link);
}
