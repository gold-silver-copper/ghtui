//! Contributors and commit activity, counted with git: one `git log` of
//! the default branch in the repository's clone (yours, or the cache's,
//! whose blobless history has every commit), merges left out, as
//! GitHub's graphs leave them out. Authors are their git names, read
//! through the repository's `.mailmap`, as `git shortlog` groups them;
//! a GitHub no-reply address says their account.

use std::collections::HashMap;

use crate::api::insights::{AUTHORS, Activity, Author, Counter, WEEK, YEAR_WEEKS, week_of};
use crate::git::Oid;
use crate::git::read::ReadError;
use crate::git::repo::Repo;

/// The most commits counted: past it, the newest are, and the page says
/// so.
pub const COMMITS_MAX: usize = 200_000;

/// The repository's commit activity at `at` (its default branch's
/// head), weeks up to `now`'s.
pub async fn activity(repo: &Repo, at: &Oid, now: u64) -> Result<Activity, ReadError> {
    let count = format!("--max-count={}", COMMITS_MAX + 1);
    let out = repo
        .run(&[
            "log",
            "--no-merges",
            "--use-mailmap",
            "--format=%at%x1f%aN%x1f%aE",
            &count,
            at,
        ])
        .await?;
    Ok(count_commits(out.lines(), now))
}

/// Commits (`time␟name␟email`, newest first) counted by week and
/// author.
pub fn count_commits<'a>(lines: impl Iterator<Item = &'a str>, now: u64) -> Activity {
    let this_week = week_of(now);
    let mut commits: Vec<(u64, usize)> = Vec::new();
    // Each author's place, by name, and their account where an email
    // says it.
    let mut names: HashMap<String, usize> = HashMap::new();
    let mut authors: Vec<(String, Option<String>, u64)> = Vec::new();
    let mut cut = false;
    for line in lines {
        if commits.len() >= COMMITS_MAX {
            cut = true;
            break;
        }
        let mut f = line.split('\u{1f}');
        let (Some(time), Some(name), email) = (f.next(), f.next(), f.next().unwrap_or_default())
        else {
            continue;
        };
        let Ok(time) = time.trim().parse::<u64>() else {
            continue;
        };
        let name = name.trim();
        let i = *names.entry(name.to_owned()).or_insert_with(|| {
            authors.push((name.to_owned(), None, 0));
            authors.len() - 1
        });
        if let Some(a) = authors.get_mut(i) {
            a.2 += 1;
            if a.1.is_none() {
                let person = crate::api::browse::Person::from_git(name, email);
                a.1 = person.login().map(str::to_owned);
            }
        }
        // A commit dated in the future counts as now.
        commits.push((time.min(now), i));
    }
    let start = (commits.iter().map(|c| week_of(c.0)).min()).unwrap_or(this_week);
    let len = (this_week.saturating_sub(start) / WEEK + 1) as usize;
    let index = |week: u64| (week.saturating_sub(start) / WEEK) as usize;
    // The authors listed: the most commits first.
    let mut order: Vec<usize> = (0..authors.len()).collect();
    order.sort_by(|&a, &b| {
        let (a, b) = (authors.get(a), authors.get(b));
        let commits = |x: Option<&(String, Option<String>, u64)>| x.map_or(0, |x| x.2);
        let name = |x: Option<&(String, Option<String>, u64)>| x.map(|x| x.0.clone());
        commits(b).cmp(&commits(a)).then(name(a).cmp(&name(b)))
    });
    let listed: HashMap<usize, usize> = (order.iter().take(AUTHORS))
        .enumerate()
        .map(|(place, &i)| (i, place))
        .collect();
    let mut listed_authors: Vec<Author> = (order.iter().take(AUTHORS))
        .filter_map(|&i| authors.get(i))
        .map(|(name, login, commits)| Author {
            name: name.clone(),
            login: login.clone(),
            commits: *commits,
            weeks: vec![0; len],
        })
        .collect();
    let mut weeks = vec![0u32; len];
    let mut weekdays = [0u32; 7];
    let year_ago = this_week.saturating_sub((YEAR_WEEKS as u64 - 1) * WEEK);
    for &(time, author) in &commits {
        let at = index(week_of(time));
        if time >= year_ago {
            // 1970-01-01 was a Thursday.
            let day = ((time / 86_400 + 4) % 7) as usize;
            if let Some(n) = weekdays.get_mut(day) {
                *n += 1;
            }
        }
        if let Some(n) = weeks.get_mut(at) {
            *n += 1;
        }
        if let Some(n) = (listed.get(&author))
            .and_then(|&place| listed_authors.get_mut(place))
            .and_then(|a| a.weeks.get_mut(at))
        {
            *n += 1;
        }
    }
    let total = commits.len() as u64;
    let cut = cut.then(|| {
        format!(
            "Counted the newest {} commits; older ones aren't",
            crate::ui::pages::compact(COMMITS_MAX as u64)
        )
    });
    Activity {
        start,
        weeks,
        weekdays,
        authors: listed_authors,
        authors_left_out: authors.len().saturating_sub(AUTHORS) as u64,
        total,
        cut,
        by: Counter::Git,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &std::path::Path, args: &[&str], when: &str) {
        let out = std::process::Command::new("git")
            .current_dir(dir)
            .args(args)
            .env("GIT_AUTHOR_DATE", when)
            .env("GIT_COMMITTER_DATE", when)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn commit(dir: &std::path::Path, who: &str, when: &str) {
        let author = format!("--author={who}");
        git(
            dir,
            &["commit", "-q", "--allow-empty", "-m", "c", &author],
            when,
        );
    }

    /// Counted by git: weeks and weekdays from each commit's date,
    /// merges left out, and two names of one person made one by the
    /// mailmap.
    #[tokio::test]
    async fn git_counts_commits_by_week_author_and_day() {
        let dir = tempfile::tempdir().unwrap();
        let top = dir.path();
        // Sunday 2026-09-27, and the Tuesday and Wednesday after.
        let (sun, tue, wed) = (
            "2026-09-27T12:00:00Z",
            "2026-09-29T12:00:00Z",
            "2026-09-30T12:00:00Z",
        );
        git(top, &["init", "-q", "-b", "main"], sun);
        git(top, &["config", "user.name", "T"], sun);
        git(top, &["config", "user.email", "t@example.com"], sun);
        commit(top, "Alice <alice@example.com>", sun);
        commit(top, "Al <al@old.example>", tue);
        commit(top, "Bob <1+bob@users.noreply.github.com>", tue);
        // A merge isn't counted.
        git(top, &["switch", "-q", "-c", "side"], tue);
        commit(top, "Bob <1+bob@users.noreply.github.com>", wed);
        git(top, &["switch", "-q", "main"], wed);
        commit(top, "Alice <alice@example.com>", wed);
        git(top, &["merge", "-q", "--no-ff", "-m", "Merge", "side"], wed);
        let mailmap = "Alice <alice@example.com> Al <al@old.example>\n";
        std::fs::write(top.join(".mailmap"), mailmap).unwrap();
        git(top, &["add", ".mailmap"], wed);
        commit(top, "Alice <alice@example.com>", wed);

        let repo = Repo::open_local(top, "origin").await.unwrap();
        let head = repo.rev_parse("HEAD").await.unwrap();
        // A week and a day after the last commit.
        let now = crate::ui::time::parse_iso8601("2026-10-08T12:00:00Z").unwrap();
        let a = activity(&repo, &head, now).await.unwrap();

        assert_eq!((a.total, a.by, a.cut.clone()), (6, Counter::Git, None));
        let sunday = crate::ui::time::parse_iso8601("2026-09-27T00:00:00Z").unwrap();
        assert_eq!(a.start, sunday);
        assert_eq!(a.weeks, vec![6, 0]);
        // Sunday 1, Tuesday 2, Wednesday 3.
        assert_eq!(a.weekdays, [1, 0, 2, 3, 0, 0, 0]);
        let authors: Vec<_> = (a.authors.iter())
            .map(|a| (a.name.as_str(), a.login.as_deref(), a.commits))
            .collect();
        assert_eq!(authors, [("Alice", None, 4), ("Bob", Some("bob"), 2)]);
    }

    #[test]
    fn only_so_many_commits_are_counted() {
        let line = "1790000000\u{1f}A\u{1f}a@example.com";
        let lines = std::iter::repeat_n(line, COMMITS_MAX + 1);
        let a = count_commits(lines, 1_790_000_000);
        assert_eq!(a.total, COMMITS_MAX as u64);
        assert!(a.cut.is_some_and(|c| c.contains("200k")));
    }
}
