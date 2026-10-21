//! Open tabs, like a browser's: each has its own history, and the one on
//! screen keeps its history where it always was ([`State::screens`] and
//! [`State::forward`]), so nothing else needs to know about tabs. The
//! others wait in [`State::before`] and [`State::after`]. Data, pull
//! requests and diffs are shared: two tabs on one pull request share its
//! fetch, its diff and its review drafts.

use crate::route::{Dest, Route, Target, code, issues, pulls, search};
use crate::ui::page::Link;
use crate::ui::text::short_sha;

use crate::diff_screen::{DiffOf, DiffPrefs, DiffScreen};
use crate::nav::PageScreen;
use crate::state::{Cmds, Screen, Screens, State};

/// An open tab off screen: its history as it was left.
pub struct Tab {
    pub screens: Screens,
    pub forward: Vec<Screen>,
    pub parked: Vec<Screen>,
}

impl Tab {
    fn new(first: Screen) -> Self {
        Self {
            screens: Screens::new(first),
            forward: Vec::new(),
            parked: Vec::new(),
        }
    }
}

/// The link that reopens a screen: the only place a screen's link is
/// written, for tabs and for `y`.
pub fn url(screen: &Screen) -> String {
    match screen {
        Screen::Page(p) => p.route.url(),
        Screen::Diff(d) => d.of.url(),
    }
}

/// A screen's short name: `repo#42`, `owner/repo`, `@user`, a file's name.
pub fn title(screen: &Screen) -> String {
    match screen {
        Screen::Page(p) => match &p.route {
            Route::Issues(issues::Issue { repo, number }) => format!("{}#{number}", repo.name),
            Route::Pulls(pulls::Pr { pr, .. }) => format!("{}#{}", pr.repo.name, pr.number),
            Route::Code(code::Blob { path, .. }) | Route::Code(code::Tree { path, .. })
                if !path.is_empty() =>
            {
                path.rsplit('/').next().unwrap_or(path).to_owned()
            }
            Route::Code(code::Commit { oid, .. }) => short_sha(oid).to_owned(),
            Route::Search(search::Search { query, .. }) => format!("⌕ {query}"),
            Route::Code(code::Repo(repo)) => repo.to_string(),
            // A repository's other pages, without the owner.
            route => match route.repo() {
                Some(repo) => route.title().replacen(&format!("{}/", repo.owner), "", 1),
                None => route.title(),
            },
        },
        Screen::Diff(d) => match &d.of {
            DiffOf::Pr(pr) => format!("{}#{} files", pr.repo.name, pr.number),
            DiffOf::Commit(_, oid) => format!("{} files", short_sha(oid)),
            DiffOf::Range(_, from, to) => format!("{}..{} files", short_sha(from), short_sha(to)),
        },
    }
}

impl State {
    pub fn tab_count(&self) -> usize {
        self.before.len() + 1 + self.after.len()
    }

    /// Which tab is on screen, from 0.
    pub fn active_tab(&self) -> usize {
        self.before.len()
    }

    /// Every open tab's title, in order.
    pub fn tab_titles(&self) -> Vec<String> {
        self.each_tab(title)
    }

    /// The route on screen in each tab, in order, for reopening them.
    pub fn tab_urls(&self) -> Vec<String> {
        self.each_tab(url)
    }

    /// `f` of the screen each tab shows, in order.
    fn each_tab<T>(&self, f: fn(&Screen) -> T) -> Vec<T> {
        let before = self.before.iter().map(|t| f(t.screens.last()));
        let after = self.after.iter().map(|t| f(t.screens.last()));
        (before.chain([f(self.screens.last())]).chain(after)).collect()
    }

    /// Puts `next` on screen, handing back the history it replaces.
    fn swap_in(&mut self, next: Tab) -> Tab {
        Tab {
            screens: std::mem::replace(&mut self.screens, next.screens),
            forward: std::mem::replace(&mut self.forward, next.forward),
            parked: std::mem::replace(&mut self.parked, next.parked),
        }
    }

    /// Opens `dest` in a new tab after this one, at its anchor. Pages in
    /// the browser just open there.
    pub fn open_tab(&mut self, Dest { target, anchor }: Dest) -> Cmds {
        let (first, mut cmds) = match target {
            Target::Page(route) => {
                let route = self.followed(route);
                let cmds = self.record_visit(&route);
                (Screen::Page(Box::new(PageScreen::new(route, anchor))), cmds)
            }
            Target::Files(of) => {
                let mut screen = DiffScreen::new(of, DiffPrefs::fit(self.size.0));
                screen.anchor = anchor;
                (Screen::Diff(Box::new(screen)), Cmds::new())
            }
            Target::External(_) => return self.go(target.into()),
        };
        self.push_tab(first);
        cmds.extend(self.opened_tab());
        cmds
    }

    /// Puts `first` on screen in a new tab after this one.
    fn push_tab(&mut self, first: Screen) {
        let left = self.swap_in(Tab::new(first));
        self.before.push(left);
    }

    /// A new tab is on screen: it loads, and says where it is.
    fn opened_tab(&mut self) -> Cmds {
        let cmds = self.load_visible(false);
        let n = self.tab_count();
        self.info(format!("Opened in tab {} of {n}", self.active_tab() + 1));
        cmds
    }

    /// `T`: what's selected in a new tab. A page's selected link, the file
    /// the tree has selected, a diff's pull request or commit; with
    /// nothing selected, this page again.
    pub fn open_in_tab(&mut self) -> Cmds {
        let dest = match self.screen() {
            Screen::Page(p) => match p.selected_link() {
                Some(Link::Url(url)) => Dest::from_url(url),
                Some(_) | None => Target::Page(p.route.clone()).into(),
            },
            Screen::Diff(d) => Target::Page(match &d.of {
                DiffOf::Pr(pr) => Route::pr(pr.clone()),
                DiffOf::Commit(repo, oid) => Route::Code(code::Commit {
                    repo: repo.clone(),
                    oid: oid.to_string(),
                    pr: None,
                }),
                DiffOf::Range(repo, from, to) => Route::Code(code::Compare {
                    repo: repo.clone(),
                    spec: format!("{from}..{to}"),
                }),
            })
            .into(),
        };
        let from_tree = self
            .screen()
            .diff()
            .filter(|d| d.focus == crate::diff_screen::Pane::Tree);
        // The file the tree has selected: the same diff, at that file.
        if let Some(screen) = from_tree.cloned() {
            self.push_tab(Screen::Diff(Box::new(screen)));
            if let Some(d) = self.screen_mut().diff_mut() {
                d.focus = crate::diff_screen::Pane::Diff;
            }
            return self.opened_tab();
        }
        self.open_tab(dest)
    }

    /// Goes to tab `i` (from 0), showing what it had and refreshing what's
    /// stale, as going back does.
    pub fn switch_to_tab(&mut self, i: usize) -> Cmds {
        if i >= self.tab_count() {
            return self.just_say(format!("No tab {} open", i + 1));
        }
        if i == self.active_tab() {
            return self.just_say(format!("Already on tab {}", i + 1));
        }
        self.move_to_tab(i);
        self.load_visible(false)
    }

    /// What the command line asked for: on screen, or with tabs reopened,
    /// in a new tab after them.
    pub fn start_at(&mut self, dest: Dest) -> Cmds {
        if self.tab_count() == 1 {
            return self.go(dest);
        }
        // Only passing through the last tab: loading it would mark its
        // data as on its way.
        self.move_to_tab(self.tab_count() - 1);
        self.open_tab(dest)
    }

    /// Puts tab `i` on screen.
    fn move_to_tab(&mut self, i: usize) {
        while self.active_tab() < i {
            let next = self.after.remove(0);
            let left = self.swap_in(next);
            self.before.push(left);
        }
        while self.active_tab() > i {
            let Some(previous) = self.before.pop() else {
                break;
            };
            let right = self.swap_in(previous);
            self.after.insert(0, right);
        }
    }

    /// `]` and `[`: the next or previous open tab, wrapping around.
    pub fn step_open_tab(&mut self, forward: bool) -> Cmds {
        let n = self.tab_count();
        if n == 1 {
            let open = self.first_key(crate::keymap::Action::OpenInTab);
            return self.just_say(format!("Only one tab open: {open} opens another"));
        }
        let i = self.active_tab();
        let next = if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        };
        self.switch_to_tab(next)
    }

    /// Closes the tab on screen, showing the next (or else the previous).
    pub fn close_tab(&mut self) -> Cmds {
        let next = if self.after.is_empty() {
            self.before.pop()
        } else {
            Some(self.after.remove(0))
        };
        let Some(next) = next else {
            let quit = self.first_key(crate::keymap::Action::Quit);
            return self.just_say(format!("This is the last tab ({quit} quits)"));
        };
        drop(self.swap_in(next));
        self.overlay = None;
        self.load_visible(false)
    }

    /// Reopens tabs saved by [`State::tab_urls`]: the first replaces the
    /// home page, each other one opens after it, and the first is shown.
    pub fn restore_tabs(&mut self, urls: &[String]) {
        let prefs = DiffPrefs::fit(self.size.0);
        let mut screens = urls.iter().filter_map(|u| match Dest::from_url(u).target {
            Target::Files(of) => Some(Screen::Diff(Box::new(DiffScreen::new(of, prefs)))),
            Target::Page(route) => Some(Screen::Page(Box::new(PageScreen::new(route, None)))),
            Target::External(_) => None,
        });
        let Some(first) = screens.next() else {
            return;
        };
        self.screens = Screens::new(first);
        self.forward.clear();
        self.after = screens.map(Tab::new).collect();
    }
}
