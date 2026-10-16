//! A repository's Pull requests tab: its list, a pull request at one of
//! its tabs (but Files changed, the diff screen's), and the new pull
//! request form.

use super::issues::{list, list_needs, list_query, list_search, list_url};
use super::{Family, OPEN, Route, Target, Url, valid_rev};
use crate::api::model::{PrDetail, PrRef, RepoId};
use crate::browse::{Build, Width};
use crate::chrome::{Chrome, Section, repo_crumbs};
use crate::data::DataKey;
use crate::diff_screen::{DiffOf, DiffState};
use crate::state::{Remote, State};
use crate::ui::pages::{self, PrTab, url as links};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Page {
    Pulls {
        repo: RepoId,
        query: String,
    },
    Pr {
        pr: PrRef,
        tab: PrTab,
    },
    /// A new pull request's form.
    NewPr(RepoId),
}

pub use Page::*;

impl Page {
    fn repo_id(&self) -> &RepoId {
        match self {
            Pulls { repo, .. } | NewPr(repo) => repo,
            Pr { pr, .. } => &pr.repo,
        }
    }

    /// The pull request whose review the page shows at a glance: its
    /// Review tab's. Its diff's threads and drafts load with it.
    pub fn glanced(&self) -> Option<&PrRef> {
        match self {
            Pr {
                pr,
                tab: PrTab::Review,
            } => Some(pr),
            _ => None,
        }
    }
}

impl Family for Page {
    fn url(&self) -> String {
        match self {
            Pulls { repo, query } => list_url(&links::pulls(repo), query),
            Pr { pr, tab } => match tab {
                // GitHub shows a review in the conversation.
                PrTab::Conversation | PrTab::Review => links::pull(pr),
                PrTab::Reviewed(id) => format!("{}#pullrequestreview-{id}", links::pull(pr)),
                PrTab::Commits => links::pull_tab(pr, "commits"),
                PrTab::Checks => links::pull_tab(pr, "checks"),
            },
            NewPr(repo) => format!("{}/compare", links::repo(repo)),
        }
    }

    fn title(&self) -> String {
        match self {
            Pulls { repo, .. } => format!("{repo} · Pull requests"),
            Pr { pr, .. } => pr.to_string(),
            NewPr(repo) => format!("{repo} · New pull request"),
        }
    }

    fn repo(&self) -> Option<&RepoId> {
        Some(self.repo_id())
    }

    fn up(&self, _: &State) -> Option<Route> {
        let repo = self.repo_id().clone();
        Some(match self {
            Pulls { .. } => Route::Code(super::code::Repo(repo)),
            Pr { .. } | NewPr(_) => Route::Pulls(Pulls {
                repo,
                query: OPEN.into(),
            }),
        })
    }

    fn needs(&self) -> Vec<DataKey> {
        use DataKey as K;
        match self {
            Pulls { repo, query } => std::iter::once(K::Repo(repo.clone()))
                .chain(list_needs(repo, "pr", query))
                .collect(),
            // Whether ghtui runs in its clone says the branch to propose.
            NewPr(repo) => vec![
                K::Repo(repo.clone()),
                K::Clone(repo.clone()),
                K::NewForm(repo.clone()),
            ],
            Pr { pr, tab } => {
                let (detail, activity) = (K::Pr(pr.clone()), K::PrActivity(pr.clone()));
                match tab {
                    PrTab::Conversation => {
                        let projects = K::InProjects(pr.repo.clone(), pr.number);
                        vec![detail, projects, activity]
                    }
                    PrTab::Commits => vec![detail, activity],
                    PrTab::Checks => vec![detail, activity, K::PrChecks(pr.clone())],
                    // The Review tab and a review list every review.
                    PrTab::Review | PrTab::Reviewed(_) => {
                        vec![detail, activity, K::Reviews(pr.clone())]
                    }
                }
            }
        }
    }

    /// The lines its threads are on, read from the clone; a new pull
    /// request's head against its base.
    fn also_reads(&self, key: &DataKey) -> bool {
        match (self, key) {
            (Pr { pr, .. }, DataKey::Lines(repo, ..)) => *repo == pr.repo,
            // Where the head chosen stands against the base chosen.
            (NewPr(repo), DataKey::Head { repo: of, .. }) => of == repo,
            _ => false,
        }
    }

    fn paged(&self) -> Option<DataKey> {
        match self {
            Pulls { repo, query } => Some(list_search(repo, "pr", query)),
            Pr { .. } | NewPr(_) => None,
        }
    }

    fn width(&self) -> Width {
        match self {
            Pr { tab, .. } => match tab {
                PrTab::Conversation => Width::ReadingAside,
                PrTab::Checks => Width::Full,
                PrTab::Commits => Width::Lists,
                PrTab::Review | PrTab::Reviewed(_) => Width::Reading,
            },
            Pulls { .. } => Width::Lists,
            NewPr(_) => Width::Reading,
        }
    }

    fn starts_on_row(&self) -> bool {
        !matches!(
            self,
            Pr {
                tab: PrTab::Conversation,
                ..
            }
        )
    }

    fn query(&self) -> Option<&str> {
        match self {
            Pulls { query, .. } => Some(query),
            _ => None,
        }
    }

    fn with_query(&self, query: String) -> Option<Route> {
        let repo = self.repo_id().clone();
        matches!(self, Pulls { .. }).then_some(Route::Pulls(Pulls { repo, query }))
    }

    fn section(&self) -> Section {
        Section::Pulls
    }

    fn chrome(&self, state: &State, c: &mut Chrome) {
        let Pr { pr, tab } = self else {
            return state.repo_chrome(self.repo_id(), self.section(), c);
        };
        repo_crumbs(&pr.repo, c);
        state.pr_tabs(pr, c);
        c.active = Some(match tab {
            PrTab::Conversation => 0,
            PrTab::Commits => 1,
            PrTab::Checks => 2,
            PrTab::Review | PrTab::Reviewed(_) => 4,
        });
        c.title = Some(pr.clone());
    }

    fn form(&self) -> Option<&'static crate::forms::Kind> {
        matches!(self, NewPr(_)).then_some(&crate::create::FORM)
    }

    fn build(&self, b: Build<'_>, page: &mut crate::ui::page::Page) {
        let (state, cx, now) = (b.state, b.cx, b.cx.now);
        let (pr, tab) = match self {
            Pulls { repo, query } => return list(b, page, repo, query, true),
            NewPr(repo) => {
                let key = DataKey::NewForm(repo.clone());
                if let Some(d) = b.get(&key).show(page, &self.title()) {
                    crate::create::build(state, page, &Route::Pulls(self.clone()), d);
                }
                return;
            }
            Pr { pr, tab } => (pr, tab),
        };
        let activity = b.get(&DataKey::PrActivity(pr.clone()));
        let reviews = || b.get(&DataKey::Reviews(pr.clone()));
        let lines = crate::talk::lines_at(state, pr);
        let marks = crate::talk::marks(state, Some(pr), &lines);
        let Some(d) = b
            .get::<PrDetail>(&DataKey::Pr(pr.clone()))
            .show(page, &pr.to_string())
        else {
            return;
        };
        match tab {
            PrTab::Conversation => {
                let projects = b.get(&DataKey::InProjects(pr.repo.clone(), pr.number));
                pages::pr_conversation(page, pr, d, activity, projects, marks, cx);
            }
            PrTab::Commits => pages::pr_commits(page, pr, d, activity, now),
            PrTab::Checks => {
                let checks = b.get(&DataKey::PrChecks(pr.clone()));
                pages::pr_checks(page, pr, d, checks, now);
            }
            PrTab::Review => {
                let diff = state.diffs.get(&DiffOf::Pr(pr.clone()));
                let inputs = diff.map(DiffState::inputs);
                let glance = pages::Glance {
                    threads: Remote::fetched(inputs.map(|i| &i.threads), b.retry),
                    drafts: inputs
                        .filter(|i| i.review_loaded)
                        .map(|i| i.review.pending.as_slice()),
                    fresh: marks.threads,
                    reviews: reviews(),
                    new_reviews: marks.reviews,
                };
                pages::pr_review(page, pr, d, activity, glance, cx);
            }
            PrTab::Reviewed(id) => {
                pages::pr_reviewed(page, pr, d, *id, activity, reviews(), marks, cx);
            }
        }
    }
}

pub(super) fn parse_repo(repo: &RepoId, rest: &[&str], url: &Url<'_>) -> Option<Target> {
    let repo = repo.clone();
    let fragment = url.fragment();
    let page = match rest {
        ["pulls"] => Pulls {
            repo,
            query: list_query(url),
        },
        // A pull request's diff is the diff viewer.
        ["pull", n] if n.ends_with(".diff") => {
            let number = n.trim_end_matches(".diff").parse().ok()?;
            return Some(Target::Files(DiffOf::Pr(PrRef { repo, number })));
        }
        ["pull", n, rest @ ..] => {
            let pr = PrRef {
                repo,
                number: n.parse().ok()?,
            };
            // Review comments are threads in the diff.
            let review_comment = fragment.is_some_and(|f| {
                f.strip_prefix("discussion_r")
                    .or_else(|| f.strip_prefix('r'))
                    .is_some_and(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
            });
            match rest {
                [] if review_comment => return Some(Target::Files(DiffOf::Pr(pr))),
                [] => Pr {
                    pr,
                    tab: PrTab::Conversation,
                },
                ["commits" | "changes", sha] if valid_rev(sha) => {
                    return Some(Target::Page(Route::Code(super::code::Commit {
                        repo: pr.repo,
                        oid: (*sha).to_owned(),
                        pr: Some(pr.number),
                    })));
                }
                ["files" | "changes", ..] => return Some(Target::Files(DiffOf::Pr(pr))),
                ["commits", ..] => Pr {
                    pr,
                    tab: PrTab::Commits,
                },
                ["checks", ..] => Pr {
                    pr,
                    tab: PrTab::Checks,
                },
                _ => return None,
            }
        }
        _ => return None,
    };
    Some(Target::Page(Route::Pulls(page)))
}

#[cfg(test)]
pub(crate) fn arb() -> impl proptest::strategy::Strategy<Value = Route> {
    use crate::route::arb::{query, repo};
    use proptest::prelude::*;
    let page = prop_oneof![
        (repo(), query()).prop_map(|(repo, query)| Pulls { repo, query }),
        repo().prop_map(NewPr),
        (repo(), 1..u64::MAX).prop_map(|(repo, number)| Pr {
            pr: PrRef { repo, number },
            tab: PrTab::Commits,
        }),
    ];
    page.prop_map(Route::Pulls)
}
