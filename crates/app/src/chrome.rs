//! The chrome around every screen, after GitHub's: where you are (the
//! header's crumbs), the tabs of the thing you're looking at, and a sticky
//! title for pull requests. Both the view and mouse handling use
//! [`State::layout`], so clicks land where things are drawn.

use ghtui_api::browse::{
    CommitDetail, Comparison, DiscussionsOf, Profile, RepoSort, SearchKind, SearchResults,
};
use ghtui_api::model::{PrRef, RepoId};
use ghtui_ui::chrome::{Crumb, PageTab};
use ghtui_ui::pages::{PrTab, ProfileTab};
use ratatui::layout::Rect;

use crate::browse::{Data, DataKey};
use crate::diff_screen::DiffOf;
use crate::route::{OPEN, Route, Target};
use crate::state::{Screen, State};

/// What the chrome shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Chrome {
    pub crumbs: Vec<(Crumb, Option<Target>)>,
    pub right: Vec<(String, Target)>,
    pub tabs: Vec<(PageTab, Target)>,
    pub active: Option<usize>,
    /// A pull request whose title stays at the top.
    pub title: Option<PrRef>,
    /// With more than one tab open, `crumbs` are the tab strip, and this is
    /// the tab each stands for (a hidden-tabs marker, the nearest hidden).
    pub open_tabs: Vec<usize>,
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

fn new_tab(icon: &'static str, label: &str, count: Option<u64>) -> PageTab {
    PageTab {
        icon,
        label: label.to_owned(),
        count,
        external: false,
    }
}

impl State {
    pub fn chrome(&self) -> Chrome {
        let mut c = self.screen_chrome();
        self.tab_strip(&mut c);
        c
    }

    fn screen_chrome(&self) -> Chrome {
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
                repo_crumbs(d.of.repo(), &mut c);
                let pr = match &d.of {
                    DiffOf::Pr(pr) => pr,
                    DiffOf::Commit(repo, oid) => {
                        self.commit_tabs(repo, oid, &mut c);
                        c.active = Some(2);
                        return c;
                    }
                    DiffOf::Range(repo, from, to) => {
                        let spec = format!("{from}..{to}");
                        compare_tabs(repo, &spec, Some((from, to)), None, &mut c);
                        c.active = Some(1);
                        return c;
                    }
                };
                self.pr_tabs(pr, &mut c);
                // The tab says which changes are shown.
                let diff = self.diffs.get(&d.of);
                let label = match diff.and_then(|d| d.range()) {
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
                c.title = Some(pr.clone());
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
            | Route::Blame { repo, .. }
            | Route::Issues { repo, .. }
            | Route::Pulls { repo, .. }
            | Route::Issue { repo, .. }
            | Route::Commits { repo, .. }
            | Route::Stargazers(repo)
            | Route::Watchers(repo)
            | Route::Forks(repo)
            | Route::Releases(repo)
            | Route::Release { repo, .. }
            | Route::Tags(repo)
            | Route::Branches(repo)
            | Route::Wiki { repo, .. }
            | Route::Deployments { repo, .. }
            | Route::Milestones { repo, .. }
            | Route::Milestone { repo, .. }
            | Route::WorkflowRun { repo, .. }
            | Route::Job { repo, .. }
            | Route::Workflow { repo, .. }
            | Route::Actions(repo)
            | Route::Discussions {
                of: DiscussionsOf::Repo(repo),
                ..
            }
            | Route::Discussion {
                of: DiscussionsOf::Repo(repo),
                ..
            }
            | Route::Advisories(Some(repo))
            | Route::Advisory {
                repo: Some(repo), ..
            } => {
                repo_crumbs(repo, c);
                self.repo_tabs(repo, c);
                let here = section(route);
                c.active = c.tabs.iter().position(|(_, t)| match t {
                    Target::Page(r) => section(r) == here,
                    _ => false,
                });
            }
            Route::Pr { pr, tab } => {
                repo_crumbs(&pr.repo, c);
                self.pr_tabs(pr, c);
                c.active = Some(match tab {
                    PrTab::Conversation => 0,
                    PrTab::Commits => 1,
                    PrTab::Checks => 2,
                });
                c.title = Some(pr.clone());
            }
            Route::Commit { repo, oid } | Route::CommitChecks { repo, oid } => {
                repo_crumbs(repo, c);
                self.commit_tabs(repo, oid, c);
                c.active = Some(usize::from(matches!(route, Route::CommitChecks { .. })));
            }
            Route::Gist { owner, id } => {
                // Its owner, from the gist when the URL doesn't say.
                let gist = match self.get(&DataKey::Gist(id.clone())) {
                    Some(Data::Gist(g)) => g.owner.clone(),
                    _ => None,
                };
                let owner = owner.clone().or(gist);
                if let Some(owner) = owner {
                    c.crumb(&owner, Some(Target::Page(Route::user(&owner))));
                    c.crumb("Gists", Some(Target::Page(Route::Gists(owner.clone()))));
                }
                c.crumb("Gist", None);
            }
            Route::Advisories(None) => c.crumb("Advisories", None),
            Route::Advisory { repo: None, ghsa } => {
                c.crumb("Advisories", Some(Target::Page(Route::Advisories(None))));
                c.crumb(ghsa, None);
            }
            Route::Teams(org) => {
                c.crumb(org, Some(Target::Page(Route::user(org))));
                c.crumb("Teams", None);
            }
            Route::Team { org, slug } => {
                c.crumb(org, Some(Target::Page(Route::user(org))));
                c.crumb("Teams", Some(Target::Page(Route::Teams(org.clone()))));
                c.crumb(slug, None);
            }
            Route::Gists(login) => {
                c.crumb(login, Some(Target::Page(Route::user(login))));
                c.crumb("Gists", None);
            }
            Route::Compare { repo, spec } => {
                repo_crumbs(repo, c);
                self.compare_page_tabs(repo, spec, c);
            }
            Route::Discussions {
                of: DiscussionsOf::Org(org),
                ..
            }
            | Route::Discussion {
                of: DiscussionsOf::Org(org),
                ..
            } => {
                c.crumb(org, Some(Target::Page(Route::user(org))));
                c.crumb("Discussions", None);
            }
            Route::User { login, tab } => {
                c.crumb(login, None);
                let profile = self.picked::<Profile>(&DataKey::Profile(login.to_lowercase()));
                let user = |tab| {
                    Target::Page(Route::User {
                        login: login.clone(),
                        tab,
                    })
                };
                let sort = match tab {
                    ProfileTab::Repositories(sort) => *sort,
                    _ => RepoSort::Updated,
                };
                let mut tabs = vec![
                    ("◫", "Overview", None, ProfileTab::Overview),
                    (
                        "▤",
                        "Repositories",
                        profile.map(|p| p.repo_count),
                        ProfileTab::Repositories(sort),
                    ),
                ];
                if profile.is_some_and(|p| p.is_org) {
                    tabs.push((
                        "⚇",
                        "People",
                        profile.map(|p| p.people_count),
                        ProfileTab::People,
                    ));
                } else {
                    tabs.extend([
                        (
                            "☆",
                            "Stars",
                            profile.map(|p| p.star_count),
                            ProfileTab::Stars,
                        ),
                        (
                            "⚇",
                            "Followers",
                            profile.and_then(|p| p.followers),
                            ProfileTab::Followers,
                        ),
                        (
                            "⚇",
                            "Following",
                            profile.and_then(|p| p.following),
                            ProfileTab::Following,
                        ),
                    ]);
                }
                c.active = tabs.iter().position(|(.., t)| t == tab);
                for (icon, label, count, tab) in tabs {
                    c.tabs.push((new_tab(icon, label, count), user(tab)));
                }
            }
            Route::Search { kind, query } => {
                c.crumb("Search", None);
                let results = self.picked::<SearchResults>(&DataKey::Search(*kind, query.clone()));
                let total = results.map(|r| r.counts().0);
                let kinds = [
                    // Short, so all seven fit.
                    (SearchKind::Repos, "▤", "Repos"),
                    (SearchKind::Issues, "◉", "Issues"),
                    (SearchKind::Pulls, "⇄", "PRs"),
                    (SearchKind::Users, "⚇", "Users"),
                    (SearchKind::Discussions, "◈", "Discussions"),
                    (SearchKind::Commits, "◷", "Commits"),
                    (SearchKind::Code, "<>", "Code"),
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

    /// With more than one tab open, the header shows them where the crumbs
    /// would be (the title row already says where you are): as many as fit
    /// around the one on screen, and how many are hidden either side.
    fn tab_strip(&self, c: &mut Chrome) {
        if self.tab_count() < 2 {
            return;
        }
        let lay = self.layout();
        let right: Vec<String> = c.right.iter().map(|(t, _)| t.clone()).collect();
        // The header keeps up to 32 columns for crumbs before dropping
        // its right-hand items; measure with that much.
        let reserve = Crumb {
            text: " ".repeat(32),
            current: false,
            tab: true,
        };
        let h = ghtui_ui::chrome::header_layout(lay.header, &[reserve], &right);
        let room = usize::from(h.search.x.saturating_sub(h.logo.right().saturating_add(5)));
        let titles: Vec<String> = self
            .tab_titles()
            .into_iter()
            .map(|t| ghtui_ui::text::truncate(&t, 20))
            .collect();
        let active = self.active_tab();
        let width = |i: usize| titles.get(i).map_or(0, |t| ghtui_ui::text::width(t) + 3);
        let (mut first, mut last) = (active, active);
        let mut used = width(active);
        // Grow right, then left, while it fits with room for markers.
        loop {
            let marker = 8;
            let grew_right = last + 1 < titles.len() && used + width(last + 1) + marker <= room;
            if grew_right {
                last += 1;
                used += width(last);
            }
            let grew_left = first > 0 && used + width(first - 1) + marker <= room;
            if grew_left {
                first -= 1;
                used += width(first);
            }
            if !grew_right && !grew_left {
                break;
            }
        }
        let crumb = |text: String, current| Crumb {
            text,
            current,
            tab: true,
        };
        c.crumbs.clear();
        c.open_tabs.clear();
        if first > 0 {
            c.crumbs.push((crumb(format!("‹ {first}"), false), None));
            c.open_tabs.push(first - 1);
        }
        for (i, title) in titles.iter().enumerate().take(last + 1).skip(first) {
            c.crumbs.push((crumb(title.clone(), i == active), None));
            c.open_tabs.push(i);
        }
        let hidden = titles.len() - last - 1;
        if hidden > 0 {
            c.crumbs.push((crumb(format!("{hidden} ›"), false), None));
            c.open_tabs.push(last + 1);
        }
    }

    /// A commit's tabs: the commit, and its files.
    fn commit_tabs(&self, repo: &RepoId, oid: &str, c: &mut Chrome) {
        let detail = self.picked::<CommitDetail>(&DataKey::Commit(repo.clone(), oid.to_owned()));
        let full = detail.map_or(oid, |d| d.oid.as_str());
        c.tabs.push((
            new_tab("◷", "Commit", None),
            Target::Page(Route::Commit {
                repo: repo.clone(),
                oid: full.to_owned(),
            }),
        ));
        c.tabs.push((
            new_tab("✓", "Checks", None),
            Target::Page(Route::CommitChecks {
                repo: repo.clone(),
                oid: full.to_owned(),
            }),
        ));
        c.tabs.push((
            new_tab("±", "Files changed", detail.and_then(|d| d.changed_files)),
            Target::Files(DiffOf::Commit(repo.clone(), full.to_owned())),
        ));
    }

    /// A comparison's tabs: its commits, and its files once known.
    fn compare_page_tabs(&self, repo: &RepoId, spec: &str, c: &mut Chrome) {
        let key = DataKey::Compare(repo.clone(), spec.to_owned());
        let comparison = match self.get(&key) {
            Some(Data::Compare(cmp)) => Some(&**cmp),
            _ => None,
        };
        let range = comparison.map(|cmp| (cmp.from.as_str(), cmp.to.as_str()));
        compare_tabs(repo, spec, range, comparison, c);
        c.active = Some(0);
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
        if o.is_some_and(|o| o.has_discussions) {
            c.tabs.push((
                new_tab("◈", "Discussions", None),
                Target::Page(Route::Discussions {
                    of: DiscussionsOf::Repo(repo.clone()),
                    category: None,
                }),
            ));
        }
        c.tabs.push((
            new_tab("▶", "Actions", None),
            Target::Page(Route::Actions(repo.clone())),
        ));
        if o.is_some_and(|o| o.has_wiki) {
            c.tabs.push((
                new_tab("▤", "Wiki", None),
                Target::Page(Route::Wiki {
                    repo: repo.clone(),
                    page: None,
                }),
            ));
        }
        c.tabs.push((
            new_tab("⛨", "Security", None),
            Target::Page(Route::Advisories(Some(repo.clone()))),
        ));
    }

    fn pr_tabs(&self, pr: &PrRef, c: &mut Chrome) {
        let activity = self.activity(pr);
        let detail = self.prs.get(pr).and_then(|r| r.data.as_ref());
        c.tabs.push((
            new_tab(
                "◌",
                "Conversation",
                activity.map(|a| a.total_comments.max(a.comments.len() as u64)),
            ),
            Target::Page(Route::pr(pr.clone())),
        ));
        c.tabs.push((
            new_tab(
                "◷",
                "Commits",
                activity.map(|a| a.total_commits.max(a.commits.len() as u64)),
            ),
            Target::Page(Route::Pr {
                pr: pr.clone(),
                tab: PrTab::Commits,
            }),
        ));
        c.tabs.push((
            new_tab("✓", "Checks", None),
            Target::Page(Route::Pr {
                pr: pr.clone(),
                tab: PrTab::Checks,
            }),
        ));
        c.tabs.push((
            new_tab("±", "Files changed", detail.map(|d| d.changed_files)),
            Target::Files(DiffOf::Pr(pr.clone())),
        ));
    }

    /// Whether the chrome has a title row and tabs: what [`State::chrome`]
    /// builds, without building it (layout runs several times a frame).
    fn chrome_rows(&self) -> (bool, bool) {
        match self.screen() {
            // A pull request's title stays on top; a commit has none.
            Screen::Diff(d) => (d.of.pr().is_some(), true),
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

/// Which of a repository's tabs a page is under, as on GitHub.
#[derive(PartialEq, Eq)]
enum Section {
    Code,
    Issues,
    Pulls,
    Discussions,
    Actions,
    Wiki,
    Security,
}

fn section(route: &Route) -> Section {
    match route {
        Route::Issues { .. }
        | Route::Issue { .. }
        | Route::Milestones { .. }
        | Route::Milestone { .. } => Section::Issues,
        Route::Pulls { .. } | Route::Pr { .. } => Section::Pulls,
        Route::Discussions { .. } | Route::Discussion { .. } => Section::Discussions,
        Route::Actions(_)
        | Route::WorkflowRun { .. }
        | Route::Job { .. }
        | Route::Workflow { .. } => Section::Actions,
        Route::Wiki { .. } => Section::Wiki,
        Route::Advisories(_) | Route::Advisory { .. } => Section::Security,
        _ => Section::Code,
    }
}

/// A comparison's tabs: its commits, and (once its ends are known) the
/// files changed between them.
fn compare_tabs(
    repo: &RepoId,
    spec: &str,
    range: Option<(&str, &str)>,
    comparison: Option<&Comparison>,
    c: &mut Chrome,
) {
    c.tabs.push((
        new_tab("◷", "Commits", comparison.map(|cmp| cmp.total_commits)),
        Target::Page(Route::Compare {
            repo: repo.clone(),
            spec: spec.to_owned(),
        }),
    ));
    if let Some((from, to)) = range {
        c.tabs.push((
            new_tab(
                "±",
                "Files changed",
                // A count GitHub capped would read as exact.
                comparison
                    .filter(|cmp| !cmp.files_capped)
                    .map(|cmp| cmp.files),
            ),
            Target::Files(DiffOf::Range(repo.clone(), from.to_owned(), to.to_owned())),
        ));
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
            tab: false,
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
        let _ = s.open_diff(DiffOf::Pr(pr));
        let c = s.chrome();
        assert_eq!(s.chrome_rows(), (c.title.is_some(), !c.tabs.is_empty()));
    }
}
