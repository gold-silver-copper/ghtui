//! The actions menu.

use super::*;

pub struct Menu {
    pub rows: Vec<Doable>,
    /// Into [`Menu::shown`].
    pub selected: usize,
    /// Typed after `/`: only matching rows show.
    pub filter: Option<String>,
}

impl Menu {
    /// The rows the filter lets through, in order.
    pub fn shown(&self) -> Vec<&Doable> {
        // An empty filter lets every row through.
        let Some(query) = self.filter.as_deref().filter(|q| !q.is_empty()) else {
            return self.rows.iter().collect();
        };
        // What's typed, as typed, before a scattered match: "revert"
        // mustn't run "Open the selected row".
        let lower = query.to_lowercase();
        let has = |d: &&Doable| {
            d.label.to_lowercase().contains(&lower)
                || d.action.description().to_lowercase().contains(&lower)
        };
        let exact: Vec<_> = self.rows.iter().filter(has).collect();
        if !exact.is_empty() {
            return exact;
        }
        let fuzzy = |text: &str| crate::picker::fuzzy_score(query, text).is_some();
        (self.rows.iter())
            .filter(|d| fuzzy(&d.label) || fuzzy(d.action.description()))
            .collect()
    }

    /// Selects the first row that can run, after the filter changed.
    pub(crate) fn reselect(&mut self) {
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

pub fn on_menu_key(state: &mut State, key: KeyEvent) -> Cmds {
    let stack = state.contexts();
    let pressed = crate::keymap::Key::from(key);
    let action = state.keymap.resolve(pressed, stack);
    let Some(Overlay::Menu(menu)) = &mut state.overlay else {
        return Cmds::new();
    };
    let last = menu.shown().len().saturating_sub(1);
    let typed = !key.modifiers.contains(KeyModifiers::CONTROL);
    let run = match (&mut menu.filter, key.code) {
        (_, KeyCode::Down) => {
            menu.selected = (menu.selected + 1).min(last);
            return Cmds::new();
        }
        (_, KeyCode::Up) => {
            menu.selected = menu.selected.saturating_sub(1);
            return Cmds::new();
        }
        (_, KeyCode::Enter) | (None, KeyCode::Right) => {
            menu.shown().get(menu.selected).map(|d| (*d).clone())
        }
        // Filtering: letters narrow the list instead of running rows.
        (Some(query), code) => {
            match code {
                KeyCode::Char(c) if typed => query.push(c),
                KeyCode::Char('u') => query.clear(),
                KeyCode::Backspace if query.pop().is_some() => {}
                KeyCode::Backspace | KeyCode::Esc => menu.filter = None,
                _ => return Cmds::new(),
            }
            menu.reselect();
            return Cmds::new();
        }
        (None, _) if action == Some(Action::Search) => {
            menu.filter = Some(String::new());
            return Cmds::new();
        }
        (None, KeyCode::Esc | KeyCode::Left) => None,
        (None, _) if action == Some(Action::Menu) => None,
        // A row's own key runs it.
        (None, _) => {
            let keys = |d: &&Doable| state.keymap.keys_in(d.action, stack).any(|k| k == pressed);
            match menu.rows.iter().find(keys).cloned() {
                Some(d) => Some(d),
                None => {
                    let shown = crate::keymap::pretty(pressed);
                    return state.just_say(format!("{shown}: no row here has that key"));
                }
            }
        }
    };
    run_menu_row(state, run)
}

/// Runs a menu row (`None` closes the menu).
pub(super) fn run_menu_row(state: &mut State, row: Option<Doable>) -> Cmds {
    let Some(d) = row else {
        state.overlay = None;
        return Cmds::new();
    };
    if let Some(why) = d.unavailable {
        return state.just_say(format!("Can't: {why}"));
    }
    state.overlay = None;
    apply(state, d.action)
}
