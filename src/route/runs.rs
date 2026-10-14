//! A repository's Actions tab: its workflow runs, a run, a job and its
//! log, a workflow and the form that runs it, and its deployments.

use super::{Family, Route, Target, Url};
use crate::api::model::RepoId;
use crate::browse::{Build, Width};
use crate::chrome::Section;
use crate::data::DataKey;
use crate::state::State;
use crate::ui::pages::{self, url as links};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Page {
    /// Workflow runs, filtered as GitHub's Actions tab filters them
    /// (`branch:main is:failure`).
    Actions { repo: RepoId, query: String },
    /// A workflow run, or one attempt of it.
    WorkflowRun {
        repo: RepoId,
        run: u64,
        attempt: Option<u64>,
    },
    /// A job: its steps and its log. `step` is the step and line a link
    /// points at (expanded and scrolled to); `query` filters the log.
    Job {
        repo: RepoId,
        run: Option<u64>,
        job: u64,
        step: Option<(u32, u32)>,
        query: String,
    },
    /// A workflow, by its file name, and its runs.
    Workflow { repo: RepoId, file: String },
    /// The form that runs a workflow by hand (GitHub's "Run workflow"
    /// menu, on the workflow's page).
    Dispatch { repo: RepoId, file: String },
    /// Deployments, to one environment if given.
    Deployments {
        repo: RepoId,
        environment: Option<String>,
    },
}

pub use Page::*;

impl Page {
    fn repo_id(&self) -> &RepoId {
        match self {
            Actions { repo, .. }
            | WorkflowRun { repo, .. }
            | Job { repo, .. }
            | Workflow { repo, .. }
            | Dispatch { repo, .. }
            | Deployments { repo, .. } => repo,
        }
    }
}

impl Family for Page {
    fn url(&self) -> String {
        match self {
            Actions { repo, query } if query.is_empty() => format!("{}/actions", links::repo(repo)),
            Actions { repo, query } => format!(
                "{}/actions?query={}",
                links::repo(repo),
                links::encode(query)
            ),
            WorkflowRun { repo, run, attempt } => {
                let url = format!("{}/actions/runs/{run}", links::repo(repo));
                match attempt {
                    Some(n) => format!("{url}/attempts/{n}"),
                    None => url,
                }
            }
            Job {
                repo,
                run,
                job,
                step,
                query,
            } => {
                let mut url = match run {
                    Some(run) => format!("{}/actions/runs/{run}/job/{job}", links::repo(repo)),
                    None => format!("{}/runs/{job}", links::repo(repo)),
                };
                if !query.is_empty() {
                    url.push_str(&format!("?q={}", links::encode(query)));
                }
                if let Some((step, line)) = step {
                    url.push_str(&format!("#step:{step}:{line}"));
                }
                url
            }
            Workflow { repo, file } | Dispatch { repo, file } => format!(
                "{}/actions/workflows/{}",
                links::repo(repo),
                links::encode_path(file)
            ),
            Deployments { repo, environment } => match environment {
                Some(env) => format!(
                    "{}/deployments/activity_log?environments_filter={}",
                    links::repo(repo),
                    links::encode(env)
                ),
                None => format!("{}/deployments", links::repo(repo)),
            },
        }
    }

    fn title(&self) -> String {
        match self {
            Actions { repo, .. } => format!("{repo} · Actions"),
            WorkflowRun { repo, run, .. } => format!("{repo} · Run {run}"),
            Job { repo, job, .. } => format!("{repo} · Job {job}"),
            Workflow { repo, file } => format!("{repo} · {file}"),
            Dispatch { repo, file } => format!("{repo} · Run {file}"),
            Deployments { repo, .. } => format!("{repo} · Deployments"),
        }
    }

    fn repo(&self) -> Option<&RepoId> {
        Some(self.repo_id())
    }

    fn up(&self, _: &State) -> Option<Route> {
        let repo = self.repo_id().clone();
        Some(Route::Runs(match self {
            Actions { .. } | Deployments { .. } => {
                return Some(Route::Code(super::code::Repo(repo)));
            }
            Dispatch { file, .. } => Workflow {
                repo,
                file: file.clone(),
            },
            WorkflowRun { .. } | Workflow { .. } | Job { run: None, .. } => Actions {
                repo,
                query: String::new(),
            },
            Job { run: Some(run), .. } => WorkflowRun {
                repo,
                run: *run,
                attempt: None,
            },
        }))
    }

    fn needs(&self) -> Vec<DataKey> {
        use DataKey as K;
        let repo = self.repo_id();
        let own = match self {
            WorkflowRun { run, attempt, .. } => vec![K::Run(repo.clone(), *run, *attempt)],
            Job { job, .. } => vec![K::Job(repo.clone(), *job), K::JobLog(repo.clone(), *job)],
            Workflow { file, .. } => std::iter::once(K::Workflow(repo.clone(), file.clone()))
                .chain(self.paged())
                .collect(),
            Dispatch { file, .. } => return crate::workflows::needs(repo, file),
            Actions { .. } | Deployments { .. } => self.paged().into_iter().collect(),
        };
        std::iter::once(K::Repo(repo.clone())).chain(own).collect()
    }

    fn paged(&self) -> Option<DataKey> {
        Some(match self {
            Actions { repo, query } => DataKey::Runs(repo.clone(), query.clone()),
            Workflow { repo, file } => DataKey::Runs(repo.clone(), format!("workflow:{file}")),
            Deployments { repo, environment } => {
                DataKey::Deployments(repo.clone(), environment.clone())
            }
            WorkflowRun { .. } | Job { .. } | Dispatch { .. } => return None,
        })
    }

    fn width(&self) -> Width {
        match self {
            Dispatch { .. } => Width::Reading,
            _ => Width::Full,
        }
    }

    fn query(&self) -> Option<&str> {
        match self {
            Actions { query, .. } => Some(query),
            _ => None,
        }
    }

    fn with_query(&self, query: String) -> Option<Route> {
        let repo = self.repo_id().clone();
        matches!(self, Actions { .. }).then_some(Route::Runs(Actions { repo, query }))
    }

    fn section(&self) -> Section {
        match self {
            Deployments { .. } => Section::Code,
            _ => Section::Actions,
        }
    }

    fn form(&self) -> Option<&'static crate::forms::Kind> {
        matches!(self, Dispatch { .. }).then_some(&crate::workflows::FORM)
    }

    fn build(&self, b: Build<'_>, page: &mut crate::ui::page::Page) {
        let (cx, now) = (b.cx, b.cx.now);
        let title = self.title();
        match self {
            Actions { repo, query } => pages::actions(page, repo, query, b.list(self.paged()), cx),
            WorkflowRun { repo, run, attempt } => {
                let key = DataKey::Run(repo.clone(), *run, *attempt);
                if let Some(r) = b.get(&key).show(page, &title) {
                    pages::workflow_run(page, repo, r, now);
                }
            }
            Job {
                repo,
                job,
                step,
                query,
                ..
            } => {
                let key = DataKey::Job(repo.clone(), *job);
                if let Some(j) = b.get(&key).show(page, &title) {
                    let log = b.get(&DataKey::JobLog(repo.clone(), *job));
                    let at = pages::JobAt {
                        step: *step,
                        query,
                        keys: cx.keys,
                        follows: crate::live::page::follows(b.state, repo, *job),
                    };
                    pages::job(page, repo, j, log, at, now);
                }
            }
            Workflow { repo, file } => {
                let wf = b.get(&DataKey::Workflow(repo.clone(), file.clone()));
                pages::workflow(page, repo, wf, b.list(self.paged()), now);
            }
            Dispatch { repo, file } => crate::workflows::build(b.state, page, repo, file),
            Deployments { repo, environment } => {
                let env = environment.as_deref();
                pages::deployments(page, repo, b.list(self.paged()), env, now);
            }
        }
    }
}

/// `step:3:12`, as GitHub links a line of a job's log.
fn step_line(fragment: &str) -> Option<(u32, u32)> {
    let mut parts = fragment.strip_prefix("step:")?.split(':');
    let step = parts.next()?.parse().ok()?;
    let line = parts.next().and_then(|l| l.parse().ok()).unwrap_or(1);
    Some((step, line))
}

pub(super) fn parse_repo(repo: &RepoId, rest: &[&str], url: &Url<'_>) -> Option<Target> {
    let repo = repo.clone();
    let step = || url.fragment().and_then(step_line);
    let query = || url.param("q").unwrap_or_default();
    let page = match rest {
        ["actions"] => Actions {
            repo,
            query: (url.param("query"))
                .map(|q| q.trim().to_owned())
                .unwrap_or_default(),
        },
        ["actions", "runs", run, rest @ ..] => {
            let run = run.parse().ok()?;
            match rest {
                // The run's workflow file shows on the run's page.
                [] | ["workflow"] => WorkflowRun {
                    repo,
                    run,
                    attempt: None,
                },
                ["attempts", n] => WorkflowRun {
                    repo,
                    run,
                    attempt: Some(n.parse().ok()?),
                },
                ["job", job] => Job {
                    repo,
                    run: Some(run),
                    job: job.parse().ok()?,
                    step: step(),
                    query: query(),
                },
                _ => return None,
            }
        }
        // A check run's URL, which is its job's for Actions.
        ["runs", job] | ["runs", _, "jobs", job] => Job {
            repo,
            run: None,
            job: job.parse().ok()?,
            step: step(),
            query: query(),
        },
        ["actions", "workflows", file]
            if !file.contains(|c: char| c.is_control() || c.is_whitespace()) =>
        {
            Workflow {
                repo,
                file: (*file).to_owned(),
            }
        }
        ["deployments", rest @ ..] => {
            let environment = match rest {
                [] => None,
                ["activity_log"] => url.param("environments_filter"),
                [env] if !env.contains(char::is_control) => Some((*env).to_owned()),
                _ => return None,
            };
            Deployments { repo, environment }
        }
        _ => return None,
    };
    Some(Target::Page(Route::Runs(page)))
}

#[cfg(test)]
pub(crate) fn arb() -> impl proptest::strategy::Strategy<Value = Route> {
    use crate::route::arb::{query, repo};
    use proptest::prelude::*;
    let page = prop_oneof![
        (repo(), query()).prop_map(|(repo, query)| Actions { repo, query }),
        (repo(), 1..u64::MAX, prop::option::of(1..100u64))
            .prop_map(|(repo, run, attempt)| WorkflowRun { repo, run, attempt }),
        (
            repo(),
            prop::option::of(1..u64::MAX),
            1..u64::MAX,
            prop::option::of((1..100u32, 1..10_000u32)),
            prop_oneof![Just(String::new()), query()],
        )
            .prop_map(|(repo, run, job, step, query)| Job {
                repo,
                run,
                job,
                step,
                query
            }),
        (repo(), "[A-Za-z0-9_-][A-Za-z0-9_.-]{0,11}\\.ya?ml")
            .prop_map(|(repo, file)| Workflow { repo, file }),
        (repo(), prop::option::of("[A-Za-z0-9 _-]{1,12}"))
            .prop_filter("not the log", |(_, e)| e.as_deref() != Some("activity_log"))
            .prop_map(|(repo, environment)| Deployments { repo, environment }),
    ];
    page.prop_map(Route::Runs)
}
