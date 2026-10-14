//! The chrome around every screen, after GitHub's: where you are (the
//! header's crumbs), the tabs of the thing you're looking at, and a sticky
//! title for pull requests. Both the view and mouse handling use
//! [`State::layout`], so clicks land where things are drawn.

use crate::api::browse::{
    CommitDetail, Comparison, DiscussionsOf, Profile, RepoSort, SearchKind, SearchResults,
};
use crate::api::model::{PrRef, RepoId};
use crate::api::project::{self, Project, ProjectOwner, ProjectRef, ProjectsOf};
use crate::git::Oid;
use crate::route::app;
use crate::route::code;
use crate::route::issues;
use crate::route::people;
use crate::route::pulls;
use crate::ui::chrome::{Crumb, PageTab};
use crate::ui::pages::{PrTab, ProfileTab};
use ratatui::layout::Rect;

use crate::browse::paged;
use crate::data::{Data, DataKey};
use crate::diff_screen::DiffOf;
use crate::route::{Family, OPEN, Route, Target};
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

fn new_tab(icon: &'static str, label: &str, count: Option<u64>) -> PageTab {
    PageTab {
        icon,
        label: label.to_owned(),
        count,
    }
}

impl State {
    pub fn chrome(&self) -> Chrome {
        let mut c = self.screen_chrome();
        // Right: offline (and what waits for GitHub), review requests,
        // your unread notifications and you.
        for offline in crate::offline::header(self) {
            c.right
                .push((offline, Target::Page(Route::App(app::Offline))));
        }
        // Review requests, once their search (Home's first section, unless
        // you changed it) is in.
        let requests = DataKey::Search(SearchKind::Pulls, crate::home::REVIEW_REQUESTS.into());
        if let Some(SearchResults::Issues(r)) = self.picked::<SearchResults>(&requests)
            && r.total > 0
        {
            c.right.push((
                format!("⇄ {} to review", r.total),
                Target::Page(Route::Search {
                    kind: SearchKind::Pulls,
                    query: crate::home::REVIEW_REQUESTS.into(),
                }),
            ));
        }
        if let Some(unread) = self.unread_label() {
            let inbox = Route::Notifications {
                query: String::new(),
            };
            c.right.push((unread, Target::Page(inbox)));
        }
        if let Some(login) = &self.viewer {
            c.right
                .push((format!("@{login}"), Target::Page(Route::user(login))));
        }
        self.tab_strip(&mut c);
        c
    }

    /// What's on screen's own chrome: its crumbs, tabs and title.
    fn screen_chrome(&self) -> Chrome {
        let mut c = Chrome::default();
        match self.screen() {
            Screen::Page(p) => self.page_chrome(&p.route, &mut c),
            Screen::Diff(d) => {
                repo_crumbs(d.of.repo(), &mut c);
                let pr = match &d.of {
                    DiffOf::Pr(pr) => pr,
                    DiffOf::Commit(repo, oid) => {
                        self.commit_tabs(repo, oid, None, &mut c);
                        c.active = Some(2);
                        return c;
                    }
                    DiffOf::Range(repo, from, to) => {
                        let spec = format!("{from}..{to}");
                        compare_tabs(repo, &spec, Some((from.clone(), to.clone())), None, &mut c);
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
                        let since = diff.and_then(|d| d.since_commit.as_deref());
                        Some(since.map_or_else(
                            || "Files · since your review".to_owned(),
                            |c| format!("Files · since {}", crate::ui::text::short_sha(c)),
                        ))
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
            Route::App(p) => p.chrome(self, c),
            Route::Code(p) => p.chrome(self, c),
            Route::People(p) => p.chrome(self, c),
            Route::Releases(p) => p.chrome(self, c),
            Route::Runs(p) => p.chrome(self, c),
            Route::Issues(p) => p.chrome(self, c),
            Route::Pulls(p) => p.chrome(self, c),
            Route::Housekeeping(p) => p.chrome(self, c),
            Route::Notifications { .. } => c.crumb("Notifications", None),
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
            Route::Gists(login) => {
                c.crumb(login, Some(Target::Page(Route::user(login))));
                c.crumb("Gists", None);
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
            Route::Projects(ProjectsOf::Owner(owner)) => {
                let login = owner.login();
                c.crumb(login, Some(Target::Page(Route::user(login))));
                let org = matches!(owner, ProjectOwner::Org(_));
                self.profile_tabs(login, None, Some(org), c);
            }
            Route::Project { project, view } | Route::ProjectItem { project, view, .. } => {
                self.project_chrome(project, *view, c);
            }
            Route::Site(p) => p.chrome(self, c),
            Route::Search { kind, query } => {
                c.crumb("Search", None);
                let results = paged(route).and_then(|key| self.picked::<SearchResults>(&key));
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
            // The rest are a repository's pages, under its tabs.
            _ => {
                if let Some(repo) = route.repo() {
                    self.repo_chrome(repo, section(route), c);
                }
            }
        }
    }

    /// A repository's page's chrome: under its repository's tabs, `here`
    /// active.
    pub(crate) fn repo_chrome(&self, repo: &RepoId, here: Section, c: &mut Chrome) {
        repo_crumbs(repo, c);
        self.repo_tabs(repo, c);
        c.active = c.tabs.iter().position(|(_, t)| match t {
            Target::Page(r) => section(r) == here,
            _ => false,
        });
    }

    /// With more than one tab open, the header shows them where the crumbs
    /// would be (the title row already says where you are): as many as fit
    /// around the one on screen, and how many are hidden either side.
    fn tab_strip(&self, c: &mut Chrome) {
        if self.tab_count() < 2 {
            return;
        }
        let header = Rect::new(0, 0, self.size.0, self.size.1.min(1));
        let right: Vec<String> = c.right.iter().map(|(t, _)| t.clone()).collect();
        // The header keeps up to 32 columns for crumbs before dropping
        // its right-hand items; measure with that much.
        let reserve = Crumb {
            text: " ".repeat(32),
            current: false,
            tab: true,
        };
        let h = crate::ui::chrome::header_layout(header, &[reserve], &right);
        let room = usize::from(h.search.x.saturating_sub(h.logo.right().saturating_add(5)));
        let titles: Vec<String> = self
            .tab_titles()
            .into_iter()
            .map(|t| crate::ui::text::truncate(&t, 20))
            .collect();
        let active = self.active_tab();
        let width = |i: usize| titles.get(i).map_or(0, |t| crate::ui::text::width(t) + 3);
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

    /// A profile's tabs, with `tab` active (none: its projects). Whether
    /// it's an organization's is the profile's to say, else `org`'s.
    pub(crate) fn profile_tabs(
        &self,
        login: &str,
        tab: Option<ProfileTab>,
        org: Option<bool>,
        c: &mut Chrome,
    ) {
        let profile = self.picked::<Profile>(&DataKey::Profile(login.to_lowercase()));
        let org = profile.map(|p| p.is_org).or(org);
        let user = |tab| {
            Target::Page(Route::People(people::User {
                login: login.to_owned(),
                tab,
            }))
        };
        let sort = match tab {
            Some(ProfileTab::Repositories(sort)) => sort,
            _ => RepoSort::Updated,
        };
        let mut tabs = vec![
            ("◫", "Overview", None, user(ProfileTab::Overview)),
            (
                "▤",
                "Repositories",
                profile.map(|p| p.repos.total()),
                user(ProfileTab::Repositories(sort)),
            ),
        ];
        // An organization's projects or a user's, once that's known.
        let projects = org.map(|org| {
            let owner = if org {
                ProjectOwner::Org(login.to_owned())
            } else {
                ProjectOwner::User(login.to_owned())
            };
            Target::Page(Route::Projects(ProjectsOf::Owner(owner)))
        });
        if let Some(projects) = &projects {
            tabs.push(("▥", "Projects", None, projects.clone()));
        }
        let packages =
            crate::site::Page::Packages(crate::api::packages::PackagesOf::Owner(login.to_owned()));
        tabs.push(("⬢", "Packages", None, Target::Page(Route::Site(packages))));
        if org == Some(true) {
            tabs.push((
                "⚇",
                "People",
                profile.map(|p| p.people.total()),
                user(ProfileTab::People),
            ));
        } else {
            tabs.extend([
                (
                    "☆",
                    "Stars",
                    profile.map(|p| p.star_count),
                    user(ProfileTab::Stars),
                ),
                (
                    "⚇",
                    "Followers",
                    profile.and_then(|p| p.followers),
                    user(ProfileTab::Followers),
                ),
                (
                    "⚇",
                    "Following",
                    profile.and_then(|p| p.following),
                    user(ProfileTab::Following),
                ),
            ]);
        }
        let active = tab.map(user).or(projects);
        c.active = tabs.iter().position(|(.., t)| Some(t) == active.as_ref());
        for (icon, label, count, target) in tabs {
            c.tabs.push((new_tab(icon, label, count), target));
        }
    }

    /// A project's chrome: its owner and their projects, and its views as
    /// tabs.
    fn project_chrome(&self, project: &ProjectRef, view: Option<u64>, c: &mut Chrome) {
        let login = project.owner.login();
        c.crumb(login, Some(Target::Page(Route::user(login))));
        let list = Route::Projects(ProjectsOf::Owner(project.owner.clone()));
        c.crumb("Projects", Some(Target::Page(list)));
        let data = self.picked::<Project>(&DataKey::Project(project.clone(), view));
        let title = data.map_or_else(
            || format!("Project {}", project.number),
            |p| p.title.clone(),
        );
        c.crumb(&title, None);
        let Some(p) = data else {
            return;
        };
        for v in &p.views {
            let icon = match v.layout {
                project::Layout::Board => "▥",
                project::Layout::Table => "▤",
                project::Layout::Roadmap => "▬",
            };
            let route = Route::Project {
                project: project.clone(),
                view: Some(v.number),
            };
            c.tabs
                .push((new_tab(icon, &v.name, None), Target::Page(route)));
        }
        c.active = p.views.iter().position(|v| v.number == p.view);
    }

    /// A commit's tabs: the commit, and its files.
    pub(crate) fn commit_tabs(&self, repo: &RepoId, oid: &str, pr: Option<u64>, c: &mut Chrome) {
        let detail = self.picked::<CommitDetail>(&DataKey::Commit(repo.clone(), oid.to_owned()));
        let full = detail.map_or(oid, |d| d.oid.as_str());
        c.tabs.push((
            new_tab("◷", "Commit", None),
            Target::Page(Route::Code(code::Commit {
                repo: repo.clone(),
                oid: full.to_owned(),
                pr,
            })),
        ));
        c.tabs.push((
            new_tab("✓", "Checks", None),
            Target::Page(Route::Code(code::CommitChecks {
                repo: repo.clone(),
                oid: full.to_owned(),
            })),
        ));
        // A short SHA or a ref has files once its full ID is known.
        if let Ok(oid) = Oid::parse(full) {
            c.tabs.push((
                new_tab("±", "Files changed", detail.and_then(|d| d.changed_files)),
                Target::Files(DiffOf::Commit(repo.clone(), oid)),
            ));
        }
    }

    fn repo_tabs(&self, repo: &RepoId, c: &mut Chrome) {
        let o = self.overview(repo);
        c.tabs.push((
            new_tab("<>", "Code", None),
            Target::Page(Route::Code(code::Repo(repo.clone()))),
        ));
        if o.is_none_or(|o| o.has_issues) {
            c.tabs.push((
                new_tab("◉", "Issues", o.map(|o| o.open_issues)),
                Target::Page(Route::Issues(issues::Issues {
                    repo: repo.clone(),
                    query: OPEN.into(),
                })),
            ));
        }
        c.tabs.push((
            new_tab("⇄", "Pull requests", o.map(|o| o.open_prs)),
            Target::Page(Route::Pulls(pulls::Pulls {
                repo: repo.clone(),
                query: OPEN.into(),
            })),
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
            Target::Page(Route::actions(repo.clone())),
        ));
        c.tabs.push((
            new_tab("▥", "Projects", None),
            Target::Page(Route::Projects(ProjectsOf::Repo(repo.clone()))),
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
        c.tabs.push((
            new_tab("⌁", "Insights", None),
            Target::Page(Route::Site(crate::site::Page::Insights {
                repo: repo.clone(),
                tab: crate::site::InsightsTab::Contributors,
            })),
        ));
        // GitHub shows the Settings tab only to admins.
        if o.is_some_and(|o| o.can_admin) {
            c.tabs.push((
                new_tab("⚙", "Settings", None),
                Target::Page(Route::Settings {
                    repo: repo.clone(),
                    page: crate::api::settings::SettingsPage::General,
                }),
            ));
        }
    }

    pub(crate) fn pr_tabs(&self, pr: &PrRef, c: &mut Chrome) {
        let activity = self.activity(pr);
        let detail = self.pr(pr);
        c.tabs.push((
            new_tab("◌", "Conversation", activity.map(|a| a.comment_count)),
            Target::Page(Route::pr(pr.clone())),
        ));
        c.tabs.push((
            new_tab("◷", "Commits", activity.map(|a| a.commits.total())),
            Target::Page(Route::Pulls(pulls::Pr {
                pr: pr.clone(),
                tab: PrTab::Commits,
            })),
        ));
        c.tabs.push((
            new_tab("✓", "Checks", None),
            Target::Page(Route::Pulls(pulls::Pr {
                pr: pr.clone(),
                tab: PrTab::Checks,
            })),
        ));
        c.tabs.push((
            new_tab("±", "Files changed", detail.map(|d| d.changed_files)),
            Target::Files(DiffOf::Pr(pr.clone())),
        ));
        c.tabs.push((
            new_tab("◎", "Review", None),
            Target::Page(Route::Pulls(pulls::Pr {
                pr: pr.clone(),
                tab: PrTab::Review,
            })),
        ));
    }

    /// Where everything goes on screen.
    pub fn layout(&self) -> Layout {
        self.layout_of(&self.screen_chrome())
    }

    /// Where everything goes around chrome `c`: rows for the title and
    /// the tabs when it has them.
    pub fn layout_of(&self, c: &Chrome) -> Layout {
        let (w, h) = self.size;
        let mut y = 0;
        let row = |y: u16, height: u16| Rect::new(0, y, w, height.min(h.saturating_sub(y)));
        let header = row(y, 1);
        y += 1;
        let title = c.title.is_some().then(|| {
            let r = row(y, 1);
            y += 1;
            r
        });
        let tabs = (!c.tabs.is_empty()).then(|| {
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Code,
    Issues,
    Pulls,
    Discussions,
    Actions,
    Projects,
    Wiki,
    Security,
    Insights,
    Settings,
}

fn section(route: &Route) -> Section {
    match route {
        Route::Discussions { .. } | Route::Discussion { .. } => Section::Discussions,
        Route::Projects(_) => Section::Projects,
        Route::Wiki { .. } | Route::WikiEdit { .. } => Section::Wiki,
        Route::Advisories(_)
        | Route::Advisory { .. }
        | Route::Alerts { .. }
        | Route::Alert { .. } => Section::Security,
        Route::App(p) => p.section(),
        Route::Code(p) => p.section(),
        Route::People(p) => p.section(),
        Route::Releases(p) => p.section(),
        Route::Runs(p) => p.section(),
        Route::Issues(p) => p.section(),
        Route::Pulls(p) => p.section(),
        Route::Site(p) => p.section(),
        Route::Housekeeping(p) => p.section(),
        Route::Settings { .. } => Section::Settings,
        _ => Section::Code,
    }
}

/// A comparison's tabs: its commits, and (once its ends are known) the
/// files changed between them.
pub(crate) fn compare_tabs(
    repo: &RepoId,
    spec: &str,
    range: Option<(Oid, Oid)>,
    comparison: Option<&Comparison>,
    c: &mut Chrome,
) {
    c.tabs.push((
        new_tab("◷", "Commits", comparison.map(|cmp| cmp.commits.total())),
        Target::Page(Route::Code(code::Compare {
            repo: repo.clone(),
            spec: spec.to_owned(),
        })),
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
            Target::Files(DiffOf::Range(repo.clone(), from, to)),
        ));
    }
}
pub(crate) fn repo_crumbs(repo: &RepoId, c: &mut Chrome) {
    c.crumb(&repo.owner, Some(Target::Page(Route::user(&repo.owner))));
    c.crumb(
        &repo.name,
        Some(Target::Page(Route::Code(code::Repo(repo.clone())))),
    );
}

impl Chrome {
    /// Adds a crumb; the last one added is where you are.
    pub(crate) fn crumb(&mut self, text: &str, target: Option<Target>) {
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
    use crate::route::Route;
    use crate::state::tests::state;

    /// A page without tabs (a gist) keeps no rows for them.
    #[test]
    fn rows_are_kept_only_for_the_chrome_there_is() {
        let mut s = state();
        let _ = s.push(Route::Gist {
            owner: None,
            id: "6cad326836d38bd3a7ae".into(),
        });
        assert!(s.chrome().tabs.is_empty());
        let lay = s.layout();
        assert_eq!((lay.title, lay.tabs, lay.content.y), (None, None, 1));
    }
}
