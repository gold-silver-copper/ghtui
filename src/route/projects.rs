//! Projects: an owner's or a repository's, a project at one of its
//! views, and an item in a view's side pane.

use super::{Family, RESERVED, Route, Target, Url, valid_login};
use crate::api::model::RepoId;
use crate::api::project::{self, ProjectOwner, ProjectRef, ProjectsOf};
use crate::browse::{Build, Width};
use crate::chrome::{Chrome, Section, new_tab};
use crate::data::DataKey;
use crate::state::State;
use crate::ui::pages::{self, url as links};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Page {
    /// An organization's or someone's projects, or a repository's.
    Projects(ProjectsOf),
    /// A project at one of its views (its first, if none is named).
    Project {
        project: ProjectRef,
        view: Option<u64>,
    },
    /// One of a project's items, as the view's side pane shows it, by the
    /// number its URL names it by (`?pane=issue&itemId=…`).
    ProjectItem {
        project: ProjectRef,
        view: Option<u64>,
        item: u64,
    },
}

pub use Page::*;

impl Family for Page {
    fn url(&self) -> String {
        match self {
            Projects(ProjectsOf::Owner(owner)) => format!("{}/projects", owner.url()),
            Projects(ProjectsOf::Repo(repo)) => format!("{}/projects", links::repo(repo)),
            Project { project, view } => links::project_view(project, *view),
            ProjectItem {
                project,
                view,
                item,
            } => links::project_item(project, *view, *item),
        }
    }

    fn title(&self) -> String {
        match self {
            Projects(ProjectsOf::Owner(owner)) => format!("@{} · Projects", owner.login()),
            Projects(ProjectsOf::Repo(repo)) => format!("{repo} · Projects"),
            Project { project, .. } => {
                format!("@{} · Project {}", project.owner.login(), project.number)
            }
            ProjectItem { project, item, .. } => format!(
                "@{} · Project {} · Item {item}",
                project.owner.login(),
                project.number
            ),
        }
    }

    fn repo(&self) -> Option<&RepoId> {
        match self {
            Projects(ProjectsOf::Repo(repo)) => Some(repo),
            _ => None,
        }
    }

    fn up(&self, _: &State) -> Option<Route> {
        Some(match self {
            Projects(ProjectsOf::Repo(repo)) => Route::code(repo.clone()),
            Projects(ProjectsOf::Owner(owner)) => Route::user(owner.login()),
            Project { project, .. } => {
                Route::Projects(Projects(ProjectsOf::Owner(project.owner.clone())))
            }
            ProjectItem { project, view, .. } => Route::Projects(Project {
                project: project.clone(),
                view: *view,
            }),
        })
    }

    fn needs(&self) -> Vec<DataKey> {
        let header = self.repo().map(|repo| DataKey::Repo(repo.clone()));
        header.into_iter().chain(self.paged()).collect()
    }

    fn paged(&self) -> Option<DataKey> {
        Some(match self {
            Projects(of) => DataKey::Projects(of.clone()),
            Project { project, view } | ProjectItem { project, view, .. } => {
                DataKey::Project(project.clone(), *view)
            }
        })
    }

    fn width(&self) -> Width {
        match self {
            Projects(_) => Width::Lists,
            Project { .. } => Width::Full,
            ProjectItem { .. } => Width::Reading,
        }
    }

    fn section(&self) -> Section {
        Section::Projects
    }

    fn chrome(&self, state: &State, c: &mut Chrome) {
        let (project, view) = match self {
            Projects(ProjectsOf::Repo(repo)) => return state.repo_chrome(repo, self.section(), c),
            Projects(ProjectsOf::Owner(owner)) => {
                let login = owner.login();
                c.crumb(login, Some(Target::Page(Route::user(login))));
                let org = matches!(owner, ProjectOwner::Org(_));
                return state.profile_tabs(login, None, Some(org), c);
            }
            Project { project, view } | ProjectItem { project, view, .. } => (project, *view),
        };
        // Its owner and their projects, and its views as tabs.
        let login = project.owner.login();
        c.crumb(login, Some(Target::Page(Route::user(login))));
        let list = Route::Projects(Projects(ProjectsOf::Owner(project.owner.clone())));
        c.crumb("Projects", Some(Target::Page(list)));
        let data = state.picked::<project::Project>(&DataKey::Project(project.clone(), view));
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
            let route = Route::Projects(Project {
                project: project.clone(),
                view: Some(v.number),
            });
            c.tabs
                .push((new_tab(icon, &v.name, None), Target::Page(route)));
        }
        c.active = p.views.iter().position(|v| v.number == p.view);
    }

    fn build(&self, b: Build<'_>, page: &mut crate::ui::page::Page) {
        let (cx, now) = (b.cx, b.cx.now);
        let title = self.title();
        match self {
            Projects(of) => pages::projects(page, of, b.list(self.paged()), now),
            Project { project, view } => {
                if let Some(p) = b.list(self.paged()).show(page, &title) {
                    pages::project_view(page, project, *view, p, cx);
                }
            }
            ProjectItem {
                project,
                view,
                item,
            } => {
                if let Some(p) = b.list::<project::Project>(self.paged()).show(page, &title) {
                    pages::project_item(page, project, *view, p, *item, cx);
                }
            }
        }
    }
}

/// An owner's projects page, from what follows `/projects` in its URL: the
/// list, a project, one of its views, or an item in a view's side pane.
/// Its settings, insights and workflows are on GitHub.
fn owners(owner: ProjectOwner, rest: &[&str], url: &Url<'_>) -> Option<Page> {
    let [number, rest @ ..] = rest else {
        return Some(Projects(ProjectsOf::Owner(owner)));
    };
    let project = ProjectRef {
        owner,
        number: number.parse().ok()?,
    };
    let view = match rest {
        [] => None,
        ["views", n] => Some(n.parse().ok()?),
        _ => return None,
    };
    Some(match url.param("itemId") {
        Some(item) => ProjectItem {
            project,
            view,
            item: item.parse().ok()?,
        },
        None => Project { project, view },
    })
}

/// An owner's projects.
pub(super) fn parse(url: &Url<'_>) -> Option<Target> {
    let page = match url.segments {
        ["orgs", login, "projects", rest @ ..] => {
            owners(ProjectOwner::Org((*login).to_owned()), rest, url)?
        }
        ["users", login, "projects", rest @ ..] => {
            owners(ProjectOwner::User((*login).to_owned()), rest, url)?
        }
        // A profile's projects are an organization's at `/orgs/…/projects`.
        [login]
            if url.param("tab").as_deref() == Some("projects")
                && !RESERVED.contains(login)
                && valid_login(login) =>
        {
            Projects(ProjectsOf::Owner(ProjectOwner::User((*login).to_owned())))
        }
        _ => return None,
    };
    Some(Target::Page(Route::Projects(page)))
}

/// Its linked projects; a classic project (`projects/1`) is gone.
pub(super) fn parse_repo(repo: &RepoId, rest: &[&str], _: &Url<'_>) -> Option<Target> {
    (rest == ["projects"])
        .then(|| Target::Page(Route::Projects(Projects(ProjectsOf::Repo(repo.clone())))))
}

#[cfg(test)]
pub(crate) fn arb() -> impl proptest::strategy::Strategy<Value = Route> {
    use crate::route::arb::{login, repo};
    use proptest::prelude::*;
    let page = prop_oneof![
        repo().prop_map(|repo| Projects(ProjectsOf::Repo(repo))),
        (
            login(),
            any::<bool>(),
            1..u64::MAX,
            prop::option::of(1..u64::MAX)
        )
            .prop_map(|(login, org, number, view)| {
                let owner = if org {
                    ProjectOwner::Org(login)
                } else {
                    ProjectOwner::User(login)
                };
                Project {
                    project: ProjectRef { owner, number },
                    view,
                }
            }),
        (
            login(),
            1..u64::MAX,
            prop::option::of(1..u64::MAX),
            1..u64::MAX
        )
            .prop_map(|(login, number, view, item)| ProjectItem {
                project: ProjectRef {
                    owner: ProjectOwner::Org(login),
                    number,
                },
                view,
                item,
            }),
    ];
    page.prop_map(Route::Projects)
}
