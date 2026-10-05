//! The chrome around every screen, after GitHub's: where you are (the
//! header's crumbs), the tabs of the thing you're looking at, and a sticky
//! title for pull requests. Both the view and mouse handling use
//! [`State::layout`], so clicks land where things are drawn.

use ghtui_api::browse::SearchKind;
use ghtui_api::model::{PrRef, RepoId};
use ghtui_ui::chrome::{Crumb, Tab};
use ghtui_ui::pages::{PrTab, ProfileTab};
use ratatui::layout::Rect;

use crate::route::{OPEN, Route, Target};
use crate::state::{Screen, State};

/// What the chrome shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Chrome {
    pub crumbs: Vec<(Crumb, Option<Target>)>,
    pub right: Vec<(String, Target)>,
    pub tabs: Vec<(Tab, Target)>,
    pub active: Option<usize>,
    /// A pull request whose title stays at the top.
    pub title: Option<PrRef>,
}

/// Where the chrome and the content go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub header: Rect,
    pub title: Option<Rect>,
    /// Two rows: labels and the underline.
    pub tabs: Option<Rect>,
    pub content: Rect,
    /// The first lasting problem, above the status bar.
    pub problem: Option<Rect>,
    pub status: Rect,
}

/// The search behind your review requests.
pub const REVIEW_REQUESTS: &str = "is:open is:pr review-requested:@me archived:false";

fn new_tab(icon: &'static str, label: &str, count: Option<u64>) -> Tab {
    Tab {
        icon,
        label: label.to_owned(),
        count,
        external: false,
    }
}

impl State {
    pub fn chrome(&self) -> Chrome {
        let mut c = Chrome::default();
        // Right: review requests and you.
        if let Some(inbox) = &self.inbox.data {
            let n = inbox
                .review_requested_total
                .max(inbox.review_requested.len() as u64);
            if n > 0 {
                c.right.push((
                    format!("⇄ {n} to review"),
                    Target::Page(Route::Search {
                        kind: SearchKind::Pulls,
                        query: REVIEW_REQUESTS.into(),
                    }),
                ));
            }
        }
        if let Some(login) = &self.viewer {
            c.right
                .push((format!("@{login}"), Target::Page(Route::user(login))));
        }
        match self.screen() {
            Screen::Page(p) => self.page_chrome(&p.route, &mut c),
            Screen::Diff(d) => {
                repo_crumbs(&d.pr.repo, &mut c);
                self.pr_tabs(&d.pr, &mut c);
                // The tab says which changes are shown.
                let diff = self.diffs.get(&d.pr);
                let label = match diff.and_then(|d| d.range.as_ref()) {
                    Some(range) => Some(format!("Files · {}", range.label)),
                    None if diff.is_some_and(|d| d.doc.since_active()) => {
                        Some("Files · since your review".to_owned())
                    }
                    None => None,
                };
                c.active = c
                    .tabs
                    .iter()
                    .position(|(_, t)| matches!(t, Target::Files(_)));
                if let (Some(label), Some((tab, _))) =
                    (label, c.active.and_then(|i| c.tabs.get_mut(i)))
                {
                    tab.label = label;
                }
                c.title = Some(d.pr.clone());
            }
        }
        c
    }

    fn page_chrome(&self, route: &Route, c: &mut Chrome) {
        match route {
            Route::Home => c.crumb("Home", None),
            Route::Repo(repo)
            | Route::Tree { repo, .. }
            | Route::Blob { repo, .. }
            | Route::Issues { repo, .. }
            | Route::Pulls { repo, .. }
            | Route::Issue { repo, .. } => {
                repo_crumbs(repo, c);
                self.repo_tabs(repo, c);
                let pulls = c.tabs.len() - 2;
                c.active = Some(match route {
                    Route::Issues { .. } | Route::Issue { .. } => 1,
                    Route::Pulls { .. } => pulls,
                    _ => 0,
                });
            }
            Route::Pr { pr, tab } => {
                repo_crumbs(&pr.repo, c);
                self.pr_tabs(pr, c);
                c.active = Some(match tab {
                    PrTab::Conversation => 0,
                    PrTab::Commits => 1,
                });
                c.title = Some(pr.clone());
            }
            Route::User { login, tab } => {
                c.crumb(login, None);
                let profile = self.profile(login);
                let user = |tab| {
                    Target::Page(Route::User {
                        login: login.clone(),
                        tab,
                    })
                };
                c.tabs
                    .push((new_tab("◫", "Overview", None), user(ProfileTab::Overview)));
                c.tabs.push((
                    new_tab("▤", "Repositories", profile.map(|p| p.repo_count)),
                    user(ProfileTab::Repositories),
                ));
                if !profile.is_some_and(|p| p.is_org) {
                    c.tabs.push((
                        new_tab("☆", "Stars", profile.map(|p| p.star_count)),
                        user(ProfileTab::Stars),
                    ));
                }
                c.active = Some(match tab {
                    ProfileTab::Overview => 0,
                    ProfileTab::Repositories => 1,
                    ProfileTab::Stars => 2,
                });
            }
            Route::Search { kind, query } => {
                c.crumb("Search", None);
                let total = self.search_results(route).map(|r| match r {
                    ghtui_api::browse::SearchResults::Repos(r) => r.total,
                    ghtui_api::browse::SearchResults::Issues(r) => r.total,
                    ghtui_api::browse::SearchResults::Users(r) => r.total,
                });
                let kinds = [
                    (SearchKind::Repos, "▤", "Repositories"),
                    (SearchKind::Issues, "◉", "Issues"),
                    (SearchKind::Pulls, "⇄", "Pull requests"),
                    (SearchKind::Users, "⚇", "Users"),
                ];
                for (i, (k, icon, label)) in kinds.into_iter().enumerate() {
                    if k == *kind {
                        c.active = Some(i);
                    }
                    let count = (k == *kind).then_some(total).flatten();
                    c.tabs.push((
                        new_tab(icon, label, count),
                        Target::Page(Route::Search {
                            kind: k,
                            query: query.clone(),
                        }),
                    ));
                }
            }
        }
    }

    fn repo_tabs(&self, repo: &RepoId, c: &mut Chrome) {
        let o = self.overview(repo);
        c.tabs.push((
            new_tab("<>", "Code", None),
            Target::Page(Route::Repo(repo.clone())),
        ));
        if o.is_none_or(|o| o.has_issues) {
            c.tabs.push((
                new_tab("◉", "Issues", o.map(|o| o.open_issues)),
                Target::Page(Route::Issues {
                    repo: repo.clone(),
                    query: OPEN.into(),
                }),
            ));
        }
        c.tabs.push((
            new_tab("⇄", "Pull requests", o.map(|o| o.open_prs)),
            Target::Page(Route::Pulls {
                repo: repo.clone(),
                query: OPEN.into(),
            }),
        ));
        let mut actions = new_tab("▶", "Actions", None);
        actions.external = true;
        c.tabs.push((
            actions,
            Target::External(format!("https://github.com/{repo}/actions")),
        ));
    }

    fn pr_tabs(&self, pr: &PrRef, c: &mut Chrome) {
        let activity = self.activity(pr);
        let detail = self.prs.get(pr).and_then(|r| r.data.as_ref());
        c.tabs.push((
            new_tab(
                "◌",
                "Conversation",
                activity.map(|a| a.comments.len() as u64),
            ),
            Target::Page(Route::Pr {
                pr: pr.clone(),
                tab: PrTab::Conversation,
            }),
        ));
        c.tabs.push((
            new_tab("◷", "Commits", activity.map(|a| a.commits.len() as u64)),
            Target::Page(Route::Pr {
                pr: pr.clone(),
                tab: PrTab::Commits,
            }),
        ));
        let mut checks = new_tab("✓", "Checks", None);
        checks.external = true;
        c.tabs
            .push((checks, Target::External(format!("{}/checks", pr.url()))));
        c.tabs.push((
            new_tab("±", "Files changed", detail.map(|d| d.changed_files)),
            Target::Files(pr.clone()),
        ));
    }

    /// Whether the chrome has a title row and tabs: what [`State::chrome`]
    /// builds, without building it (layout runs several times a frame).
    fn chrome_rows(&self) -> (bool, bool) {
        match self.screen() {
            Screen::Diff(_) => (true, true),
            Screen::Page(p) => (
                matches!(p.route, Route::Pr { .. }),
                !matches!(p.route, Route::Home),
            ),
        }
    }

    /// Where everything goes on screen.
    pub fn layout(&self) -> Layout {
        let (w, h) = self.size;
        let (has_title, has_tabs) = self.chrome_rows();
        let mut y = 0;
        let row = |y: u16, height: u16| Rect::new(0, y, w, height.min(h.saturating_sub(y)));
        let header = row(y, 1);
        y += 1;
        let title = has_title.then(|| {
            let r = row(y, 1);
            y += 1;
            r
        });
        let tabs = has_tabs.then(|| {
            let r = row(y, 2);
            y += 2;
            r
        });
        let status_y = h.saturating_sub(1);
        let problem_y = (!self.problems.is_empty() && status_y > y).then(|| status_y - 1);
        let bottom = problem_y.unwrap_or(status_y);
        Layout {
            header,
            title: title.filter(|r| r.height > 0 && r.y < status_y),
            tabs: tabs.filter(|r| r.height > 0 && r.y < status_y),
            content: Rect::new(0, y.min(bottom), w, bottom.saturating_sub(y)),
            problem: problem_y.map(|y| row(y, 1)),
            status: row(status_y, 1),
        }
    }
}

fn repo_crumbs(repo: &RepoId, c: &mut Chrome) {
    c.crumb(&repo.owner, Some(Target::Page(Route::user(&repo.owner))));
    c.crumb(&repo.name, Some(Target::Page(Route::Repo(repo.clone()))));
}

impl Chrome {
    /// Adds a crumb; the last one added is where you are.
    fn crumb(&mut self, text: &str, target: Option<Target>) {
        if let Some((last, _)) = self.crumbs.last_mut() {
            last.current = false;
        }
        let crumb = Crumb {
            text: text.to_owned(),
            current: true,
        };
        self.crumbs.push((crumb, target));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::tests::state;
    use ghtui_api::model::PrRef;

    /// `chrome_rows` says what `chrome` builds, on every kind of screen.
    #[test]
    fn layout_shape_matches_the_chrome() {
        let repo = RepoId::new("o", "r");
        let pr = PrRef::parse("o/r#1").unwrap();
        let routes = [
            Route::Home,
            Route::Repo(repo.clone()),
            Route::Issues {
                repo: repo.clone(),
                query: OPEN.into(),
            },
            Route::Issue { repo, number: 2 },
            Route::Pr {
                pr: pr.clone(),
                tab: PrTab::Commits,
            },
            Route::user("octocat"),
            Route::Search {
                kind: SearchKind::Users,
                query: "x".into(),
            },
        ];
        let mut s = state();
        for route in routes {
            let _ = s.push(route);
            let c = s.chrome();
            assert_eq!(s.chrome_rows(), (c.title.is_some(), !c.tabs.is_empty()));
        }
        let _ = s.open_diff(pr);
        let c = s.chrome();
        assert_eq!(s.chrome_rows(), (c.title.is_some(), !c.tabs.is_empty()));
    }
}
