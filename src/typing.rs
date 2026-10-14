//! Typing into ghtui's boxes: a paste arrives whole, as text rather than
//! keys, and the comment box completes `@` mentions and `#` references.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui_textarea::TextArea;

use crate::api::ApiError;
use crate::api::browse::{IssueDetail, PrActivity};
use crate::api::mention::Mentionable;
use crate::api::model::{PrRef, RepoId};
use crate::data::DataKey;
use crate::diff_screen::DiffOf;
use crate::picker;
use crate::route::Route;
use crate::state::{Api, Cmd, Overlay, Screen, State};

/// Choices the completion list shows at most.
pub const SHOWN: usize = 6;

/// A paste: into the comment box or the review summary as it is, newlines
/// and all; into a one-line box (a search, a filter, the palette, a
/// picker) as one line.
#[must_use]
pub fn on_paste(state: &mut State, text: &str) -> Vec<Cmd> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let line = || {
        let lines: Vec<&str> = text.split('\n').filter(|l| !l.trim().is_empty()).collect();
        lines.join(" ")
    };
    match &mut state.overlay {
        Some(Overlay::Compose(c)) => {
            if !c.sending {
                (c.discard_armed, c.error) = (None, None);
                c.input.insert_str(&text);
            }
        }
        Some(Overlay::Submit(d)) => {
            if !d.sending {
                (d.discard_armed, d.error) = (None, None);
                d.input.insert_str(&text);
            }
        }
        Some(Overlay::Find(input)) => {
            input.insert_str(line());
        }
        Some(Overlay::Search(sb)) => {
            sb.input.insert_str(line());
            return crate::nav::search_box_changed(sb);
        }
        Some(Overlay::Picker(p)) if !matches!(p.kind, picker::Kind::Commits { .. }) => {
            p.input.insert_str(line());
            p.selected = 0;
        }
        Some(Overlay::Menu(menu)) if menu.filter.is_some() => {
            if let Some(filter) = &mut menu.filter {
                filter.push_str(&line());
            }
            menu.reselect();
        }
        _ => state.info("Nothing to paste into here: a paste goes into a comment or a box"),
    }
    Vec::new()
}

/// The comment box's completion list: which choice is selected, and where
/// `esc` closed it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Completing {
    pub selected: usize,
    /// The `@` or `#` (row and column) whose list `esc` closed: it stays
    /// closed while that's what's typed.
    dismissed: Option<(usize, usize)>,
}

/// A choice the list offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// As the list shows it.
    pub label: String,
    /// What replaces what's typed.
    pub insert: String,
}

/// An `@` or `#` being typed: where it is (row, column in characters), and
/// what follows it up to the cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Typed {
    sigil: char,
    at: (usize, usize),
    query: String,
}

/// What's typed before the cursor, if it's an `@` or `#` and the word
/// after it: one at the start of a word, so `a@b.c` and `C#` don't count.
fn typed(input: &TextArea<'_>) -> Option<Typed> {
    let cursor = input.cursor();
    let (row, col) = (cursor.0, cursor.1);
    let before: Vec<char> = input.lines().get(row)?.chars().take(col).collect();
    let word = |c: &char| c.is_alphanumeric() || matches!(c, '-' | '_');
    let query_len = before.iter().rev().take_while(|c| word(c)).count();
    let at = col.checked_sub(query_len.checked_add(1)?)?;
    let sigil = *before.get(at)?;
    let starts_word = at
        .checked_sub(1)
        .and_then(|i| before.get(i))
        .is_none_or(|c| !word(c) && !matches!(c, '@' | '#' | '/' | '.'));
    if !matches!(sigil, '@' | '#') || !starts_word {
        return None;
    }
    let query = before.get(at.saturating_add(1)..)?.iter().collect();
    Some(Typed {
        sigil,
        at: (row, at),
        query,
    })
}

/// The repository a comment written here is in.
fn repo_here(state: &State) -> Option<RepoId> {
    match state.screen() {
        Screen::Diff(screen) => Some(screen.of.repo().clone()),
        Screen::Page(p) => p.route.repo().cloned(),
    }
}

/// Who has taken part in the pull request or issue on screen, as far as
/// what's loaded shows: its author first.
fn participants(state: &State) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let pr = match (state.screen(), state.route()) {
        (Screen::Diff(screen), _) => screen.of.pr().cloned(),
        (_, Some(Route::Pr { pr, .. })) => Some(pr.clone()),
        _ => None,
    };
    if let Some(pr) = &pr {
        out.extend(pr_participants(state, pr));
    }
    if let Some(Route::Issue { repo, number }) = state.route() {
        let issue =
            state.picked::<Option<Box<IssueDetail>>>(&DataKey::Issue(repo.clone(), *number));
        if let Some(Some(issue)) = issue {
            out.push(issue.author.clone());
            out.extend(issue.comments().map(|c| c.author.clone()));
            out.extend(issue.assignees.iter().cloned());
        }
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|login| !login.is_empty() && seen.insert(login.to_lowercase()));
    out
}

fn pr_participants(state: &State, pr: &PrRef) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(d) = state.pr(pr) {
        out.push(d.author.clone());
    }
    if let Some(a) = state.picked::<PrActivity>(&DataKey::PrActivity(pr.clone())) {
        out.extend(a.comments().map(|c| c.author.clone()));
        out.extend(a.reviews().map(|r| r.author.clone()));
        out.extend(a.requested.iter().filter_map(|r| match &r.who {
            crate::api::browse::Reviewer::User(login) => Some(login.clone()),
            crate::api::browse::Reviewer::Team(_) => None,
        }));
    }
    if let Some(diff) = state.diffs.get(&DiffOf::Pr(pr.clone())) {
        let threads = diff.inputs().thread_list();
        out.extend(
            threads
                .iter()
                .flat_map(|t| t.comments.iter().map(|c| c.author.clone())),
        );
    }
    out
}

/// The comment box's choices for what's typed at its cursor, if it's an
/// `@` or `#` whose list wasn't closed: who to mention (the people taking
/// part first, then the repository's), or which issue or pull request.
pub fn completions(state: &State) -> Option<Vec<Choice>> {
    let Some(Overlay::Compose(compose)) = &state.overlay else {
        return None;
    };
    let typed = typed(&compose.input)?;
    if compose.completing.dismissed == Some(typed.at) {
        return None;
    }
    let known = repo_here(state).and_then(|repo| state.mentionable.get(&repo));
    let query = typed.query.to_lowercase();
    let choices: Vec<Choice> = if typed.sigil == '@' {
        let mut people = participants(state);
        for login in known.iter().flat_map(|m| &m.users) {
            if !people.iter().any(|p| p.eq_ignore_ascii_case(login)) {
                people.push(login.clone());
            }
        }
        // Logins that start with what's typed, then those that have it.
        let (mut first, rest): (Vec<String>, Vec<String>) = people
            .into_iter()
            .filter(|login| login.to_lowercase().contains(&query))
            .partition(|login| login.to_lowercase().starts_with(&query));
        first.extend(rest);
        first
            .into_iter()
            .map(|login| Choice {
                label: format!("@{login}"),
                insert: format!("@{login} "),
            })
            .collect()
    } else {
        let issues = known.iter().flat_map(|m| &m.issues);
        issues
            .filter(|(number, title)| {
                number.to_string().starts_with(&query) || title.to_lowercase().contains(&query)
            })
            .map(|(number, title)| Choice {
                label: format!("#{number} {title}"),
                insert: format!("#{number} "),
            })
            .collect()
    };
    (!choices.is_empty()).then(|| choices.into_iter().take(SHOWN).collect())
}

/// A key in the comment box while its completion list shows: `⇥` or `↵`
/// takes the choice, `↑` `↓` (or ctrl-p, ctrl-n) choose, `esc` closes the
/// list. Any other key is the box's (`None`).
pub fn on_compose_key(state: &mut State, key: KeyEvent) -> Option<Vec<Cmd>> {
    let choices = completions(state)?;
    let Some(Overlay::Compose(compose)) = &mut state.overlay else {
        return None;
    };
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let last = choices.len().saturating_sub(1);
    let completing = &mut compose.completing;
    match key.code {
        KeyCode::Down => completing.selected = (completing.selected + 1).min(last),
        KeyCode::Char('n') if ctrl => completing.selected = (completing.selected + 1).min(last),
        KeyCode::Up => completing.selected = completing.selected.saturating_sub(1),
        KeyCode::Char('p') if ctrl => completing.selected = completing.selected.saturating_sub(1),
        KeyCode::Esc => completing.dismissed = typed(&compose.input).map(|t| t.at),
        KeyCode::Tab | KeyCode::Enter if key.modifiers.is_empty() => {
            let choice = choices.get(completing.selected.min(last))?;
            let typed = typed(&compose.input)?;
            // The sigil and what follows it give way to the choice.
            for _ in 0..=typed.query.chars().count() {
                compose.input.delete_char();
            }
            compose.input.insert_str(&choice.insert);
            compose.completing = Completing::default();
        }
        _ => return None,
    }
    Some(Vec::new())
}

/// Asks GitHub who and what the repository on screen can mention, once a
/// session, when a comment box opens.
#[must_use]
pub fn fetch_mentionable(state: &mut State) -> Vec<Cmd> {
    let Some(repo) = repo_here(state) else {
        return Vec::new();
    };
    if state.mentionable.contains_key(&repo) {
        return Vec::new();
    }
    state
        .mentionable
        .insert(repo.clone(), Mentionable::default());
    vec![Cmd::Api(Api::FetchMentionable(repo))]
}

/// GitHub's answer to [`fetch_mentionable`].
pub fn on_mentionable(state: &mut State, repo: RepoId, result: Result<Mentionable, ApiError>) {
    match result {
        Ok(found) => {
            state.mentionable.insert(repo, found);
        }
        // Asked again with the next comment box.
        Err(err) => {
            tracing::warn!(%repo, %err, "couldn't read who to mention");
            state.mentionable.remove(&repo);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Option<(char, (usize, usize), String)> {
        let mut input = TextArea::default();
        input.insert_str(text);
        typed(&input).map(|t| (t.sigil, t.at, t.query))
    }

    #[test]
    fn only_a_word_that_starts_with_the_sigil_completes() {
        assert_eq!(at("cc @oc"), Some(('@', (0, 3), "oc".into())));
        assert_eq!(at("@"), Some(('@', (0, 0), String::new())));
        assert_eq!(at("see\n(#12"), Some(('#', (1, 1), "12".into())));
        assert_eq!(at("mail a@b"), None);
        assert_eq!(at("C#"), None);
        assert_eq!(at("@octo cat"), None);
    }
}
