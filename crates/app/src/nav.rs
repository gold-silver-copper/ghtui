//! Moving around pages, the way you'd use the website: selecting rows,
//! scrolling, following links (by keys, letters or the mouse), tabs,
//! history, the search box with its suggestions, go to file, branches, the
//! actions menu, and the keys hinted in the status bar.

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ghtui_api::browse::{RepoSummary, SearchKind};
use ghtui_api::model::RepoId;
use ghtui_theme::{Bg, Theme};
use ghtui_ui::bars::Notice;
use ghtui_ui::chrome::{self, KeyRow, SuggestRow};
use ghtui_ui::overlays::PaletteItem;
use ghtui_ui::page::{self, HintLabel};
use ghtui_ui::pages::{self, PrTab};
use ghtui_ui::{PAD_X, PAD_Y};
use ratatui::layout::Rect;
use ratatui_textarea::TextArea;
use serde::{Deserialize, Serialize};

use crate::browse::{self, Data, DataKey, PageScreen};
use crate::keymap::{Action, Scope};
use crate::review::{Compose, ComposeTarget};
use crate::route::{self, Route, Target};
use crate::state::{Cmd, Overlay, Remote, Screen, State, apply, fuzzy_score, new_palette};

/// Rows `j`/`k` scroll when there's no row to move to nearby.
const STEP: usize = 3;
/// Rows kept around the selection when scrolling to it.
const MARGIN: usize = 2;
/// Recently visited pages kept for the search box.
const VISITS: usize = 300;

// ---- history -----------------------------------------------------------------------

/// A page you visited, for "jump to" suggestions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Visit {
    pub url: String,
    pub title: String,
    pub count: u32,
    pub last: u64,
}

impl Visit {
    /// Frecency: visits, discounted by age in days.
    fn score(&self, now: u64) -> u64 {
        let days = now.saturating_sub(self.last) / 86_400;
        u64::from(self.count) * 100 / (days + 1)
    }
}

impl State {
    fn record_visit(&mut self, route: &Route) -> Vec<Cmd> {
        if *route == Route::Home {
            return Vec::new();
        }
        let url = route.url();
        let now = (self.clock)();
        match self.visits.iter_mut().find(|v| v.url == url) {
            Some(v) => {
                v.count += 1;
                v.last = now;
                v.title = route.title();
            }
            None => self.visits.push(Visit {
                url,
                title: route.title(),
                count: 1,
                last: now,
            }),
        }
        if self.visits.len() > VISITS {
            self.visits.sort_by_key(|v| std::cmp::Reverse(v.score(now)));
            self.visits.truncate(VISITS);
        }
        vec![Cmd::SaveVisits(self.visits.clone())]
    }

    /// Navigates to a page, keeping the current one in history.
    pub fn push(&mut self, route: Route) -> Vec<Cmd> {
        self.forward.clear();
        self.screens
            .push(Screen::Page(Box::new(PageScreen::new(route.clone()))));
        let mut cmds = self.ensure_route(&route, true);
        cmds.extend(self.record_visit(&route));
        cmds
    }

    /// Replaces the page on screen (switching tabs, changing a filter).
    pub fn replace(&mut self, route: Route, force: bool) -> Vec<Cmd> {
        match self.screen_mut() {
            Screen::Page(p) => **p = PageScreen::new(route.clone()),
            Screen::Diff(_) => return self.push(route),
        }
        let mut cmds = self.ensure_route(&route, force);
        cmds.extend(self.record_visit(&route));
        cmds
    }

    pub fn back(&mut self) -> Vec<Cmd> {
        if self.screens.len() > 1 {
            let screen = self.screens.pop().expect("more than one");
            self.forward.push(screen);
            return self.load_visible(false);
        }
        self.notice = Some(Notice::Info("This is the first page".into()));
        Vec::new()
    }

    pub fn go_forward(&mut self) -> Vec<Cmd> {
        match self.forward.pop() {
            Some(screen) => {
                self.screens.push(screen);
                self.load_visible(false)
            }
            None => {
                self.notice = Some(Notice::Info("Nothing to go forward to".into()));
                Vec::new()
            }
        }
    }

    /// Follows a link.
    pub fn go(&mut self, target: Target) -> Vec<Cmd> {
        match target {
            Target::Page(route) => self.push(route),
            Target::Files(pr) => {
                self.forward.clear();
                let mut cmds = self.ensure_pr(&pr, false);
                cmds.extend(self.open_diff(pr));
                cmds
            }
            Target::External(url) => {
                self.notice = Some(Notice::Info(format!("Opened {url} in the browser")));
                vec![Cmd::OpenUrl(url)]
            }
        }
    }

    /// Follows a link on a page: a GitHub URL, or one of the page's actions.
    pub fn follow(&mut self, url: &str) -> Vec<Cmd> {
        match url {
            pages::MORE => self.load_more(),
            pages::STAR => star(self),
            pages::COMMENT => comment(self),
            pages::FILTER => self.open_search(),
            pages::SORT => cycle_sort(self),
            pages::BRANCH => self.open_finder(FinderKind::Branches),
            pages::FIND_FILE => self.open_finder(FinderKind::Files),
            _ => match url.strip_prefix(pages::STATE) {
                Some(state) => set_list_state(self, state),
                None => self.go(Target::from_url(url)),
            },
        }
    }

    fn load_more(&mut self) -> Vec<Cmd> {
        let Some((kind, query)) = self.route().and_then(Route::search) else {
            return Vec::new();
        };
        let key = DataKey::Search(kind, query);
        let Some(remote) = self.data.get_mut(&key) else {
            return Vec::new();
        };
        let after = match &remote.data {
            Some(Data::Search(results)) if !remote.loading => browse::next_cursor(results),
            _ => None,
        };
        let Some(after) = after.map(str::to_owned) else {
            return Vec::new();
        };
        remote.loading = true;
        vec![Cmd::FetchMore { key, after }]
    }
}

// ---- the page on screen -----------------------------------------------------------------

impl State {
    /// Where pages draw: the content area below a row of breathing room.
    pub fn page_area(&self) -> Rect {
        let c = self.layout().content;
        Rect {
            y: c.y + PAD_Y.min(c.height),
            height: c.height.saturating_sub(PAD_Y),
            ..c
        }
    }

    /// Columns pages are built for.
    pub fn page_width(&self) -> u16 {
        self.size
            .0
            .saturating_sub(2 * PAD_X + 1)
            .min(browse::MAX_WIDTH)
    }

    /// Rows a page shows.
    pub fn page_height(&self) -> usize {
        usize::from(self.page_area().height).max(1)
    }

    /// Rebuilds the page on screen if its data or the width changed, and
    /// keeps its scroll and selection valid.
    pub fn sync_page(&mut self) {
        let width = self.page_width();
        let generation = self.data_gen;
        let height = self.page_height();
        let rebuild = match self.screen() {
            Screen::Page(p) if p.built != Some((generation, width)) => Some(p.route.clone()),
            Screen::Page(_) => None,
            Screen::Diff(_) => return,
        };
        let page = rebuild.map(|route| self.build_page(&route, width, (self.clock)()));
        if let Screen::Page(p) = self.screen_mut() {
            if let Some(page) = page {
                p.page = Arc::new(page);
                p.built = Some((generation, width));
            }
            settle(p, height);
        }
    }
}

fn visible(p: &PageScreen, item: usize, height: usize) -> bool {
    p.page
        .items
        .get(item)
        .is_some_and(|i| i.start < p.scroll + height && i.end > p.scroll)
}

/// Keeps a page's scroll in range and its selection on an item; a fresh
/// page selects its first visible item.
fn settle(p: &mut PageScreen, height: usize) {
    let max = p.page.height().saturating_sub(height);
    p.scroll = p.scroll.min(max);
    if p.selected.is_some_and(|s| s >= p.page.items.len()) {
        p.selected = None;
    }
    if p.fresh && p.selected.is_none() {
        p.selected = (0..p.page.items.len()).find(|&i| visible(p, i, height));
    }
}

fn reveal(p: &mut PageScreen, item: usize, height: usize) {
    if let Some(i) = p.page.items.get(item) {
        p.scroll = page::reveal(i.start, i.end, p.scroll, height, p.page.height(), MARGIN);
    }
}

/// `j`: the next row if it's near, else scroll on through the text.
fn next(p: &mut PageScreen, height: usize) {
    p.fresh = false;
    let items = &p.page.items;
    let candidate = match p.selected {
        Some(i) => i + 1,
        None => items
            .iter()
            .position(|it| it.start >= p.scroll)
            .unwrap_or(items.len()),
    };
    if let Some(it) = items.get(candidate)
        && it.start < p.scroll + height + STEP
    {
        p.selected = Some(candidate);
        reveal(p, candidate, height);
        return;
    }
    scroll_by(p, STEP as i64, height);
}

/// `k`: the previous row if it's near, else scroll back through the text.
fn prev(p: &mut PageScreen, height: usize) {
    p.fresh = false;
    let items = &p.page.items;
    let candidate = match p.selected {
        Some(i) => i.checked_sub(1),
        None => items.iter().rposition(|it| it.end <= p.scroll + height),
    };
    if let Some(c) = candidate
        && items[c].end + STEP > p.scroll
    {
        p.selected = Some(c);
        reveal(p, c, height);
        return;
    }
    scroll_by(p, -(STEP as i64), height);
}

/// Scrolls; a selection that leaves the screen is dropped.
fn scroll_by(p: &mut PageScreen, rows: i64, height: usize) {
    p.fresh = false;
    let max = p.page.height().saturating_sub(height) as i64;
    p.scroll = (p.scroll as i64 + rows).clamp(0, max.max(0)) as usize;
    if p.selected.is_some_and(|s| !visible(p, s, height)) {
        p.selected = None;
    }
}

/// Pages and jumps keep a selection on screen when there's one to keep.
fn scroll_keep(p: &mut PageScreen, rows: i64, height: usize) {
    let had = p.selected.is_some();
    scroll_by(p, rows, height);
    if had && p.selected.is_none() {
        let mut visible_items = (0..p.page.items.len()).filter(|&i| visible(p, i, height));
        p.selected = if rows > 0 {
            visible_items.next()
        } else {
            visible_items.next_back()
        };
    }
}

// ---- keys on pages ----------------------------------------------------------------------

/// Actions on a page screen; `None` for actions it doesn't handle.
pub fn page_action(state: &mut State, action: Action) -> Option<Vec<Cmd>> {
    let height = state.page_height();
    let Screen::Page(p) = state.screen_mut() else {
        return None;
    };
    let half = (height / 2).max(1) as i64;
    match action {
        Action::Down => next(p, height),
        Action::Up => prev(p, height),
        Action::ScrollDown => scroll_by(p, 1, height),
        Action::ScrollUp => scroll_by(p, -1, height),
        Action::HalfPageDown => scroll_keep(p, half, height),
        Action::HalfPageUp => scroll_keep(p, -half, height),
        Action::PageDown => scroll_keep(p, height.saturating_sub(2).max(1) as i64, height),
        Action::PageUp => scroll_keep(p, -(height.saturating_sub(2).max(1) as i64), height),
        Action::UpLevel => {
            let route = p.route.clone();
            return Some(match up(state, &route) {
                Some(parent) => state.push(parent),
                None => {
                    state.notice = Some(Notice::Info("Already at the top".into()));
                    Vec::new()
                }
            });
        }
        Action::Top => {
            scroll_by(p, i64::MIN / 2, height);
            p.selected = (!p.page.items.is_empty() && visible(p, 0, height)).then_some(0);
        }
        Action::Bottom => {
            scroll_by(p, i64::MAX / 2, height);
            let last = p.page.items.len().checked_sub(1);
            p.selected = last.filter(|&l| visible(p, l, height));
        }
        Action::Open => {
            return Some(match p.selected_url().map(str::to_owned) {
                Some(url) => state.follow(&url),
                None => {
                    let hints = state.first_key(Action::Hints);
                    state.notice = Some(Notice::Info(format!(
                        "Nothing selected: j/k select a row, {hints} picks any link"
                    )));
                    Vec::new()
                }
            });
        }
        Action::Hints | Action::HintsBrowser => {
            start_hints(state, action == Action::HintsBrowser);
        }
        Action::Search => return Some(state.open_search()),
        Action::Star => return Some(star(state)),
        Action::Comment => return Some(comment(state)),
        Action::FindFile => return Some(state.open_finder(FinderKind::Files)),
        Action::Branch => return Some(state.open_finder(FinderKind::Branches)),
        Action::ToggleState => return Some(cycle_state(state)),
        Action::Sort => return Some(cycle_sort(state)),
        Action::Menu => open_menu(state),
        Action::NextTab => return Some(step_tab(state, true)),
        Action::PrevTab => return Some(step_tab(state, false)),
        Action::GoCode => {
            let repo = state.context_repo().cloned();
            return Some(match repo {
                Some(repo) if state.route() != Some(&Route::Repo(repo.clone())) => {
                    state.push(Route::Repo(repo))
                }
                Some(_) => Vec::new(),
                None => no_repo(state),
            });
        }
        _ => return None,
    }
    Some(Vec::new())
}

/// The page above this one, like GitHub's breadcrumbs.
fn up(state: &State, route: &Route) -> Option<Route> {
    let default = |repo: &RepoId| state.overview(repo).and_then(|o| o.default_branch.clone());
    let folder = |repo: &RepoId, rev: &str, path: &str| {
        let parent = path.rsplit_once('/').map_or("", |(p, _)| p);
        if parent.is_empty() && default(repo).as_deref() == Some(rev) {
            Route::Repo(repo.clone())
        } else {
            Route::Tree {
                repo: repo.clone(),
                rev: rev.to_owned(),
                path: parent.to_owned(),
            }
        }
    };
    Some(match route {
        Route::Home => return None,
        Route::Repo(repo) => Route::user(&repo.owner),
        Route::Tree { repo, path, .. } if path.is_empty() => Route::Repo(repo.clone()),
        Route::Tree { repo, rev, path } | Route::Blob { repo, rev, path } => {
            folder(repo, rev, path)
        }
        Route::Issues { repo, .. } | Route::Pulls { repo, .. } => Route::Repo(repo.clone()),
        Route::Issue { repo, .. } => Route::Issues {
            repo: repo.clone(),
            query: crate::route::OPEN.into(),
        },
        Route::Pr { pr, .. } => Route::Pulls {
            repo: pr.repo.clone(),
            query: crate::route::OPEN.into(),
        },
        Route::User { .. } | Route::Search { .. } => Route::Home,
    })
}

fn no_repo(state: &mut State) -> Vec<Cmd> {
    state.notice = Some(Notice::Info("Open a repository first".into()));
    Vec::new()
}

/// Switches to the chrome's tab `n` (1-based).
pub fn switch_tab(state: &mut State, n: usize) -> Vec<Cmd> {
    let chrome = state.chrome();
    let Some((_, target)) = n.checked_sub(1).and_then(|i| chrome.tabs.get(i)).cloned() else {
        return Vec::new();
    };
    if chrome.active == Some(n - 1) {
        return Vec::new();
    }
    match (target, state.screen().clone()) {
        (Target::Page(route), Screen::Page(_)) => state.replace(route, false),
        (Target::Page(route), Screen::Diff(screen)) => {
            // From the files back to the pull request's other tabs.
            state.screens.pop();
            let same = matches!(state.route(), Some(Route::Pr { pr, .. }) if *pr == screen.pr);
            if same {
                state.replace(route, false)
            } else {
                state.push(route)
            }
        }
        (target, _) => state.go(target),
    }
}

fn step_tab(state: &mut State, forward: bool) -> Vec<Cmd> {
    let chrome = state.chrome();
    let n = chrome.tabs.len();
    let Some(active) = chrome.active else {
        return Vec::new();
    };
    let mut i = active;
    for _ in 0..n {
        i = if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        };
        if !chrome.tabs[i].0.external {
            return switch_tab(state, i + 1);
        }
    }
    Vec::new()
}

// ---- letter hints -----------------------------------------------------------------------------

pub struct Hints {
    pub labels: Vec<HintLabel>,
    pub links: Vec<String>,
    pub typed: String,
    pub browser: bool,
}

const LETTERS: &str = "asdfghjklqwertyuiopzxcvbnm";

/// Labels for `n` targets: single letters while they last, then pairs.
fn hint_labels(n: usize) -> Vec<String> {
    let letters: Vec<char> = LETTERS.chars().collect();
    if n <= letters.len() {
        return letters.iter().take(n).map(|c| c.to_string()).collect();
    }
    // Spread over as many first letters as possible, so one key narrows the
    // choice to a few.
    let per = n.div_ceil(letters.len()).min(letters.len());
    (0..n.min(letters.len() * letters.len()))
        .map(|i| format!("{}{}", letters[i / per], letters[i % per]))
        .collect()
}

fn start_hints(state: &mut State, browser: bool) {
    let area = state.page_area();
    let Screen::Page(p) = state.screen() else {
        return;
    };
    let spots = page::spots(&p.page, area, p.scroll);
    if spots.is_empty() {
        state.notice = Some(Notice::Info("No links on screen".into()));
        return;
    }
    let labels = hint_labels(spots.len());
    let links = spots
        .iter()
        .map(|s| p.page.links[s.link as usize].clone())
        .collect();
    let labels = spots
        .iter()
        .zip(labels)
        .map(|(s, label)| HintLabel {
            x: s.x,
            y: s.y,
            label,
        })
        .collect();
    state.overlay = Some(Overlay::Hints(Box::new(Hints {
        labels,
        links,
        typed: String::new(),
        browser,
    })));
}

pub fn on_hints_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    let Some(Overlay::Hints(hints)) = &mut state.overlay else {
        return Vec::new();
    };
    match key.code {
        KeyCode::Backspace => {
            hints.typed.pop();
            return Vec::new();
        }
        KeyCode::Char(c) if c.is_ascii_lowercase() => hints.typed.push(c),
        _ => {
            state.overlay = None;
            return Vec::new();
        }
    }
    let matching: Vec<usize> = (0..hints.labels.len())
        .filter(|&i| hints.labels[i].label.starts_with(&hints.typed))
        .collect();
    match matching.as_slice() {
        [] => {
            state.overlay = None;
            state.notice = Some(Notice::Info("No link has those letters".into()));
            Vec::new()
        }
        [i] if hints.labels[*i].label == hints.typed => {
            let url = hints.links[*i].clone();
            let browser = hints.browser;
            state.overlay = None;
            if browser {
                if url.starts_with("ghtui:") {
                    return state.follow(&url);
                }
                state.notice = Some(Notice::Info(format!("Opened {url} in the browser")));
                vec![Cmd::OpenUrl(url)]
            } else {
                state.follow(&url)
            }
        }
        _ => Vec::new(),
    }
}

impl Hints {
    /// Labels still matching what's typed.
    pub fn shown(&self) -> Vec<HintLabel> {
        self.labels
            .iter()
            .filter(|l| l.label.starts_with(&self.typed))
            .cloned()
            .collect()
    }
}

// ---- the search box --------------------------------------------------------------------------

pub struct SearchBox {
    pub input: TextArea<'static>,
    /// Filtering the list on screen rather than searching GitHub.
    pub filter: bool,
    pub selected: usize,
    /// Live repository suggestions, and the input they're for.
    pub remote: Vec<RepoSummary>,
    pub remote_for: String,
}

/// What choosing a suggestion does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    Go(Target),
    Search(SearchKind, String),
    Filter(String),
}

pub fn new_input(theme: &Theme, bg: Bg) -> TextArea<'static> {
    let mut input = TextArea::default();
    input.set_style(theme.body(bg));
    input.set_cursor_line_style(theme.body(bg));
    input.set_cursor_style(theme.fill(Bg::Primary));
    input
}

impl State {
    /// `/`: GitHub search in the header, or the list's filter on a list.
    pub fn open_search(&mut self) -> Vec<Cmd> {
        let filter_query = match self.route() {
            Some(
                Route::Issues { query, .. }
                | Route::Pulls { query, .. }
                | Route::Search { query, .. },
            ) => Some(query.clone()),
            _ => None,
        };
        let mut input = new_input(&self.theme, Bg::ContainerHighest);
        if let Some(q) = &filter_query {
            input.insert_str(q);
            input.insert_char(' ');
        }
        self.overlay = Some(Overlay::Search(Box::new(SearchBox {
            input,
            filter: filter_query.is_some(),
            selected: 0,
            remote: Vec::new(),
            remote_for: String::new(),
        })));
        Vec::new()
    }

    /// The search box's suggestions: rows to draw, and what choosing each
    /// item does (headings have none).
    pub fn suggestions(&self, sb: &SearchBox) -> Vec<(SuggestRow, Option<Pick>)> {
        let input = sb.input.lines().join("");
        let q = input.trim();
        let mut out: Vec<(SuggestRow, Option<Pick>)> = Vec::new();
        let item = |icon: &str, label: String, detail: &str, hint: &str| SuggestRow::Item {
            icon: icon.to_owned(),
            label,
            detail: detail.to_owned(),
            hint: hint.to_owned(),
        };
        let heading = |out: &mut Vec<(SuggestRow, Option<Pick>)>, h: &str| {
            out.push((SuggestRow::Heading(h.to_owned()), None));
        };
        let current = self.route().map(Route::url);
        let now = (self.clock)();
        if sb.filter {
            if !q.is_empty() {
                out.push((
                    item("⌕", q.to_owned(), "filter this list", "↵"),
                    Some(Pick::Filter(q.to_owned())),
                ));
            }
            heading(&mut out, "Quick filters");
            for (f, what) in [
                ("is:open", "open"),
                ("is:closed", "closed"),
                ("is:open author:@me", "yours"),
                ("is:open assignee:@me", "assigned to you"),
                ("is:open mentions:@me", "mentioning you"),
                ("is:open no:assignee", "unassigned"),
            ] {
                out.push((
                    item("⚑", f.to_owned(), what, ""),
                    Some(Pick::Filter(f.to_owned())),
                ));
            }
            if !q.is_empty() {
                heading(&mut out, "Search all of GitHub");
                out.push((
                    item("⌕", q.to_owned(), "issues and pull requests", ""),
                    Some(Pick::Search(SearchKind::Issues, q.to_owned())),
                ));
            }
            return out;
        }
        if let Some(target) = route::parse_input(q, self.context_repo()) {
            let label = match &target {
                Target::Page(r) => r.title(),
                Target::Files(pr) => format!("Files changed in {pr}"),
                Target::External(url) => url.clone(),
            };
            out.push((item("→", label, "go to", "↵"), Some(Pick::Go(target))));
        }
        // Jump to: pages you've visited, your repositories, live matches.
        let mut jumps: Vec<(usize, String, String, &str)> = Vec::new();
        let mut visits: Vec<&Visit> = self
            .visits
            .iter()
            .filter(|v| Some(&v.url) != current.as_ref())
            .collect();
        visits.sort_by_key(|v| std::cmp::Reverse(v.score(now)));
        for v in visits {
            if let Some(score) = fuzzy_score(q, &v.title) {
                jumps.push((score, v.title.clone(), v.url.clone(), "◷"));
            }
        }
        if let Some(Data::Repos(repos)) = self.get(&DataKey::ViewerRepos) {
            for r in repos {
                let name = r.repo.to_string();
                if let Some(score) = fuzzy_score(q, &name) {
                    jumps.push((score + 1, name, pages::url::repo(&r.repo), "▤"));
                }
            }
        }
        if q.is_empty() {
            // Most recent first, as they came.
        } else {
            jumps.sort_by_key(|(score, ..)| *score);
        }
        let mut seen: Vec<String> = Vec::new();
        for (_, title, url, icon) in jumps {
            if seen.contains(&url) || seen.len() >= if q.is_empty() { 8 } else { 5 } {
                continue;
            }
            if seen.is_empty() {
                heading(&mut out, if q.is_empty() { "Recent" } else { "Jump to" });
            }
            seen.push(url.clone());
            let detail = if icon == "◷" {
                "visited"
            } else {
                "your repository"
            };
            out.push((
                item(icon, title, detail, ""),
                Some(Pick::Go(Target::from_url(&url))),
            ));
        }
        if !q.is_empty() {
            heading(&mut out, "Search");
            if let Some(repo) = self.context_repo() {
                out.push((
                    item("⌕", q.to_owned(), &format!("in {repo}"), ""),
                    Some(Pick::Search(SearchKind::Issues, format!("repo:{repo} {q}"))),
                ));
            }
            for (kind, what) in [
                (SearchKind::Repos, "repositories"),
                (SearchKind::Issues, "issues"),
                (SearchKind::Pulls, "pull requests"),
                (SearchKind::Users, "users"),
            ] {
                out.push((
                    item("⌕", q.to_owned(), what, ""),
                    Some(Pick::Search(kind, q.to_owned())),
                ));
            }
        }
        // Live matches come last, so they never move what's selected.
        if sb.remote_for == q && !q.is_empty() {
            let live: Vec<&RepoSummary> = sb
                .remote
                .iter()
                .filter(|r| !seen.contains(&pages::url::repo(&r.repo)))
                .collect();
            if !live.is_empty() {
                heading(&mut out, "Repositories");
            }
            for r in live {
                let detail = format!("☆ {}", pages::compact(r.stars));
                out.push((
                    item("▤", r.repo.to_string(), &detail, ""),
                    Some(Pick::Go(Target::Page(Route::Repo(r.repo.clone())))),
                ));
            }
        }
        out
    }
}

pub fn on_search_box_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    let Some(Overlay::Search(sb)) = &state.overlay else {
        return Vec::new();
    };
    let count = state
        .suggestions(sb)
        .iter()
        .filter(|(_, p)| p.is_some())
        .count();
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let Some(Overlay::Search(sb)) = &mut state.overlay else {
        return Vec::new();
    };
    match key.code {
        KeyCode::Esc => {
            state.overlay = None;
            Vec::new()
        }
        KeyCode::Down | KeyCode::Tab => {
            sb.selected = (sb.selected + 1).min(count.saturating_sub(1));
            Vec::new()
        }
        KeyCode::Char('n') if ctrl => {
            sb.selected = (sb.selected + 1).min(count.saturating_sub(1));
            Vec::new()
        }
        KeyCode::Up | KeyCode::BackTab => {
            sb.selected = sb.selected.saturating_sub(1);
            Vec::new()
        }
        KeyCode::Char('p') if ctrl => {
            sb.selected = sb.selected.saturating_sub(1);
            Vec::new()
        }
        KeyCode::Enter => {
            let selected = sb.selected;
            let pick = {
                let Some(Overlay::Search(sb)) = &state.overlay else {
                    return Vec::new();
                };
                state
                    .suggestions(sb)
                    .into_iter()
                    .filter_map(|(_, p)| p)
                    .nth(selected)
            };
            state.overlay = None;
            pick.map_or_else(Vec::new, |p| choose(state, p))
        }
        _ => {
            sb.input.input(key);
            sb.selected = 0;
            let q = sb.input.lines().join("").trim().to_owned();
            if !sb.filter && q.chars().count() >= 2 && route::parse_input(&q, None).is_none() {
                vec![Cmd::SuggestLater(q)]
            } else {
                Vec::new()
            }
        }
    }
}

fn choose(state: &mut State, pick: Pick) -> Vec<Cmd> {
    match pick {
        Pick::Go(target) => state.go(target),
        Pick::Search(kind, query) => state.push(Route::Search { kind, query }),
        Pick::Filter(query) => match state.route().cloned() {
            Some(Route::Issues { repo, .. }) => state.replace(Route::Issues { repo, query }, true),
            Some(Route::Pulls { repo, .. }) => state.replace(Route::Pulls { repo, query }, true),
            Some(Route::Search { kind, .. }) => state.replace(Route::Search { kind, query }, true),
            _ => state.push(Route::Search {
                kind: SearchKind::Issues,
                query,
            }),
        },
    }
}

// ---- lists: state and sort ------------------------------------------------------------------

/// The list query with `is:open`/`is:closed` replaced by `state`.
fn with_state(query: &str, state: &str) -> String {
    let mut words: Vec<&str> = query
        .split_whitespace()
        .filter(|w| !matches!(*w, "is:open" | "is:closed" | "state:open" | "state:closed"))
        .collect();
    let qualifier = match state {
        "open" => Some("is:open"),
        "closed" => Some("is:closed"),
        _ => None,
    };
    if let Some(q) = qualifier {
        words.insert(0, q);
    }
    words.join(" ")
}

fn with_list_query(route: &Route, query: String) -> Option<Route> {
    Some(match route {
        Route::Issues { repo, .. } => Route::Issues {
            repo: repo.clone(),
            query,
        },
        Route::Pulls { repo, .. } => Route::Pulls {
            repo: repo.clone(),
            query,
        },
        Route::Search { kind, .. } => Route::Search { kind: *kind, query },
        _ => return None,
    })
}

fn list_query(state: &State) -> Option<(Route, String)> {
    match state.route()? {
        r @ (Route::Issues { query, .. }
        | Route::Pulls { query, .. }
        | Route::Search { query, .. }) => Some((r.clone(), query.clone())),
        _ => None,
    }
}

fn set_list_state(state: &mut State, which: &str) -> Vec<Cmd> {
    let Some((route, query)) = list_query(state) else {
        return Vec::new();
    };
    match with_list_query(&route, with_state(&query, which)) {
        Some(r) if r != route => state.replace(r, true),
        _ => Vec::new(),
    }
}

fn cycle_state(state: &mut State) -> Vec<Cmd> {
    let Some((_, query)) = list_query(state) else {
        state.notice = Some(Notice::Info("Only lists have open and closed".into()));
        return Vec::new();
    };
    let next = match pages::list_state(&query) {
        "open" => "closed",
        "closed" => "all",
        _ => "open",
    };
    set_list_state(state, next)
}

fn cycle_sort(state: &mut State) -> Vec<Cmd> {
    let Some((route, query)) = list_query(state) else {
        state.notice = Some(Notice::Info("Only lists can be sorted".into()));
        return Vec::new();
    };
    let current = pages::sort_label(&query);
    let i = pages::SORTS
        .iter()
        .position(|(_, l)| *l == current)
        .unwrap_or(0);
    let (next, label) = pages::SORTS[(i + 1) % pages::SORTS.len()];
    let mut words: Vec<&str> = query
        .split_whitespace()
        .filter(|w| !w.starts_with("sort:"))
        .collect();
    let sort = format!("sort:{next}");
    if next != "created-desc" {
        words.push(&sort);
    }
    state.notice = Some(Notice::Info(format!("Sorted: {label}")));
    match with_list_query(&route, words.join(" ")) {
        Some(r) => state.replace(r, true),
        None => Vec::new(),
    }
}

// ---- writing: star and comment ----------------------------------------------------------------

pub fn star(state: &mut State) -> Vec<Cmd> {
    let Some(repo) = state.route().and_then(Route::repo).cloned() else {
        state.notice = Some(Notice::Info("Open a repository to star it".into()));
        return Vec::new();
    };
    let Some(overview) = state.overview(&repo) else {
        state.notice = Some(Notice::Info("The repository hasn't loaded yet".into()));
        return Vec::new();
    };
    let (id, starred) = (overview.id.clone(), !overview.starred);
    set_starred(state, &repo, starred);
    vec![Cmd::SetStarred { repo, id, starred }]
}

/// Shows a repository as starred or not (optimistically, before GitHub
/// confirms).
pub fn set_starred(state: &mut State, repo: &RepoId, starred: bool) {
    if let Some(Remote {
        data: Some(Data::Repo(o)),
        ..
    }) = state.data.get_mut(&DataKey::Repo(repo.clone()))
        && o.starred != starred
    {
        o.starred = starred;
        o.summary.stars = if starred {
            o.summary.stars + 1
        } else {
            o.summary.stars.saturating_sub(1)
        };
    }
    state.data_gen += 1;
}

pub fn comment(state: &mut State) -> Vec<Cmd> {
    let target = match state.route() {
        Some(Route::Issue { repo, number }) => {
            let key = DataKey::Issue(repo.clone(), *number);
            match state.get(&key) {
                Some(Data::Issue(Some(issue))) => {
                    Some((issue.id.clone(), format!("{repo}#{number}"), key))
                }
                _ => None,
            }
        }
        Some(Route::Pr { pr, .. }) => {
            let key = DataKey::PrActivity(pr.clone());
            state
                .activity(pr)
                .map(|a| (a.id.clone(), pr.to_string(), key))
        }
        _ => {
            state.notice = Some(Notice::Info(
                "Comments go on issues and pull requests".into(),
            ));
            return Vec::new();
        }
    };
    let Some((subject_id, name, refresh)) = target else {
        state.notice = Some(Notice::Info("Still loading".into()));
        return Vec::new();
    };
    let compose = Compose::new(
        &state.theme,
        ComposeTarget::Conversation {
            subject_id,
            name,
            refresh,
        },
        "",
    );
    state.overlay = Some(Overlay::Compose(Box::new(compose)));
    Vec::new()
}

// ---- go to file, branches ----------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinderKind {
    Files,
    Branches,
}

pub struct Finder {
    pub kind: FinderKind,
    pub repo: RepoId,
    /// The revision files are listed at.
    pub rev: String,
    /// Where you were, to land on the same path on another branch.
    pub path: String,
    pub file: bool,
    pub input: TextArea<'static>,
    pub selected: usize,
}

impl Finder {
    pub fn title(&self) -> &'static str {
        match self.kind {
            FinderKind::Files => "Go to file",
            FinderKind::Branches => "Switch branches/tags",
        }
    }
}

impl State {
    /// The repository, revision and path on screen.
    fn code_context(&self) -> Option<(RepoId, String, String, bool)> {
        let default = |repo: &RepoId| {
            self.overview(repo)
                .and_then(|o| o.default_branch.clone())
                .unwrap_or_else(|| "HEAD".into())
        };
        Some(match self.route()? {
            Route::Tree { repo, rev, path } => (repo.clone(), rev.clone(), path.clone(), false),
            Route::Blob { repo, rev, path } => (repo.clone(), rev.clone(), path.clone(), true),
            route => {
                let repo = route.repo()?.clone();
                let rev = default(&repo);
                (repo, rev, String::new(), false)
            }
        })
    }

    pub fn open_finder(&mut self, kind: FinderKind) -> Vec<Cmd> {
        let Some((repo, rev, path, file)) = self.code_context() else {
            return no_repo(self);
        };
        let mut input = new_palette(&self.theme).input;
        input.set_placeholder_text(match kind {
            FinderKind::Files => "Type a file name",
            FinderKind::Branches => "Find a branch or tag",
        });
        let key = match kind {
            FinderKind::Files => DataKey::Files(repo.clone(), rev.clone()),
            FinderKind::Branches => DataKey::Refs(repo.clone()),
        };
        self.overlay = Some(Overlay::Finder(Box::new(Finder {
            kind,
            repo,
            rev,
            path,
            file,
            input,
            selected: 0,
        })));
        self.ensure(crate::browse::Need::Data(key), false)
    }

    /// The finder's rows: what to show and where each leads.
    pub fn finder_rows(&self, f: &Finder) -> Vec<(PaletteItem, Option<Target>)> {
        let q = f.input.lines().join("");
        let q = q.trim();
        let loading = |what: &str| {
            vec![(
                PaletteItem {
                    label: format!("Loading {what}…"),
                    hint: String::new(),
                },
                None,
            )]
        };
        match f.kind {
            FinderKind::Files => {
                let Some(Data::Files(files, truncated)) =
                    self.get(&DataKey::Files(f.repo.clone(), f.rev.clone()))
                else {
                    return loading("files");
                };
                let mut hits: Vec<(usize, &String)> = files
                    .iter()
                    .filter_map(|p| path_score(q, p).map(|s| (s, p)))
                    .collect();
                hits.sort_by_key(|(s, p)| (*s, p.len()));
                let mut rows: Vec<(PaletteItem, Option<Target>)> = hits
                    .into_iter()
                    .take(200)
                    .map(|(_, p)| {
                        (
                            PaletteItem {
                                label: p.clone(),
                                hint: String::new(),
                            },
                            Some(Target::Page(Route::Blob {
                                repo: f.repo.clone(),
                                rev: f.rev.clone(),
                                path: p.clone(),
                            })),
                        )
                    })
                    .collect();
                if *truncated && rows.len() < 200 {
                    rows.push((
                        PaletteItem {
                            label: "GitHub listed only part of this repository".into(),
                            hint: String::new(),
                        },
                        None,
                    ));
                }
                rows
            }
            FinderKind::Branches => {
                let Some(Data::Refs(refs)) = self.get(&DataKey::Refs(f.repo.clone())) else {
                    return loading("branches");
                };
                let default = self
                    .overview(&f.repo)
                    .and_then(|o| o.default_branch.clone());
                let mut rows: Vec<(usize, PaletteItem, Target)> = Vec::new();
                for (names, what) in [(&refs.branches, "branch"), (&refs.tags, "tag")] {
                    for name in names {
                        let Some(score) = fuzzy_score(q, name) else {
                            continue;
                        };
                        let hint = if Some(name) == default.as_ref() {
                            "default".to_owned()
                        } else if *name == f.rev {
                            "current".to_owned()
                        } else {
                            what.to_owned()
                        };
                        let target = if f.file {
                            Route::Blob {
                                repo: f.repo.clone(),
                                rev: name.clone(),
                                path: f.path.clone(),
                            }
                        } else if f.path.is_empty() && Some(name) == default.as_ref() {
                            Route::Repo(f.repo.clone())
                        } else {
                            Route::Tree {
                                repo: f.repo.clone(),
                                rev: name.clone(),
                                path: f.path.clone(),
                            }
                        };
                        rows.push((
                            score,
                            PaletteItem {
                                label: name.clone(),
                                hint,
                            },
                            Target::Page(target),
                        ));
                    }
                }
                rows.sort_by_key(|(s, ..)| *s);
                rows.into_iter().map(|(_, i, t)| (i, Some(t))).collect()
            }
        }
    }
}

/// Fuzzy file matching that prefers the file name.
fn path_score(q: &str, path: &str) -> Option<usize> {
    if q.is_empty() {
        return Some(path.matches('/').count());
    }
    let name = path.rsplit('/').next().unwrap_or(path);
    match fuzzy_score(q, name) {
        Some(s) => Some(s),
        None => fuzzy_score(q, path).map(|s| s + 2000),
    }
}

pub fn on_finder_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    let Some(Overlay::Finder(f)) = &state.overlay else {
        return Vec::new();
    };
    let count = state.finder_rows(f).len();
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let Some(Overlay::Finder(f)) = &mut state.overlay else {
        return Vec::new();
    };
    let last = count.saturating_sub(1);
    match key.code {
        KeyCode::Esc => state.overlay = None,
        KeyCode::Down | KeyCode::Tab => f.selected = (f.selected + 1).min(last),
        KeyCode::Char('n') if ctrl => f.selected = (f.selected + 1).min(last),
        KeyCode::Up | KeyCode::BackTab => f.selected = f.selected.saturating_sub(1),
        KeyCode::Char('p') if ctrl => f.selected = f.selected.saturating_sub(1),
        KeyCode::Enter => {
            let selected = f.selected;
            let target = {
                let Some(Overlay::Finder(f)) = &state.overlay else {
                    return Vec::new();
                };
                state
                    .finder_rows(f)
                    .into_iter()
                    .nth(selected)
                    .and_then(|(_, t)| t)
            };
            if let Some(target) = target {
                state.overlay = None;
                return state.go(target);
            }
        }
        _ => {
            f.input.input(key);
            f.selected = 0;
        }
    }
    Vec::new()
}

// ---- the actions menu and hints ----------------------------------------------------------------

/// Something you can do here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Doable {
    pub action: Action,
    /// For the menu.
    pub label: String,
    /// For the status bar.
    pub short: &'static str,
    pub unavailable: Option<String>,
}

fn doable(action: Action, label: impl Into<String>, short: &'static str) -> Doable {
    Doable {
        action,
        label: label.into(),
        short,
        unavailable: None,
    }
}

impl State {
    /// What you can do on this screen, most useful first.
    pub fn doables(&self) -> Vec<Doable> {
        let mut out = Vec::new();
        match self.screen() {
            Screen::Page(p) => {
                let selected = p
                    .selected
                    .and_then(|i| p.page.items.get(i))
                    .map(|i| &p.page.links[i.link as usize]);
                let mut open = doable(Action::Open, "Open the selected row", "open");
                match selected {
                    Some(url) => open.label = describe(url),
                    None => open.unavailable = Some("nothing is selected".into()),
                }
                out.push(open);
                out.push(doable(
                    Action::Hints,
                    "Follow a link by its letters",
                    "links",
                ));
                let route = &p.route;
                let list = matches!(
                    route,
                    Route::Issues { .. } | Route::Pulls { .. } | Route::Search { .. }
                );
                out.push(doable(
                    Action::Search,
                    if list {
                        "Filter this list"
                    } else {
                        "Search or jump to…"
                    },
                    if list { "filter" } else { "search" },
                ));
                if list {
                    let state =
                        pages::list_state(route.search().map(|(_, q)| q).as_deref().unwrap_or(""));
                    let next = match state {
                        "open" => "closed",
                        "closed" => "all",
                        _ => "open",
                    };
                    out.push(doable(
                        Action::ToggleState,
                        format!("Show {next}"),
                        "open/closed",
                    ));
                    out.push(doable(Action::Sort, "Change the sort", "sort"));
                }
                if route.repo().is_some() {
                    out.push(doable(Action::FindFile, "Go to file", "go to file"));
                    if matches!(
                        route,
                        Route::Repo(_) | Route::Tree { .. } | Route::Blob { .. }
                    ) {
                        out.push(doable(Action::Branch, "Switch branches or tags", "branch"));
                        let starred = route
                            .repo()
                            .and_then(|r| self.overview(r))
                            .is_some_and(|o| o.starred);
                        out.push(doable(
                            Action::Star,
                            if starred { "Unstar" } else { "Star" },
                            "star",
                        ));
                    }
                }
                if matches!(route, Route::Issue { .. } | Route::Pr { .. }) {
                    out.push(doable(Action::Comment, "Write a comment", "comment"));
                }
                if let Route::Pr { tab, .. } = route {
                    if *tab == PrTab::Conversation {
                        out.push(doable(Action::Tab2, "Commits", "commits"));
                    }
                    out.push(doable(Action::Tab4, "Files changed (review)", "files"));
                }
                if !self.chrome().tabs.is_empty() {
                    out.push(doable(Action::NextTab, "Next tab", "tab"));
                }
                out.push(doable(
                    Action::OpenInBrowser,
                    if selected.is_some() {
                        "Open the selection on GitHub"
                    } else {
                        "Open this page on GitHub"
                    },
                    "browser",
                ));
                out.push(doable(
                    Action::Copy,
                    if selected.is_some() {
                        "Copy the selection's link"
                    } else {
                        "Copy this page's link"
                    },
                    "copy link",
                ));
                out.push(doable(Action::Refresh, "Refresh", "refresh"));
                let mut back = doable(Action::Back, "Back", "back");
                if self.screens.len() < 2 {
                    back.unavailable = Some("this is the first page".into());
                }
                out.push(back);
                let mut fwd = doable(Action::Forward, "Forward", "forward");
                if self.forward.is_empty() {
                    fwd.unavailable = Some("nothing to go forward to".into());
                }
                out.push(fwd);
                out.push(doable(Action::GoHome, "Home", "home"));
                out.push(doable(
                    Action::CommandPalette,
                    "Command palette",
                    "commands",
                ));
                out.push(doable(Action::Help, "All keyboard shortcuts", "keys"));
            }
            Screen::Diff(_) => {
                // The status bar's few, then everything else the diff does.
                let hinted = [
                    (Action::NextHunk, "hunk"),
                    (Action::NextFile, "file"),
                    (Action::ToggleViewed, "viewed"),
                    (Action::Comment, "comment"),
                    (Action::SubmitReview, "submit"),
                    (Action::FindFile, "go to file"),
                ];
                for (action, short) in hinted {
                    out.push(doable(action, action.description(), short));
                }
                for action in Action::ALL {
                    if action.scope() == Scope::Diff && !hinted.iter().any(|(a, _)| *a == action) {
                        out.push(doable(action, action.description(), ""));
                    }
                }
                out.push(doable(
                    Action::Tab1,
                    "Back to the conversation",
                    "conversation",
                ));
                out.push(doable(Action::Help, "All keyboard shortcuts", "keys"));
            }
        }
        out
    }

    /// The status bar's key hints.
    pub fn key_hints(&self) -> Vec<(String, String)> {
        let pair = |keys: &str, what: &str| (keys.to_owned(), what.to_owned());
        match &self.overlay {
            Some(Overlay::Search(sb)) if sb.filter => {
                return vec![
                    pair("↵", "filter"),
                    pair("↑↓", "choose"),
                    pair("esc", "close"),
                ];
            }
            Some(Overlay::Search(_)) => {
                return vec![pair("↵", "go"), pair("↑↓", "choose"), pair("esc", "close")];
            }
            Some(Overlay::Hints(_)) => {
                return vec![pair("a-z", "type a link's letters"), pair("esc", "cancel")];
            }
            Some(Overlay::Menu(_)) => {
                return vec![
                    pair("↵", "run"),
                    pair("key", "run it"),
                    pair("esc", "close"),
                ];
            }
            Some(Overlay::Finder(_) | Overlay::Palette(_)) => {
                return vec![
                    pair("↵", "open"),
                    pair("↑↓", "choose"),
                    pair("esc", "close"),
                ];
            }
            Some(_) => return Vec::new(),
            None => {}
        }
        let mut out = Vec::new();
        for d in self.doables() {
            if d.unavailable.is_some()
                || matches!(
                    d.action,
                    Action::Back
                        | Action::Forward
                        | Action::GoHome
                        | Action::Refresh
                        | Action::CommandPalette
                )
            {
                continue;
            }
            if d.action == Action::Help || d.short.is_empty() {
                continue;
            }
            out.push((self.first_key(d.action), d.short.to_owned()));
        }
        out.truncate(7);
        out.push((self.first_key(Action::Menu), "more".into()));
        out.push((self.first_key(Action::Help), "keys".into()));
        out
    }

    /// Keys that can follow the pending prefix (which-key).
    pub fn continuations(&self) -> Vec<KeyRow> {
        if self.pending.is_empty() || self.overlay.is_some() {
            return Vec::new();
        }
        self.keymap
            .continuations(&self.pending, self.scope())
            .into_iter()
            .map(|(rest, action)| KeyRow {
                key: crate::keymap::pretty(&rest),
                label: action.description().to_owned(),
                unavailable: None,
            })
            .collect()
    }
}

/// What following `url` does, for the menu.
fn describe(url: &str) -> String {
    match url {
        pages::MORE => "Load more".into(),
        pages::COMMENT => "Write a comment".into(),
        pages::STAR => "Star or unstar".into(),
        pages::FILTER => "Edit the filter".into(),
        pages::SORT => "Change the sort".into(),
        pages::BRANCH => "Switch branches".into(),
        pages::FIND_FILE => "Go to file".into(),
        _ => match Target::from_url(url) {
            Target::Page(r) => format!("Open {}", r.title()),
            Target::Files(pr) => format!("Review {pr}'s files"),
            Target::External(u) => format!("Open {u} in the browser"),
        },
    }
}

pub struct Menu {
    pub rows: Vec<Doable>,
    pub selected: usize,
}

pub fn open_menu(state: &mut State) {
    let rows = state.doables();
    let selected = rows
        .iter()
        .position(|d| d.unavailable.is_none())
        .unwrap_or(0);
    state.overlay = Some(Overlay::Menu(Box::new(Menu { rows, selected })));
}

impl State {
    pub fn menu_rows(&self, menu: &Menu) -> Vec<KeyRow> {
        menu.rows
            .iter()
            .map(|d| KeyRow {
                key: self.first_key(d.action),
                label: match &d.unavailable {
                    Some(why) => format!("{} ({why})", d.label),
                    None => d.label.clone(),
                },
                unavailable: d.unavailable.clone(),
            })
            .collect()
    }
}

pub fn on_menu_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    let scope = state.scope();
    let Some(Overlay::Menu(menu)) = &mut state.overlay else {
        return Vec::new();
    };
    let last = menu.rows.len().saturating_sub(1);
    let run = match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('.') | KeyCode::Left => {
            state.overlay = None;
            return Vec::new();
        }
        KeyCode::Down | KeyCode::Char('j') => {
            menu.selected = (menu.selected + 1).min(last);
            return Vec::new();
        }
        KeyCode::Up | KeyCode::Char('k') => {
            menu.selected = menu.selected.saturating_sub(1);
            return Vec::new();
        }
        KeyCode::Enter | KeyCode::Right => menu.rows.get(menu.selected).cloned(),
        _ => {
            // A row's own key runs it.
            let pressed = crate::keymap::Key::from(key);
            menu.rows
                .iter()
                .find(|d| {
                    state
                        .keymap
                        .keys_in(d.action, scope)
                        .iter()
                        .any(|k| k.as_slice() == [pressed])
                })
                .cloned()
        }
    };
    let Some(d) = run else {
        return Vec::new();
    };
    if let Some(why) = d.unavailable {
        state.notice = Some(Notice::Info(format!("Can't: {why}")));
        return Vec::new();
    }
    state.overlay = None;
    apply(state, d.action)
}

// ---- copying -------------------------------------------------------------------------------------

/// `y`: the selection's link, or the page's.
pub fn copy_link(state: &mut State) -> Vec<Cmd> {
    let url = match state.screen() {
        Screen::Page(p) => p
            .selected_url()
            .filter(|u| u.starts_with("http"))
            .map(str::to_owned)
            .unwrap_or_else(|| p.route.url()),
        Screen::Diff(d) => format!("{}/files", d.pr.url()),
    };
    state.notice = Some(Notice::Info(format!("Copied {url}")));
    vec![Cmd::Copy(url)]
}

// ---- the mouse -------------------------------------------------------------------------------------

pub fn on_mouse(state: &mut State, ev: MouseEvent) -> Vec<Cmd> {
    let (x, y) = (ev.column, ev.row);
    match ev.kind {
        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
            let down = ev.kind == MouseEventKind::ScrollDown;
            if state.overlay.is_some() {
                return Vec::new();
            }
            let height = state.page_height();
            match state.screen_mut() {
                Screen::Page(p) => scroll_by(p, if down { 3 } else { -3 }, height),
                Screen::Diff(_) => {
                    let action = if down { Action::Down } else { Action::Up };
                    let mut cmds = Vec::new();
                    for _ in 0..3 {
                        cmds.extend(apply(state, action));
                    }
                    return cmds;
                }
            }
            Vec::new()
        }
        MouseEventKind::Down(button) => click(state, x, y, button),
        _ => Vec::new(),
    }
}

fn click(state: &mut State, x: u16, y: u16, button: MouseButton) -> Vec<Cmd> {
    let inside = |r: Rect| r.contains((x, y).into());
    // Overlays first: a click outside closes them.
    match &state.overlay {
        Some(Overlay::Search(sb)) => {
            let lay = state.layout();
            let chrome = state.chrome();
            let crumbs: Vec<_> = chrome.crumbs.iter().map(|(c, _)| c.clone()).collect();
            let right: Vec<_> = chrome.right.iter().map(|(t, _)| t.clone()).collect();
            let field = chrome::header_layout(lay.header, &crumbs, &right).search;
            let rows = state.suggestions(sb);
            let panel = chrome::SearchPanel {
                ctx: state.ctx(0),
                rows: &rows.iter().map(|(r, _)| r.clone()).collect::<Vec<_>>(),
                selected: 0,
                field,
                help: "",
            }
            .area(Rect::new(0, 0, state.size.0, state.size.1));
            if inside(field) {
                return Vec::new();
            }
            if inside(panel) {
                let row = usize::from(y - panel.y).saturating_sub(1);
                if let Some((_, Some(pick))) = rows.get(row).cloned() {
                    state.overlay = None;
                    return choose(state, pick);
                }
                return Vec::new();
            }
            state.overlay = None;
            return Vec::new();
        }
        Some(Overlay::Compose(_) | Overlay::Submit(_)) => return Vec::new(),
        Some(_) => {
            state.overlay = None;
            return Vec::new();
        }
        None => {}
    }
    let lay = state.layout();
    let chrome = state.chrome();
    if inside(lay.header) {
        let crumbs: Vec<_> = chrome.crumbs.iter().map(|(c, _)| c.clone()).collect();
        let right: Vec<_> = chrome.right.iter().map(|(t, _)| t.clone()).collect();
        let h = chrome::header_layout(lay.header, &crumbs, &right);
        if inside(h.search) {
            return state.open_search();
        }
        if inside(h.logo) {
            return state.push(Route::Home);
        }
        for (r, (_, target)) in h.crumbs.iter().zip(&chrome.crumbs) {
            if inside(*r)
                && let Some(t) = target.clone()
            {
                return state.go(t);
            }
        }
        for (r, (_, target)) in h.right.iter().zip(&chrome.right) {
            if inside(*r) {
                return state.go(target.clone());
            }
        }
        return Vec::new();
    }
    if let Some(tabs) = lay.tabs
        && inside(tabs)
    {
        let tab_list: Vec<_> = chrome.tabs.iter().map(|(t, _)| t.clone()).collect();
        for (i, r) in chrome::tab_layout(tabs, &tab_list, chrome.active)
            .iter()
            .enumerate()
        {
            if x >= r.x && x < r.right() {
                return switch_tab(state, i + 1);
            }
        }
        return Vec::new();
    }
    let area = state.page_area();
    let height = state.page_height();
    let Screen::Page(p) = state.screen_mut() else {
        return Vec::new();
    };
    if !inside(area) {
        return Vec::new();
    }
    p.fresh = false;
    let hit = page::hit(&p.page, area, p.scroll, x, y);
    let item = hit.line.and_then(|l| p.page.item_at(l));
    if button == MouseButton::Right {
        p.selected = item.or(p.selected);
        open_menu(state);
        return Vec::new();
    }
    if let Some(link) = hit.link {
        let url = p.page.links[link as usize].clone();
        p.selected = item.or(p.selected);
        return state.follow(&url);
    }
    match item {
        // A second click on a row opens it.
        Some(i) if p.selected == Some(i) => {
            let url = p.selected_url().map(str::to_owned);
            url.map_or_else(Vec::new, |u| state.follow(&u))
        }
        Some(i) => {
            p.selected = Some(i);
            reveal(p, i, height);
            Vec::new()
        }
        None => Vec::new(),
    }
}

/// The repository the screen is about.
impl State {
    pub fn context_repo(&self) -> Option<&RepoId> {
        match self.screen() {
            Screen::Page(p) => p.route.repo(),
            Screen::Diff(d) => Some(&d.pr.repo),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hint_labels_are_short_and_unique() {
        assert_eq!(hint_labels(3), ["a", "s", "d"]);
        let many = hint_labels(30);
        assert_eq!(many.len(), 30);
        assert_eq!(
            &many[..4],
            ["aa", "as", "sa", "ss"],
            "first letters spread out"
        );
        assert!(many.iter().all(|l| l.len() == 2));
        let mut sorted = many.clone();
        sorted.dedup();
        assert_eq!(sorted.len(), 30);
    }

    #[test]
    fn list_states_rewrite_the_query() {
        assert_eq!(
            with_state("is:open label:bug", "closed"),
            "is:closed label:bug"
        );
        assert_eq!(with_state("is:closed", "all"), "");
        assert_eq!(with_state("label:bug", "open"), "is:open label:bug");
    }

    #[test]
    fn file_matches_prefer_names() {
        assert!(path_score("main", "src/main.rs") < path_score("main", "maintainers/x.rs"));
        assert_eq!(path_score("zzz", "src/main.rs"), None);
    }
}
