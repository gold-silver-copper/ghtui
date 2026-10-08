//! What ghtui assumes of GitHub, checked against GitHub. Each test is a
//! dated fact on data that doesn't change (old tags, merged PRs), checked
//! through ghtui's own client where it can be. They read only, and are
//! ignored by default:
//!
//! ```text
//! cargo test -p ghtui-api --test contract -- --ignored contract
//! ```
//!
//! With no token (`GH_TOKEN`, `GITHUB_TOKEN` or `gh auth token`) each
//! passes without checking anything, so CI without the secret skips them.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "tests: failing loudly is the point"
)]

mod common;

use ghtui_api::GitHub;
use ghtui_api::model::{PrRef, RepoId};
use ghtui_store::Store;
use serde_json::{Value, json};

fn github() -> Option<GitHub> {
    let (token, _) = ghtui_api::auth::resolve_token().ok()?;
    Some(GitHub::new(token, Store::disabled()))
}

/// The client, or a quiet pass without a token.
macro_rules! github {
    () => {
        match github() {
            Some(gh) => gh,
            None => return,
        }
    };
}

async fn rest(gh: &GitHub, path: &str) -> Value {
    serde_json::from_str(&gh.rest_get(path).await.unwrap()).unwrap()
}

async fn graphql(gh: &GitHub, query: &str, at: &str) -> Value {
    gh.graphql_json(query, json!({}), "contract", at)
        .await
        .unwrap()
}

fn str_at<'a>(v: &'a Value, pointer: &str) -> &'a str {
    v.pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("no {pointer} in {v}"))
}

/// 2026-10-07. A comparison lists its newest 250 commits only without
/// `per_page`: with it, its first page is the oldest. (ghtui listed the
/// oldest 250 of big comparisons as if they were the newest.)
#[tokio::test]
#[ignore = "reads github.com"]
async fn contract_a_comparison_lists_its_newest_commits() {
    let gh = github!();
    let path = "/repos/ratatui/ratatui/compare/v0.26.0...v0.26.1";
    let whole = rest(&gh, path).await;
    let commits = whole["commits"].as_array().unwrap();
    assert_eq!(commits.len(), 20);
    let head = "efd1e476425477b0bce15e3dd96d5cdeb0e1174b";
    assert_eq!(str_at(&whole, "/commits/19/sha"), head);
    assert!(str_at(&whole, "/permalink_url").ends_with("...ratatui:efd1e47"));
    let paged = rest(&gh, &format!("{path}?per_page=1")).await;
    assert_eq!(
        str_at(&paged, "/commits/0/sha"),
        str_at(&whole, "/commits/0/sha")
    );
    // Through ghtui: the head is the head, and nothing doesn't add up.
    let c = gh
        .compare(&RepoId::new("ratatui", "ratatui"), "v0.26.0...v0.26.1")
        .await
        .unwrap();
    assert_eq!((c.to.as_str(), c.commits.len()), (head, 20));
    assert_eq!(gh.take_doubts(), Vec::<String>::new());
}

/// 2026-10-07. Branches ordered by `TAG_COMMIT_DATE` come back by name,
/// backwards (GitHub's order by name, which isn't bytes': `wm-x` comes
/// before `wm/x`), so ghtui asks for them by name.
#[tokio::test]
#[ignore = "reads github.com"]
async fn contract_branches_by_commit_date_are_by_name_backwards() {
    let gh = github!();
    let names = |order: &'static str| {
        let gh = gh.clone();
        async move {
            let query = format!(
                r#"query {{ repository(owner: "cli", name: "cli") {{ refs(refPrefix: "refs/heads/", first: 30, orderBy: {{{order}}}) {{ nodes {{ name }} }} }} }}"#
            );
            let data = graphql(&gh, &query, "/repository/refs").await;
            data["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n["name"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        }
    };
    let by_date = names("field: TAG_COMMIT_DATE, direction: DESC").await;
    assert_eq!(by_date, names("field: ALPHABETICAL, direction: DESC").await);
    // ghtui's own: by name, and as many as GitHub has, or saying so.
    let by_name = names("field: ALPHABETICAL, direction: ASC").await;
    let refs = gh.refs(&RepoId::new("cli", "cli")).await.unwrap();
    assert_eq!(refs.branches.get(..30), Some(by_name.as_slice()));
    assert!(refs.branches.left_out() == 0 || refs.branches.len() == 1000);
}

/// 2026-10-07. A branch's associated pull requests include other
/// repositories' (forks' PRs from a branch of the same name).
#[tokio::test]
#[ignore = "reads github.com"]
async fn contract_a_branch_has_other_repositories_prs() {
    let gh = github!();
    let data = graphql(
        &gh,
        r#"query { repository(owner: "cli", name: "cli") { ref(qualifiedName: "refs/heads/trunk") { associatedPullRequests(first: 5, orderBy: {field: CREATED_AT, direction: ASC}) { nodes { repository { nameWithOwner } } } } } }"#,
        "/repository/ref/associatedPullRequests/nodes",
    )
    .await;
    let repos: Vec<&str> = data
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["repository"]["nameWithOwner"].as_str().unwrap())
        .collect();
    assert!(repos.iter().any(|r| *r != "cli/cli"), "{repos:?}");
}

/// 2026-10-07. A long PR's `commits(last: 1)` is its head, so its checks
/// are its head's.
#[tokio::test]
#[ignore = "reads github.com"]
async fn contract_a_long_prs_last_commit_is_its_head() {
    let gh = github!();
    let data = graphql(
        &gh,
        r#"query { repository(owner: "rust-lang", name: "rust") { pullRequest(number: 160692) { headRefOid commits(last: 1) { totalCount nodes { commit { oid } } } } } }"#,
        "/repository/pullRequest",
    )
    .await;
    let pr = &data;
    assert!(pr["commits"]["totalCount"].as_u64().unwrap() > 200);
    let head = "9e7db05c303aa06c15baa8d9c6c9b98adcaae23d";
    assert_eq!(str_at(pr, "/headRefOid"), head);
    assert_eq!(str_at(pr, "/commits/nodes/0/commit/oid"), head);
    let checks = gh
        .pr_checks(&PrRef::parse("rust-lang/rust#160692").unwrap())
        .await
        .unwrap();
    assert_eq!(checks.oid, head);
    assert_eq!(gh.take_doubts(), Vec::<String>::new());
}

/// 2026-10-07. A GraphQL search stops at 1,000 results, however many it
/// counts: the page from the 900th has no next.
#[tokio::test]
#[ignore = "reads github.com"]
async fn contract_a_search_stops_at_1000() {
    let gh = github!();
    let data = graphql(
        &gh,
        r#"query { search(type: ISSUE, query: "repo:rust-lang/rust is:issue is:closed", first: 100, after: "Y3Vyc29yOjkwMA==") { issueCount pageInfo { hasNextPage } nodes { __typename } } }"#,
        "/search",
    )
    .await;
    let search = &data;
    assert!(search["issueCount"].as_u64().unwrap() > 1000);
    assert_eq!(search["nodes"].as_array().unwrap().len(), 100);
    assert_eq!(search["pageInfo"]["hasNextPage"], false);
}

/// 2026-10-07. A commit search's dates carry the committer's offset, not
/// UTC.
#[tokio::test]
#[ignore = "reads github.com"]
async fn contract_commit_search_dates_carry_offsets() {
    let gh = github!();
    let found = rest(
        &gh,
        "/search/commits?q=repo:torvalds/linux+author-date:2020-01-01..2020-01-02&per_page=10",
    )
    .await;
    let dates: Vec<&str> = found["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["commit"]["author"]["date"].as_str().unwrap())
        .collect();
    let offset = |d: &&str| {
        let tail = d.get(d.len().saturating_sub(6)..).unwrap_or_default();
        tail.starts_with(['+', '-']) && tail.get(3..4) == Some(":")
    };
    assert!(
        dates.iter().any(|d| offset(d) && !d.ends_with("+00:00")),
        "{dates:?}"
    );
}

/// 2026-10-07. A PR merged with a merge commit has its head behind its
/// base branch, so a diff from the branch is empty; from `baseRefOid` it's
/// the PR's.
#[tokio::test]
#[ignore = "reads github.com"]
async fn contract_a_merged_prs_diff_is_from_its_base_oid() {
    let gh = github!();
    let data = graphql(
        &gh,
        r#"query { repository(owner: "cli", name: "cli") { pullRequest(number: 14592) { mergeCommit { parents { totalCount } } baseRefName baseRefOid headRefOid changedFiles } } }"#,
        "/repository/pullRequest",
    )
    .await;
    let pr = &data;
    assert_eq!(pr["mergeCommit"]["parents"]["totalCount"], 2);
    let (base, oid, head) = (
        str_at(pr, "/baseRefName"),
        str_at(pr, "/baseRefOid"),
        str_at(pr, "/headRefOid"),
    );
    let from_branch = rest(&gh, &format!("/repos/cli/cli/compare/{base}...{head}")).await;
    assert_eq!(from_branch["status"], "behind");
    assert_eq!(from_branch["files"].as_array().map_or(0, Vec::len), 0);
    let from_oid = rest(&gh, &format!("/repos/cli/cli/compare/{oid}...{head}")).await;
    assert_eq!(
        from_oid["files"].as_array().unwrap().len() as u64,
        pr["changedFiles"].as_u64().unwrap()
    );
}

/// 2026-10-07. Lists come in the order ghtui shows them: a conversation's
/// comments and reviews oldest first (ghtui asks for the newest with
/// `last:`), releases and tags newest first, and a run's jobs a page at a
/// time by `per_page` and `page`.
#[tokio::test]
#[ignore = "reads github.com"]
async fn contract_lists_keep_their_order() {
    let gh = github!();
    let pr = PrRef::parse("cli/cli#9000").unwrap();
    let activity = gh.pr_activity(&pr).await.unwrap();
    assert!(!activity.comments.is_empty() && !activity.reviews.is_empty());
    let sorted = |dates: Vec<&str>| dates.windows(2).all(|w| w[0] <= w[1]);
    assert!(sorted(
        activity
            .comments
            .iter()
            .map(|c| c.created_at.as_str())
            .collect()
    ));
    assert!(sorted(
        activity
            .reviews
            .iter()
            .map(|r| r.submitted_at.as_str())
            .collect()
    ));
    assert!(sorted(
        activity.commits.iter().map(|c| c.date.as_str()).collect()
    ));

    let ratatui = RepoId::new("ratatui", "ratatui");
    let releases = gh.releases(&ratatui, None).await.unwrap();
    let published: Vec<&str> = releases
        .items
        .iter()
        .filter_map(|r| r.published_at.as_deref())
        .collect();
    assert!(published.windows(2).all(|w| w[0] >= w[1]), "{published:?}");
    let tags = gh.tags(&ratatui, None).await.unwrap();
    let dated: Vec<&str> = tags
        .items
        .iter()
        .filter_map(|t| t.date.as_deref())
        .collect();
    assert!(dated.windows(2).all(|w| w[0] >= w[1]), "{dated:?}");

    // The newest finished CI run's jobs, one to a page.
    let runs = rest(
        &gh,
        "/repos/ratatui/ratatui/actions/runs?status=completed&per_page=20",
    )
    .await;
    let run = runs["workflow_runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "Continuous Integration")
        .expect("a CI run")["id"]
        .as_u64()
        .unwrap();
    let jobs = |page: u32| {
        let gh = gh.clone();
        async move {
            rest(
                &gh,
                &format!("/repos/ratatui/ratatui/actions/runs/{run}/jobs?per_page=1&page={page}"),
            )
            .await
        }
    };
    let (one, two) = (jobs(1).await, jobs(2).await);
    assert!(one["total_count"].as_u64().unwrap() >= 2);
    assert_ne!(one["jobs"][0]["id"], two["jobs"][0]["id"]);
}

/// Records the corpus `tests/corpus.rs` replays: the requests in
/// `common::run`, live, each checked as it comes. They're written to
/// `$GHTUI_RECORD` (default: the temporary directory's `ghtui-corpus`), for
/// a person to look over and copy into `tests/corpus`.
#[tokio::test]
#[ignore = "reads github.com"]
async fn contract_record_corpus() {
    let gh = github!();
    let dir = std::env::var("GHTUI_RECORD").map_or_else(
        |_| std::env::temp_dir().join("ghtui-corpus"),
        std::path::PathBuf::from,
    );
    std::fs::create_dir_all(&dir).unwrap();
    common::run(&gh.recording(dir)).await;
}

/// Records the job-log corpus `crates/ui/tests/job_logs.rs` replays: each
/// case's job (as ghtui models it) and its raw log, into `$GHTUI_RECORD`
/// (default: the temporary directory's `ghtui-logs`). GitHub keeps logs
/// for 90 days, so these job IDs stop working; the corpus doesn't.
#[tokio::test]
#[ignore = "reads github.com"]
async fn contract_record_logs() {
    let gh = github!();
    let dir = std::env::var("GHTUI_RECORD").map_or_else(
        |_| std::env::temp_dir().join("ghtui-logs"),
        std::path::PathBuf::from,
    );
    std::fs::create_dir_all(&dir).unwrap();
    // A BOM, a composite action, groups and `[command]` lines.
    // A re-run (attempt 2) with debug logging.
    // A log over 2 MB.
    // A `##[warning]`.
    // A container job's `##[command]` lines.
    let cases = [
        ("ratatui-clippy", "ratatui/ratatui", 112_630_463_742_u64),
        ("deno-rerun-debug", "denoland/deno", 112_490_402_200),
        ("rust-dist-big", "rust-lang/rust", 112_723_124_879),
        ("cli-warning", "cli/cli", 95_690_755_657),
        ("iputils-container", "iputils/iputils", 112_025_073_907),
    ];
    for (name, repo, id) in cases {
        let repo = RepoId::parse(repo).unwrap();
        let job = gh.job(&repo, id).await.unwrap();
        let json = serde_json::to_string_pretty(&job).unwrap();
        std::fs::write(dir.join(format!("{name}.job.json")), json).unwrap();
        let path = format!("/repos/{}/{}/actions/jobs/{id}/logs", repo.owner, repo.name);
        let log = gh.rest_get(&path).await.unwrap();
        std::fs::write(dir.join(format!("{name}.log")), log).unwrap();
    }
}
