//! Open tabs, like a browser's: each has its own history, and the one on
//! screen keeps its history where it always was ([`State::screens`] and
//! [`State::forward`]), so nothing else needs to know about tabs. The
//! others wait in [`State::before`] and [`State::after`]. Data, pull
//! requests and diffs are shared: two tabs on one pull request share its
//! fetch, its diff and its review drafts.

use ghtui_ui::page::Link;
use ghtui_ui::text::short_sha;

use crate::browse::PageScreen;
use crate::diff_screen::{DiffOf, DiffScreen};
use crate::route::{Route, Target};
use crate::state::{Cmd, Screen, Screens, State};

/// An open tab off screen: its history as it was left.
pub struct Tab {
    pub screens: Screens,
    pub forward: Vec<Screen>,
}

impl Tab {
    fn new(first: Screen) -> Self {
        Self {
            screens: Screens::new(first),
            forward: Vec::new(),
        }
    }

    /// What the tab strip calls it.
    pub fn title(&self) -> String {
        title(self.screens.last())
    }
}

/// A screen's short name: `#42`, `owner/repo`, `@user`, a file's name.
pub fn title(screen: &Screen) -> String {
    match screen {
        Screen::Page(p) => match &p.route {
            Route::Issue { number, .. } => format!("#{number}"),
            Route::Pr { pr, .. } => format!("#{}", pr.number),
            Route::Blob { path, .. } | Route::Tree { path, .. } if !path.is_empty() => {
                path.rsplit('/').next().unwrap_or(path).to_owned()
            }
            Route::Commit { oid, .. } => short_sha(oid).to_owned(),
            Route::Search { query, .. } => format!("⌕ {query}"),
            Route::Repo(repo) => repo.to_string(),
            // A repository's other pages, without the owner.
            route => match route.repo() {
                Some(repo) => route.title().replacen(&format!("{}/", repo.owner), "", 1),
                None => route.title(),
            },
        },
        Screen::Diff(d) => match &d.of {
            DiffOf::Pr(pr) => format!("#{} files", pr.number),
            DiffOf::Commit(_, oid) => format!("{} files", short_sha(oid)),
            DiffOf::Range(_, from, to) => format!("{}...{} files", short_sha(from), short_sha(to)),
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
        let current = title(self.screens.last());
        let before = self.before.iter().map(Tab::title);
        let after = self.after.iter().map(Tab::title);
        before
            .chain(std::iter::once(current))
            .chain(after)
            .collect()
    }

    /// Puts `next` on screen, handing back the history it replaces.
    fn swap_in(&mut self, next: Tab) -> Tab {
        Tab {
            screens: std::mem::replace(&mut self.screens, next.screens),
            forward: std::mem::replace(&mut self.forward, next.forward),
        }
    }

    /// Opens `target` in a new tab after this one. Pages in the browser
    /// just open there.
    #[must_use]
    pub fn open_tab(&mut self, target: Target) -> Vec<Cmd> {
        let first = match &target {
            Target::External(_) => return self.go(target),
            Target::Page(route) => Screen::Page(Box::new(PageScreen::new(route.clone()))),
            Target::Files(of) => Screen::Diff(Box::new(DiffScreen::new(of.clone(), self.size.0))),
        };
        let left = self.swap_in(Tab::new(first));
        self.before.push(left);
        let mut cmds = match &target {
            Target::Page(route) => self.record_visit(route),
            Target::Files(DiffOf::Pr(pr)) => self.ensure_pr(pr, false),
            _ => Vec::new(),
        };
        cmds.extend(self.load_visible(false));
        let n = self.tab_count();
        self.info(format!("Opened in tab {} of {n}", self.active_tab() + 1));
        cmds
    }

    /// `T`: what's selected in a new tab. A page's selected link, the file
    /// the tree has selected, a diff's pull request or commit; with
    /// nothing selected, this page again.
    #[must_use]
    pub fn open_in_tab(&mut self) -> Vec<Cmd> {
        let target = match self.screen() {
            Screen::Page(p) => match p.selected_link() {
                Some(Link::Url(url)) => Target::from_url(url),
                Some(_) | None => Target::Page(p.route.clone()),
            },
            Screen::Diff(d) => match &d.of {
                DiffOf::Pr(pr) => Target::Page(Route::pr(pr.clone())),
                DiffOf::Commit(repo, oid) => Target::Page(Route::Commit {
                    repo: repo.clone(),
                    oid: oid.clone(),
                }),
                DiffOf::Range(repo, from, to) => Target::Page(Route::Compare {
                    repo: repo.clone(),
                    spec: format!("{from}..{to}"),
                }),
            },
        };
        let from_tree = match self.screen() {
            Screen::Diff(d) if d.focus == crate::diff_screen::Pane::Tree => Some((*d).clone()),
            _ => None,
        };
        // The file the tree has selected: the same diff, at that file.
        if let Some(screen) = from_tree {
            let left = self.swap_in(Tab::new(Screen::Diff(screen)));
            self.before.push(left);
            if let Screen::Diff(d) = self.screen_mut() {
                d.focus = crate::diff_screen::Pane::Diff;
            }
            self.info(format!(
                "Opened in tab {} of {}",
                self.active_tab() + 1,
                self.tab_count()
            ));
            return self.load_visible(false);
        }
        self.open_tab(target)
    }

    /// Goes to tab `i` (from 0), showing what it had and refreshing what's
    /// stale, as going back does.
    #[must_use]
    pub fn switch_to_tab(&mut self, i: usize) -> Vec<Cmd> {
        if i >= self.tab_count() {
            self.info(format!("No tab {} open", i + 1));
            return Vec::new();
        }
        if i == self.active_tab() {
            self.info(format!("Already on tab {}", i + 1));
            return Vec::new();
        }
        self.move_to_tab(i);
        self.load_visible(false)
    }

    /// What the command line asked for: on screen, or with tabs reopened,
    /// in a new tab after them.
    #[must_use]
    pub fn start_at(&mut self, target: Target) -> Vec<Cmd> {
        if self.tab_count() == 1 {
            return self.go(target);
        }
        // Only passing through the last tab: loading it would mark its
        // data as on its way.
        self.move_to_tab(self.tab_count() - 1);
        self.open_tab(target)
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
    #[must_use]
    pub fn step_open_tab(&mut self, forward: bool) -> Vec<Cmd> {
        let n = self.tab_count();
        if n == 1 {
            let open = self.first_key(crate::keymap::Action::OpenInTab);
            self.info(format!("Only one tab open: {open} opens another"));
            return Vec::new();
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
    #[must_use]
    pub fn close_tab(&mut self) -> Vec<Cmd> {
        let next = if self.after.is_empty() {
            self.before.pop()
        } else {
            Some(self.after.remove(0))
        };
        let Some(next) = next else {
            let quit = self.first_key(crate::keymap::Action::Quit);
            self.info(format!("This is the last tab ({quit} quits)"));
            return Vec::new();
        };
        drop(self.swap_in(next));
        self.overlay = None;
        self.load_visible(false)
    }

    /// The route on screen in each tab, in order, for reopening them.
    pub fn tab_urls(&self) -> Vec<String> {
        let url = |screen: &Screen| match screen {
            Screen::Page(p) => p.route.url(),
            Screen::Diff(d) => match &d.of {
                DiffOf::Pr(pr) => format!("{}/files", pr.url()),
                DiffOf::Commit(repo, oid) => {
                    format!("{}#files", ghtui_ui::pages::url::commit(repo, oid))
                }
                DiffOf::Range(..) => format!("{}#files", crate::route::compare_url(&d.of)),
            },
        };
        let before = self.before.iter().map(|t| url(t.screens.last()));
        let after = self.after.iter().map(|t| url(t.screens.last()));
        before
            .chain(std::iter::once(url(self.screens.last())))
            .chain(after)
            .collect()
    }

    /// Reopens tabs saved by [`State::tab_urls`]: the first replaces the
    /// home page, each other one opens after it, and the first is shown.
    pub fn restore_tabs(&mut self, urls: &[String]) {
        let mut targets = urls.iter().filter_map(|u| match Target::from_url(u) {
            Target::External(_) => None,
            target => Some(target),
        });
        let Some(first) = targets.next() else {
            return;
        };
        let screen = |target: Target, width| match target {
            Target::Files(of) => Some(Screen::Diff(Box::new(DiffScreen::new(of, width)))),
            Target::Page(route) => Some(Screen::Page(Box::new(PageScreen::new(route)))),
            Target::External(_) => None,
        };
        let width = self.size.0;
        if let Some(first) = screen(first, width) {
            self.screens = Screens::new(first);
            self.forward.clear();
        }
        self.after = targets
            .filter_map(|t| screen(t, width))
            .map(Tab::new)
            .collect();
    }
}
