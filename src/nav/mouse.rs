//! The mouse.

use super::*;
use crate::route::app;

pub fn on_mouse(state: &mut State, ev: MouseEvent) -> Cmds {
    let (x, y) = (ev.column, ev.row);
    match ev.kind {
        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
            let down = ev.kind == MouseEventKind::ScrollDown;
            if state.overlay.is_some() {
                return Cmds::new();
            }
            let height = state.page_height();
            match state.screen_mut() {
                Screen::Page(p) => {
                    scroll_by(p, if down { STEP } else { -STEP }, height);
                }
                Screen::Diff(_) => {
                    let action = if down { Action::Down } else { Action::Up };
                    return (0..3).flat_map(|_| apply(state, action)).collect();
                }
            }
            Cmds::new()
        }
        MouseEventKind::Down(button) => click(state, x, y, button),
        _ => Cmds::new(),
    }
}

pub(super) fn click(state: &mut State, x: u16, y: u16, button: MouseButton) -> Cmds {
    let inside = |r: Rect| r.contains((x, y).into());
    // Overlays first: a click outside closes them.
    match &state.overlay {
        Some(Overlay::Search(sb)) => {
            let (field, _) = state.search_field(sb);
            let (rows, picks): (Vec<_>, Vec<_>) = state.suggestions(sb).into_iter().unzip();
            let panel = chrome::SearchPanel {
                ctx: state.ctx(0),
                rows: &rows,
                selected: sb.selected,
                field,
            };
            let screen = Rect::new(0, 0, state.size.0, state.size.1);
            if inside(field) {
                return Cmds::new();
            }
            if inside(panel.area(screen)) {
                let pick = panel.row_at(screen, y).and_then(|r| picks.get(r));
                let Some(Some(pick)) = pick.cloned() else { return Cmds::new() };
                state.overlay = None;
                return choose(state, pick);
            }
            state.overlay = None;
            // Away from a filter, the header's field still searches GitHub.
            if inside(state.header_search()) {
                return state.open_search(true);
            }
            return Cmds::new();
        }
        Some(Overlay::Compose(_) | Overlay::Submit(_)) => return Cmds::new(),
        Some(_) => {
            state.overlay = None;
            return Cmds::new();
        }
        None => {}
    }
    let lay = state.layout();
    let chrome = state.chrome();
    if inside(lay.header) {
        let h = header_of(&chrome, lay.header);
        if inside(h.search) {
            return state.open_search(true);
        }
        if inside(h.logo) {
            return state.push(Route::App(app::Home));
        }
        let hit = |rects: &[Rect]| rects.iter().position(|r| inside(*r));
        // In the tab strip, a tab (or a hidden-tabs marker) switches to it.
        if let Some(&tab) = hit(&h.crumbs).and_then(|i| chrome.open_tabs.get(i)) {
            return state.switch_to_tab(tab);
        }
        let crumb = hit(&h.crumbs).and_then(|i| chrome.crumbs.get(i)?.1.clone());
        let right = hit(&h.right)
            .and_then(|i| chrome.right.get(i))
            .map(|(_, t)| t.clone());
        return crumb
            .or(right)
            .map_or_else(Cmds::new, |target| state.go(target.into()));
    }
    if let Some(tabs) = lay.tabs
        && inside(tabs)
    {
        return match tabs_of(&chrome, tabs)
            .iter()
            .position(|r| x >= r.x && x < r.right())
        {
            Some(i) => switch_tab(state, i + 1),
            None => Cmds::new(),
        };
    }
    let content = lay.content;
    if let Some((screen, diff)) = state.diff_parts() {
        crate::diff_screen::click(screen, diff, content, x, y);
        return Cmds::new();
    }
    let area = state.page_area();
    let height = state.page_height();
    let Screen::Page(p) = state.screen_mut() else { return Cmds::new() };
    if !inside(area) {
        return Cmds::new();
    }
    p.fresh = false;
    let hit = page::hit(&p.page, area, p.scroll, x, y);
    let item = hit.line.and_then(|l| p.page.item_at(l));
    if button == MouseButton::Right {
        p.selected = item.or(p.selected);
        open_menu(state);
        return Cmds::new();
    }
    if let Some(link) = hit.link.and_then(|l| p.page.target(l)) {
        let link = link.clone();
        p.selected = item.or(p.selected);
        return state.follow(&link);
    }
    match item {
        // A second click on a row opens it.
        Some(i) if p.selected == Some(i) => {
            let link = p.selected_link().cloned();
            link.map_or_else(Cmds::new, |l| state.follow(&l))
        }
        Some(i) => {
            p.selected = Some(i);
            reveal(p, i, height);
            Cmds::new()
        }
        None => Cmds::new(),
    }
}

impl State {
    /// The repository the screen is about.
    pub fn context_repo(&self) -> Option<&RepoId> {
        match self.screen() {
            Screen::Page(p) => p.route.repo(),
            Screen::Diff(d) => Some(d.of.repo()),
        }
    }
}
