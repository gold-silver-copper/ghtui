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
use ghtui_ui::page::{self, HintLabel};
use ghtui_ui::pages::{self, PrTab};
use ghtui_ui::{PAD_X, PAD_Y};
use ratatui::layout::Rect;
use ratatui_textarea::TextArea;
use serde::{Deserialize, Serialize};

use crate::browse::{self, Data, DataKey, PageScreen};
use crate::keymap::{Action, Resolution};
use crate::picker::{fuzzy_score, move_in_list};
use crate::review::ComposeTarget;
use crate::route::{self, Route, Target};
use crate::state::{Cmd, Overlay, Remote, Screen, State, apply};

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
        let mut cmds = self.record_visit(&route);
        self.screens
            .push(Screen::Page(Box::new(PageScreen::new(route))));
        if let Some(route) = self.route().cloned() {
            cmds.extend(self.ensure_route(&route, true));
        }
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
        if let Some(screen) = self.screens.pop() {
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
            pages::COMMENT => comment_with(self, ""),
            pages::FILTER => self.open_search(),
            pages::SORT => cycle_sort(self),
            pages::BRANCH => self.open_finder(true),
            pages::FIND_FILE => self.open_finder(false),
            _ => {
                if let Some(state) = url.strip_prefix(pages::STATE) {
                    return set_list_state(self, state);
                }
                if let Some(quoted) = url.strip_prefix(pages::QUOTE) {
                    let (author, body) = quoted.split_once('\n').unwrap_or((quoted, ""));
                    let mut text = format!("> @{author} wrote:\n");
                    for line in body.trim().lines() {
                        text.push_str(&format!("> {line}\n"));
                    }
                    text.push('\n');
                    return comment_with(self, &text);
                }
                self.go(Target::from_url(url))
            }
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

/// `↓`/`↑`: more of the selected item if it continues off screen; else the
/// next item; else (past the last one) scroll on.
fn step(p: &mut PageScreen, height: usize, down: bool) {
    p.fresh = false;
    let n = p.page.items.len();
    let items = &p.page.items;
    let target = match p.selected.map(|i| (i, items.get(i))) {
        Some((_, Some(item))) if down && item.end > p.scroll + height => None,
        Some((_, Some(item))) if !down && item.start < p.scroll => None,
        Some((i, _)) if down => (i + 1 < n).then_some(i + 1),
        Some((i, _)) => i.checked_sub(1),
        // Reading: pick up the first item that's on screen.
        None if down => (0..n).find(|&i| visible(p, i, height)),
        None => (0..n).rev().find(|&i| visible(p, i, height)),
    };
    match target {
        Some(i) => {
            p.selected = Some(i);
            reveal(p, i, height);
        }
        None if down => scroll_by(p, STEP as i64, height),
        None => scroll_by(p, -(STEP as i64), height),
    }
}

/// Scrolls; a selection that leaves the screen is dropped.
fn scroll_by(p: &mut PageScreen, rows: i64, height: usize) {
    p.fresh = false;
    let max = p.page.height().saturating_sub(height);
    let to = (p.scroll as i64).saturating_add(rows).max(0);
    p.scroll = usize::try_from(to).unwrap_or(usize::MAX).min(max);
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
        Action::Down => step(p, height, true),
        Action::Up => step(p, height, false),
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
                    let (down, hints) = (
                        state.first_key(Action::Down),
                        state.first_key(Action::Hints),
                    );
                    state.notice = Some(Notice::Info(format!(
                        "Nothing selected: {down} selects a row, {hints} picks any link"
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
        Action::Comment => return Some(comment_with(state, "")),
        Action::FindFile => return Some(state.open_finder(false)),
        Action::Branch => return Some(state.open_finder(true)),
        Action::ToggleState => return Some(cycle_state(state)),
        Action::Sort => return Some(cycle_sort(state)),
        Action::Menu => open_menu(state),
        Action::NextTab => return Some(step_tab(state, true)),
        Action::PrevTab => return Some(step_tab(state, false)),
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

pub fn no_repo(state: &mut State) -> Vec<Cmd> {
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
    // The tabs after the active one, wrapping around, in that direction.
    let after = (1..n).map(|k| {
        if forward {
            (active + k) % n
        } else {
            (active + n - k) % n
        }
    });
    let mut internal = after.filter(|&i| chrome.tabs.get(i).is_some_and(|(t, _)| !t.external));
    match internal.next() {
        Some(i) => switch_tab(state, i + 1),
        None => Vec::new(),
    }
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
    letters
        .iter()
        .flat_map(|a| letters.iter().take(per).map(move |b| format!("{a}{b}")))
        .take(n)
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
        .map(|s| {
            p.page
                .links
                .get(s.link as usize)
                .cloned()
                .unwrap_or_default()
        })
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
    let matching: Vec<(&HintLabel, &String)> = hints
        .labels
        .iter()
        .zip(&hints.links)
        .filter(|(h, _)| h.label.starts_with(&hints.typed))
        .collect();
    match matching.as_slice() {
        [] => {
            state.overlay = None;
            state.notice = Some(Notice::Info("No link has those letters".into()));
            Vec::new()
        }
        [(hint, url)] if hint.label == hints.typed => {
            let url = (*url).clone();
            let browser = hints.browser;
            state.overlay = None;
            if browser && !url.starts_with("ghtui:") {
                state.go(Target::External(url))
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
        let filter_query = self.route().and_then(Route::list_query).map(str::to_owned);
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
    let picks: Vec<Pick> = state
        .suggestions(sb)
        .into_iter()
        .filter_map(|(_, p)| p)
        .collect();
    let Some(Overlay::Search(sb)) = &mut state.overlay else {
        return Vec::new();
    };
    if move_in_list(key, &mut sb.selected, picks.len()) {
        return Vec::new();
    }
    match key.code {
        KeyCode::Esc => {
            state.overlay = None;
            Vec::new()
        }
        KeyCode::Enter => {
            let pick = picks.into_iter().nth(sb.selected);
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
        Pick::Filter(query) => match state.route().and_then(|r| r.with_query(query.clone())) {
            Some(route) => state.replace(route, true),
            None => state.push(Route::Search {
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

/// Open → closed → all.
fn next_list_state(query: &str) -> &'static str {
    match pages::list_state(query) {
        "open" => "closed",
        "closed" => "all",
        _ => "open",
    }
}

/// The list on screen and its filter.
fn list_query(state: &State) -> Option<(Route, String)> {
    let route = state.route()?;
    Some((route.clone(), route.list_query()?.to_owned()))
}

fn set_list_state(state: &mut State, which: &str) -> Vec<Cmd> {
    let Some((route, query)) = list_query(state) else {
        return Vec::new();
    };
    match route.with_query(with_state(&query, which)) {
        Some(r) if r != route => state.replace(r, true),
        _ => Vec::new(),
    }
}

fn cycle_state(state: &mut State) -> Vec<Cmd> {
    let Some((_, query)) = list_query(state) else {
        state.notice = Some(Notice::Info("Only lists have open and closed".into()));
        return Vec::new();
    };
    set_list_state(state, next_list_state(&query))
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
    let Some(&(next, label)) = pages::SORTS.iter().cycle().nth(i + 1) else {
        return Vec::new();
    };
    let mut words: Vec<&str> = query
        .split_whitespace()
        .filter(|w| !w.starts_with("sort:"))
        .collect();
    let sort = format!("sort:{next}");
    if next != "created-desc" {
        words.push(&sort);
    }
    state.notice = Some(Notice::Info(format!("Sorted: {label}")));
    match route.with_query(words.join(" ")) {
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

/// Opens the composer on the issue or pull request on screen, starting
/// with `text`.
fn comment_with(state: &mut State, text: &str) -> Vec<Cmd> {
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
    let target = ComposeTarget::Conversation {
        subject_id,
        name,
        refresh,
    };
    state.compose(target, text)
}

// ---- the actions menu and hints ----------------------------------------------------------------

/// Something you can do here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Doable {
    pub section: &'static str,
    pub action: Action,
    /// For the menu.
    pub label: String,
    /// For the status bar; empty keeps it out.
    pub short: &'static str,
    pub unavailable: Option<String>,
}

fn doable(
    section: &'static str,
    action: Action,
    label: impl Into<String>,
    short: &'static str,
) -> Doable {
    Doable {
        section,
        action,
        label: label.into(),
        short,
        unavailable: None,
    }
}

/// The diff's menu, section by section, with the status bar's few.
const DIFF_DOABLES: &[(&str, &[(Action, &str)])] = {
    use Action as A;
    &[
        (
            "Move",
            &[
                (A::NextHunk, "change"),
                (A::PrevHunk, ""),
                (A::NextFile, "file"),
                (A::PrevFile, ""),
                (A::NextUnviewed, ""),
                (A::NextThread, ""),
                (A::PrevThread, ""),
                (A::JumpMove, ""),
                (A::FindFile, ""),
                (A::SwitchPane, ""),
            ],
        ),
        (
            "View",
            &[
                (A::ToggleTree, ""),
                (A::ToggleSplit, ""),
                (A::IgnoreWhitespace, ""),
                (A::ExpandContext, ""),
                (A::FullFile, ""),
                (A::ToggleSinceReview, ""),
                (A::PickCommits, ""),
            ],
        ),
        (
            "Review",
            &[
                (A::ToggleViewed, "viewed"),
                (A::MarkReviewed, ""),
                (A::Comment, "comment"),
                (A::VisualLines, ""),
                (A::Suggest, ""),
                (A::FileComment, ""),
                (A::ResolveThread, ""),
                (A::DeleteDraft, ""),
                (A::UndoDelete, ""),
                (A::SubmitReview, "submit review"),
            ],
        ),
    ]
};

impl State {
    /// What you can do on this screen, in sections, most useful first.
    pub fn doables(&self) -> Vec<Doable> {
        let mut out = Vec::new();
        let selected = match self.screen() {
            Screen::Page(p) => {
                self.page_doables(p, &mut out);
                p.selected_url()
            }
            Screen::Diff(_) => {
                for (section, actions) in DIFF_DOABLES {
                    for &(action, short) in *actions {
                        out.push(doable(section, action, action.description(), short));
                    }
                }
                None
            }
        };
        let go = "Go";
        if !self.chrome().tabs.is_empty() {
            out.push(doable(go, Action::NextTab, "Next tab", "tabs"));
        }
        if let Screen::Diff(_) = self.screen() {
            out.push(doable(go, Action::Tab1, "Back to the conversation", ""));
        }
        let (open, copy) = match selected {
            Some(_) => ("Open the selection on GitHub", "Copy the selection's link"),
            None => ("Open this page on GitHub", "Copy this page's link"),
        };
        out.push(doable(go, Action::OpenInBrowser, open, "browser"));
        out.push(doable(go, Action::Copy, copy, "copy link"));
        if let Screen::Page(_) = self.screen() {
            out.push(doable(
                go,
                Action::Hints,
                "Follow any link by its letters",
                "links",
            ));
            out.push(doable(go, Action::UpLevel, "Up a level", ""));
        }
        let mut back = doable(go, Action::Back, "Back", "");
        if self.screens.len() < 2 {
            back.unavailable = Some("this is the first page".into());
        }
        out.push(back);
        let mut fwd = doable(go, Action::Forward, "Forward", "");
        if self.forward.is_empty() {
            fwd.unavailable = Some("nothing to go forward to".into());
        }
        out.push(fwd);
        out.push(doable(go, Action::GoHome, "Home", ""));
        out.push(doable(go, Action::Refresh, "Refresh", ""));
        out.push(doable(go, Action::CommandPalette, "Command palette", ""));
        let mut messages = doable(go, Action::Messages, "Recent messages and errors", "");
        if self.messages.is_empty() {
            messages.unavailable = Some("nothing yet".into());
        }
        out.push(messages);
        for action in [
            Action::Down,
            Action::Up,
            Action::Top,
            Action::Bottom,
            Action::PageDown,
            Action::PageUp,
            Action::HalfPageDown,
            Action::HalfPageUp,
            Action::Quit,
        ] {
            out.push(doable("Keys", action, action.description(), ""));
        }
        out
    }

    fn page_doables(&self, p: &PageScreen, out: &mut Vec<Doable>) {
        let here = "This page";
        let mut open = doable(here, Action::Open, "Open the selected row", "open");
        match p.selected_url() {
            Some(url) => open.label = describe(url),
            None => open.unavailable = Some("nothing is selected".into()),
        }
        out.push(open);
        let route = &p.route;
        let list = route.search().map(|(_, q)| q);
        out.push(match list {
            Some(_) => doable(here, Action::Search, "Filter this list", "filter"),
            None => doable(here, Action::Search, "Search or jump to…", "search"),
        });
        if let Some(query) = &list {
            out.push(doable(
                here,
                Action::ToggleState,
                format!("Show {}", next_list_state(query)),
                "open/closed",
            ));
            out.push(doable(here, Action::Sort, "Change the sort", "sort"));
        }
        if let Some(repo) = route.repo() {
            out.push(doable(here, Action::FindFile, "Go to file", "go to file"));
            if matches!(
                route,
                Route::Repo(_) | Route::Tree { .. } | Route::Blob { .. }
            ) {
                out.push(doable(
                    here,
                    Action::Branch,
                    "Switch branches or tags",
                    "branch",
                ));
                let starred = self.overview(repo).is_some_and(|o| o.starred);
                let label = if starred { "Unstar" } else { "Star" };
                out.push(doable(here, Action::Star, label, "star"));
            }
        }
        if matches!(route, Route::Issue { .. } | Route::Pr { .. }) {
            out.push(doable(here, Action::Comment, "Write a comment", "comment"));
        }
        if let Route::Pr { tab, .. } = route {
            if *tab == PrTab::Conversation {
                out.push(doable(here, Action::Tab2, "Commits", "commits"));
            }
            out.push(doable(
                here,
                Action::Tab4,
                "Files changed (review)",
                "files",
            ));
        }
    }

    /// The status bar's key hints.
    pub fn key_hints(&self) -> Vec<(String, String)> {
        let pair = |keys: &str, what: &str| (keys.to_owned(), what.to_owned());
        match &self.overlay {
            Some(Overlay::Search(sb)) => {
                let enter = if sb.filter { "filter" } else { "go" };
                return vec![pair("↵", enter), pair("↑↓", "choose"), pair("esc", "close")];
            }
            Some(Overlay::Hints(_)) => {
                return vec![pair("a-z", "type a link's letters"), pair("esc", "cancel")];
            }
            Some(Overlay::Menu(menu)) if menu.filter.is_some() => {
                return vec![
                    pair("↵", "run"),
                    pair("↑↓", "choose"),
                    pair("esc", "stop filtering"),
                ];
            }
            Some(Overlay::Menu(_)) => {
                return vec![
                    pair("↵", "run"),
                    pair("key", "run it"),
                    (self.first_key(Action::Search), "filter".into()),
                    pair("esc", "close"),
                ];
            }
            Some(Overlay::Picker(_)) => {
                return vec![
                    pair("↵", "open"),
                    pair("↑↓", "choose"),
                    pair("esc", "close"),
                ];
            }
            Some(_) => return Vec::new(),
            None => {}
        }
        // The menu first: everything else is in it, so it must never be
        // the hint a narrow terminal drops. Then what fits the thing under
        // the cursor, then the screen's usual verbs.
        let menu = (self.first_key(Action::Menu), "menu".to_owned());
        let focused = self.focus_hints();
        let usual = self
            .doables()
            .into_iter()
            .filter(|d| d.unavailable.is_none() && !d.short.is_empty())
            .filter(|d| !focused.iter().any(|(a, _)| *a == d.action))
            .map(|d| (d.action, d.short.to_owned()));
        let hints = focused
            .iter()
            .cloned()
            .chain(usual)
            .filter_map(|(action, what)| {
                // Pairs read as one hint.
                let key = match action {
                    Action::NextTab => Some("←→".to_owned()),
                    Action::NextHunk => Some("n/p".to_owned()),
                    action => self.key_here(action),
                };
                Some((key?, what))
            })
            .take(7);
        std::iter::once(menu).chain(hints).collect()
    }

    /// Verbs for what's under the diff's cursor (a thread, a draft) and for
    /// a review in progress, most useful first.
    fn focus_hints(&self) -> Vec<(Action, String)> {
        let Screen::Diff(screen) = self.screen() else {
            return Vec::new();
        };
        let Some(diff) = self.diffs.get(&screen.pr) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let here = diff
            .doc
            .annotation_at(screen.cursor)
            .and_then(|i| diff.doc.annotations().get(i as usize));
        match here {
            Some(a) if a.is_draft() => {
                out.push((Action::Open, "edit".to_owned()));
                out.push((Action::DeleteDraft, "delete".to_owned()));
            }
            Some(_) => {
                out.push((Action::Comment, "reply".to_owned()));
                out.push((Action::ResolveThread, "resolve".to_owned()));
            }
            None => {}
        }
        let pending = diff.review.pending.len();
        if pending > 0 {
            out.push((Action::SubmitReview, format!("submit ({pending} pending)")));
        }
        out
    }

    /// The first key that runs `action` on this screen, if any.
    pub fn key_here(&self, action: Action) -> Option<String> {
        self.keymap
            .keys_in(action, self.scope())
            .first()
            .map(|k| crate::keymap::pretty(k))
    }

    /// Keys that can follow the pending prefix (which-key).
    pub fn continuations(&self) -> Vec<KeyRow> {
        if self.pending.is_empty() || self.overlay.is_some() {
            return Vec::new();
        }
        self.keymap
            .continuations(&self.pending, self.scope())
            .into_iter()
            .map(|(rest, action)| KeyRow::key(crate::keymap::pretty(&rest), action.description()))
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
        u if u.starts_with(pages::QUOTE) => "Quote reply".into(),
        _ => match Target::from_url(url) {
            Target::Page(r) => format!("Open {}", r.title()),
            Target::Files(pr) => format!("Review {pr}'s files"),
            Target::External(u) => format!("Open {u} in the browser"),
        },
    }
}

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
    fn reselect(&mut self) {
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

pub fn on_menu_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    let scope = state.scope();
    let pressed = [crate::keymap::Key::from(key)];
    let action = match state.keymap.resolve(&pressed, scope) {
        Resolution::Action(action) => Some(action),
        _ => None,
    };
    let Some(Overlay::Menu(menu)) = &mut state.overlay else {
        return Vec::new();
    };
    let last = menu.shown().len().saturating_sub(1);
    let typed = !key.modifiers.contains(KeyModifiers::CONTROL);
    let run = match (&mut menu.filter, key.code) {
        (_, KeyCode::Down) | (None, KeyCode::Char('j')) => {
            menu.selected = (menu.selected + 1).min(last);
            return Vec::new();
        }
        (_, KeyCode::Up) | (None, KeyCode::Char('k')) => {
            menu.selected = menu.selected.saturating_sub(1);
            return Vec::new();
        }
        (_, KeyCode::Enter) | (None, KeyCode::Right) => {
            menu.shown().get(menu.selected).map(|d| (*d).clone())
        }
        // Filtering: letters narrow the list instead of running rows.
        (Some(query), code) => {
            match code {
                KeyCode::Char(c) if typed => query.push(c),
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
        (None, KeyCode::Esc | KeyCode::Char('q') | KeyCode::Left) => None,
        (None, _) if action == Some(Action::Menu) => None,
        // A row's own key runs it.
        (None, _) => {
            let row = menu.rows.iter().find(|d| {
                state
                    .keymap
                    .keys_in(d.action, scope)
                    .iter()
                    .any(|k| *k == pressed)
            });
            match row {
                Some(d) => Some(d.clone()),
                None => return Vec::new(),
            }
        }
    };
    run_menu_row(state, run)
}

/// Runs a menu row (`None` closes the menu).
fn run_menu_row(state: &mut State, row: Option<Doable>) -> Vec<Cmd> {
    let Some(d) = row else {
        state.overlay = None;
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

impl State {
    /// The link of the selection, or of the page.
    pub fn here_url(&self) -> String {
        match self.screen() {
            Screen::Page(p) => p
                .selected_url()
                .filter(|u| u.starts_with("http"))
                .map_or_else(|| p.route.url(), str::to_owned),
            Screen::Diff(d) => format!("{}/files", d.pr.url()),
        }
    }
}

/// `y`: the selection's link, or the page's.
pub fn copy_link(state: &mut State) -> Vec<Cmd> {
    let url = state.here_url();
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
                Screen::Page(p) => {
                    scroll_by(p, if down { STEP as i64 } else { -(STEP as i64) }, height)
                }
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
                selected: sb.selected,
                field,
            };
            let screen = Rect::new(0, 0, state.size.0, state.size.1);
            if inside(field) {
                return Vec::new();
            }
            if inside(panel.area(screen)) {
                let row = panel.row_at(screen, y);
                if let Some((_, Some(pick))) = row.and_then(|r| rows.get(r)).cloned() {
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
    if let Some(url) = hit.link.and_then(|l| p.page.links.get(l as usize)) {
        let url = url.clone();
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
        let mut sorted = many;
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
}
