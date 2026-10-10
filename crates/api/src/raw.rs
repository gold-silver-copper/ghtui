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
pub(crate) const BRANCHES: &str = "query($owner: String!, $name: String!, $after: String) { repository(owner: $owner, name: $name) { defaultBranchRef { name } refs(refPrefix: \"refs/heads/\", first: 30, after: $after, orderBy: {field: ALPHABETICAL, direction: ASC}) { totalCount pageInfo { hasNextPage endCursor } nodes { name target { ... on Commit { oid messageHeadline committedDate author { name user { login } } } } associatedPullRequests(first: 10, orderBy: {field: CREATED_AT, direction: DESC}) { nodes { number state isDraft headRefName repository { nameWithOwner } } } } } } }";

/// Deployments, newest first, and the environments.
pub(crate) const DEPLOYMENTS: &str = "query($owner: String!, $name: String!, $after: String, $envs: [String!]) { repository(owner: $owner, name: $name) { environments(first: 50) { totalCount nodes { name } } deployments(first: 25, after: $after, environments: $envs, orderBy: {field: CREATED_AT, direction: DESC}) { totalCount pageInfo { hasNextPage endCursor } nodes { environment state createdAt creator { login } ref { name } commitOid latestStatus { logUrl environmentUrl } } } } }";

/// A file's blame.
pub(crate) const BLAME: &str = "query($owner: String!, $name: String!, $rev: String!, $path: String!) { repository(owner: $owner, name: $name) { object(expression: $rev) { ... on Commit { blame(path: $path) { ranges { startingLine endingLine age commit { oid messageHeadline committedDate author { name user { login } } } } } } } } }";

/// Someone's public gists, most recently updated first.
pub(crate) const GISTS: &str = "query($login: String!, $after: String) { user(login: $login) { gists(first: 30, after: $after, privacy: PUBLIC, orderBy: {field: UPDATED_AT, direction: DESC}) { totalCount pageInfo { hasNextPage endCursor } nodes { name description updatedAt stargazerCount files(limit: 5) { name } comments { totalCount } } } } }";

/// Discussions' URLs and repositories, to find where an organization's are.
pub(crate) const DISCUSSION_URLS: &str = "query($q: String!, $after: String) { search(type: DISCUSSION, query: $q, first: 100, after: $after) { discussionCount pageInfo { hasNextPage endCursor } nodes { ... on Discussion { url repository { nameWithOwner } } } } }";

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

/// Which of `names` are a branch or tag: one `ref` each (`$r0`, `$r1`…),
/// null for a name that isn't one.
pub(crate) fn refs_named(names: usize) -> String {
    let params: String = (0..names).map(|i| format!(", $r{i}: String!")).collect();
    let fields: String = (0..names)
        .map(|i| format!("r{i}: ref(qualifiedName: $r{i}) {{ name }} "))
        .collect();
    format!(
        "query($owner: String!, $name: String!{params}) {{ repository(owner: $owner, name: $name) {{ {fields}}} }}"
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

/// How many each search finds (`$q0`, `$q1`… of `types`), none of them read.
pub(crate) fn search_counts(types: &[crate::browse::SearchType]) -> String {
    use crate::browse::SearchType as T;
    let params: String = (0..types.len())
        .map(|i| format!("$q{i}: String!"))
        .collect::<Vec<_>>()
        .join(", ");
    let fields: String = (types.iter().enumerate())
        .map(|(i, t)| {
            let (name, count) = match t {
                T::Repository => ("REPOSITORY", "repositoryCount"),
                T::User => ("USER", "userCount"),
                T::Discussion => ("DISCUSSION", "discussionCount"),
                T::Issue | T::IssueAdvanced | T::IssueHybrid | T::IssueSemantic => {
                    ("ISSUE", "issueCount")
                }
            };
            format!("c{i}: search(type: {name}, query: $q{i}, first: 0) {{ {count} }} ")
        })
        .collect();
    format!("query({params}) {{ {fields}}}")
}

/// Every query here, by name, built with sample input.
#[cfg(test)]
pub(crate) fn all() -> Vec<(&'static str, String)> {
    let mut all: Vec<(&str, String)> = vec![
        ("DISCUSSION_SEARCH", DISCUSSION_SEARCH.to_owned()),
        ("BRANCHES", BRANCHES.to_owned()),
        ("DEPLOYMENTS", DEPLOYMENTS.to_owned()),
        ("BLAME", BLAME.to_owned()),
        ("GISTS", GISTS.to_owned()),
        ("DISCUSSION_URLS", DISCUSSION_URLS.to_owned()),
        ("DISCUSSION_CATEGORIES", DISCUSSION_CATEGORIES.to_owned()),
        ("DISCUSSIONS", DISCUSSIONS.to_owned()),
        ("DISCUSSION", DISCUSSION.to_owned()),
        ("REFS", REFS.to_owned()),
        (
            "last_commits",
            last_commits(&["src/a \"b\".rs".into(), "README.md".into()]),
        ),
        ("refs_named", refs_named(3)),
        (
            "search_counts",
            search_counts(&[
                crate::browse::SearchType::Issue,
                crate::browse::SearchType::Repository,
                crate::browse::SearchType::User,
                crate::browse::SearchType::Discussion,
            ]),
        ),
        ("milestones(open)", milestones(false)),
        ("milestones(closed)", milestones(true)),
        ("milestone", milestone()),
        ("teams", teams()),
        ("team", team()),
    ];
    for field in ["stargazers", "watchers"] {
        all.push((
            "users(repository)",
            users("repository(owner: $owner, name: $name)", field),
        ));
    }
    for field in ["followers", "following"] {
        all.push(("users(user)", users("user(login: $login)", field)));
    }
    all.push((
        "users(organization)",
        users("organization(login: $login)", "membersWithRole"),
    ));
    all
}
