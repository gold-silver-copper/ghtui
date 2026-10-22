//! The chrome around every screen, after GitHub's: where you are (the
//! header's crumbs), the tabs of the thing you're looking at, and a sticky
//! title for pull requests. Both the view and mouse handling use
//! [`State::layout`], so clicks land where things are drawn.

use crate::api::browse::{
    CommitDetail, Comparison, DiscussionsOf, Profile, RepoSort, SearchKind, SearchResults,
};
use crate::api::model::{PrRef, RepoId};
use crate::api::project::{ProjectOwner, ProjectsOf};
use crate::git::Oid;
use crate::route::{
    OPEN, Route, Target, app, code, discussions, issues, people, projects, pulls, search, security,
    settings, wiki,
};
use crate::ui::chrome::{Crumb, PageTab};
use crate::ui::pages::{PrTab, ProfileTab};
use ratatui::layout::Rect;

use crate::data::DataKey;
use crate::diff_screen::DiffOf;
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

pub(crate) fn new_tab(icon: &'static str, label: &str, count: Option<u64>) -> PageTab {
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
                Target::Page(Route::Search(search::Search {
                    kind: SearchKind::Pulls,
                    query: crate::home::REVIEW_REQUESTS.into(),
                })),
            ));
        }
        if let Some(unread) = self.unread_label() {
            let inbox = Route::Search(search::Notifications {
                query: String::new(),
            });
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
    pub(crate) fn screen_chrome(&self) -> Chrome {
        let mut c = Chrome::default();
        match self.screen() {
            Screen::Page(p) => p.route.family().chrome(self, &mut c),
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

    /// A repository's page's chrome: under its repository's tabs, `here`
    /// active.
    pub(crate) fn repo_chrome(&self, repo: &RepoId, here: Section, c: &mut Chrome) {
        repo_crumbs(repo, c);
        self.repo_tabs(repo, c);
        c.active = c.tabs.iter().position(|(_, t)| match t {
            Target::Page(r) => r.family().section() == here,
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
        let hidden = titles.len() - last - 1;
        for (i, title) in titles.into_iter().enumerate().take(last + 1).skip(first) {
            c.crumbs.push((crumb(title, i == active), None));
            c.open_tabs.push(i);
        }
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
        c.tab("◫", "Overview", None, user(ProfileTab::Overview));
        let repos = profile.map(|p| p.repos.total());
        c.tab(
            "▤",
            "Repositories",
            repos,
            user(ProfileTab::Repositories(sort)),
        );
        // An organization's projects or a user's, once that's known.
        let projects = org.map(|org| {
            let owner = if org {
                ProjectOwner::Org
            } else {
                ProjectOwner::User
            };
            let of = ProjectsOf::Owner(owner(login.to_owned()));
            Target::Page(Route::Projects(projects::Projects(of)))
        });
        if let Some(projects) = &projects {
            c.tab("▥", "Projects", None, projects.clone());
        }
        let packages =
            crate::site::Page::Packages(crate::api::packages::PackagesOf::Owner(login.to_owned()));
        c.tab("⬢", "Packages", None, Route::Site(packages));
        if org == Some(true) {
            let people = profile.map(|p| p.people.total());
            c.tab("⚇", "People", people, user(ProfileTab::People));
        } else {
            let stars = profile.map(|p| p.star_count);
            c.tab("☆", "Stars", stars, user(ProfileTab::Stars));
            let followers = profile.and_then(|p| p.followers);
            c.tab("⚇", "Followers", followers, user(ProfileTab::Followers));
            let following = profile.and_then(|p| p.following);
            c.tab("⚇", "Following", following, user(ProfileTab::Following));
        }
        let active = tab.map(user).or(projects);
        c.active = c.tabs.iter().position(|(_, t)| Some(t) == active.as_ref());
    }

    /// A commit's tabs: the commit, and its files.
    pub(crate) fn commit_tabs(&self, repo: &RepoId, oid: &str, pr: Option<u64>, c: &mut Chrome) {
        let detail = self.picked::<CommitDetail>(&DataKey::Commit(repo.clone(), oid.to_owned()));
        let full = detail.map_or(oid, |d| d.oid.as_str());
        let commit = code::Commit {
            repo: repo.clone(),
            oid: full.to_owned(),
            pr,
        };
        c.tab("◷", "Commit", None, Route::Code(commit));
        let checks = code::CommitChecks {
            repo: repo.clone(),
            oid: full.to_owned(),
        };
        c.tab("✓", "Checks", None, Route::Code(checks));
        // A short SHA or a ref has files once its full ID is known.
        if let Ok(oid) = Oid::parse(full) {
            let files = detail.and_then(|d| d.changed_files);
            let diff = Target::Files(DiffOf::Commit(repo.clone(), oid));
            c.tab("±", "Files changed", files, diff);
        }
    }

    fn repo_tabs(&self, repo: &RepoId, c: &mut Chrome) {
        let o = self.overview(repo);
        let r = || repo.clone();
        c.tab("<>", "Code", None, Route::code(r()));
        if o.is_none_or(|o| o.has_issues) {
            let issues = issues::Issues {
                repo: r(),
                query: OPEN.into(),
            };
            let open = o.map(|o| o.open_issues);
            c.tab("◉", "Issues", open, Route::Issues(issues));
        }
        let pulls = pulls::Pulls {
            repo: r(),
            query: OPEN.into(),
        };
        let open = o.map(|o| o.open_prs);
        c.tab("⇄", "Pull requests", open, Route::Pulls(pulls));
        if o.is_some_and(|o| o.has_discussions) {
            let of = DiscussionsOf::Repo(r());
            let route = Route::Discussions(discussions::Discussions { of, category: None });
            c.tab("◈", "Discussions", None, route);
        }
        c.tab("▶", "Actions", None, Route::actions(r()));
        let projects = projects::Projects(ProjectsOf::Repo(r()));
        c.tab("▥", "Projects", None, Route::Projects(projects));
        if o.is_some_and(|o| o.has_wiki) {
            let wiki = wiki::Wiki {
                repo: r(),
                page: None,
            };
            c.tab("▤", "Wiki", None, Route::Wiki(wiki));
        }
        let advisories = security::Advisories(Some(r()));
        c.tab("⛨", "Security", None, Route::Security(advisories));
        let tab = crate::ui::pages::InsightsTab::Contributors;
        let insights = crate::site::Page::Insights { repo: r(), tab };
        c.tab("⌁", "Insights", None, Route::Site(insights));
        // GitHub shows the Settings tab only to admins.
        if o.is_some_and(|o| o.can_admin) {
            let page = crate::api::settings::SettingsPage::General;
            let settings = settings::Page { repo: r(), page };
            c.tab("⚙", "Settings", None, Route::Settings(settings));
        }
    }

    pub(crate) fn pr_tabs(&self, pr: &PrRef, c: &mut Chrome) {
        let activity = self.activity(pr);
        let files = self.pr(pr).map(|d| d.changed_files);
        let tab = |tab| {
            Route::Pulls(pulls::Pr {
                pr: pr.clone(),
                tab,
            })
        };
        let comments = activity.map(|a| a.comment_count);
        c.tab("◌", "Conversation", comments, Route::pr(pr.clone()));
        let commits = activity.map(|a| a.commits.total());
        c.tab("◷", "Commits", commits, tab(PrTab::Commits));
        c.tab("✓", "Checks", None, tab(PrTab::Checks));
        let diff = Target::Files(DiffOf::Pr(pr.clone()));
        c.tab("±", "Files changed", files, diff);
        c.tab("◎", "Review", None, tab(PrTab::Review));
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

impl From<Route> for Target {
    fn from(route: Route) -> Self {
        Target::Page(route)
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

/// A comparison's tabs: its commits, and (once its ends are known) the
/// files changed between them.
pub(crate) fn compare_tabs(
    repo: &RepoId,
    spec: &str,
    range: Option<(Oid, Oid)>,
    comparison: Option<&Comparison>,
    c: &mut Chrome,
) {
    let commits = comparison.map(|cmp| cmp.commits.total());
    let compare = code::Compare {
        repo: repo.clone(),
        spec: spec.to_owned(),
    };
    c.tab("◷", "Commits", commits, Route::Code(compare));
    if let Some((from, to)) = range {
        // A count GitHub capped would read as exact.
        let files = comparison
            .filter(|cmp| !cmp.files_capped)
            .map(|cmp| cmp.files);
        let diff = Target::Files(DiffOf::Range(repo.clone(), from, to));
        c.tab("±", "Files changed", files, diff);
    }
}

pub(crate) fn repo_crumbs(repo: &RepoId, c: &mut Chrome) {
    c.crumb(&repo.owner, Some(Target::Page(Route::user(&repo.owner))));
    c.crumb(&repo.name, Some(Target::Page(Route::code(repo.clone()))));
}

impl Chrome {
    /// Adds a tab.
    fn tab(&mut self, icon: &'static str, label: &str, count: Option<u64>, to: impl Into<Target>) {
        self.tabs.push((new_tab(icon, label, count), to.into()));
    }

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
    use crate::route::{Route, gists};
    use crate::state::tests::state;

    /// A page without tabs (a gist) keeps no rows for them.
    #[test]
    fn rows_are_kept_only_for_the_chrome_there_is() {
        let mut s = state();
        let _ = s.push(Route::Gists(gists::Gist {
            owner: None,
            id: "6cad326836d38bd3a7ae".into(),
        }));
        assert!(s.chrome().tabs.is_empty());
        let lay = s.layout();
        assert_eq!((lay.title, lay.tabs, lay.content.y), (None, None, 1));
    }
}
