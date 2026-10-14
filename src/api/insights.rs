//! A repository's Insights, as GitHub's API has them: its commit
//! activity and contributors (counted by GitHub; ghtui counts them with
//! git where it can, see [`crate::site::insights`]), its traffic, and its
//! dependency graph.

use serde::{Deserialize, Serialize};

use crate::api::model::{Capped, RepoId};
use crate::api::{ApiError, GitHub};

/// A week, in seconds.
pub const WEEK: u64 = 7 * 86_400;
/// The weeks commit activity covers, as GitHub's graph does.
pub const YEAR_WEEKS: usize = 52;
/// The most authors listed, as GitHub's contributors graph does.
pub const AUTHORS: usize = 100;

/// The Sunday 00:00 UTC that starts the week `secs` is in, as GitHub's
/// weeks start.
pub fn week_of(secs: u64) -> u64 {
    // 1970-01-01 was a Thursday: Sunday is four days later.
    const SUNDAY: u64 = 3 * 86_400;
    secs.saturating_sub(SUNDAY) / WEEK * WEEK + SUNDAY
}

/// How many whole weeks from `from` to `to`.
pub fn weeks_between(from: u64, to: u64) -> usize {
    usize::try_from(to.saturating_sub(from) / WEEK).unwrap_or(usize::MAX)
}

/// Who counted the commits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Counter {
    /// git, in a clone: the default branch's commits, merges left out,
    /// authors by their git names (with `.mailmap`).
    Git,
    /// GitHub's statistics: its top 100 contributors, by account.
    GitHub,
}

/// Commits week by week, in all and by author.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activity {
    /// The Sunday that starts the first week counted (unix seconds).
    pub start: u64,
    /// Commits in each week from `start` to the week counted in.
    pub weeks: Vec<u32>,
    /// Commits on each day of the week, Sunday first, in the last
    /// [`YEAR_WEEKS`] weeks.
    pub weekdays: [u32; 7],
    /// The authors with the most commits, most first.
    pub authors: Vec<Author>,
    /// Authors past the [`AUTHORS`] listed.
    pub authors_left_out: u64,
    /// Every commit counted.
    pub total: u64,
    /// What wasn't counted, said plainly.
    pub cut: Option<String>,
    pub by: Counter,
}

/// One author's commits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Author {
    /// Their git name, or GitHub login.
    pub name: String,
    /// Their GitHub account, where known.
    pub login: Option<String>,
    pub commits: u64,
    /// Commits in each week, as [`Activity::weeks`].
    pub weeks: Vec<u32>,
}

impl Activity {
    /// The last [`YEAR_WEEKS`] weeks, oldest first (none before the
    /// first commit), and the Sunday the first of them starts on.
    pub fn last_year(&self) -> (u64, Vec<u32>) {
        let skip = self.weeks.len().saturating_sub(YEAR_WEEKS);
        let had = self.weeks.get(skip..).unwrap_or_default();
        let before = YEAR_WEEKS.saturating_sub(had.len());
        let mut weeks = vec![0; before];
        weeks.extend_from_slice(had);
        let first = self.start + skip as u64 * WEEK;
        (first.saturating_sub(before as u64 * WEEK), weeks)
    }
}

/// Views or clones: in all, and day by day.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Counted {
    pub count: u64,
    pub uniques: u64,
    /// Each day, oldest first: its date (ISO 8601), count and uniques.
    pub days: Vec<(String, u64, u64)>,
}

/// Where visitors came from, or what they read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Popular {
    /// The site, or the path.
    pub name: String,
    /// A path's page title.
    pub title: Option<String>,
    pub count: u64,
    pub uniques: u64,
}

/// A repository's traffic in the last 14 days, as GitHub keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Traffic {
    pub views: Counted,
    pub clones: Counted,
    /// The top 10 referring sites.
    pub referrers: Vec<Popular>,
    /// The top 10 paths.
    pub paths: Vec<Popular>,
}

/// A package a manifest needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dependency {
    pub package: String,
    /// Its ecosystem (`RUST`, `NPM`).
    pub manager: String,
    /// The versions it takes (`>= 1.0, < 2`), as the manifest says.
    pub requirements: String,
    /// Its repository, where GitHub knows it.
    pub repo: Option<RepoId>,
}

/// A manifest GitHub read, and the dependencies it found there.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Its path in the repository.
    pub path: String,
    pub dependencies: Capped<Dependency>,
    /// GitHub couldn't read it.
    pub unparseable: bool,
    /// Too big for GitHub to read.
    pub too_big: bool,
}

/// A repository's dependency graph: its manifests and their dependencies.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Dependencies {
    pub manifests: Capped<Manifest>,
}

/// GitHub serves the dependency graph's dependencies only under this
/// preview (without it, every manifest counts none).
const HAWKGIRL: &str = "application/vnd.github.hawkgirl-preview+json";

impl GitHub {
    /// A repository's commit activity and contributors, as GitHub's
    /// statistics count them: asked where git can't count them. GitHub
    /// counts them on the first ask, and says so (202) until it has.
    pub async fn activity(&self, repo: &RepoId) -> Result<Activity, ApiError> {
        let path = format!("/repos/{repo}/stats/contributors");
        let answer = self.rest_answer(&path).await?;
        if answer.status() == 202 {
            return Err(ApiError::Http {
                status: 202,
                message: "GitHub is still counting the commits, as it does the first time it's asked; ask again in a moment".into(),
            });
        }
        answer.checked(format!("{repo}'s contributors"))?;
        // An empty repository: no content.
        let contributors: Vec<wire::Contributor> = if answer.body().trim().is_empty() {
            Vec::new()
        } else {
            serde_json::from_str(answer.body())?
        };
        let days = self.commit_days(repo).await;
        let activity = from_stats(contributors, days, crate::store::now());
        Ok(activity)
    }

    /// The last year's commits by weekday, as GitHub counts them; none
    /// while it's still counting.
    async fn commit_days(&self, repo: &RepoId) -> [u32; 7] {
        let path = format!("/repos/{repo}/stats/commit_activity");
        let weeks = match self.rest_answer(&path).await {
            Ok(a) if a.status() == 200 => serde_json::from_str::<Vec<wire::Week>>(a.body()),
            _ => return [0; 7],
        };
        let mut days = [0; 7];
        for week in weeks.unwrap_or_default() {
            for (day, n) in days.iter_mut().zip(week.days) {
                *day += n;
            }
        }
        days
    }

    /// A repository's views, clones, referrers and popular paths. GitHub
    /// shows them only to people who can push to it.
    pub async fn traffic(&self, repo: &RepoId) -> Result<Traffic, ApiError> {
        let base = format!("/repos/{repo}/traffic");
        let paths = [
            format!("{base}/views"),
            format!("{base}/clones"),
            format!("{base}/popular/referrers"),
            format!("{base}/popular/paths"),
        ];
        let [views, clones, referrers, paths] = &paths;
        let (views, clones, referrers, paths) = tokio::join!(
            self.traffic_part(views),
            self.traffic_part(clones),
            self.traffic_part(referrers),
            self.traffic_part(paths),
        );
        let views: wire::Counted = serde_json::from_str(&views?)?;
        let clones: wire::Counted = serde_json::from_str(&clones?)?;
        let referrers: Vec<wire::Referrer> = serde_json::from_str(&referrers?)?;
        let paths: Vec<wire::Path> = serde_json::from_str(&paths?)?;
        let traffic = Traffic {
            views: views.into_counted(|d| d.views),
            clones: clones.into_counted(|d| d.clones),
            referrers: (referrers.into_iter())
                .map(|r| Popular {
                    name: r.referrer,
                    title: None,
                    count: r.count,
                    uniques: r.uniques,
                })
                .collect(),
            paths: (paths.into_iter())
                .map(|p| Popular {
                    name: p.path,
                    title: Some(p.title).filter(|t| !t.is_empty()),
                    count: p.count,
                    uniques: p.uniques,
                })
                .collect(),
        };
        Ok(traffic)
    }

    /// One of the traffic's answers, or what GitHub's refusal means.
    async fn traffic_part(&self, path: &str) -> Result<String, ApiError> {
        let what = path.trim_start_matches("/repos/");
        let answer = self.rest_refusable(path, what, |status, message, _| {
            (status == 403).then(|| traffic_refusal(message))
        });
        let answer = answer.await?.map_err(ApiError::Denied)?;
        Ok(answer.body().to_owned())
    }

    /// A repository's manifests and their dependencies, as its dependency
    /// graph has them: 20 manifests, the first 100 dependencies of each.
    pub async fn dependencies(&self, repo: &RepoId) -> Result<Dependencies, ApiError> {
        let vars = serde_json::json!({ "owner": repo.owner, "name": repo.name });
        let found: wire::Manifests = self
            .graphql_preview(
                crate::api::raw::DEPENDENCY_GRAPH,
                vars,
                repo,
                "/repository/dependencyGraphManifests",
                HAWKGIRL,
            )
            .await?;
        let manifests = found.into_capped(|m| {
            Some(Manifest {
                path: m.blob_path.map_or(m.filename, |p| blob_path(&p)),
                dependencies: Capped::new(
                    (m.dependencies.nodes.into_iter().flatten())
                        .map(|d| Dependency {
                            package: d.package_name,
                            manager: d.package_manager.unwrap_or_default(),
                            requirements: d.requirements,
                            repo: d.repository.and_then(|r| RepoId::parse(&r.name_with_owner)),
                        })
                        .collect(),
                    (m.dependencies.total_count).max(m.dependencies_count.unwrap_or(0)),
                ),
                unparseable: !m.parseable,
                too_big: m.exceeds_max_size,
            })
        });
        Ok(Dependencies { manifests })
    }
}

/// A manifest's path from its blob's (`/o/r/blob/main/Cargo.toml`).
fn blob_path(path: &str) -> String {
    let rest = path.trim_start_matches('/').splitn(5, '/').nth(4);
    rest.unwrap_or(path).to_owned()
}

/// What GitHub's 403 for traffic means.
fn traffic_refusal(message: &str) -> String {
    let lower = message.to_lowercase();
    if lower.contains("push access") || lower.is_empty() {
        "GitHub shows a repository's traffic only to people who can push to it".to_owned()
    } else if lower.contains("not accessible by") {
        format!(
            "GitHub refused: {message}. A fine-grained token needs Administration: read; a classic one, the repo scope"
        )
    } else {
        format!("GitHub refused: {message}")
    }
}

/// [`Activity`] from GitHub's statistics: its contributors' weeks, and
/// the last year's days.
fn from_stats(contributors: Vec<wire::Contributor>, weekdays: [u32; 7], now: u64) -> Activity {
    let first = (contributors.iter())
        .flat_map(|c| c.weeks.iter().filter(|w| w.c > 0).map(|w| w.w))
        .min()
        .unwrap_or_else(|| week_of(now));
    let start = week_of(first);
    let len = weeks_between(start, week_of(now)).saturating_add(1);
    let mut weeks = vec![0u32; len];
    let mut authors: Vec<Author> = (contributors.into_iter())
        .map(|c| {
            let mut own = vec![0u32; len];
            for w in c.weeks.iter().filter(|w| w.c > 0) {
                let at = weeks_between(start, week_of(w.w));
                if let (Some(mine), Some(all)) = (own.get_mut(at), weeks.get_mut(at)) {
                    *mine += w.c;
                    *all += w.c;
                }
            }
            let (name, login) = match c.author {
                Some(a) => (a.login.clone(), Some(a.login)),
                None => ("Someone without an account".to_owned(), None),
            };
            Author {
                name,
                login,
                commits: c.total,
                weeks: own,
            }
        })
        .collect();
    authors.sort_by(|a, b| b.commits.cmp(&a.commits).then(a.name.cmp(&b.name)));
    let total = weeks.iter().map(|&n| u64::from(n)).sum();
    let listed = authors.len();
    let cut = (listed >= AUTHORS).then(|| {
        format!("GitHub's statistics list only the top {AUTHORS} contributors, and count only their commits")
    });
    Activity {
        start,
        weeks,
        weekdays,
        authors,
        authors_left_out: 0,
        total,
        cut,
        by: Counter::GitHub,
    }
}

/// The REST and GraphQL answers, as GitHub sends them.
mod wire {
    use serde::Deserialize;

    use crate::api::browse::UserLogin as Login;

    #[derive(Deserialize)]
    pub struct Contributor {
        pub author: Option<Login>,
        pub total: u64,
        pub weeks: Vec<StatWeek>,
    }

    #[derive(Deserialize)]
    pub struct StatWeek {
        /// The week's Sunday.
        pub w: u64,
        /// Commits.
        pub c: u32,
    }

    #[derive(Deserialize)]
    pub struct Week {
        pub days: Vec<u32>,
    }

    #[derive(Deserialize)]
    pub struct Day {
        pub timestamp: String,
        pub count: u64,
        pub uniques: u64,
    }

    #[derive(Deserialize)]
    pub struct Counted {
        pub count: u64,
        pub uniques: u64,
        #[serde(default)]
        pub views: Vec<Day>,
        #[serde(default)]
        pub clones: Vec<Day>,
    }

    impl Counted {
        pub fn into_counted(self, days: fn(Self) -> Vec<Day>) -> super::Counted {
            let (count, uniques) = (self.count, self.uniques);
            super::Counted {
                count,
                uniques,
                days: (days(self).into_iter())
                    .map(|d| (d.timestamp, d.count, d.uniques))
                    .collect(),
            }
        }
    }

    #[derive(Deserialize)]
    pub struct Referrer {
        pub referrer: String,
        pub count: u64,
        pub uniques: u64,
    }

    #[derive(Deserialize)]
    pub struct Path {
        pub path: String,
        #[serde(default)]
        pub title: String,
        pub count: u64,
        pub uniques: u64,
    }

    use crate::api::browse::wire::{Counted as Connection, RepoName as Repo};

    pub type Manifests = Connection<Manifest>;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Manifest {
        pub filename: String,
        pub blob_path: Option<String>,
        pub parseable: bool,
        pub exceeds_max_size: bool,
        /// All of them: the connection's own count is only its page's.
        pub dependencies_count: Option<u64>,
        pub dependencies: Connection<Dependency>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Dependency {
        pub package_name: String,
        pub package_manager: Option<String>,
        pub requirements: String,
        pub repository: Option<Repo>,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weeks_start_on_sunday() {
        // 2026-10-12 is a Monday; its week started on Sunday the 11th.
        let monday = crate::ui::time::parse_iso8601("2026-10-12T15:00:00Z").unwrap();
        let sunday = crate::ui::time::parse_iso8601("2026-10-11T00:00:00Z").unwrap();
        assert_eq!(week_of(monday), sunday);
        assert_eq!(week_of(sunday), sunday);
    }

    #[test]
    fn statistics_become_weeks_by_author() {
        let sunday = crate::ui::time::parse_iso8601("2026-09-27T00:00:00Z").unwrap();
        let json = format!(
            r#"[{{"author":{{"login":"alice"}},"total":3,"weeks":[{{"w":{sunday},"a":0,"d":0,"c":1}},{{"w":{},"a":0,"d":0,"c":2}}]}},
                {{"author":{{"login":"bob"}},"total":5,"weeks":[{{"w":{},"a":0,"d":0,"c":5}}]}}]"#,
            sunday + WEEK,
            sunday + WEEK
        );
        let contributors: Vec<wire::Contributor> = serde_json::from_str(&json).unwrap();
        let a = from_stats(contributors, [1, 2, 3, 0, 0, 0, 2], sunday + 2 * WEEK + 5);
        assert_eq!(a.start, sunday);
        assert_eq!(a.weeks, vec![1, 7, 0]);
        assert_eq!(a.total, 8);
        let names: Vec<_> = a
            .authors
            .iter()
            .map(|a| (a.name.as_str(), a.commits))
            .collect();
        assert_eq!(names, [("bob", 5), ("alice", 3)]);
        assert_eq!(
            a.authors.get(1).map(|a| a.weeks.clone()),
            Some(vec![1, 2, 0])
        );
    }

    #[test]
    fn a_manifests_path_is_read_from_its_blob() {
        assert_eq!(
            blob_path("/o/r/blob/main/xtask/Cargo.toml"),
            "xtask/Cargo.toml"
        );
        assert_eq!(blob_path("Cargo.lock"), "Cargo.lock");
    }
}
