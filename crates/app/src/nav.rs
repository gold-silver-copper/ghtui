//! Moving around pages, the way you'd use the website: selecting rows,
//! scrolling, following links (by keys, letters or the mouse), tabs,
//! history, the search box with its suggestions, go to file, branches, the
//! actions menu, and the keys hinted in the status bar.

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ghtui_api::browse::{DiscussionsOf, RepoSummary, SearchKind};
use ghtui_api::model::RepoId;
use ghtui_theme::{Bg, Theme};
use ghtui_ui::chrome::{self, KeyRow, SuggestRow};
use ghtui_ui::page::{self, HintLabel, Link};
use ghtui_ui::pages::{self, PrTab, ProfileTab};
use ghtui_ui::{PAD_X, PAD_Y};
use ratatui::layout::Rect;
use ratatui_textarea::TextArea;
use serde::{Deserialize, Serialize};

use crate::browse::{self, Data, DataKey, PageScreen};
use crate::diff_screen::DiffOf;
use crate::keymap::Action;
use crate::picker::{fuzzy_score, move_in_list};
use crate::review::ComposeTarget;
use crate::route::{self, Route, Target};
use crate::state::{Api, Cmd, Overlay, Remote, Screen, State, apply};

/// Rows `j`/`k` scroll when there's no row to move to nearby.
const STEP: isize = 3;
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
    #[must_use]
    pub(crate) fn record_visit(&mut self, route: &Route) -> Vec<Cmd> {
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
        vec![Cmd::Api(Api::SaveVisits(self.visits.clone()))]
    }

    /// Navigates to a page, keeping the current one in history.
    #[must_use]
    pub fn push(&mut self, route: Route) -> Vec<Cmd> {
        self.forward.clear();
        let mut cmds = self.record_visit(&route);
        self.screens
            .push(Screen::Page(Box::new(PageScreen::new(route, None))));
        if let Some(route) = self.route().cloned() {
            cmds.extend(self.ensure_route(&route, true));
        }
        cmds
    }

    /// Replaces the page on screen (switching tabs, changing a filter).
    #[must_use]
    pub fn replace(&mut self, route: Route, force: bool, anchor: Option<String>) -> Vec<Cmd> {
        match self.screen_mut() {
            Screen::Page(p) => **p = PageScreen::new(route.clone(), anchor),
            Screen::Diff(_) => return self.push(route),
        }
        let mut cmds = self.ensure_route(&route, force);
        cmds.extend(self.record_visit(&route));
        cmds
    }

    #[must_use]
    pub fn back(&mut self) -> Vec<Cmd> {
        if let Some(screen) = self.screens.pop() {
            self.forward.push(screen);
            return self.load_visible(false);
        }
        self.info("This is the first page");
        Vec::new()
    }

    #[must_use]
    pub fn go_forward(&mut self) -> Vec<Cmd> {
        match self.forward.pop() {
            Some(screen) => {
                self.screens.push(screen);
                self.load_visible(false)
            }
            None => {
                self.info("Nothing to go forward to");
                Vec::new()
            }
        }
    }

    /// Follows a link.
    #[must_use]
    pub fn go(&mut self, target: Target) -> Vec<Cmd> {
        match target {
            Target::Page(route) => self.push(route),
            Target::Files(of) => {
                self.forward.clear();
                let mut cmds = match &of {
                    DiffOf::Pr(pr) => self.ensure_pr(pr, false),
                    DiffOf::Commit(..) | DiffOf::Range(..) => Vec::new(),
                };
                cmds.extend(self.open_diff(of));
                cmds
            }
            Target::External(url) => {
                self.info(format!("Opened {url} in the browser"));
                vec![Cmd::OpenUrl(url)]
            }
        }
    }

    /// Opens a URL (in a new tab with `tab`), going once it's loaded to
    /// what its `#fragment` names: a comment, a file in a diff, a review
    /// comment's thread.
    #[must_use]
    pub fn open_url(&mut self, url: &str, tab: bool) -> Vec<Cmd> {
        let target = Target::from_url(url);
        let external = matches!(target, Target::External(_));
        let cmds = if tab {
            self.open_tab(target)
        } else {
            self.go(target)
        };
        if !external {
            self.anchor_at(url);
        }
        cmds
    }

    /// Notes `url`'s `#fragment` on the screen it opened.
    pub fn anchor_at(&mut self, url: &str) {
        let with_scheme = if url.starts_with("github.com/") {
            format!("https://{url}")
        } else {
            url.to_owned()
        };
        let fragment = url::Url::parse(&with_scheme)
            .ok()
            .and_then(|u| u.fragment().map(str::to_owned));
        match (fragment, self.screen_mut()) {
            (Some(fragment), Screen::Page(p)) => p.anchor = Some(fragment),
            (Some(fragment), Screen::Diff(d)) => d.anchor = Some(fragment),
            _ => {}
        }
    }

    /// Follows a link on the page on screen: a URL, or one of the page's
    /// actions.
    #[must_use]
    pub fn follow(&mut self, link: &Link) -> Vec<Cmd> {
        match link {
            Link::Url(url) => self.open_url(url, false),
            Link::More => self.load_more(),
            Link::Star => star(self),
            Link::Comment => comment_with(self, ""),
            Link::Filter => self.open_search(),
            Link::Sort => cycle_sort(self),
            Link::Branch => self.open_finder(true),
            Link::FindFile => self.open_finder(false),
            Link::State(state) => set_list_state(self, state),
            Link::Quote(n) => {
                let quote = match self.screen() {
                    Screen::Page(p) => p.page.quotes.get(*n as usize).cloned(),
                    Screen::Diff(_) => None,
                };
                let Some(quote) = quote else {
                    return Vec::new();
                };
                let mut text = format!("> @{} wrote:\n", quote.author);
                for line in quote.body.trim().lines() {
                    text.push_str(&format!("> {line}\n"));
                }
                text.push('\n');
                comment_with(self, &text)
            }
        }
    }

    #[must_use]
    fn load_more(&mut self) -> Vec<Cmd> {
        let Some(key) = self.route().and_then(browse::paged) else {
            return Vec::new();
        };
        let Some(remote) = self.data.get_mut(&key) else {
            return Vec::new();
        };
        let after = match &remote.data {
            Some(data) if !remote.loading && !remote.loading_more => data.next_cursor(),
            _ => None,
        };
        let Some(after) = after.map(str::to_owned) else {
            return Vec::new();
        };
        remote.loading_more = true;
        vec![Cmd::Api(Api::FetchMore { key, after })]
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
        let built = Some((self.data_gen, width, self.compact()));
        let height = self.page_height();
        let rebuild = match self.screen() {
            Screen::Page(p) if p.built != built => Some(p.route.clone()),
            Screen::Page(_) => None,
            Screen::Diff(_) => return,
        };
        let page = rebuild.map(|route| self.build_page(&route, width, (self.clock)()));
        if let Screen::Page(p) = self.screen_mut() {
            if let Some(page) = page {
                p.page = Arc::new(page);
                p.built = built;
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
    let anchor = p
        .anchor
        .as_ref()
        .and_then(|a| p.page.anchors.get(a))
        .copied();
    if !p.jumped
        && let Some(jump) = p.page.jump.or(anchor)
    {
        p.scroll = jump.saturating_sub(MARGIN);
        p.jumped = true;
    }
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
        None if down => scroll_by(p, STEP, height),
        None => scroll_by(p, -STEP, height),
    }
}

/// Scrolls; a selection that leaves the screen is dropped.
fn scroll_by(p: &mut PageScreen, rows: isize, height: usize) {
    p.fresh = false;
    let max = p.page.height().saturating_sub(height);
    p.scroll = p.scroll.saturating_add_signed(rows).min(max);
    if p.selected.is_some_and(|s| !visible(p, s, height)) {
        p.selected = None;
    }
}

/// Pages and jumps keep a selection on screen when there's one to keep.
/// When the view can't scroll that way, the selection goes to the edge.
fn scroll_keep(p: &mut PageScreen, rows: isize, height: usize) {
    let had = p.selected.is_some();
    let scroll = p.scroll;
    scroll_by(p, rows, height);
    if (had && p.selected.is_none()) || p.scroll == scroll {
        let mut visible_items = (0..p.page.items.len()).filter(|&i| visible(p, i, height));
        p.selected = if rows > 0 {
            visible_items.next()
        } else {
            visible_items.next_back()
        };
    }
}

// ---- keys on pages ----------------------------------------------------------------------

/// Actions on a page screen. Those that don't work on pages say where
/// they do.
#[must_use]
pub fn page_action(state: &mut State, action: Action) -> Vec<Cmd> {
    let height = state.page_height();
    let Screen::Page(p) = state.screen_mut() else {
        return Vec::new();
    };
    let half = (height / 2).max(1).cast_signed();
    let page = height.saturating_sub(2).max(1).cast_signed();
    match action {
        Action::Down => step(p, height, true),
        Action::Up => step(p, height, false),
        Action::HalfPageDown => scroll_keep(p, half, height),
        Action::HalfPageUp => scroll_keep(p, -half, height),
        Action::PageDown => scroll_keep(p, page, height),
        Action::PageUp => scroll_keep(p, -page, height),
        Action::Top => {
            scroll_by(p, isize::MIN, height);
            p.selected = (!p.page.items.is_empty() && visible(p, 0, height)).then_some(0);
        }
        Action::Bottom => {
            scroll_by(p, isize::MAX, height);
            let last = p.page.items.len().checked_sub(1);
            p.selected = last.filter(|&l| visible(p, l, height));
        }
        Action::Open => {
            return match p.selected_link().cloned() {
                Some(link) => state.follow(&link),
                None => {
                    let (down, hints) = (
                        state.first_key(Action::Down),
                        state.first_key(Action::Hints),
                    );
                    state.info(format!(
                        "Nothing selected: {down} selects a row, {hints} picks any link"
                    ));
                    Vec::new()
                }
            };
        }
        Action::Hints | Action::HintsBrowser => {
            start_hints(state, action == Action::HintsBrowser);
        }
        Action::Star => return star(state),
        Action::Comment => return comment_with(state, ""),
        Action::Branch => return state.open_finder(true),
        Action::ToggleState => return cycle_state(state),
        Action::Sort => return cycle_sort(state),
        _ => state.info(action.not_here()),
    }
    Vec::new()
}

/// Goes to the page above this one.
#[must_use]
pub fn up_level(state: &mut State) -> Vec<Cmd> {
    let Some(route) = state.route().cloned() else {
        return Vec::new();
    };
    match up(state, &route) {
        Some(parent) => state.push(parent),
        None => {
            state.info("Already at the top");
            Vec::new()
        }
    }
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
        Route::Tree { repo, rev, path }
        | Route::Blob {
            repo, rev, path, ..
        } => folder(repo, rev, path),
        Route::Blame {
            repo, rev, path, ..
        } => Route::blob(repo.clone(), rev.clone(), path.clone()),
        Route::Issues { repo, .. }
        | Route::Discussions {
            of: DiscussionsOf::Repo(repo),
            ..
        }
        | Route::Pulls { repo, .. }
        | Route::Commits { repo, .. }
        | Route::Stargazers(repo)
        | Route::Watchers(repo)
        | Route::Forks(repo)
        | Route::Releases(repo)
        | Route::Tags(repo)
        | Route::Branches(repo)
        | Route::Compare { repo, .. }
        | Route::Deployments { repo, .. }
        | Route::Milestones { repo, .. }
        | Route::Wiki { repo, page: None }
        | Route::Actions(repo) => Route::Repo(repo.clone()),
        Route::Discussion { of, .. } => Route::Discussions {
            of: of.clone(),
            category: None,
        },
        Route::Discussions {
            of: DiscussionsOf::Org(org),
            ..
        } => Route::user(org),
        Route::Release { repo, .. } => Route::Releases(repo.clone()),
        Route::Milestone { repo, .. } => Route::Milestones {
            repo: repo.clone(),
            closed: false,
        },
        Route::WorkflowRun { repo, .. }
        | Route::Workflow { repo, .. }
        | Route::Job {
            repo, run: None, ..
        } => Route::Actions(repo.clone()),
        Route::Job {
            repo,
            run: Some(run),
            ..
        } => Route::WorkflowRun {
            repo: repo.clone(),
            run: *run,
            attempt: None,
        },
        Route::CommitChecks { repo, oid } => Route::Commit {
            repo: repo.clone(),
            oid: oid.clone(),
        },
        // A commit's history leads up to it.
        Route::Commit { repo, oid } => Route::Commits {
            repo: repo.clone(),
            rev: oid.clone(),
            path: String::new(),
        },
        Route::Issue { repo, .. } => Route::Issues {
            repo: repo.clone(),
            query: crate::route::OPEN.into(),
        },
        Route::Pr { pr, .. } => Route::Pulls {
            repo: pr.repo.clone(),
            query: crate::route::OPEN.into(),
        },
        Route::User { .. } | Route::Search { .. } => Route::Home,
        Route::Gist { owner, .. } => Route::Gists(owner.clone()?),
        Route::Gists(login) | Route::Teams(login) => Route::user(login),
        Route::Team { org, .. } => Route::Teams(org.clone()),
        Route::Wiki {
            repo,
            page: Some(_),
        } => Route::Wiki {
            repo: repo.clone(),
            page: None,
        },
        Route::Advisory { repo, .. } => Route::Advisories(repo.clone()),
        Route::Advisories(repo) => repo.clone().map_or(Route::Home, Route::Repo),
    })
}

/// Switches to the chrome's tab `n` (1-based).
#[must_use]
pub fn switch_tab(state: &mut State, n: usize) -> Vec<Cmd> {
    let chrome = state.chrome();
    let Some((tab, target)) = n.checked_sub(1).and_then(|i| chrome.tabs.get(i)).cloned() else {
        state.info(format!("No tab {n} here"));
        return Vec::new();
    };
    if chrome.active == Some(n - 1) {
        state.info(format!("Already on {}", tab.label));
        return Vec::new();
    }
    let diff_of = match state.screen() {
        Screen::Diff(screen) => Some(screen.of.clone()),
        Screen::Page(_) => None,
    };
    match (target, diff_of) {
        (Target::Page(route), None) => state.replace(route, false, None),
        (Target::Page(route), Some(diff_pr)) => {
            // From the files back to the pull request's other tabs.
            state.screens.pop();
            let same = match (state.route(), &diff_pr) {
                (Some(Route::Pr { pr, .. }), DiffOf::Pr(diff)) => pr == diff,
                (Some(Route::Commit { repo, oid }), DiffOf::Commit(r, full)) => {
                    repo == r && full.starts_with(oid.as_str())
                }
                (Some(Route::Compare { repo, .. }), DiffOf::Range(r, ..)) => repo == r,
                _ => false,
            };
            if same {
                state.replace(route, false, None)
            } else {
                state.push(route)
            }
        }
        (target, _) => state.go(target),
    }
}

#[must_use]
pub fn step_tab(state: &mut State, forward: bool) -> Vec<Cmd> {
    let chrome = state.chrome();
    let n = chrome.tabs.len();
    // The first tab in that direction, wrapping around, that isn't a link.
    let next = chrome.active.and_then(|active| {
        (1..n)
            .map(|k| if forward { active + k } else { active + n - k })
            .map(|i| i % n)
            .find(|&i| chrome.tabs.get(i).is_some_and(|(t, _)| !t.external))
    });
    match next {
        Some(i) => switch_tab(state, i + 1),
        None => {
            state.info("No other tabs here");
            Vec::new()
        }
    }
}

// ---- letter hints -----------------------------------------------------------------------------

pub struct Hints {
    pub labels: Vec<HintLabel>,
    pub links: Vec<Link>,
    pub typed: String,
    pub browser: bool,
    /// A capital was typed: follow the link in a new tab.
    pub tab: bool,
}

const LETTERS: &str = "asdfghjklqwertyuiopzxcvbnm";

/// Labels for `n` targets: single letters while they last, then pairs.
fn hint_labels(n: usize) -> Vec<String> {
    let letters: Vec<char> = LETTERS.chars().collect();
    if n <= letters.len() {
        return letters
            .iter()
            .take(n)
            .map(std::string::ToString::to_string)
            .collect();
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
        state.info("No links on screen");
        return;
    }
    let labels = hint_labels(spots.len());
    let (spots, links): (Vec<&page::Spot>, Vec<Link>) = spots
        .iter()
        .filter_map(|s| Some((s, p.page.target(s.link)?.clone())))
        .unzip();
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
        tab: false,
    })));
}

#[must_use]
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
        // A capital follows the link in a new tab.
        KeyCode::Char(c) if c.is_ascii_uppercase() => {
            hints.typed.push(c.to_ascii_lowercase());
            hints.tab = true;
        }
        _ => {
            state.overlay = None;
            return Vec::new();
        }
    }
    let matching: Vec<(&HintLabel, &Link)> = hints
        .labels
        .iter()
        .zip(&hints.links)
        .filter(|(h, _)| h.label.starts_with(&hints.typed))
        .collect();
    match matching.as_slice() {
        [] => {
            state.overlay = None;
            state.info("No link has those letters");
            Vec::new()
        }
        [(hint, link)] if hint.label == hints.typed => {
            let link = (*link).clone();
            let (browser, tab) = (hints.browser, hints.tab);
            state.overlay = None;
            match link {
                Link::Url(url) if browser => state.go(Target::External(url)),
                Link::Url(url) if tab => state.open_url(&url, true),
                link => state.follow(&link),
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
    Url(String),
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
    #[must_use]
    pub fn open_search(&mut self) -> Vec<Cmd> {
        let filter_query = self.route().and_then(Route::list_query).map(str::to_owned);
        let mut input = new_input(&self.theme, Bg::ContainerHighest);
        if let Some(q) = &filter_query {
            input.insert_str(q);
            input.insert_char(' ');
        }
        // A profile's repositories filter by searching that owner's.
        if let Some(Route::User {
            login,
            tab: ProfileTab::Repositories(_),
        }) = self.route()
        {
            input.insert_str(format!("user:{login} "));
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
        let mut out = Vec::new();
        if sb.filter {
            // A job's log filters by its text alone.
            if let Some(Route::Job { .. }) = self.route() {
                let pick = Pick::Filter(q.to_owned());
                let what = if q.is_empty() {
                    "show the whole log"
                } else {
                    "lines with this"
                };
                out.push(suggestion("⌕", q, what, "↵", pick));
                return out;
            }
            if !q.is_empty() {
                let pick = Pick::Filter(q.to_owned());
                out.push(suggestion("⌕", q, "filter this list", "↵", pick));
            }
            out.push(heading("Quick filters"));
            for (f, what) in [
                ("is:open", "open"),
                ("is:closed", "closed"),
                ("is:open author:@me", "yours"),
                ("is:open assignee:@me", "assigned to you"),
                ("is:open mentions:@me", "mentioning you"),
                ("is:open no:assignee", "unassigned"),
            ] {
                out.push(suggestion("⚑", f, what, "", Pick::Filter(f.to_owned())));
            }
            if !q.is_empty() {
                out.push(heading("Search all of GitHub"));
                let pick = Pick::Search(SearchKind::Issues, q.to_owned());
                out.push(suggestion("⌕", q, "issues and pull requests", "", pick));
            }
            return out;
        }
        if let Some(target) = route::parse_input(q, self.context_repo()) {
            let label = match &target {
                Target::Page(r) => r.title(),
                Target::Files(of) => format!("Files changed in {of}"),
                Target::External(url) => url.clone(),
            };
            // A URL keeps its `#fragment`.
            let pick = if q.contains("://") || q.starts_with("github.com/") {
                Pick::Url(q.to_owned())
            } else {
                Pick::Go(target)
            };
            out.push(suggestion("→", label, "go to", "↵", pick));
        }
        // Jump to: pages you've visited, your repositories, live matches.
        let current = self.route().map(Route::url);
        let now = (self.clock)();
        let mut visits: Vec<&Visit> = self
            .visits
            .iter()
            .filter(|v| Some(&v.url) != current.as_ref())
            .collect();
        visits.sort_by_key(|v| std::cmp::Reverse(v.score(now)));
        let mut jumps: Vec<(usize, String, String, &str, &str)> = visits
            .into_iter()
            .filter_map(|v| {
                let score = fuzzy_score(q, &v.title)?;
                Some((score, v.title.clone(), v.url.clone(), "◷", "visited"))
            })
            .collect();
        if let Some(Data::Repos(repos)) = self.get(&DataKey::ViewerRepos) {
            for r in repos {
                let name = r.repo.to_string();
                if let Some(score) = fuzzy_score(q, &name) {
                    let url = pages::url::repo(&r.repo);
                    jumps.push((score + 1, name, url, "▤", "your repository"));
                }
            }
        }
        // Most recent first, as they came, until something is typed.
        if !q.is_empty() {
            jumps.sort_by_key(|(score, ..)| *score);
        }
        let most = if q.is_empty() { 8 } else { 5 };
        let mut seen: Vec<String> = Vec::new();
        for (_, title, url, icon, detail) in jumps {
            if seen.contains(&url) || seen.len() >= most {
                continue;
            }
            if seen.is_empty() {
                out.push(heading(if q.is_empty() { "Recent" } else { "Jump to" }));
            }
            let pick = Pick::Go(Target::from_url(&url));
            out.push(suggestion(icon, title, detail, "", pick));
            seen.push(url);
        }
        if !q.is_empty() {
            out.push(heading("Search"));
            if let Some(repo) = self.context_repo() {
                let pick = Pick::Search(SearchKind::Issues, format!("repo:{repo} {q}"));
                out.push(suggestion("⌕", q, &format!("in {repo}"), "", pick));
            }
            for (kind, what) in [
                (SearchKind::Repos, "repositories"),
                (SearchKind::Issues, "issues"),
                (SearchKind::Pulls, "pull requests"),
                (SearchKind::Users, "users"),
            ] {
                let pick = Pick::Search(kind, q.to_owned());
                out.push(suggestion("⌕", q, what, "", pick));
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
                out.push(heading("Repositories"));
            }
            for r in live {
                let detail = format!("☆ {}", pages::compact(r.stars));
                let pick = Pick::Go(Target::Page(Route::Repo(r.repo.clone())));
                out.push(suggestion("▤", r.repo.to_string(), &detail, "", pick));
            }
        }
        out
    }
}

/// A suggestion to choose.
fn suggestion(
    icon: &str,
    label: impl Into<String>,
    detail: &str,
    hint: &str,
    pick: Pick,
) -> (SuggestRow, Option<Pick>) {
    let row = SuggestRow::Item {
        icon: icon.to_owned(),
        label: label.into(),
        detail: detail.to_owned(),
        hint: hint.to_owned(),
    };
    (row, Some(pick))
}

fn heading(text: &str) -> (SuggestRow, Option<Pick>) {
    (SuggestRow::Heading(text.to_owned()), None)
}

#[must_use]
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

#[must_use]
fn choose(state: &mut State, pick: Pick) -> Vec<Cmd> {
    match pick {
        Pick::Go(target) => state.go(target),
        Pick::Url(url) => state.open_url(&url, false),
        Pick::Search(kind, query) => state.push(Route::Search { kind, query }),
        Pick::Filter(query) => match state.route().and_then(|r| r.with_query(query.clone())) {
            Some(route) => state.replace(route, true, None),
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

#[must_use]
fn set_list_state(state: &mut State, which: &str) -> Vec<Cmd> {
    let Some((route, query)) = list_query(state) else {
        return Vec::new();
    };
    match route.with_query(with_state(&query, which)) {
        Some(r) if r != route => state.replace(r, true, None),
        _ => Vec::new(),
    }
}

#[must_use]
fn cycle_state(state: &mut State) -> Vec<Cmd> {
    let Some((_, query)) = list_query(state) else {
        state.info("Only lists have open and closed");
        return Vec::new();
    };
    set_list_state(state, next_list_state(&query))
}

#[must_use]
fn cycle_sort(state: &mut State) -> Vec<Cmd> {
    // A profile's repositories sort by GraphQL order, not a search word.
    if let Some(Route::User {
        login,
        tab: ProfileTab::Repositories(sort),
    }) = state.route()
    {
        let next = sort.next();
        let route = Route::User {
            login: login.clone(),
            tab: ProfileTab::Repositories(next),
        };
        state.info(format!("Sorted: {}", next.label()));
        return state.replace(route, true, None);
    }
    let Some((route, query)) = list_query(state) else {
        state.info("Only lists can be sorted");
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
    state.info(format!("Sorted: {label}"));
    match route.with_query(words.join(" ")) {
        Some(r) => state.replace(r, true, None),
        None => Vec::new(),
    }
}

// ---- writing: star and comment ----------------------------------------------------------------

#[must_use]
pub fn star(state: &mut State) -> Vec<Cmd> {
    let Some(repo) = state.route().and_then(Route::repo).cloned() else {
        state.info("Open a repository to star it");
        return Vec::new();
    };
    let Some(overview) = state.overview(&repo) else {
        state.info("The repository hasn't loaded yet");
        return Vec::new();
    };
    let (id, starred) = (overview.id.clone(), !overview.starred);
    set_starred(state, &repo, starred);
    vec![Cmd::Api(Api::SetStarred { repo, id, starred })]
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
#[must_use]
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
            state.info("Comments go on issues and pull requests");
            return Vec::new();
        }
    };
    let Some((subject_id, name, refresh)) = target else {
        state.info("Still loading");
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

fn doable(section: &'static str, action: Action, label: &str, short: &'static str) -> Doable {
    Doable {
        section,
        action,
        label: label.to_owned(),
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
                p.selected_link()
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
        let go = |action, label: &str, short| doable("Go", action, label, short);
        if !self.chrome().tabs.is_empty() {
            out.push(go(Action::NextTab, "Next tab", "tabs"));
        }
        if let Screen::Diff(_) = self.screen() {
            out.push(go(Action::Tab1, "Back to the conversation", ""));
        }
        let (open, copy) = match selected {
            Some(_) => ("Open the selection on GitHub", "Copy the selection's link"),
            None => ("Open this page on GitHub", "Copy this page's link"),
        };
        out.push(go(Action::OpenInBrowser, open, "browser"));
        out.push(go(Action::Copy, copy, "copy link"));
        if let Screen::Page(_) = self.screen() {
            out.push(go(Action::Hints, "Follow any link by its letters", "links"));
            out.push(go(Action::UpLevel, "Up a level", ""));
        }
        let mut back = go(Action::Back, "Back", "");
        if self.screens.len() < 2 {
            back.unavailable = Some("this is the first page".into());
        }
        out.push(back);
        let mut fwd = go(Action::Forward, "Forward", "");
        if self.forward.is_empty() {
            fwd.unavailable = Some("nothing to go forward to".into());
        }
        out.push(fwd);
        out.push(go(Action::GoHome, "Home", ""));
        out.push(go(Action::Refresh, "Refresh", ""));
        out.push(go(Action::CommandPalette, "Command palette", ""));
        let mut messages = go(Action::Messages, "Recent messages and errors", "");
        if self.messages.is_empty() {
            messages.unavailable = Some("nothing yet".into());
        }
        out.push(messages);
        let tab = |action, label: &str| doable("Tabs", action, label, "");
        out.push(tab(Action::OpenInTab, "Open in a new tab"));
        if self.tab_count() > 1 {
            out.push(tab(Action::NextOpenTab, "Next open tab"));
            out.push(tab(Action::PrevOpenTab, "Previous open tab"));
            out.push(tab(Action::CloseTab, "Close the tab"));
        }
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
        let here = |action, label: &str, short| doable("This page", action, label, short);
        let mut open = here(Action::Open, "Open the selected row", "open");
        match p.selected_link() {
            Some(link) => open.label = describe(link),
            None => open.unavailable = Some("nothing is selected".into()),
        }
        out.push(open);
        let route = &p.route;
        let list = route.search().map(|(_, q)| q);
        out.push(match list {
            Some(_) => here(Action::Search, "Filter this list", "filter"),
            None => here(Action::Search, "Search or jump to…", "search"),
        });
        if let Some(query) = &list {
            let show = format!("Show {}", next_list_state(query));
            out.push(here(Action::ToggleState, &show, "open/closed"));
            out.push(here(Action::Sort, "Change the sort", "sort"));
        }
        if let Some(repo) = route.repo() {
            out.push(here(Action::FindFile, "Go to file", "go to file"));
            if matches!(
                route,
                Route::Repo(_) | Route::Tree { .. } | Route::Blob { .. }
            ) {
                out.push(here(Action::Branch, "Switch branches or tags", "branch"));
                let starred = self.overview(repo).is_some_and(|o| o.starred);
                let label = if starred { "Unstar" } else { "Star" };
                out.push(here(Action::Star, label, "star"));
            }
        }
        if matches!(route, Route::Issue { .. } | Route::Pr { .. }) {
            out.push(here(Action::Comment, "Write a comment", "comment"));
        }
        if let Route::Pr { tab, .. } = route {
            if *tab == PrTab::Conversation {
                out.push(here(Action::Tab2, "Commits", "commits"));
            }
            out.push(here(Action::Tab4, "Files changed (review)", "files"));
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
        let Some(diff) = self.diffs.get(&screen.of) else {
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
        let pending = diff.inputs().review.pending.len();
        if pending > 0 {
            out.push((Action::SubmitReview, format!("submit ({pending} pending)")));
        }
        out
    }

    /// The first key that runs `action` on this screen, if any.
    pub fn key_here(&self, action: Action) -> Option<String> {
        self.keymap
            .keys_in(action, self.scope())
            .next()
            .map(crate::keymap::pretty)
    }
}

/// What following `link` does, for the menu.
fn describe(link: &Link) -> String {
    match link {
        Link::Url(url) => match Target::from_url(url) {
            Target::Page(r) => format!("Open {}", r.title()),
            Target::Files(of) => format!("Files changed in {of}"),
            Target::External(u) => format!("Open {u} in the browser"),
        },
        Link::More => "Load more".into(),
        Link::Comment => "Write a comment".into(),
        Link::Star => "Star or unstar".into(),
        Link::Filter => "Edit the filter".into(),
        Link::Sort => "Change the sort".into(),
        Link::State(state) => format!("Show {state}"),
        Link::Branch => "Switch branches".into(),
        Link::FindFile => "Go to file".into(),
        Link::Quote(_) => "Quote reply".into(),
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

#[must_use]
pub fn on_menu_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    let scope = state.scope();
    let pressed = crate::keymap::Key::from(key);
    let action = state.keymap.resolve(pressed, scope);
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
        (_, KeyCode::Enter) | (None, KeyCode::Right | KeyCode::Char('l')) => {
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
        (None, KeyCode::Esc | KeyCode::Char('q') | KeyCode::Left | KeyCode::Char('h')) => None,
        (None, _) if action == Some(Action::Menu) => None,
        // A row's own key runs it.
        (None, _) => {
            let row = menu
                .rows
                .iter()
                .find(|d| state.keymap.keys_in(d.action, scope).any(|k| k == pressed));
            match row {
                Some(d) => Some(d.clone()),
                None => return Vec::new(),
            }
        }
    };
    run_menu_row(state, run)
}

/// Runs a menu row (`None` closes the menu).
#[must_use]
fn run_menu_row(state: &mut State, row: Option<Doable>) -> Vec<Cmd> {
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

// ---- copying -------------------------------------------------------------------------------------

impl State {
    /// The link of the selection, or of the page.
    pub fn here_url(&self) -> String {
        match self.screen() {
            Screen::Page(p) => p
                .selected_link()
                .and_then(Link::url)
                .map_or_else(|| p.route.url(), str::to_owned),
            Screen::Diff(d) => match &d.of {
                DiffOf::Pr(pr) => format!("{}/files", pr.url()),
                DiffOf::Commit(repo, oid) => pages::url::commit(repo, oid),
                DiffOf::Range(..) => crate::route::compare_url(&d.of),
            },
        }
    }
}

/// `y`: the selection's link, or the page's.
#[must_use]
pub fn copy_link(state: &mut State) -> Vec<Cmd> {
    let url = state.here_url();
    state.info(format!("Copied {url}"));
    vec![Cmd::Copy(url)]
}

// ---- the mouse -------------------------------------------------------------------------------------

#[must_use]
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
                    scroll_by(p, if down { STEP } else { -STEP }, height);
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

#[must_use]
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
                let row = panel.row_at(screen, y).and_then(|r| rows.get(r));
                let Some((_, Some(pick))) = row.cloned() else {
                    return Vec::new();
                };
                state.overlay = None;
                return choose(state, pick);
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
            .map_or_else(Vec::new, |target| state.go(target));
    }
    if let Some(tabs) = lay.tabs
        && inside(tabs)
    {
        let tab_list: Vec<_> = chrome.tabs.iter().map(|(t, _)| t.clone()).collect();
        let rects = chrome::tab_layout(tabs, &tab_list, chrome.active);
        return match rects.iter().position(|r| x >= r.x && x < r.right()) {
            Some(i) => switch_tab(state, i + 1),
            None => Vec::new(),
        };
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
    if let Some(link) = hit.link.and_then(|l| p.page.target(l)) {
        let link = link.clone();
        p.selected = item.or(p.selected);
        return state.follow(&link);
    }
    match item {
        // A second click on a row opens it.
        Some(i) if p.selected == Some(i) => {
            let link = p.selected_link().cloned();
            link.map_or_else(Vec::new, |l| state.follow(&l))
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
            Screen::Diff(d) => Some(d.of.repo()),
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
