//! Saved replies in the comment composer: `ctrl-g` lists yours (as
//! GitHub's `ctrl-.` does), typing narrows them, and `↵` puts the one
//! chosen where the cursor is. GitHub's API reads them but can't make,
//! change or delete them, so the list's last row says where that's done.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::api::model::Capped;
use crate::api::replies::SavedReply;
use crate::data::Picked;
use crate::state::{Cmd, Overlay, Remote, State};

use crate::data::DataKey;

/// The list of saved replies, open in the composer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Choosing {
    /// What's typed to narrow them.
    pub query: String,
    pub selected: usize,
}

/// A row of the list: what it says, and the reply it inserts.
struct Row {
    label: String,
    insert: Option<String>,
}

/// The list's rows, for what's typed: the replies whose title or text
/// has it, else why there are none; then where they're made.
fn rows(state: &State, query: &str) -> Vec<Row> {
    let key = DataKey::SavedReplies;
    #[expect(clippy::disallowed_methods, reason = "said as the list's row")]
    let fetched = Remote::fetched(state.remote(&key), "")
        .pick(<Capped<SavedReply> as Picked>::pick)
        .text("your saved replies");
    let note = |label: String| Row {
        label,
        insert: None,
    };
    let mut rows = match fetched {
        // Said short: the list is narrow.
        Err(why) if why.contains("read:user") => vec![
            note("Your token lacks the read:user scope:".to_owned()),
            note(format!("{} adds it", crate::api::refresh("read:user"))),
        ],
        Err(why) => vec![note(why)],
        Ok(replies) if replies.is_empty() => vec![note("You have no saved replies".to_owned())],
        Ok(replies) => {
            let q = query.to_lowercase();
            let mut found: Vec<Row> = (replies.iter())
                .filter(|r| {
                    r.title.to_lowercase().contains(&q) || r.body.to_lowercase().contains(&q)
                })
                .map(|r| Row {
                    label: r.title.clone(),
                    insert: Some(r.body.clone()),
                })
                .collect();
            if found.is_empty() {
                found.push(note(format!("No saved reply has “{query}”")));
            }
            if replies.left_out() > 0 {
                found.push(note(format!("{} more on GitHub", replies.left_out())));
            }
            found
        }
    };
    let url = crate::api::replies::settings_url();
    let url = url.trim_start_matches("https://");
    rows.push(note(format!("Make or change them at {url}")));
    rows
}

/// The list as the composer shows it, and the row selected; `None`
/// while it's closed.
pub fn shown(state: &State) -> Option<(Vec<String>, usize)> {
    let Some(Overlay::Compose(c)) = &state.overlay else {
        return None;
    };
    let choosing = c.saved.as_ref()?;
    let mut labels = vec![format!("Saved replies ⌕ {}", choosing.query)];
    labels.extend(rows(state, &choosing.query).into_iter().map(|r| r.label));
    let selected = choosing.selected.min(labels.len().saturating_sub(2)) + 1;
    Some((labels, selected))
}

/// A key in the composer: `ctrl-g` opens the list (or closes it); while
/// it's open, keys are the list's. `None` for the composer's own keys.
pub fn on_key(state: &mut State, key: KeyEvent) -> Option<Vec<Cmd>> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let open = matches!(&state.overlay, Some(Overlay::Compose(c)) if c.saved.is_some());
    if key.code == KeyCode::Char('g') && ctrl {
        if let Some(Overlay::Compose(c)) = &mut state.overlay {
            c.saved = match c.saved {
                Some(_) => None,
                None => Some(Choosing::default()),
            };
        }
        // Asked anew each time it opens: they may have changed on GitHub.
        return Some(if open {
            Vec::new()
        } else {
            state.ensure(DataKey::SavedReplies, true)
        });
    }
    if !open {
        return None;
    }
    let query = match &state.overlay {
        Some(Overlay::Compose(c)) => c.saved.as_ref().map(|s| s.query.clone()),
        _ => None,
    }?;
    let rows = rows(state, &query);
    let Some(Overlay::Compose(c)) = &mut state.overlay else {
        return None;
    };
    let choosing = c.saved.as_mut()?;
    let last = rows.len().saturating_sub(1);
    match key.code {
        KeyCode::Esc => c.saved = None,
        KeyCode::Down | KeyCode::Tab => choosing.selected = (choosing.selected + 1).min(last),
        KeyCode::Char('n') if ctrl => choosing.selected = (choosing.selected + 1).min(last),
        KeyCode::Up | KeyCode::BackTab => choosing.selected = choosing.selected.saturating_sub(1),
        KeyCode::Char('p') if ctrl => choosing.selected = choosing.selected.saturating_sub(1),
        KeyCode::Backspace => {
            choosing.query.pop();
            choosing.selected = 0;
        }
        KeyCode::Char(ch) if !ctrl => {
            choosing.query.push(ch);
            choosing.selected = 0;
        }
        KeyCode::Enter => {
            if let Some(text) = rows
                .into_iter()
                .nth(choosing.selected)
                .and_then(|r| r.insert)
            {
                c.input.insert_str(&text);
                c.saved = None;
                c.sent.error = None;
            }
        }
        _ => {}
    }
    Some(Vec::new())
}
