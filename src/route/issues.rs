//! A repository's Issues tab: its issue list, an issue, the new issue
//! form, and its labels and milestones.

use std::collections::HashSet;

use super::{Family, OPEN, Route, Target, Url};
use crate::api::browse::{IssueDetail, RepoOverview, SearchKind, SearchResults, StateCounts};
use crate::api::model::RepoId;
use crate::browse::{Build, Width};
use crate::chrome::Section;
use crate::data::DataKey;
use crate::state::State;
use crate::ui::page::{Role, Seg};
use crate::ui::pages::{self, url as links};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Page {
    Issues {
        repo: RepoId,
        query: String,
    },
    /// An issue (or, until GitHub says otherwise, a pull request: those
    /// redirect).
    Issue {
        repo: RepoId,
        number: u64,
    },
    /// A new issue: its templates, then its form.
    NewIssue(RepoId),
    /// A repository's labels.
    Labels(RepoId),
    /// Open or closed milestones.
    Milestones {
        repo: RepoId,
        closed: bool,
    },
    Milestone {
        repo: RepoId,
        number: u64,
    },
}

pub use Page::*;

impl Page {
    fn repo_id(&self) -> &RepoId {
        match self {
            Issues { repo, .. }
            | Issue { repo, .. }
            | NewIssue(repo)
            | Labels(repo)
            | Milestones { repo, .. }
            | Milestone { repo, .. } => repo,
        }
    }
}

impl Family for Page {
    fn url(&self) -> String {
        match self {
            Issues { repo, query } => list_url(&links::issues(repo), query),
            Issue { repo, number } => links::issue(repo, *number),
            NewIssue(repo) => format!("{}/issues/new", links::repo(repo)),
            Labels(repo) => format!("{}/labels", links::repo(repo)),
            Milestones { repo, closed } => format!(
                "{}/milestones{}",
                links::repo(repo),
                if *closed { "?state=closed" } else { "" }
            ),
            Milestone { repo, number } => format!("{}/milestone/{number}", links::repo(repo)),
        }
    }

    fn title(&self) -> String {
        match self {
            Issues { repo, .. } => format!("{repo} · Issues"),
            Issue { repo, number } => format!("{repo}#{number}"),
            NewIssue(repo) => format!("{repo} · New issue"),
            Labels(repo) => format!("{repo} · Labels"),
            Milestones { repo, .. } => format!("{repo} · Milestones"),
            Milestone { repo, number } => format!("{repo} · Milestone {number}"),
        }
    }

    fn repo(&self) -> Option<&RepoId> {
        Some(self.repo_id())
    }

    fn up(&self, _: &State) -> Option<Route> {
        let repo = self.repo_id().clone();
        Some(match self {
            Issues { .. } | Milestones { .. } => Route::code(repo),
            Milestone { .. } => Route::Issues(Milestones {
                repo,
                closed: false,
            }),
            Issue { .. } | NewIssue(_) | Labels(_) => Route::Issues(Issues {
                repo,
                query: OPEN.into(),
            }),
        })
    }

    fn needs(&self) -> Vec<DataKey> {
        let header = DataKey::Repo(self.repo_id().clone());
        let own = match self {
            Issues { repo, query } => list_needs(repo, "issue", query),
            Issue { repo, number } => vec![
                DataKey::InProjects(repo.clone(), *number),
                DataKey::Issue(repo.clone(), *number),
            ],
            NewIssue(repo) => vec![DataKey::NewForm(repo.clone())],
            Labels(_) | Milestones { .. } | Milestone { .. } => self.paged().into_iter().collect(),
        };
        std::iter::once(header).chain(own).collect()
    }

    fn paged(&self) -> Option<DataKey> {
        Some(match self {
            Issues { repo, query } => list_search(repo, "issue", query),
            Labels(repo) => DataKey::Labels(repo.clone()),
            Milestones { repo, closed } => DataKey::Milestones(repo.clone(), *closed),
            Milestone { repo, number } => DataKey::Milestone(repo.clone(), *number),
            Issue { .. } | NewIssue(_) => return None,
        })
    }

    fn width(&self) -> Width {
        match self {
            Issue { .. } => Width::ReadingAside,
            NewIssue(_) => Width::Reading,
            Issues { .. } | Labels(_) | Milestones { .. } | Milestone { .. } => Width::Lists,
        }
    }

    fn starts_on_row(&self) -> bool {
        !matches!(self, Issue { .. })
    }

    fn query(&self) -> Option<&str> {
        match self {
            Issues { query, .. } => Some(query),
            _ => None,
        }
    }

    fn with_query(&self, query: String) -> Option<Route> {
        let repo = self.repo_id().clone();
        matches!(self, Issues { .. }).then_some(Route::Issues(Issues { repo, query }))
    }

    fn section(&self) -> Section {
        Section::Issues
    }

    fn form(&self) -> Option<&'static crate::forms::Kind> {
        matches!(self, NewIssue(_)).then_some(&crate::create::FORM)
    }

    fn build(&self, b: Build<'_>, page: &mut crate::ui::page::Page) {
        let (cx, now) = (b.cx, b.cx.now);
        let title = self.title();
        match self {
            Issues { repo, query } => list(b, page, repo, query, false),
            Issue { repo, number } => {
                let issue =
                    b.get::<Option<Box<IssueDetail>>>(&DataKey::Issue(repo.clone(), *number));
                let none = HashSet::new();
                let route = Route::Issues(self.clone());
                let fresh = crate::live::page::fresh(b.state, &route).unwrap_or(&none);
                match issue.show(page, &title) {
                    Some(Some(issue)) => {
                        let lines = pages::LinesAt::new();
                        let mut marks = crate::talk::marks(b.state, None, &lines);
                        marks.comments = fresh;
                        let projects = b.get(&DataKey::InProjects(repo.clone(), *number));
                        pages::issue(page, issue, projects, marks, cx);
                    }
                    // Redirecting to the pull request.
                    Some(None) => page.line(vec![Seg::new("Loading…", Role::Meta)]),
                    None => {}
                }
            }
            NewIssue(repo) => {
                let key = DataKey::NewForm(repo.clone());
                if let Some(d) = b.get(&key).show(page, &title) {
                    crate::create::build(b.state, page, &Route::Issues(self.clone()), d);
                }
            }
            Labels(repo) => crate::housekeeping::build(b, page, repo),
            Milestones { repo, closed } => {
                pages::milestones(page, repo, b.list(self.paged()), *closed, now);
            }
            Milestone { repo, .. } => {
                if let Some(m) = b.list(self.paged()).show(page, &title) {
                    pages::milestone(page, repo, m, cx.icons, now);
                }
            }
        }
    }
}

/// The search for a repository's issue or pull request list (`is` is
/// `issue` or `pr`). Newest first, as on GitHub, unless the filter sorts.
pub(super) fn list_search(repo: &RepoId, is: &str, query: &str) -> DataKey {
    let mut q = format!("repo:{repo} is:{is} {}", query.trim());
    if !query.contains("sort:") {
        q.push_str(" sort:created-desc");
    }
    DataKey::Search(SearchKind::Issues, q)
}

/// The search an issue or pull request list's open and closed counts come
/// from, when its filter is more than a state and a sort (else they're the
/// repository's own).
fn state_counts(repo: &RepoId, is: &str, query: &str) -> Option<String> {
    let state = ["is:open", "is:closed", "state:open", "state:closed"];
    let filter: Vec<&str> = (query.split_whitespace())
        .filter(|w| !state.contains(w) && !w.starts_with("sort:"))
        .collect();
    (!filter.is_empty()).then(|| format!("repo:{repo} is:{is} {}", filter.join(" ")))
}

/// What an issue or pull request list needs besides its header.
pub(super) fn list_needs(repo: &RepoId, is: &str, query: &str) -> Vec<DataKey> {
    let counts = state_counts(repo, is, query).map(DataKey::StateCounts);
    std::iter::once(list_search(repo, is, query))
        .chain(counts)
        .collect()
}

/// An issue or pull request list.
pub(super) fn list(
    b: Build<'_>,
    page: &mut crate::ui::page::Page,
    repo: &RepoId,
    query: &str,
    is_pr: bool,
) {
    let is = if is_pr { "pr" } else { "issue" };
    #[expect(clippy::disallowed_methods, reason = "extra: shown once loaded")]
    let overview = b
        .get::<RepoOverview>(&DataKey::Repo(repo.clone()))
        .ready_unchecked();
    // A filtered list counts what the filter finds, as GitHub's does;
    // until it's counted, no counts show.
    #[expect(clippy::disallowed_methods, reason = "extra: shown once loaded")]
    let counts = match state_counts(repo, is, query) {
        Some(q) => (b.get::<StateCounts>(&DataKey::StateCounts(q)))
            .ready_unchecked()
            .map(|c| (c.open, c.closed)),
        None => overview.map(|o| {
            if is_pr {
                (o.open_prs, o.closed_prs)
            } else {
                (o.open_issues, o.closed_issues)
            }
        }),
    };
    let results = b
        .get::<SearchResults>(&list_search(repo, is, query))
        .pick(|r| match r {
            SearchResults::Issues(r) => Some(r),
            _ => None,
        });
    // Pinned issues, above the list, as on GitHub.
    if let Some(o) = overview.filter(|_| !is_pr) {
        pages::pinned_issues(page, repo, &o.pinned_issues, b.cx.icons);
    }
    // Labels and milestones, beside the list as on GitHub.
    pages::labels_and_milestones(page, repo);
    pages::issue_list(page, query, counts, is_pr, results, b.cx);
}

pub(super) fn list_url(base: &str, query: &str) -> String {
    if query == OPEN {
        base.to_owned()
    } else {
        format!("{base}?q={}", links::encode(query))
    }
}

/// A list's filter from its URL. GitHub's list URLs repeat `is:issue` or
/// `is:pr`; the route adds them back.
pub(super) fn list_query(url: &Url<'_>) -> String {
    let Some(query) = url.param("q") else {
        return OPEN.to_owned();
    };
    let words: Vec<&str> = query
        .split_whitespace()
        .filter(|w| !matches!(*w, "is:issue" | "is:pr"))
        .collect();
    if words.is_empty() {
        OPEN.to_owned()
    } else {
        words.join(" ")
    }
}

pub(super) fn parse_repo(repo: &RepoId, rest: &[&str], url: &Url<'_>) -> Option<Target> {
    let repo = repo.clone();
    let page = match rest {
        ["issues"] => Issues {
            repo,
            query: list_query(url),
        },
        ["labels"] => Labels(repo),
        // A label's issues. Its name goes into a search, which control
        // characters don't survive.
        ["labels", label] if label.contains(char::is_control) => return None,
        ["labels", label] => {
            // As a search reads it: runs of spaces are one.
            let label = label.split_whitespace().collect::<Vec<_>>().join(" ");
            let label = if label.contains(' ') {
                format!("\"{label}\"")
            } else {
                label
            };
            Issues {
                repo,
                query: format!("{OPEN} label:{label}"),
            }
        }
        // A milestone by its title is in the list.
        ["milestones", ..] => Milestones {
            repo,
            closed: url.param("state").as_deref() == Some("closed"),
        },
        ["milestone", n] => Milestone {
            repo,
            number: n.parse().ok()?,
        },
        // The form, after the template chooser or straight to it.
        ["issues", "new"] | ["issues", "new", "choose"] => NewIssue(repo),
        ["issues", n] => Issue {
            repo,
            number: n.parse().ok()?,
        },
        _ => return None,
    };
    Some(Target::Page(Route::Issues(page)))
}

#[cfg(test)]
pub(crate) fn arb() -> impl proptest::strategy::Strategy<Value = Route> {
    use crate::route::arb::{query, repo};
    use proptest::prelude::*;
    let page = prop_oneof![
        (repo(), query()).prop_map(|(repo, query)| Issues { repo, query }),
        (repo(), 1..u64::MAX).prop_map(|(repo, number)| Issue { repo, number }),
        repo().prop_map(NewIssue),
        (repo(), any::<bool>()).prop_map(|(repo, closed)| Milestones { repo, closed }),
        (repo(), 1..u64::MAX).prop_map(|(repo, number)| Milestone { repo, number }),
    ];
    page.prop_map(Route::Issues)
}
