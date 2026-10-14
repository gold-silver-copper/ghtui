//! History: the pages you visited, the page above this one, and the
//! page's tabs.

use super::*;
use crate::route::{app, code};

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
    pub(super) fn score(&self, now: u64) -> u64 {
        let days = now.saturating_sub(self.last) / 86_400;
        u64::from(self.count) * 100 / (days + 1)
    }
}

impl State {
    #[must_use]
    pub(crate) fn record_visit(&mut self, route: &Route) -> Vec<Cmd> {
        // An unsplit page is recorded as the page it turns out to be.
        if matches!(
            route,
            Route::App(app::Home) | Route::Code(code::Unsplit { .. })
        ) {
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
        self.go(Target::Page(route).into())
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
            None => self.just_say("Nothing to go forward to"),
        }
    }

    /// Follows a link, going once it's loaded to what its `#fragment`
    /// names: a comment, a file in a diff, a review comment's thread.
    #[must_use]
    pub fn go(&mut self, Dest { target, anchor }: Dest) -> Vec<Cmd> {
        match target {
            Target::Page(route) => {
                let route = self.followed(route);
                self.forward.clear();
                let mut cmds = self.record_visit(&route);
                let screen = PageScreen::new(route, anchor);
                self.screens.push(Screen::Page(Box::new(screen)));
                if let Some(route) = self.route().cloned() {
                    cmds.extend(self.ensure_route(&route, true));
                }
                cmds
            }
            Target::Files(of) => {
                self.forward.clear();
                self.open_diff(of, anchor)
            }
            Target::External(url) => {
                self.info(format!("Opened {url} in the browser"));
                vec![Cmd::OpenUrl(url)]
            }
        }
    }

    /// Follows a link on the page on screen: a URL, or one of the page's
    /// actions.
    #[must_use]
    pub fn follow(&mut self, link: &Link) -> Vec<Cmd> {
        match link {
            Link::Url(url) => self.go(Dest::from_url(url)),
            Link::More => self.load_more(),
            Link::Star => star(self),
            Link::Comment => comment_with(self, "", None),
            Link::Filter => self.open_search(false),
            Link::Sort => cycle_sort(self),
            Link::Branch => self.open_finder(true),
            Link::FindFile => self.open_finder(false),
            Link::Section(i) => self.open_section(*i),
            Link::Form(row) => crate::forms::row(self, row),
            Link::Artifact { id, name } => crate::workflows::save(self, *id, name),
            Link::Asset(name) => crate::release::assets::open(self, name),
            Link::Review(link) => crate::talk::follow(self, link),
            Link::Setting(link) => crate::settings::follow_link(self, link),
            Link::Queued(id) => crate::offline::follow(self, *id),
            Link::State(state) => set_list_state(self, state),
            // Enter on a comment doesn't start writing: `>` does.
            Link::Quote { .. } => {
                let (quote, comment) = (
                    self.first_key(Action::QuoteReply),
                    self.first_key(Action::Comment),
                );
                self.info(format!(
                    "{quote} replies quoting this comment · {comment} comments"
                ));
                Vec::new()
            }
        }
    }

    #[must_use]
    pub(super) fn load_more(&mut self) -> Vec<Cmd> {
        let Some(key) = self.route().and_then(|r| r.family().paged()) else {
            return Vec::new();
        };
        let after = self.data.get_mut(&key).and_then(Remote::ask_more);
        after.map_or_else(Vec::new, |after| {
            vec![Cmd::Api(Api::FetchMore { key, after })]
        })
    }
}

/// Goes to the page above this one.
#[must_use]
pub fn up_level(state: &mut State) -> Vec<Cmd> {
    let Some(route) = state.route().cloned() else {
        return Vec::new();
    };
    match route.family().up(state) {
        Some(parent) => state.push(parent),
        None => state.just_say("Already at the top"),
    }
}

/// Switches to the chrome's tab `n` (1-based).
#[must_use]
pub fn switch_tab(state: &mut State, n: usize) -> Vec<Cmd> {
    let chrome = state.chrome();
    let Some((tab, target)) = n.checked_sub(1).and_then(|i| chrome.tabs.get(i)).cloned() else {
        return state.just_say(format!("No tab {n} here"));
    };
    if chrome.active == Some(n - 1) {
        return state.just_say(format!("Already on {}", tab.label));
    }
    // A page's tabs aren't history: the tab replaces the screen, and the
    // one left is kept to come back to as it was.
    let (screen, mut cmds) = match state.unpark(&target) {
        Some(screen) => (screen, Vec::new()),
        None => match target {
            Target::Page(route) => {
                let cmds = state.ensure_route(&route, false);
                (Screen::Page(Box::new(PageScreen::new(route, None))), cmds)
            }
            Target::Files(of) => state.diff_screen(of, None),
            target @ Target::External(_) => return state.go(target.into()),
        },
    };
    if let Screen::Page(p) = &screen {
        cmds.extend(state.record_visit(&p.route.clone()));
    }
    let left = mem::replace(state.screen_mut(), screen);
    state.park(left);
    state.sync_page();
    cmds.extend(state.load_visible(false));
    cmds
}

/// `gr`: the pull request's Review tab, from any of its tabs.
#[must_use]
pub fn review_glance(state: &mut State) -> Vec<Cmd> {
    match state.review_tab() {
        Some(n) => switch_tab(state, n),
        None => state.just_say("A review at a glance is a pull request's"),
    }
}

/// The most page tabs kept to come back to.
pub(super) const PARKED: usize = 16;

impl State {
    /// Which of the page's tabs (from 1) is a pull request's Review tab.
    pub(super) fn review_tab(&self) -> Option<usize> {
        let tabs = self.chrome().tabs;
        let review =
            |(_, t): &(_, Target)| matches!(t, Target::Page(r) if browse::glanced(r).is_some());
        tabs.iter().position(review).map(|i| i + 1)
    }

    /// Keeps a page tab left, as it was.
    pub(super) fn park(&mut self, screen: Screen) {
        let url = crate::tabs::url(&screen);
        self.parked.retain(|s| crate::tabs::url(s) != url);
        self.parked.push(screen);
        if self.parked.len() > PARKED {
            self.parked.remove(0);
        }
    }

    /// The page tab kept for `target`, if there is one.
    pub(super) fn unpark(&mut self, target: &Target) -> Option<Screen> {
        let i = self.parked.iter().position(|s| match (s, target) {
            (Screen::Page(p), Target::Page(route)) => p.route == *route,
            (Screen::Diff(d), Target::Files(of)) => d.of == *of,
            _ => false,
        })?;
        Some(self.parked.remove(i))
    }
}

/// The next or previous tab of the page; they don't wrap around.
#[must_use]
pub fn step_tab(state: &mut State, forward: bool) -> Vec<Cmd> {
    let chrome = state.chrome();
    let n = chrome.tabs.len();
    let Some(active) = chrome.active.filter(|_| n > 1) else {
        return state.just_say("No other tabs here");
    };
    let next = if forward {
        Some(active + 1).filter(|&i| i < n)
    } else {
        active.checked_sub(1)
    };
    match next {
        Some(i) => switch_tab(state, i + 1),
        None => {
            state.info(if forward {
                "This is the last tab"
            } else {
                "This is the first tab"
            });
            Vec::new()
        }
    }
}
