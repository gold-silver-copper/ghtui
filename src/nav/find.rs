//! Finding text on the page, and marks.

use super::*;
use crate::route::{people, runs};

/// Whether `/` here is the page's own search box: a list's filter, a
/// job's log search, a profile's repositories.
pub fn searches_itself(state: &State) -> bool {
    match state.route() {
        Some(
            Route::Runs(runs::Job { .. })
            | Route::People(people::User {
                tab: ProfileTab::Repositories(_),
                ..
            }),
        ) => true,
        Some(route) => route.family().query().is_some(),
        None => false,
    }
}

/// Searches the page on screen for `query` (empty clears it), going to
/// the first match from the top of the view.
pub fn find(state: &mut State, query: &str, forward: bool) {
    let Screen::Page(p) = state.screen_mut() else {
        return;
    };
    p.find = Some(query.to_owned()).filter(|q| !q.is_empty());
    (p.built, p.mark) = (None, None);
    state.sync_page();
    let Screen::Page(p) = state.screen_mut() else {
        return;
    };
    // The first match from the top of the view down, or (`?`) up.
    let first = if forward {
        p.page.marks.iter().position(|&line| line >= p.scroll)
    } else {
        p.page.marks.iter().rposition(|&line| line < p.scroll)
    };
    let first = first.or_else(|| (!p.page.marks.is_empty()).then_some(0));
    let text = match first.and_then(|at| go_to_mark(p, at)) {
        _ if query.is_empty() => "Cleared the search".to_owned(),
        Some((n, of)) => format!("Match {n} of {of}"),
        None => format!("No matches for “{query}”"),
    };
    state.info(text);
}

/// Searches the word under the cursor as `/` would, as a whole word,
/// going to its next match below the cursor's line.
#[must_use]
pub fn search_word(state: &mut State) -> Vec<Cmd> {
    if let Some((screen, diff)) = state.diff_parts() {
        let code = diff.doc.code_at(screen.cursor, screen.half).unwrap_or("");
        let words = text::words(code);
        let word = text::word_at(&words, screen.word.unwrap_or(0));
        match word.and_then(|w| code.get(w.bytes.clone())) {
            Some(word) => {
                // The word cursor goes with the match.
                screen.word = Some(screen.word.unwrap_or(0));
                let found = diff_screen::search(screen, diff, text::whole_word(word), true);
                state.notices.say(found);
            }
            None => state.info("No word under the cursor: this row has no code"),
        }
        return Vec::new();
    }
    let Screen::Page(p) = state.screen_mut() else {
        return Vec::new();
    };
    let Some(word) = p.word_text().map(|w| text::whole_word(&w)) else {
        let down = state.first_key(Action::Down);
        return state.just_say(format!(
            "No word under the cursor: {down} moves to a line of a file or a log"
        ));
    };
    // The selection stays on its line as the search rebuilds the page,
    // and the word cursor goes with the match.
    p.fresh = false;
    p.word = Some(p.word.unwrap_or(0));
    if matches!(p.route, Route::Runs(runs::Job { .. })) {
        search_log(state, &word);
    } else {
        p.find = Some(word);
        (p.built, p.mark) = (None, None);
        state.sync_page();
    }
    let Screen::Page(p) = state.screen_mut() else {
        return Vec::new();
    };
    // From the cursor's line, around to the other end.
    let line = (p.selected.and_then(|i| p.page.items.get(i))).map_or(0, |it| it.start);
    let marks = &p.page.marks;
    let next = (marks.iter().position(|&m| m > line)).or_else(|| (!marks.is_empty()).then_some(0));
    if let Some((n, of)) = next.and_then(|n| go_to_mark(p, n)) {
        state.info(format!("Match {n} of {of}"));
    }
    Vec::new()
}

/// Goes to the next (or previous) search match on the page: which
/// one it is, of how many. Before any, the one the page opened at is.
pub(super) fn step_mark(p: &mut PageScreen, next: bool) -> Option<(usize, usize)> {
    let n = match (p.mark_at(), next) {
        (None, true) => 0,
        (None, false) => return None,
        (Some(at), true) => at + 1,
        (Some(at), false) => at.checked_sub(1)?,
    };
    go_to_mark(p, n)
}

/// Scrolls to search match `n` and selects its row; in code, the word
/// cursor goes on the match, as vim's does: which one it is, of how many.
pub(super) fn go_to_mark(p: &mut PageScreen, n: usize) -> Option<(usize, usize)> {
    let line = *p.page.marks.get(n)?;
    p.scroll = line.saturating_sub(MARGIN);
    (p.mark, p.fresh) = (Some(n), false);
    // The match's row is selected, so ↵ opens what was found; in code,
    // the word cursor goes on the match too.
    if let Some(i) = p.page.item_at(line) {
        p.selected = Some(i);
    }
    let code = p.page.lines.get(line).filter(|l| l.code().is_some());
    if let Some(col) = code.and_then(page::PageLine::first_match) {
        p.word = Some(col);
    }
    Some((n + 1, p.page.marks.len()))
}
