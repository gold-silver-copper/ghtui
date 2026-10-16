//! Keys on pages: moving, scrolling, following links, cancelling.

use super::*;
use crate::route::{code, runs};

/// Actions on a page screen. Those that don't work on pages say where
/// they do.
#[must_use]
pub fn page_action(state: &mut State, action: Action) -> Vec<Cmd> {
    let height = state.page_height();
    let Screen::Page(p) = state.screen_mut() else {
        return Vec::new();
    };
    let page = height.saturating_sub(2).max(1).cast_signed();
    match action {
        Action::Down => step(p, height, true),
        Action::Up => step(p, height, false),
        Action::PageDown => scroll_keep(p, page, height),
        Action::PageUp => scroll_keep(p, -page, height),
        // The first or last row on screen there: a note (an empty Home
        // section's) isn't one.
        Action::Top => {
            scroll_by(p, isize::MIN, height);
            let n = p.page.items.len();
            p.selected = (0..n).find(|&i| is_row(p, i) && visible(p, i, height));
        }
        Action::Bottom => {
            scroll_by(p, isize::MAX, height);
            let n = p.page.items.len();
            p.selected = (0..n)
                .rev()
                .find(|&i| is_row(p, i) && visible(p, i, height));
        }
        // A file's line is this page: say what you can do with it.
        Action::Open
            if matches!(p.route, Route::Code(code::Blob { .. })) && p.selected.is_some() =>
        {
            let line = p.selected.and_then(|i| p.line_of(i)).unwrap_or(1);
            let (copy, blame) = (
                state.first_key(Action::Copy),
                state.first_key(Action::Blame),
            );
            state.info(format!(
                "Line {line} · {copy} copies its link · {blame} blames it"
            ));
        }
        // So is a line of a job's log.
        Action::Open
            if matches!(p.route, Route::Runs(runs::Job { .. })) && p.code_line().is_some() =>
        {
            let at = match p.selected_link().and_then(Link::url).map(Dest::from_url) {
                Some(Dest {
                    target:
                        Target::Page(Route::Runs(runs::Job {
                            step: Some((step, line)),
                            ..
                        })),
                    ..
                }) => format!("Line {line} of step {step}"),
                _ => "A line of the log".to_owned(),
            };
            let (copy, word) = (
                state.first_key(Action::Copy),
                state.first_key(Action::SearchWord),
            );
            state.info(format!(
                "{at} · {copy} copies its link · {word} searches the word under the cursor"
            ));
        }
        Action::Open => {
            return match p.selected_link().cloned() {
                Some(link) => state.follow(&link),
                None => {
                    let (down, hints) = (
                        state.first_key(Action::Down),
                        state.first_key(Action::Hints),
                    );
                    state.just_say(format!(
                        "Nothing selected: {down} selects a row, {hints} picks any link"
                    ))
                }
            };
        }
        Action::Hints | Action::HintsBrowser => {
            start_hints(state, action == Action::HintsBrowser);
        }
        Action::SearchNext | Action::SearchPrev => {
            let next = action == Action::SearchNext;
            match step_mark(p, next) {
                Some((n, of)) => state.info(format!("Match {n} of {of}")),
                None if p.page.marks.is_empty() => {
                    let query = match &p.route {
                        Route::Runs(runs::Job { query, .. }) => Some(query.clone()),
                        _ => p.find.clone(),
                    };
                    let search = state.first_key(Action::Search);
                    state.info(match query.filter(|q| !q.is_empty()) {
                        Some(q) => format!("No matches for “{q}”"),
                        None => format!("No search yet: {search} searches here"),
                    });
                }
                None => state.info(if next {
                    "No later match"
                } else {
                    "No earlier match"
                }),
            }
        }
        Action::NextSection | Action::PrevSection => {
            let at = (p.selected.and_then(|i| p.page.items.get(i))).map_or(p.scroll, |it| it.start);
            let sections = p.page.sections();
            // Back is to the section before the one the selection is in.
            let current = sections.iter().rev().find(|&&s| s <= at).copied();
            let to = if action == Action::NextSection {
                sections.iter().find(|&&s| s > at)
            } else {
                sections
                    .iter()
                    .rev()
                    .find(|&&s| current.is_none_or(|c| s < c))
            };
            let before = (p.scroll, p.selected);
            if let Some(line) = to.copied() {
                p.fresh = false;
                p.scroll = line.saturating_sub(MARGIN).min(max_scroll(p, height));
                // Its first row, if it has one before the next section.
                let end = sections.iter().find(|&&s| s > line).copied();
                let n = p.page.items.len();
                p.selected = (0..n).find(|&i| {
                    let start = p.page.items.get(i).map_or(0, |it| it.start);
                    start >= line
                        && end.is_none_or(|e| start < e)
                        && is_row(p, i)
                        && visible(p, i, height)
                });
            }
            if (p.scroll, p.selected) == before {
                state.info(if action == Action::NextSection {
                    "No more sections below"
                } else {
                    "No more sections above"
                });
            }
        }
        Action::ScrollLeft | Action::ScrollRight => {
            let (before, wide) = (p.shift, p.wide);
            scroll_view(p, action);
            if p.shift == before {
                state.info(match action {
                    Action::ScrollLeft => "Already at the left edge",
                    _ if wide == 0 => "Nothing here is wider than the screen",
                    _ => "Already at the right edge",
                });
            }
        }
        Action::WordNext | Action::WordPrev => {
            let forward = action == Action::WordNext;
            let Some((_, code, room)) = p.code_line() else {
                let down = state.first_key(Action::Down);
                return state.just_say(format!(
                    "Words are a line of code's: {down} moves to a line of a file or a log"
                ));
            };
            match text::step_word(&code, p.word, forward) {
                Some(word) => {
                    p.word = Some(word.cols.start);
                    p.shift = text::reveal(p.shift, &word.cols, room);
                }
                None if forward => state.info("No more words on this line"),
                None => state.info("No earlier word on this line"),
            }
        }
        Action::Blame => {
            let line = p.selected.and_then(|i| p.line_of(i));
            let lines = line.map(|n| (n, n));
            let route = match p.route.clone() {
                Route::Code(code::Blob {
                    repo, rev, path, ..
                }) => Route::Code(code::Blame {
                    repo,
                    rev,
                    path,
                    lines,
                }),
                Route::Code(code::Blame {
                    repo, rev, path, ..
                }) => Route::Code(code::Blob {
                    repo,
                    rev,
                    path,
                    lines,
                }),
                _ => {
                    return state.just_say(action.not_here());
                }
            };
            return state.push(route);
        }
        Action::VisualLines if matches!(p.route, Route::Code(code::Blob { .. })) => {
            let (range, what) = match p.range {
                Some(_) => (None, "Selection cleared"),
                None => (p.selected, "Selecting lines · l copies their link"),
            };
            p.range = range;
            p.fresh = false;
            state.info(what);
        }
        Action::NextCommit | Action::PrevCommit => {
            let Route::Code(code::Commit {
                repo,
                oid,
                pr: Some(number),
            }) = p.route.clone()
            else {
                return state.just_say(
                    "Next and previous commits are a pull request's: open one from its Commits tab",
                );
            };
            let at = state.pr_commit_at(&repo, number, &oid);
            let to = match &at {
                Some(at) if action == Action::NextCommit => at.next.clone(),
                Some(at) => at.previous.clone(),
                None => None,
            };
            return match (to, at) {
                (Some(url), _) => state.go(Dest::from_url(&url)),
                (None, Some(_)) => state.just_say(if action == Action::NextCommit {
                    "This is the newest commit"
                } else {
                    "No earlier commit here"
                }),
                (None, None) => state.just_say("The pull request's commits haven't loaded yet"),
            };
        }
        Action::Star => return star(state),
        Action::Delete | Action::UndoDelete => return crate::home::act(state, action),
        Action::Comment => return comment_with(state, "", None),
        Action::QuoteReply => return quote_reply(state),
        Action::Branch => return state.open_finder(true),
        Action::ToggleState => return cycle_state(state),
        Action::Sort => return cycle_sort(state),
        // A long review in a conversation.
        Action::FoldToggle => match crate::talk::fold_key(state) {
            Some(cmds) => return cmds,
            None => state.info("Select a long review or a hidden comment to fold or unfold it"),
        },
        _ => state.info(action.not_here()),
    }
    Vec::new()
}

/// Columns `h`/`l` scroll code sideways.
pub(super) const SHIFT: usize = 8;

/// `←` `→`: code moves sideways, the selection stays.
pub(super) fn scroll_view(p: &mut PageScreen, action: Action) {
    p.shift = match action {
        Action::ScrollLeft => p.shift.saturating_sub(SHIFT),
        _ => p.shift.saturating_add(SHIFT).min(p.wide),
    };
}

pub(super) fn max_scroll(p: &PageScreen, height: usize) -> usize {
    p.page.height().saturating_sub(height)
}

/// Line `n` of a file (in the diff, of the file the cursor is in), else
/// row `n`.
#[must_use]
pub fn go_to_line(state: &mut State, n: usize) -> Vec<Cmd> {
    let height = state.page_height();
    // In the diff: that line of the file the cursor is in.
    if let Some((screen, diff)) = state.diff_parts() {
        // In its file tree, that row.
        if screen.focus == Pane::Tree {
            crate::diff_screen::select_tree_row(screen, diff, n.saturating_sub(1));
            return Vec::new();
        }
        let line = u32::try_from(n).unwrap_or(u32::MAX);
        let line = crate::diff::anchor::LinePos {
            side: Side::Right,
            line,
        };
        match diff.doc.line_row(screen.cursor.file, line) {
            Some(pos) => screen.cursor = pos,
            None => state.info(format!("There's no line {n} here")),
        }
        return Vec::new();
    }
    let Screen::Page(p) = state.screen_mut() else {
        return Vec::new();
    };
    let item = match p.page.numbered.get(n.saturating_sub(1)) {
        Some(&item) => Some(item),
        None if p.page.numbered.is_empty() => n.checked_sub(1).filter(|&i| i < p.page.items.len()),
        None => None,
    };
    match item {
        Some(i) => {
            p.fresh = false;
            p.selected = Some(i);
            if let Some(it) = p.page.items.get(i) {
                p.scroll = it
                    .start
                    .saturating_sub(height / 2)
                    .min(max_scroll(p, height));
            }
        }
        None => state.info(format!("There's no line {n} here")),
    }
    Vec::new()
}

/// The link under the cursor (a web address in the word under it,
/// else the selected row's), in the browser.
#[must_use]
pub fn open_link(state: &mut State) -> Vec<Cmd> {
    match state.link_under_cursor() {
        Some(url) => state.go(Target::External(url).into()),
        None => state.just_say("No link under the cursor"),
    }
}

/// Esc on a page: ends a search on it or a selection of lines; with
/// neither, goes back.
#[must_use]
pub fn cancel(state: &mut State) -> Vec<Cmd> {
    if let Screen::Page(p) = state.screen_mut() {
        if p.range.take().is_some() {
            state.info("Selection cleared");
            return Vec::new();
        }
        if p.find.is_some() {
            find(state, "", true);
            return Vec::new();
        }
    }
    state.back()
}
