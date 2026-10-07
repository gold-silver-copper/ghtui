//! The GraphQL queries not typed with cynic: those built at run time, and
//! those read as JSON. They're all here so a test can check each against
//! GitHub's schema, and that none cuts a list short without saying so.

/// A team's fields, as [`crate::browse::wire_teams::Team`] reads them.
const TEAM: &str =
    "slug name description privacy members { totalCount } repositories { totalCount }";

/// A milestone's fields, as [`crate::browse::wire_milestones::Milestone`] reads them.
const MILESTONE: &str = "number title description dueOn closed closedAt updatedAt openIssues: issues(states: OPEN) { totalCount } doneIssues: issues(states: CLOSED) { totalCount } openPrs: pullRequests(states: OPEN) { totalCount } donePrs: pullRequests(states: [CLOSED, MERGED]) { totalCount }";

/// A search for discussions, 30 at a time.
pub(crate) const DISCUSSION_SEARCH: &str = "query($q: String!, $after: String) { search(type: DISCUSSION, query: $q, first: 30, after: $after) { discussionCount pageInfo { hasNextPage endCursor } nodes { ... on Discussion { repository { nameWithOwner } number title author { login } category { name } comments { totalCount } isAnswered upvoteCount updatedAt } } } }";

/// Branches by name, 30 at a time, each with its pull requests.
pub(crate) const BRANCHES: &str = "query($owner: String!, $name: String!, $after: String) { repository(owner: $owner, name: $name) { defaultBranchRef { name } refs(refPrefix: \"refs/heads/\", first: 30, after: $after, orderBy: {field: ALPHABETICAL, direction: ASC}) { totalCount pageInfo { hasNextPage endCursor } nodes { name target { ... on Commit { oid messageHeadline committedDate author { name user { login } } } } associatedPullRequests(first: 10, orderBy: {field: CREATED_AT, direction: DESC}) { nodes { number state headRefName repository { nameWithOwner } } } } } } }";

/// Deployments, newest first, and the environments.
pub(crate) const DEPLOYMENTS: &str = "query($owner: String!, $name: String!, $after: String, $envs: [String!]) { repository(owner: $owner, name: $name) { environments(first: 50) { totalCount nodes { name } } deployments(first: 25, after: $after, environments: $envs, orderBy: {field: CREATED_AT, direction: DESC}) { totalCount pageInfo { hasNextPage endCursor } nodes { environment state createdAt creator { login } ref { name } commitOid latestStatus { logUrl environmentUrl } } } } }";

/// A file's blame.
pub(crate) const BLAME: &str = "query($owner: String!, $name: String!, $rev: String!, $path: String!) { repository(owner: $owner, name: $name) { object(expression: $rev) { ... on Commit { blame(path: $path) { ranges { startingLine endingLine age commit { oid messageHeadline committedDate author { name user { login } } } } } } } } }";

/// Someone's public gists, most recently updated first.
pub(crate) const GISTS: &str = "query($login: String!, $after: String) { user(login: $login) { gists(first: 30, after: $after, privacy: PUBLIC, orderBy: {field: UPDATED_AT, direction: DESC}) { totalCount pageInfo { hasNextPage endCursor } nodes { name description updatedAt stargazerCount files(limit: 5) { name } comments { totalCount } } } } }";

/// Discussions' URLs and repositories, to find where an organization's are.
pub(crate) const DISCUSSION_URLS: &str = "query($q: String!, $after: String) { search(type: DISCUSSION, query: $q, first: 50, after: $after) { pageInfo { hasNextPage endCursor } nodes { ... on Discussion { url repository { nameWithOwner } } } } }";

/// A repository's discussion categories.
pub(crate) const DISCUSSION_CATEGORIES: &str = "query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { discussionCategories(first: 50) { totalCount nodes { id name slug } } } }";

/// A repository's discussions, most recently updated first.
pub(crate) const DISCUSSIONS: &str = "query($owner: String!, $name: String!, $after: String, $category: ID) { repository(owner: $owner, name: $name) { discussions(first: 25, after: $after, categoryId: $category, orderBy: {field: UPDATED_AT, direction: DESC}) { totalCount pageInfo { hasNextPage endCursor } nodes { number title author { login } category { name } comments { totalCount } isAnswered upvoteCount updatedAt } } } }";

/// A discussion and its comments.
pub(crate) const DISCUSSION: &str = "query($owner: String!, $name: String!, $number: Int!) { repository(owner: $owner, name: $name) { discussion(number: $number) { number title body author { login } createdAt category { name } isAnswered upvoteCount comments(first: 50) { totalCount nodes { databaseId author { login } body createdAt isAnswer upvoteCount replies(first: 30) { totalCount nodes { databaseId author { login } body createdAt } } } } } } }";

/// Branches by name, 100 at a time; with the first page, the newest 100 tags.
pub(crate) const REFS: &str = "query($owner: String!, $name: String!, $after: String, $tags: Boolean!) { repository(owner: $owner, name: $name) { heads: refs(refPrefix: \"refs/heads/\", first: 100, after: $after, orderBy: {field: ALPHABETICAL, direction: ASC}) { totalCount pageInfo { hasNextPage endCursor } nodes { name } } tags: refs(refPrefix: \"refs/tags/\", first: 100, orderBy: {field: TAG_COMMIT_DATE, direction: DESC}) @include(if: $tags) { totalCount pageInfo { hasNextPage endCursor } nodes { name } } } }";

/// The latest commit touching each of `paths`: one `history` each.
pub(crate) fn last_commits(paths: &[String]) -> String {
    let quote = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let fields: String = paths
        .iter()
        .enumerate()
        .map(|(i, path)| {
            format!(
                "e{i}: history(first: 1, path: \"{}\") {{ nodes {{ oid messageHeadline committedDate author {{ name user {{ login }} }} }} }}\n",
                quote(path)
            )
        })
        .collect();
    format!(
        "query($owner: String!, $name: String!, $rev: String!) {{ repository(owner: $owner, name: $name) {{ object(expression: $rev) {{ ... on Commit {{ {fields} }} }} }} }}"
    )
}

/// People in `field` of `root` (`repository(owner: $owner, name: $name)`
/// or `user(login: $login)`), 30 at a time.
pub(crate) fn users(root: &str, field: &str) -> String {
    let params = if root.starts_with("repository") {
        "$owner: String!, $name: String!"
    } else {
        "$login: String!"
    };
    format!(
        "query({params}, $after: String) {{ node: {root} {{ list: {field}(first: 30, after: $after) {{ totalCount pageInfo {{ hasNextPage endCursor }} nodes {{ login name bio }} }} }} }}"
    )
}

/// Open or closed milestones, 25 at a time, soonest due first (closed:
/// latest due first), with how many there are of each.
pub(crate) fn milestones(closed: bool) -> String {
    let state = if closed { "CLOSED" } else { "OPEN" };
    format!(
        "query($owner: String!, $name: String!, $after: String) {{ repository(owner: $owner, name: $name) {{ open: milestones(states: OPEN) {{ totalCount }} closed: milestones(states: CLOSED) {{ totalCount }} milestones(first: 25, after: $after, states: {state}, orderBy: {{field: DUE_DATE, direction: {}}}) {{ totalCount pageInfo {{ hasNextPage endCursor }} nodes {{ {MILESTONE} }} }} }} }}",
        if closed { "DESC" } else { "ASC" }
    )
}

/// A milestone.
pub(crate) fn milestone() -> String {
    format!(
        "query($owner: String!, $name: String!, $number: Int!) {{ repository(owner: $owner, name: $name) {{ milestone(number: $number) {{ {MILESTONE} }} }} }}"
    )
}

/// An organization's teams by name, 30 at a time.
pub(crate) fn teams() -> String {
    format!(
        "query($org: String!, $after: String) {{ organization(login: $org) {{ teams(first: 30, after: $after, orderBy: {{field: NAME, direction: ASC}}) {{ totalCount pageInfo {{ hasNextPage endCursor }} nodes {{ {TEAM} }} }} }} }}"
    )
}

/// A team: its members, repositories and child teams.
pub(crate) fn team() -> String {
    format!(
        "query($org: String!, $slug: String!) {{ organization(login: $org) {{ team(slug: $slug) {{ {TEAM} parentTeam {{ {TEAM} }} memberList: members(first: 50) {{ totalCount nodes {{ login name }} }} repoList: repositories(first: 50) {{ totalCount nodes {{ nameWithOwner description stargazerCount }} }} childTeams(first: 50) {{ totalCount nodes {{ {TEAM} }} }} }} }} }}"
    )
}
