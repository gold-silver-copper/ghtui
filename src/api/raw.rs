//! The GraphQL queries not typed with cynic: those built at run time, and
//! those read as JSON. They're all here so a test can check each against
//! GitHub's schema, and that none cuts a list short without saying so.

/// Queries given as text, one row each, and `RAW`: every one by name, so
/// the schema checks can't miss one.
macro_rules! raw {
    ($($(#[$doc:meta])* $name:ident = $query:literal;)*) => {
        $($(#[$doc])* pub(crate) const $name: &str = $query;)*

        #[cfg(test)]
        pub(crate) const RAW: &[(&str, &str)] = &[$((stringify!($name), $name)),*];
    };
}

pub(crate) use raw;

/// A team's fields, as [`crate::api::browse::wire_teams::Team`] reads them.
const TEAM: &str =
    "slug name description privacy members { totalCount } repositories { totalCount }";

/// A milestone's fields, as [`crate::api::browse::wire_milestones::Milestone`] reads them.
const MILESTONE: &str = "number title description dueOn closed closedAt updatedAt openIssues: issues(states: OPEN) { totalCount } doneIssues: issues(states: CLOSED) { totalCount } openPrs: pullRequests(states: OPEN) { totalCount } donePrs: pullRequests(states: [CLOSED, MERGED]) { totalCount }";

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

/// A field's ID, of the fields a view or a value names.
const FIELD_ID: &str = "... on ProjectV2FieldCommon { id }";

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

/// What a starred list's repository row shows.
const LIST_REPO: &str = "nameWithOwner description stargazerCount forkCount primaryLanguage { name color } pushedAt isPrivate isFork isArchived";

/// A starred list's repositories, 30 at a time, newest added first.
pub(crate) fn star_list_items() -> String {
    format!(
        "query($id: ID!, $after: String) {{ node(id: $id) {{ ... on UserList {{ items(first: 30, after: $after) {{ totalCount pageInfo {{ hasNextPage endCursor }} nodes {{ ... on Repository {{ {LIST_REPO} }} }} }} }} }} }}"
    )
}

raw! {
    /// A search for discussions, 30 at a time.
    DISCUSSION_SEARCH = "query($q: String!, $after: String) { search(type: DISCUSSION, query: $q, first: 30, after: $after) { discussionCount pageInfo { hasNextPage endCursor } nodes { ... on Discussion { repository { nameWithOwner } number title author { login } category { name } comments { totalCount } isAnswered upvoteCount updatedAt } } } }";

    /// Branches by name, 30 at a time, each with its pull requests.
    BRANCHES = "query($owner: String!, $name: String!, $after: String) { repository(owner: $owner, name: $name) { defaultBranchRef { name } refs(refPrefix: \"refs/heads/\", first: 30, after: $after, orderBy: {field: ALPHABETICAL, direction: ASC}) { totalCount pageInfo { hasNextPage endCursor } nodes { name target { ... on Commit { oid messageHeadline committedDate author { name user { login } } } } associatedPullRequests(first: 10, orderBy: {field: CREATED_AT, direction: DESC}) { nodes { number state isDraft headRefName repository { nameWithOwner } } } } } } }";

    /// Deployments, newest first, and the environments.
    DEPLOYMENTS = "query($owner: String!, $name: String!, $after: String, $envs: [String!]) { repository(owner: $owner, name: $name) { environments(first: 50) { totalCount nodes { name } } deployments(first: 25, after: $after, environments: $envs, orderBy: {field: CREATED_AT, direction: DESC}) { totalCount pageInfo { hasNextPage endCursor } nodes { environment state createdAt creator { login } ref { name } commitOid latestStatus { logUrl environmentUrl } } } } }";

    /// A file's blame.
    BLAME = "query($owner: String!, $name: String!, $rev: String!, $path: String!) { repository(owner: $owner, name: $name) { object(expression: $rev) { ... on Commit { blame(path: $path) { ranges { startingLine endingLine age commit { oid messageHeadline committedDate author { name user { login } } } } } } } } }";

    /// Someone's public gists, most recently updated first.
    GISTS = "query($login: String!, $after: String) { user(login: $login) { gists(first: 30, after: $after, privacy: PUBLIC, orderBy: {field: UPDATED_AT, direction: DESC}) { totalCount pageInfo { hasNextPage endCursor } nodes { name description updatedAt stargazerCount files(limit: 5) { name } comments { totalCount } } } } }";

    /// Discussions' URLs and repositories, to find where an organization's are.
    DISCUSSION_URLS = "query($q: String!, $after: String) { search(type: DISCUSSION, query: $q, first: 100, after: $after) { discussionCount pageInfo { hasNextPage endCursor } nodes { ... on Discussion { url repository { nameWithOwner } } } } }";

    /// A repository's discussion categories.
    DISCUSSION_CATEGORIES = "query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { discussionCategories(first: 50) { totalCount nodes { id name slug } } } }";

    /// A repository's discussions, most recently updated first.
    DISCUSSIONS = "query($owner: String!, $name: String!, $after: String, $category: ID) { repository(owner: $owner, name: $name) { discussions(first: 25, after: $after, categoryId: $category, orderBy: {field: UPDATED_AT, direction: DESC}) { totalCount pageInfo { hasNextPage endCursor } nodes { number title author { login } category { name } comments { totalCount } isAnswered upvoteCount updatedAt } } } }";

    /// A comment on a discussion, or a reply in a comment's thread.
    ADD_DISCUSSION_COMMENT = "mutation($discussion: ID!, $replyTo: ID, $body: String!) { addDiscussionComment(input: {discussionId: $discussion, replyToId: $replyTo, body: $body}) { comment { id } } }";

    /// New text for a discussion comment or reply.
    UPDATE_DISCUSSION_COMMENT = "mutation($id: ID!, $body: String!) { updateDiscussionComment(input: {commentId: $id, body: $body}) { comment { id } } }";

    /// New text for a discussion's post.
    UPDATE_DISCUSSION = "mutation($id: ID!, $body: String!) { updateDiscussion(input: {discussionId: $id, body: $body}) { discussion { id } } }";

    DELETE_DISCUSSION_COMMENT = "mutation($id: ID!) { deleteDiscussionComment(input: {id: $id}) { comment { id } } }";

    MARK_ANSWER = "mutation($id: ID!) { markDiscussionCommentAsAnswer(input: {id: $id}) { discussion { id } } }";

    UNMARK_ANSWER = "mutation($id: ID!) { unmarkDiscussionCommentAsAnswer(input: {id: $id}) { discussion { id } } }";

    /// What says a discussion changed: its title, text and answer, and how
    /// many comments it has, with the newest and its newest reply.
    DISCUSSION_PROBE = "query($owner: String!, $name: String!, $number: Int!) { repository(owner: $owner, name: $name) { discussion(number: $number) { title body updatedAt isAnswered comments(last: 1) { totalCount nodes { databaseId updatedAt replies(last: 1) { totalCount nodes { databaseId updatedAt } } } } } } }";

    /// What a new issue or pull request can have: labels, people to assign,
    /// open milestones, the issue templates' files, the pull request template,
    /// and what you may do.
    NEW_FORM = "query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { id viewerCanCreateIssues hasIssuesEnabled isArchived viewerPermission defaultBranchRef { name } labels(first: 100, orderBy: {field: NAME, direction: ASC}) { totalCount nodes { id name color description } } assignableUsers(first: 100) { totalCount nodes { id login name } } milestones(first: 100, states: OPEN, orderBy: {field: DUE_DATE, direction: ASC}) { totalCount nodes { id title dueOn } } issueTemplates: object(expression: \"HEAD:.github/ISSUE_TEMPLATE\") { ... on Tree { entries { name object { ... on Blob { text } } } } } pullRequestTemplates { filename body } } }";

    /// Branches by name, 100 at a time; with the first page, the newest 100 tags.
    REFS = "query($owner: String!, $name: String!, $after: String, $tags: Boolean!) { repository(owner: $owner, name: $name) { heads: refs(refPrefix: \"refs/heads/\", first: 100, after: $after, orderBy: {field: ALPHABETICAL, direction: ASC}) { totalCount pageInfo { hasNextPage endCursor } nodes { name } } tags: refs(refPrefix: \"refs/tags/\", first: 100, orderBy: {field: TAG_COMMIT_DATE, direction: DESC}) @include(if: $tags) { totalCount pageInfo { hasNextPage endCursor } nodes { name } } } }";

    /// Dependabot's pull request for a vulnerability alert, or why it
    /// couldn't make one.
    DEPENDABOT_UPDATE = "query($owner: String!, $name: String!, $number: Int!) { repository(owner: $owner, name: $name) { vulnerabilityAlert(number: $number) { dependabotUpdate { pullRequest { number title state } error { title body } } } } }";

    /// An owner's projects, most recently updated first, 30 at a time.
    OWNER_PROJECTS = "query($login: String!, $after: String) { repositoryOwner(login: $login) { ... on ProjectV2Owner { projectsV2(first: 30, after: $after, orderBy: {field: UPDATED_AT, direction: DESC}) { totalCount pageInfo { hasNextPage endCursor } nodes { number title shortDescription url closed public updatedAt items { totalCount } } } } } }";

    /// The projects linked to a repository, most recently updated first, 30
    /// at a time.
    REPO_PROJECTS = "query($owner: String!, $name: String!, $after: String) { repository(owner: $owner, name: $name) { projectsV2(first: 30, after: $after, orderBy: {field: UPDATED_AT, direction: DESC}) { totalCount pageInfo { hasNextPage endCursor } nodes { number title shortDescription url closed public updatedAt items { totalCount } } } } }";

    /// Sets an item's field.
    SET_PROJECT_FIELD = "mutation($project: ID!, $item: ID!, $field: ID!, $value: ProjectV2FieldValue!) { updateProjectV2ItemFieldValue(input: {projectId: $project, itemId: $item, fieldId: $field, value: $value}) { projectV2Item { id } } }";

    /// Clears an item's field.
    CLEAR_PROJECT_FIELD = "mutation($project: ID!, $item: ID!, $field: ID!) { clearProjectV2ItemFieldValue(input: {projectId: $project, itemId: $item, fieldId: $field}) { projectV2Item { id } } }";

    /// An issue's or pull request's node ID, to add it to a project.
    ISSUE_OR_PR_ID = "query($owner: String!, $name: String!, $number: Int!) { repository(owner: $owner, name: $name) { issueOrPullRequest(number: $number) { ... on Issue { id } ... on PullRequest { id } } } }";

    /// Adds an issue or pull request to a project.
    ADD_PROJECT_ITEM = "mutation($project: ID!, $content: ID!) { addProjectV2ItemById(input: {projectId: $project, contentId: $content}) { item { id } } }";

    /// Adds a draft issue to a project.
    ADD_PROJECT_DRAFT = "mutation($project: ID!, $title: String!, $body: String) { addProjectV2DraftIssue(input: {projectId: $project, title: $title, body: $body}) { projectItem { id } } }";

    ARCHIVE_PROJECT_ITEM = "mutation($project: ID!, $item: ID!) { archiveProjectV2Item(input: {projectId: $project, itemId: $item}) { item { id } } }";

    UNARCHIVE_PROJECT_ITEM = "mutation($project: ID!, $item: ID!) { unarchiveProjectV2Item(input: {projectId: $project, itemId: $item}) { item { id } } }";

    /// A draft issue's new title and body.
    UPDATE_PROJECT_DRAFT = "mutation($draft: ID!, $title: String!, $body: String!) { updateProjectV2DraftIssue(input: {draftIssueId: $draft, title: $title, body: $body}) { draftIssue { id } } }";

    // ---- managing an issue, a pull request or a discussion ([`crate::api::manage`]) ----

    CONVERT_TO_DRAFT = "mutation($id: ID!) { convertPullRequestToDraft(input: {pullRequestId: $id}) { pullRequest { id } } }";

    CHANGE_BASE = "mutation($id: ID!, $base: String!) { updatePullRequest(input: {pullRequestId: $id, baseRefName: $base}) { pullRequest { id } } }";

    LOCK = "mutation($id: ID!, $reason: LockReason) { lockLockable(input: {lockableId: $id, lockReason: $reason}) { lockedRecord { locked } } }";

    UNLOCK = "mutation($id: ID!) { unlockLockable(input: {lockableId: $id}) { unlockedRecord { locked } } }";

    PIN_ISSUE = "mutation($id: ID!) { pinIssue(input: {issueId: $id}) { issue { id } } }";

    UNPIN_ISSUE = "mutation($id: ID!) { unpinIssue(input: {issueId: $id}) { issue { id } } }";

    HIDE_COMMENT = "mutation($id: ID!, $reason: ReportedContentClassifiers!) { minimizeComment(input: {subjectId: $id, classifier: $reason}) { minimizedComment { isMinimized } } }";

    UNHIDE_COMMENT = "mutation($id: ID!) { unminimizeComment(input: {subjectId: $id}) { unminimizedComment { isMinimized } } }";

    CLOSE_AS_DUPLICATE = "mutation($id: ID!, $of: ID!) { closeIssue(input: {issueId: $id, stateReason: DUPLICATE, duplicateIssueId: $of}) { issue { id } } }";

    ADD_SUB_ISSUE = "mutation($id: ID!, $child: ID!) { addSubIssue(input: {issueId: $id, subIssueId: $child}) { issue { id } } }";

    REMOVE_SUB_ISSUE = "mutation($id: ID!, $child: ID!) { removeSubIssue(input: {issueId: $id, subIssueId: $child}) { issue { id } } }";

    MOVE_SUB_ISSUE = "mutation($id: ID!, $child: ID!, $before: ID, $after: ID) { reprioritizeSubIssue(input: {issueId: $id, subIssueId: $child, beforeId: $before, afterId: $after}) { issue { id } } }";

    ADD_BLOCKED_BY = "mutation($id: ID!, $by: ID!) { addBlockedBy(input: {issueId: $id, blockingIssueId: $by}) { issue { id } } }";

    REMOVE_BLOCKED_BY = "mutation($id: ID!, $by: ID!) { removeBlockedBy(input: {issueId: $id, blockingIssueId: $by}) { issue { id } } }";

    /// The commit a repository's default branch is at.
    DEFAULT_BRANCH_OID = "query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { defaultBranchRef { target { oid } } } }";

    LINK_BRANCH = "mutation($id: ID!, $oid: GitObjectID!, $name: String!) { createLinkedBranch(input: {issueId: $id, oid: $oid, name: $name}) { linkedBranch { id } } }";

    CLOSE_DISCUSSION = "mutation($id: ID!, $reason: DiscussionCloseReason) { closeDiscussion(input: {discussionId: $id, reason: $reason}) { discussion { id } } }";

    REOPEN_DISCUSSION = "mutation($id: ID!) { reopenDiscussion(input: {discussionId: $id}) { discussion { id } } }";

    REVERT_PR = "mutation($id: ID!) { revertPullRequest(input: {pullRequestId: $id}) { revertPullRequest { number } } }";

    TRANSFER_ISSUE = "mutation($id: ID!, $repo: ID!) { transferIssue(input: {issueId: $id, repositoryId: $repo}) { issue { number repository { nameWithOwner } } } }";

    START_DISCUSSION = "mutation($repo: ID!, $category: ID!, $title: String!, $body: String!) { createDiscussion(input: {repositoryId: $repo, categoryId: $category, title: $title, body: $body}) { discussion { number } } }";

    /// An owner's own repositories, most recently pushed first, with what you
    /// may do in each: where an issue can move.
    TRANSFER_TARGETS = "query($login: String!) { repositoryOwner(login: $login) { repositories(first: 100, ownerAffiliations: [OWNER], orderBy: {field: PUSHED_AT, direction: DESC}) { totalCount nodes { id nameWithOwner hasIssuesEnabled isArchived viewerPermission } } } }";

    /// A repository's open issues, most recently updated first.
    OPEN_ISSUES = "query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { issues(first: 50, states: OPEN, orderBy: {field: UPDATED_AT, direction: DESC}) { totalCount nodes { number title } } } }";

    /// A repository's discussion categories, and their forms' files.
    DISCUSSION_FORMS = "query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { id discussionCategories(first: 50) { totalCount nodes { id name slug description isAnswerable } } templates: object(expression: \"HEAD:.github/DISCUSSION_TEMPLATE\") { ... on Tree { entries { name object { ... on Blob { text } } } } } } }";

    // ---- Insights, Explore, starred lists, saved replies -------------------------

    /// A repository's dependency graph: 20 manifests, the first 100
    /// dependencies of each (under the hawkgirl preview).
    DEPENDENCY_GRAPH = "query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { dependencyGraphManifests(first: 20) { totalCount nodes { filename blobPath parseable exceedsMaxSize dependenciesCount dependencies(first: 100) { totalCount nodes { packageName packageManager requirements repository { nameWithOwner } } } } } } }";

    /// Where a repository's security policy is, if it has one: what the
    /// community profile leaves out.
    SECURITY_POLICY = "query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { securityPolicyUrl } }";

    /// A topic's stars and the topics GitHub relates to it.
    TOPIC = "query($name: String!) { topic(name: $name) { name stargazerCount relatedTopics(first: 10) { name } } }";

    /// Someone's starred lists (yours include the private ones).
    STAR_LISTS = "query($login: String!) { user(login: $login) { lists(first: 100) { totalCount nodes { id name slug description isPrivate lastAddedAt items { totalCount } } } } }";

    /// Your lists, with what's in them, and a repository's ID: what adding
    /// it to a list or taking it out starts from.
    MY_LISTS = "query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { id viewerHasStarred } viewer { login lists(first: 100) { totalCount nodes { id name slug isPrivate items(first: 100) { totalCount pageInfo { hasNextPage endCursor } nodes { ... on Repository { id } } } } } } }";

    /// More of one of your lists' repositories, after the first 100.
    LIST_MEMBERS = "query($id: ID!, $after: String) { node(id: $id) { ... on UserList { items(first: 100, after: $after) { totalCount pageInfo { hasNextPage endCursor } nodes { ... on Repository { id } } } } } }";

    /// Makes a starred list.
    CREATE_LIST = "mutation($name: String!) { createUserList(input: {name: $name}) { list { id } } }";

    /// Sets the lists a repository is in.
    SET_LISTS = "mutation($item: ID!, $lists: [ID!]!) { updateUserListsForItem(input: {itemId: $item, listIds: $lists}) { lists { id } } }";

    /// Deletes one of your starred lists.
    DELETE_LIST = "mutation($list: ID!) { deleteUserList(input: {listId: $list}) { clientMutationId } }";

    /// Your saved replies, most recently changed first.
    SAVED_REPLIES = "query { viewer { savedReplies(first: 100) { totalCount nodes { id title body } } } }";

    // ---- a repository's settings ([`crate::api::settings`]) ----

    /// A repository's branch protection rules, with what each asks.
    PROTECTION_RULES = "query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { id branchProtectionRules(first: 100) { totalCount nodes { id pattern requiresApprovingReviews requiredApprovingReviewCount dismissesStaleReviews requiresCodeOwnerReviews requireLastPushApproval requiresStatusChecks requiresStrictStatusChecks requiredStatusChecks { context app { id } } requiresConversationResolution requiresCommitSignatures requiresLinearHistory lockBranch isAdminEnforced allowsForcePushes allowsDeletions matchingRefs(first: 10) { totalCount nodes { name } } } } } }";

    CREATE_PROTECTION_RULE = "mutation($input: CreateBranchProtectionRuleInput!) { createBranchProtectionRule(input: $input) { branchProtectionRule { id } } }";

    UPDATE_PROTECTION_RULE = "mutation($input: UpdateBranchProtectionRuleInput!) { updateBranchProtectionRule(input: $input) { branchProtectionRule { id } } }";

    DELETE_PROTECTION_RULE = "mutation($id: ID!) { deleteBranchProtectionRule(input: {branchProtectionRuleId: $id}) { clientMutationId } }";
}

/// Every query here and in [`crate::api::housekeeping`], by name; those
/// built at run time with sample input.
#[cfg(test)]
pub(crate) fn all() -> Vec<(&'static str, String)> {
    let built = [
        ("discussion", discussion()),
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
        ("project", project()),
        ("projects_of", projects_of()),
        ("project_items", project_items()),
        ("star_list_items", star_list_items()),
        (
            "users(repository)",
            users("repository(owner: $owner, name: $name)", "stargazers"),
        ),
        (
            "users(repository)",
            users("repository(owner: $owner, name: $name)", "watchers"),
        ),
        ("users(user)", users("user(login: $login)", "followers")),
        ("users(user)", users("user(login: $login)", "following")),
        (
            "users(organization)",
            users("organization(login: $login)", "membersWithRole"),
        ),
    ];
    RAW.iter()
        .chain(crate::api::housekeeping::RAW)
        .map(|&(name, query)| (name, query.to_owned()))
        .chain(built)
        .collect()
}
