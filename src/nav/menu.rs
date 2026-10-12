//! The actions menu.

use super::*;

pub struct Menu {
    pub rows: Vec<Doable>,
    /// Into [`Menu::shown`].
    pub selected: usize,
    /// Typed after `/`: only matching rows show.
    pub filter: Option<String>,
    /// Keys typed so far that start a row's keys (`]` of `]c`).
    pub typed: Vec<crate::keymap::Key>,
}

impl Menu {
    /// The rows the filter lets through, in order.
    pub fn shown(&self) -> Vec<&Doable> {
        let query = self.filter.as_deref().unwrap_or_default();
        self.rows
            .iter()
            .filter(|d| {
                crate::picker::fuzzy_score(query, &d.label).is_some()
                    || crate::picker::fuzzy_score(query, d.action.description()).is_some()
            })
            .collect()
    }

    /// Selects the first row that can run, after the filter changed.
    pub(super) fn reselect(&mut self) {
        self.selected = self
            .shown()
            .iter()
            .position(|d| d.unavailable.is_none())
            .unwrap_or(0);
    }
}

pub fn open_menu(state: &mut State) {
    let mut menu = Menu {
        rows: state.doables(),
        selected: 0,
        filter: None,
        typed: Vec::new(),
    };
    menu.reselect();
    state.overlay = Some(Overlay::Menu(Box::new(menu)));
}

impl State {
    /// The menu's rows under their section headings, and the selected row.
    pub fn menu_rows(&self, menu: &Menu) -> (Vec<KeyRow>, usize) {
        let mut rows = Vec::new();
        let mut selected = 0;
        let mut previous = None;
        for (i, d) in menu.shown().into_iter().enumerate() {
            if previous != Some(d.section) {
                previous = Some(d.section);
                rows.push(KeyRow::heading(d.section));
            }
            if i == menu.selected {
                selected = rows.len();
            }
            let key = self.key_here(d.action).unwrap_or_default();
            rows.push(match &d.unavailable {
                Some(why) => KeyRow {
                    dim: true,
                    ..KeyRow::key(key, format!("{} ({why})", d.label))
                },
                None => KeyRow::key(key, &d.label),
            });
        }
        (rows, selected)
    }
}

#[must_use]
pub fn on_menu_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    let stack = state.contexts();
    let pressed = [crate::keymap::Key::from(key)];
    let action = match state.keymap.resolve(&pressed, stack) {
        crate::keymap::Resolved::Run(action) => Some(action),
        _ => None,
    };
    let Some(Overlay::Menu(menu)) = &mut state.overlay else {
        return Vec::new();
    };
    let last = menu.shown().len().saturating_sub(1);
    let typed = !key.modifiers.contains(KeyModifiers::CONTROL);
    let run = match (&mut menu.filter, key.code) {
        // Keys of a row typed so far: Esc drops them, more keys finish them.
        (None, KeyCode::Esc) if !menu.typed.is_empty() => {
            menu.typed.clear();
            return Vec::new();
        }
        (None, _) if !menu.typed.is_empty() => return run_menu_keys(state, pressed),
        (_, KeyCode::Down) | (None, KeyCode::Char('j')) => {
            menu.selected = (menu.selected + 1).min(last);
            return Vec::new();
        }
        (_, KeyCode::Up) | (None, KeyCode::Char('k')) => {
            menu.selected = menu.selected.saturating_sub(1);
            return Vec::new();
        }
        (_, KeyCode::Enter) | (None, KeyCode::Right | KeyCode::Char('l')) => {
            menu.shown().get(menu.selected).map(|d| (*d).clone())
        }
        // Filtering: letters narrow the list instead of running rows.
        (Some(query), code) => {
            match code {
                KeyCode::Char(c) if typed => query.push(c),
                KeyCode::Char('u') => query.clear(),
                KeyCode::Backspace if query.pop().is_some() => {}
                KeyCode::Backspace | KeyCode::Esc => menu.filter = None,
                _ => return Vec::new(),
            }
            menu.reselect();
            return Vec::new();
        }
        (None, _) if action == Some(Action::Search) => {
            menu.filter = Some(String::new());
            return Vec::new();
        }
        (None, KeyCode::Esc | KeyCode::Char('q') | KeyCode::Left | KeyCode::Char('h')) => None,
        (None, _) if action == Some(Action::Menu) => None,
        // A row's own keys run it.
        (None, _) => return run_menu_keys(state, pressed),
    };
    run_menu_row(state, run)
}

/// A key toward a row's own keys: runs the row once they're all typed,
/// waits while they could still be.
#[must_use]
pub(super) fn run_menu_keys(state: &mut State, [pressed]: [crate::keymap::Key; 1]) -> Vec<Cmd> {
    let stack = state.contexts();
    let Some(Overlay::Menu(menu)) = &mut state.overlay else {
        return Vec::new();
    };
    menu.typed.push(pressed);
    let typed = menu.typed.clone();
    let keys = |d: &Doable| {
        state
            .keymap
            .keys_in(d.action, stack)
            .map(<[_]>::to_vec)
            .collect::<Vec<_>>()
    };
    let row = menu.rows.iter().find(|d| keys(d).contains(&typed)).cloned();
    let longer = menu.rows.iter().any(|d| {
        keys(d)
            .iter()
            .any(|k| k.len() > typed.len() && k.starts_with(&typed))
    });
    match row {
        Some(d) => {
            menu.typed.clear();
            run_menu_row(state, Some(d))
        }
        None if longer => Vec::new(),
        None => {
            menu.typed.clear();
            let shown = crate::keymap::pretty_keys(&typed);
            state.info(format!("{shown}: no row here has those keys"));
            Vec::new()
        }
    }
}

/// Runs a menu row (`None` closes the menu).
#[must_use]
pub(super) fn run_menu_row(state: &mut State, row: Option<Doable>) -> Vec<Cmd> {
    let Some(d) = row else {
        state.overlay = None;
        return Vec::new();
    };
    if let Some(why) = d.unavailable {
        state.info(format!("Can't: {why}"));
        return Vec::new();
    }
    state.overlay = None;
    apply(state, d.action)
}
