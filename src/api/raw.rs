//! The GraphQL queries not typed with cynic: those built at run time, and
//! those read as JSON. They're all here so a test can check each against
//! GitHub's schema, and that none cuts a list short without saying so.

/// A team's fields, as [`crate::api::browse::wire_teams::Team`] reads them.
const TEAM: &str =
    "slug name description privacy members { totalCount } repositories { totalCount }";

/// A milestone's fields, as [`crate::api::browse::wire_milestones::Milestone`] reads them.
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

/// What a discussion's comments and replies show, and what you may do
/// to them.
const DISCUSSION_REPLY: &str = "id databaseId author { login } body createdAt viewerCanUpdate viewerCanDelete viewerCanReact reactionGroups { content viewerHasReacted reactors { totalCount } } isMinimized minimizedReason viewerCanMinimize viewerCanUnminimize";

/// A discussion, its answer and its comments, with what you may do.
pub(crate) fn discussion() -> String {
    let comment = format!(
        "{DISCUSSION_REPLY} isAnswer upvoteCount viewerCanMarkAsAnswer viewerCanUnmarkAsAnswer"
    );
    format!(
        "query($owner: String!, $name: String!, $number: Int!) {{ repository(owner: $owner, name: $name) {{ discussion(number: $number) {{ id number title body author {{ login }} createdAt category {{ name isAnswerable }} isAnswered locked activeLockReason closed stateReason viewerCanClose viewerCanReopen viewerCanLabel upvoteCount viewerCanUpdate viewerCanReact reactionGroups {{ content viewerHasReacted reactors {{ totalCount }} }} answerChosenBy {{ login }} answer {{ {comment} }} comments(first: 50) {{ totalCount nodes {{ {comment} replies(first: 30) {{ totalCount nodes {{ {DISCUSSION_REPLY} }} }} }} }} }} }} }}"
    )
}

/// A comment on a discussion, or a reply in a comment's thread.
pub(crate) const ADD_DISCUSSION_COMMENT: &str = "mutation($discussion: ID!, $replyTo: ID, $body: String!) { addDiscussionComment(input: {discussionId: $discussion, replyToId: $replyTo, body: $body}) { comment { id } } }";

/// New text for a discussion comment or reply.
pub(crate) const UPDATE_DISCUSSION_COMMENT: &str = "mutation($id: ID!, $body: String!) { updateDiscussionComment(input: {commentId: $id, body: $body}) { comment { id } } }";

/// New text for a discussion's post.
pub(crate) const UPDATE_DISCUSSION: &str = "mutation($id: ID!, $body: String!) { updateDiscussion(input: {discussionId: $id, body: $body}) { discussion { id } } }";

pub(crate) const DELETE_DISCUSSION_COMMENT: &str =
    "mutation($id: ID!) { deleteDiscussionComment(input: {id: $id}) { comment { id } } }";

pub(crate) const MARK_ANSWER: &str =
    "mutation($id: ID!) { markDiscussionCommentAsAnswer(input: {id: $id}) { discussion { id } } }";

pub(crate) const UNMARK_ANSWER: &str = "mutation($id: ID!) { unmarkDiscussionCommentAsAnswer(input: {id: $id}) { discussion { id } } }";

/// What says a discussion changed: its title, text and answer, and how
/// many comments it has, with the newest and its newest reply.
pub(crate) const DISCUSSION_PROBE: &str = "query($owner: String!, $name: String!, $number: Int!) { repository(owner: $owner, name: $name) { discussion(number: $number) { title body updatedAt isAnswered comments(last: 1) { totalCount nodes { databaseId updatedAt replies(last: 1) { totalCount nodes { databaseId updatedAt } } } } } } }";

/// What a new issue or pull request can have: labels, people to assign,
/// open milestones, the issue templates' files, the pull request template,
/// and what you may do.
pub(crate) const NEW_FORM: &str = "query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { id viewerCanCreateIssues hasIssuesEnabled isArchived viewerPermission defaultBranchRef { name } labels(first: 100, orderBy: {field: NAME, direction: ASC}) { totalCount nodes { id name color description } } assignableUsers(first: 100) { totalCount nodes { id login name } } milestones(first: 100, states: OPEN, orderBy: {field: DUE_DATE, direction: ASC}) { totalCount nodes { id title dueOn } } issueTemplates: object(expression: \"HEAD:.github/ISSUE_TEMPLATE\") { ... on Tree { entries { name object { ... on Blob { text } } } } } pullRequestTemplates { filename body } } }";

/// Branches by name, 100 at a time; with the first page, the newest 100 tags.
pub(crate) const REFS: &str = "query($owner: String!, $name: String!, $after: String, $tags: Boolean!) { repository(owner: $owner, name: $name) { heads: refs(refPrefix: \"refs/heads/\", first: 100, after: $after, orderBy: {field: ALPHABETICAL, direction: ASC}) { totalCount pageInfo { hasNextPage endCursor } nodes { name } } tags: refs(refPrefix: \"refs/tags/\", first: 100, orderBy: {field: TAG_COMMIT_DATE, direction: DESC}) @include(if: $tags) { totalCount pageInfo { hasNextPage endCursor } nodes { name } } } }";

/// Dependabot's pull request for a vulnerability alert, or why it
/// couldn't make one.
pub(crate) const DEPENDABOT_UPDATE: &str = "query($owner: String!, $name: String!, $number: Int!) { repository(owner: $owner, name: $name) { vulnerabilityAlert(number: $number) { dependabotUpdate { pullRequest { number title state } error { title body } } } } }";

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

/// An owner's projects, most recently updated first, 30 at a time.
pub(crate) const OWNER_PROJECTS: &str = "query($login: String!, $after: String) { repositoryOwner(login: $login) { ... on ProjectV2Owner { projectsV2(first: 30, after: $after, orderBy: {field: UPDATED_AT, direction: DESC}) { totalCount pageInfo { hasNextPage endCursor } nodes { number title shortDescription url closed public updatedAt items { totalCount } } } } } }";

/// The projects linked to a repository, most recently updated first, 30
/// at a time.
pub(crate) const REPO_PROJECTS: &str = "query($owner: String!, $name: String!, $after: String) { repository(owner: $owner, name: $name) { projectsV2(first: 30, after: $after, orderBy: {field: UPDATED_AT, direction: DESC}) { totalCount pageInfo { hasNextPage endCursor } nodes { number title shortDescription url closed public updatedAt items { totalCount } } } } }";

/// A field's ID, of the fields a view or a value names.
const FIELD_ID: &str = "... on ProjectV2FieldCommon { id }";

/// A project, its fields and its views.
/// A project's fields, with a single select's options and an
/// iteration field's iterations.
const FIELDS: &str = "fields(first: 50) { totalCount nodes { ... on ProjectV2FieldCommon { id name dataType } ... on ProjectV2SingleSelectField { options { id name color description } } ... on ProjectV2IterationField { configuration { iterations { id title startDate duration } completedIterations { id title startDate duration } } } } }";

/// An item's values of its project's fields.
fn values() -> String {
    format!(
        "fieldValues(first: 50) {{ totalCount nodes {{ __typename ... on ProjectV2ItemFieldTextValue {{ text field {{ {FIELD_ID} }} }} ... on ProjectV2ItemFieldNumberValue {{ number field {{ {FIELD_ID} }} }} ... on ProjectV2ItemFieldDateValue {{ date field {{ {FIELD_ID} }} }} ... on ProjectV2ItemFieldSingleSelectValue {{ optionId name field {{ {FIELD_ID} }} }} ... on ProjectV2ItemFieldIterationValue {{ iterationId title field {{ {FIELD_ID} }} }} }} }}"
    )
}

/// A project, its fields and its views.
pub(crate) fn project() -> String {
    format!(
        "query($login: String!, $number: Int!) {{ repositoryOwner(login: $login) {{ ... on ProjectV2Owner {{ projectV2(number: $number) {{ id title shortDescription url closed public viewerCanUpdate {FIELDS} views(first: 20) {{ totalCount nodes {{ number name layout filter fields(first: 50) {{ totalCount nodes {{ {FIELD_ID} }} }} verticalGroupByFields(first: 1) {{ nodes {{ {FIELD_ID} }} }} groupByFields(first: 1) {{ nodes {{ {FIELD_ID} }} }} sortByFields(first: 5) {{ totalCount nodes {{ direction field {{ {FIELD_ID} }} }} }} }} }} }} }} }} }}"
    )
}

/// The projects an issue or pull request is in: each one's fields, and
/// the item's values of them.
pub(crate) fn projects_of() -> String {
    let items = format!(
        "projectItems(first: 10, includeArchived: false) {{ totalCount nodes {{ id project {{ id title url viewerCanUpdate {FIELDS} }} {} }} }}",
        values()
    );
    format!(
        "query($owner: String!, $name: String!, $number: Int!) {{ repository(owner: $owner, name: $name) {{ issueOrPullRequest(number: $number) {{ ... on Issue {{ {items} }} ... on PullRequest {{ {items} }} }} }} }}"
    )
}

/// What an issue or pull request in a project shows.
const LINKED: &str = "number title url state repository { nameWithOwner } assignees(first: 5) { totalCount nodes { login } } labels(first: 5) { totalCount nodes { name color } } milestone { title }";

/// A project's items that a view's filter finds, 50 at a time, with
/// their values.
pub(crate) fn project_items() -> String {
    format!(
        "query($login: String!, $number: Int!, $after: String, $query: String, $archived: [ProjectV2ItemArchivedState!]) {{ repositoryOwner(login: $login) {{ ... on ProjectV2Owner {{ projectV2(number: $number) {{ items(first: 50, after: $after, query: $query, archivedStates: $archived) {{ totalCount pageInfo {{ hasNextPage endCursor }} nodes {{ id fullDatabaseId isArchived content {{ __typename ... on DraftIssue {{ id title body assignees(first: 5) {{ totalCount nodes {{ login }} }} }} ... on Issue {{ {LINKED} stateReason }} ... on PullRequest {{ {LINKED} isDraft }} }} {} }} }} }} }} }} }}",
        values()
    )
}

/// Sets an item's field.
pub(crate) const SET_PROJECT_FIELD: &str = "mutation($project: ID!, $item: ID!, $field: ID!, $value: ProjectV2FieldValue!) { updateProjectV2ItemFieldValue(input: {projectId: $project, itemId: $item, fieldId: $field, value: $value}) { projectV2Item { id } } }";

/// Clears an item's field.
pub(crate) const CLEAR_PROJECT_FIELD: &str = "mutation($project: ID!, $item: ID!, $field: ID!) { clearProjectV2ItemFieldValue(input: {projectId: $project, itemId: $item, fieldId: $field}) { projectV2Item { id } } }";

/// An issue's or pull request's node ID, to add it to a project.
pub(crate) const ISSUE_OR_PR_ID: &str = "query($owner: String!, $name: String!, $number: Int!) { repository(owner: $owner, name: $name) { issueOrPullRequest(number: $number) { ... on Issue { id } ... on PullRequest { id } } } }";

/// Adds an issue or pull request to a project.
pub(crate) const ADD_PROJECT_ITEM: &str = "mutation($project: ID!, $content: ID!) { addProjectV2ItemById(input: {projectId: $project, contentId: $content}) { item { id } } }";

/// Adds a draft issue to a project.
pub(crate) const ADD_PROJECT_DRAFT: &str = "mutation($project: ID!, $title: String!, $body: String) { addProjectV2DraftIssue(input: {projectId: $project, title: $title, body: $body}) { projectItem { id } } }";

pub(crate) const ARCHIVE_PROJECT_ITEM: &str = "mutation($project: ID!, $item: ID!) { archiveProjectV2Item(input: {projectId: $project, itemId: $item}) { item { id } } }";

pub(crate) const UNARCHIVE_PROJECT_ITEM: &str = "mutation($project: ID!, $item: ID!) { unarchiveProjectV2Item(input: {projectId: $project, itemId: $item}) { item { id } } }";

/// A draft issue's new title and body.
pub(crate) const UPDATE_PROJECT_DRAFT: &str = "mutation($draft: ID!, $title: String!, $body: String!) { updateProjectV2DraftIssue(input: {draftIssueId: $draft, title: $title, body: $body}) { draftIssue { id } } }";

// ---- managing an issue, a pull request or a discussion ([`crate::api::manage`]) ----

pub(crate) const CONVERT_TO_DRAFT: &str = "mutation($id: ID!) { convertPullRequestToDraft(input: {pullRequestId: $id}) { pullRequest { id } } }";

pub(crate) const CHANGE_BASE: &str = "mutation($id: ID!, $base: String!) { updatePullRequest(input: {pullRequestId: $id, baseRefName: $base}) { pullRequest { id } } }";

pub(crate) const LOCK: &str = "mutation($id: ID!, $reason: LockReason) { lockLockable(input: {lockableId: $id, lockReason: $reason}) { lockedRecord { locked } } }";

pub(crate) const UNLOCK: &str =
    "mutation($id: ID!) { unlockLockable(input: {lockableId: $id}) { unlockedRecord { locked } } }";

pub(crate) const PIN_ISSUE: &str =
    "mutation($id: ID!) { pinIssue(input: {issueId: $id}) { issue { id } } }";

pub(crate) const UNPIN_ISSUE: &str =
    "mutation($id: ID!) { unpinIssue(input: {issueId: $id}) { issue { id } } }";

pub(crate) const HIDE_COMMENT: &str = "mutation($id: ID!, $reason: ReportedContentClassifiers!) { minimizeComment(input: {subjectId: $id, classifier: $reason}) { minimizedComment { isMinimized } } }";

pub(crate) const UNHIDE_COMMENT: &str = "mutation($id: ID!) { unminimizeComment(input: {subjectId: $id}) { unminimizedComment { isMinimized } } }";

pub(crate) const CLOSE_AS_DUPLICATE: &str = "mutation($id: ID!, $of: ID!) { closeIssue(input: {issueId: $id, stateReason: DUPLICATE, duplicateIssueId: $of}) { issue { id } } }";

pub(crate) const ADD_SUB_ISSUE: &str = "mutation($id: ID!, $child: ID!) { addSubIssue(input: {issueId: $id, subIssueId: $child}) { issue { id } } }";

pub(crate) const REMOVE_SUB_ISSUE: &str = "mutation($id: ID!, $child: ID!) { removeSubIssue(input: {issueId: $id, subIssueId: $child}) { issue { id } } }";

pub(crate) const MOVE_SUB_ISSUE: &str = "mutation($id: ID!, $child: ID!, $before: ID, $after: ID) { reprioritizeSubIssue(input: {issueId: $id, subIssueId: $child, beforeId: $before, afterId: $after}) { issue { id } } }";

pub(crate) const ADD_BLOCKED_BY: &str = "mutation($id: ID!, $by: ID!) { addBlockedBy(input: {issueId: $id, blockingIssueId: $by}) { issue { id } } }";

pub(crate) const REMOVE_BLOCKED_BY: &str = "mutation($id: ID!, $by: ID!) { removeBlockedBy(input: {issueId: $id, blockingIssueId: $by}) { issue { id } } }";

/// The commit a repository's default branch is at.
pub(crate) const DEFAULT_BRANCH_OID: &str = "query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { defaultBranchRef { target { oid } } } }";

pub(crate) const LINK_BRANCH: &str = "mutation($id: ID!, $oid: GitObjectID!, $name: String!) { createLinkedBranch(input: {issueId: $id, oid: $oid, name: $name}) { linkedBranch { id } } }";

pub(crate) const CLOSE_DISCUSSION: &str = "mutation($id: ID!, $reason: DiscussionCloseReason) { closeDiscussion(input: {discussionId: $id, reason: $reason}) { discussion { id } } }";

pub(crate) const REOPEN_DISCUSSION: &str =
    "mutation($id: ID!) { reopenDiscussion(input: {discussionId: $id}) { discussion { id } } }";

pub(crate) const REVERT_PR: &str = "mutation($id: ID!) { revertPullRequest(input: {pullRequestId: $id}) { revertPullRequest { number } } }";

pub(crate) const TRANSFER_ISSUE: &str = "mutation($id: ID!, $repo: ID!) { transferIssue(input: {issueId: $id, repositoryId: $repo}) { issue { number repository { nameWithOwner } } } }";

pub(crate) const START_DISCUSSION: &str = "mutation($repo: ID!, $category: ID!, $title: String!, $body: String!) { createDiscussion(input: {repositoryId: $repo, categoryId: $category, title: $title, body: $body}) { discussion { number } } }";

/// An owner's own repositories, most recently pushed first, with what you
/// may do in each: where an issue can move.
pub(crate) const TRANSFER_TARGETS: &str = "query($login: String!) { repositoryOwner(login: $login) { repositories(first: 100, ownerAffiliations: [OWNER], orderBy: {field: PUSHED_AT, direction: DESC}) { totalCount nodes { id nameWithOwner hasIssuesEnabled isArchived viewerPermission } } } }";

/// A repository's open issues, most recently updated first.
pub(crate) const OPEN_ISSUES: &str = "query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { issues(first: 50, states: OPEN, orderBy: {field: UPDATED_AT, direction: DESC}) { totalCount nodes { number title } } } }";

/// A repository's discussion categories, and their forms' files.
pub(crate) const DISCUSSION_FORMS: &str = "query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { id discussionCategories(first: 50) { totalCount nodes { id name slug description isAnswerable } } templates: object(expression: \"HEAD:.github/DISCUSSION_TEMPLATE\") { ... on Tree { entries { name object { ... on Blob { text } } } } } } }";

// ---- Insights, Explore, starred lists, saved replies -------------------------

/// A repository's dependency graph: 20 manifests, the first 100
/// dependencies of each (under the hawkgirl preview).
pub(crate) const DEPENDENCY_GRAPH: &str = "query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { dependencyGraphManifests(first: 20) { totalCount nodes { filename blobPath parseable exceedsMaxSize dependenciesCount dependencies(first: 100) { totalCount nodes { packageName packageManager requirements repository { nameWithOwner } } } } } } }";

/// A topic's stars and the topics GitHub relates to it.
pub(crate) const TOPIC: &str = "query($name: String!) { topic(name: $name) { name stargazerCount relatedTopics(first: 10) { name } } }";

/// What a starred list's repository row shows.
const LIST_REPO: &str = "nameWithOwner description stargazerCount forkCount primaryLanguage { name color } pushedAt isPrivate isFork isArchived";

/// Someone's starred lists (yours include the private ones).
pub(crate) const STAR_LISTS: &str = "query($login: String!) { user(login: $login) { lists(first: 100) { totalCount nodes { id name slug description isPrivate lastAddedAt items { totalCount } } } } }";

/// A starred list's repositories, 30 at a time, newest added first.
pub(crate) fn star_list_items() -> String {
    format!(
        "query($id: ID!, $after: String) {{ node(id: $id) {{ ... on UserList {{ items(first: 30, after: $after) {{ totalCount pageInfo {{ hasNextPage endCursor }} nodes {{ ... on Repository {{ {LIST_REPO} }} }} }} }} }} }}"
    )
}

/// Your lists, with what's in them, and a repository's ID: what adding
/// it to a list or taking it out starts from.
pub(crate) const MY_LISTS: &str = "query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { id viewerHasStarred } viewer { login lists(first: 100) { totalCount nodes { id name slug isPrivate items(first: 100) { totalCount pageInfo { hasNextPage endCursor } nodes { ... on Repository { id } } } } } } }";

/// More of one of your lists' repositories, after the first 100.
pub(crate) const LIST_MEMBERS: &str = "query($id: ID!, $after: String) { node(id: $id) { ... on UserList { items(first: 100, after: $after) { totalCount pageInfo { hasNextPage endCursor } nodes { ... on Repository { id } } } } } }";

/// Makes a starred list.
pub(crate) const CREATE_LIST: &str =
    "mutation($name: String!) { createUserList(input: {name: $name}) { list { id } } }";

/// Sets the lists a repository is in.
pub(crate) const SET_LISTS: &str = "mutation($item: ID!, $lists: [ID!]!) { updateUserListsForItem(input: {itemId: $item, listIds: $lists}) { lists { id } } }";

/// Deletes one of your starred lists.
pub(crate) const DELETE_LIST: &str =
    "mutation($list: ID!) { deleteUserList(input: {listId: $list}) { clientMutationId } }";

/// Your saved replies, most recently changed first.
pub(crate) const SAVED_REPLIES: &str =
    "query { viewer { savedReplies(first: 100) { totalCount nodes { id title body } } } }";

// ---- a repository's settings ([`crate::api::settings`]) ----

/// A repository's branch protection rules, with what each asks.
pub(crate) const PROTECTION_RULES: &str = "query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { id branchProtectionRules(first: 100) { totalCount nodes { id pattern requiresApprovingReviews requiredApprovingReviewCount dismissesStaleReviews requiresCodeOwnerReviews requireLastPushApproval requiresStatusChecks requiresStrictStatusChecks requiredStatusChecks { context app { id } } requiresConversationResolution requiresCommitSignatures requiresLinearHistory lockBranch isAdminEnforced allowsForcePushes allowsDeletions matchingRefs(first: 10) { totalCount nodes { name } } } } } }";

pub(crate) const CREATE_PROTECTION_RULE: &str = "mutation($input: CreateBranchProtectionRuleInput!) { createBranchProtectionRule(input: $input) { branchProtectionRule { id } } }";

pub(crate) const UPDATE_PROTECTION_RULE: &str = "mutation($input: UpdateBranchProtectionRuleInput!) { updateBranchProtectionRule(input: $input) { branchProtectionRule { id } } }";

pub(crate) const DELETE_PROTECTION_RULE: &str = "mutation($id: ID!) { deleteBranchProtectionRule(input: {branchProtectionRuleId: $id}) { clientMutationId } }";

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
        ("discussion", discussion()),
        ("ADD_DISCUSSION_COMMENT", ADD_DISCUSSION_COMMENT.to_owned()),
        (
            "UPDATE_DISCUSSION_COMMENT",
            UPDATE_DISCUSSION_COMMENT.to_owned(),
        ),
        ("UPDATE_DISCUSSION", UPDATE_DISCUSSION.to_owned()),
        (
            "DELETE_DISCUSSION_COMMENT",
            DELETE_DISCUSSION_COMMENT.to_owned(),
        ),
        ("MARK_ANSWER", MARK_ANSWER.to_owned()),
        ("UNMARK_ANSWER", UNMARK_ANSWER.to_owned()),
        ("DISCUSSION_PROBE", DISCUSSION_PROBE.to_owned()),
        ("REFS", REFS.to_owned()),
        ("NEW_FORM", NEW_FORM.to_owned()),
        (
            "last_commits",
            last_commits(&["src/a \"b\".rs".into(), "README.md".into()]),
        ),
        ("refs_named", refs_named(3)),
        ("milestones(open)", milestones(false)),
        ("milestones(closed)", milestones(true)),
        ("milestone", milestone()),
        ("teams", teams()),
        ("team", team()),
        ("OWNER_PROJECTS", OWNER_PROJECTS.to_owned()),
        ("REPO_PROJECTS", REPO_PROJECTS.to_owned()),
        ("project", project()),
        ("projects_of", projects_of()),
        ("project_items", project_items()),
        ("SET_PROJECT_FIELD", SET_PROJECT_FIELD.to_owned()),
        ("CLEAR_PROJECT_FIELD", CLEAR_PROJECT_FIELD.to_owned()),
        ("DEPENDABOT_UPDATE", DEPENDABOT_UPDATE.to_owned()),
        ("ISSUE_OR_PR_ID", ISSUE_OR_PR_ID.to_owned()),
        ("ADD_PROJECT_ITEM", ADD_PROJECT_ITEM.to_owned()),
        ("ADD_PROJECT_DRAFT", ADD_PROJECT_DRAFT.to_owned()),
        ("ARCHIVE_PROJECT_ITEM", ARCHIVE_PROJECT_ITEM.to_owned()),
        ("UNARCHIVE_PROJECT_ITEM", UNARCHIVE_PROJECT_ITEM.to_owned()),
        ("UPDATE_PROJECT_DRAFT", UPDATE_PROJECT_DRAFT.to_owned()),
        ("DEPENDENCY_GRAPH", DEPENDENCY_GRAPH.to_owned()),
        ("TOPIC", TOPIC.to_owned()),
        ("STAR_LISTS", STAR_LISTS.to_owned()),
        ("star_list_items", star_list_items()),
        ("MY_LISTS", MY_LISTS.to_owned()),
        ("LIST_MEMBERS", LIST_MEMBERS.to_owned()),
        ("CREATE_LIST", CREATE_LIST.to_owned()),
        ("SET_LISTS", SET_LISTS.to_owned()),
        ("DELETE_LIST", DELETE_LIST.to_owned()),
        ("SAVED_REPLIES", SAVED_REPLIES.to_owned()),
    ];
    // Managing an issue, a pull request or a discussion.
    all.extend([
        ("CONVERT_TO_DRAFT", CONVERT_TO_DRAFT.to_owned()),
        ("CHANGE_BASE", CHANGE_BASE.to_owned()),
        ("LOCK", LOCK.to_owned()),
        ("UNLOCK", UNLOCK.to_owned()),
        ("PIN_ISSUE", PIN_ISSUE.to_owned()),
        ("UNPIN_ISSUE", UNPIN_ISSUE.to_owned()),
        ("HIDE_COMMENT", HIDE_COMMENT.to_owned()),
        ("UNHIDE_COMMENT", UNHIDE_COMMENT.to_owned()),
        ("CLOSE_AS_DUPLICATE", CLOSE_AS_DUPLICATE.to_owned()),
        ("ADD_SUB_ISSUE", ADD_SUB_ISSUE.to_owned()),
        ("REMOVE_SUB_ISSUE", REMOVE_SUB_ISSUE.to_owned()),
        ("MOVE_SUB_ISSUE", MOVE_SUB_ISSUE.to_owned()),
        ("ADD_BLOCKED_BY", ADD_BLOCKED_BY.to_owned()),
        ("REMOVE_BLOCKED_BY", REMOVE_BLOCKED_BY.to_owned()),
        ("DEFAULT_BRANCH_OID", DEFAULT_BRANCH_OID.to_owned()),
        ("LINK_BRANCH", LINK_BRANCH.to_owned()),
        ("CLOSE_DISCUSSION", CLOSE_DISCUSSION.to_owned()),
        ("REOPEN_DISCUSSION", REOPEN_DISCUSSION.to_owned()),
        ("REVERT_PR", REVERT_PR.to_owned()),
        ("TRANSFER_ISSUE", TRANSFER_ISSUE.to_owned()),
        ("START_DISCUSSION", START_DISCUSSION.to_owned()),
        ("TRANSFER_TARGETS", TRANSFER_TARGETS.to_owned()),
        ("OPEN_ISSUES", OPEN_ISSUES.to_owned()),
        ("DISCUSSION_FORMS", DISCUSSION_FORMS.to_owned()),
    ]);
    // A repository's settings.
    all.extend([
        ("PROTECTION_RULES", PROTECTION_RULES.to_owned()),
        ("CREATE_PROTECTION_RULE", CREATE_PROTECTION_RULE.to_owned()),
        ("UPDATE_PROTECTION_RULE", UPDATE_PROTECTION_RULE.to_owned()),
        ("DELETE_PROTECTION_RULE", DELETE_PROTECTION_RULE.to_owned()),
    ]);
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
