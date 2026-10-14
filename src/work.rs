//! Work in the background besides fetching data: writes to GitHub, and
//! git in your clone. Each is a [`Job`]: what's done, run away from the
//! screen, and what its answer does to the state. [`Work`] is every job,
//! sent as [`Cmd::Work`]; [`Done`] is a job with what came of it,
//! answered as [`Msg::Done`](crate::state::Msg::Done).

use std::future::Future;

use crate::api::{ApiError, GitHub};
use crate::diff_job::GitContext;
use crate::state::{Cmd, State};

/// What a job runs with: GitHub, and where your clone and git's are.
pub struct Cx {
    pub gh: GitHub,
    pub git: GitContext,
}

/// A piece of background work and what its answer does.
pub trait Job: Clone + std::fmt::Debug + PartialEq + Eq + Send + Sync + 'static {
    type Out: std::fmt::Debug + Send + 'static;
    /// Whether it changes GitHub when it works: then what any page shows
    /// may be old.
    const WRITES: bool = true;
    /// Whether GitHub changed, as it answered: when it worked, if it
    /// [`WRITES`](Job::WRITES).
    fn wrote(result: &Result<Self::Out, Failure>) -> bool {
        Self::WRITES && result.is_ok()
    }
    fn run(&self, cx: &Cx) -> impl Future<Output = Result<Self::Out, Failure>> + Send;
    fn done(self, state: &mut State, result: Result<Self::Out, Failure>) -> Vec<Cmd>;
}

/// Starting `job`.
pub fn start(job: impl Into<Work>) -> Vec<Cmd> {
    vec![Cmd::Work(job.into())]
}

/// Why background work failed. Shows as GitHub's own words when GitHub
/// refused it, else as its message with its causes (`a: b`); `{:?}` has
/// everything, for the log.
#[derive(Debug)]
pub struct Failure(anyhow::Error);

impl Failure {
    pub fn msg(text: impl std::fmt::Display + std::fmt::Debug + Send + Sync + 'static) -> Self {
        Self(anyhow::Error::msg(text))
    }

    /// GitHub's error, when it was one.
    pub fn api(&self) -> Option<&ApiError> {
        self.0.downcast_ref()
    }
}

impl<E: std::error::Error + Send + Sync + 'static> From<E> for Failure {
    fn from(err: E) -> Self {
        Self(err.into())
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.api() {
            Some(ApiError::GraphQl(messages)) => write!(f, "{}", messages.join("; ")),
            Some(ApiError::Http { message, .. }) => write!(f, "{message}"),
            _ => write!(f, "{:#}", self.0),
        }
    }
}

/// [`Work`] and [`Done`], with a variant for each job, and how each is
/// run and answered.
macro_rules! jobs {
    ($($(#[$meta:meta])* $name:ident($job:ty),)*) => {
        /// Work in the background, answered by [`Done`].
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub enum Work {
            $($(#[$meta])* $name($job),)*
        }

        /// A job, and what came of it.
        #[derive(Debug)]
        pub enum Done {
            $($name($job, Result<<$job as Job>::Out, Failure>),)*
        }

        $(impl From<$job> for Work {
            fn from(job: $job) -> Self {
                Work::$name(job)
            }
        })*

        impl Work {
            pub async fn run(self, cx: &Cx) -> Done {
                match self {
                    $(Work::$name(job) => {
                        let result = job.run(cx).await;
                        Done::$name(job, result)
                    })*
                }
            }

            /// Its answer, had it failed: for when the task running it
            /// panics.
            pub fn failed(&self) -> Done {
                match self {
                    $(Work::$name(job) => {
                        Done::$name(job.clone(), Err(Failure::msg("ghtui hit a bug")))
                    })*
                }
            }
        }

        impl Done {
            /// Whether GitHub changed: what any page shows may be old.
            pub fn wrote(&self) -> bool {
                match self {
                    $(Done::$name(_, result) => <$job as Job>::wrote(result),)*
                }
            }

            #[must_use]
            pub fn apply(self, state: &mut State) -> Vec<Cmd> {
                match self {
                    $(Done::$name(job, result) => job.done(state, result),)*
                }
            }
        }

        $(#[cfg(test)]
        impl Sent for $job {
            fn of(work: &Work) -> Option<&Self> {
                match work {
                    Work::$name(job) => Some(job),
                    _ => None,
                }
            }

            fn answered(self, result: Result<Self::Out, Failure>) -> Done {
                Done::$name(self, result)
            }
        })*
    };
}

/// A job as tests find it among the commands and answer it.
#[cfg(test)]
pub trait Sent: Job {
    fn of(work: &Work) -> Option<&Self>;
    fn answered(self, result: Result<Self::Out, Failure>) -> Done;
}

jobs! {
    Change(crate::act::Changing),
    Subscribe(crate::watch::Subscribe),
    SetLists(crate::site::lists::SetLists),
    DeleteList(crate::site::lists::DeleteList),
    DeleteVersion(crate::site::packages::DeleteVersion),
    Label(crate::housekeeping::labels::Save),
    Milestone(crate::housekeeping::milestones::Save),
    Ref(crate::housekeeping::refs::Save),
    Fork(crate::housekeeping::repos::Fork),
    NewRepo(crate::housekeeping::repos::Make),
    CloneTo(crate::housekeeping::repos::CloneTo),
    Commit(crate::housekeeping::files::Commit),
    Release(crate::release::Save),
    DeleteRelease(crate::release::Delete),
    DeleteTag(crate::release::DeleteTag),
    Notes(crate::release::Generate),
    SaveAsset(crate::release::assets::Save),
    Upload(crate::release::assets::Upload),
    DeleteAsset(crate::release::assets::Delete),
    NewGist(crate::gist::Create),
    Gist(crate::gist::Update),
    DeleteGist(crate::gist::Delete),
    Wiki(crate::wiki::form::Save),
    Revert(crate::manage::Revert),
    Transfer(crate::manage::Transfer),
    StartDiscussion(crate::manage::Start),
    Checkout(crate::clone::Checkout),
    Push(crate::create::Push),
    Submit(crate::create::Submit),
    SaveArtifact(crate::workflows::SaveArtifact),
    SetViewed(crate::review::SetViewed),
    Resolve(crate::review::Resolve),
    SubmitReview(crate::review::Submit),
    Dispatch(crate::workflows::Dispatch),
}
