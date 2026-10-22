# ghtui

A keyboard-driven terminal client for GitHub, written in Rust.

The aim is simple: never open github.com for day-to-day work. You browse
repositories, issues, pull requests, Actions and profiles the way you would
on the website, and you review pull requests in a diff viewer that does more
than GitHub's. It looks like GitHub, in flat Material 3 colors adapted to
the terminal.

## Where it stands

ghtui is pre-release, but it's usable every day.

- **Browsing** covers most of GitHub: your home page and notifications,
  repositories (files, README, branches, tags, releases, commits, blame,
  comparisons), issues, pull requests, Actions runs and job logs,
  discussions, wikis, milestones, deployments, security advisories and
  alerts (Dependabot, code scanning, secret scanning), gists,
  teams, profiles, projects and search, a repository's Insights
  (contributors, commit activity, traffic, the dependency graph),
  Explore and topics, starred lists and packages, on github.com or a
  GitHub Enterprise Server. Every GitHub link opens in ghtui unless it can only work in a
  browser (a download, most of a repository's settings), and then it
  says why.
- **Reviewing** is where ghtui goes furthest. Diffs are computed locally
  with git, compared by syntax where the language allows, highlighted with
  tree-sitter, and shown unified or split. You
  can reply, resolve, comment on lines or ranges, suggest changes, and
  submit a review. Every review reads as on GitHub, with the threads it
  started, and opens the changes it reviewed and what changed since.
- **Acting** works from a page or from a row in a list: merge (now, when
  checks pass, or through a merge queue), approve, mark ready for review,
  update a branch, close or reopen, re-run or cancel a workflow run, run
  a workflow by hand with its inputs, save a run's artifacts, comment
  and star, mark notifications read or done, one at a time or
  all at once, answer and reply in discussions and mark their answer,
  add, move, archive and restore a project's cards, set their fields
  (on the board, or from an issue's or pull request's own page) and edit
  its drafts, dismiss or reopen a security alert, put a repository in
  your starred lists or take it out, and delete a package's version.
- **Managing** an issue, a pull request or a discussion, as GitHub's
  sidebar and menus do: delete a merged pull request's branch and
  restore it, convert one back to a draft, change its base, revert it;
  lock a conversation with GitHub's reasons, pin an issue, transfer it,
  close it as a duplicate, give it sub-issues (in order) and mark what
  blocks it, make a branch for it and check that out; hide a comment
  (hidden ones show folded, saying why); start a discussion from its
  category's form, and close one with GitHub's reasons.
- **Editing** what's there: your comments and descriptions (or anyone's,
  where GitHub lets you), titles, reactions, and an issue's or pull
  request's labels, assignees, reviewers and milestone.
- **Creating**: open issues (from the repository's templates) and pull
  requests, and check a pull request out in your clone; make, edit and
  delete releases, and upload, save and delete their assets; write gists
  and wiki pages; insert your saved replies in a comment; fork a
  repository or make a new one.
- **Housekeeping**: a repository's labels (a page of their own) and
  milestones, made, edited and deleted; branches and tags made at any
  commit and deleted; files edited, made, renamed and deleted, and
  committed, as the website's pencil does. In your clone, git does what
  git can (pushing a branch, a tag or a commit), after asking.
- **Notifications you choose**: subscribe to a thread or ignore it, and
  set how you watch a repository.
- **Settings**, for a repository's admins: branch protection rules
  (made, edited and deleted) and rulesets (turned on or off),
  collaborators and invitations with their roles, webhooks and their
  deliveries (redelivered), deploy keys, and archiving. Each change asks
  first and names what it changes; deleting a repository is left to
  github.com.
- **Git first**: files, folders, history, blame, comparisons, tags,
  code search, and a repository's contributors and commit activity come
  from your clone (or ghtui's own partial one), at
  GitHub's revision, and code navigation (`g`, `:repo symbols`, `:references`) from an
  index of the repository's definitions built there.
- **Live**: a pull request, an issue, a discussion, the Actions tab, a
  run and a running job update while they're on screen, and what came
  in since you looked is marked New.
- **Offline**: when GitHub can't be reached, the pages you've read
  still open, from the cache, marked offline with their age, and code,
  history and blame come from the clones. Comments, reviews, reactions,
  labels, closing and the like wait in a queue, shown on their pages,
  and go to GitHub in order (once) when it's back. Repositories you
  choose are fetched ahead and kept fresh, to read offline.

Not there yet: code navigation with a language server's precision, and
a pull request's Files changed offline.

## Goals

- **Replace the website for daily use.** If you'd reach for the browser,
  that's a gap worth closing.
- **A better diff than GitHub's.** Show what really changed, and only what
  you haven't seen.
- **Say what GitHub said.** A list cut short says what it left out. A
  failure says why. A change you made shows once GitHub shows it, not
  before.
- **One verb per key.** A key means the same thing on every screen. Where
  it doesn't apply, it says so instead of doing something else.

## Install

You need Rust 1.97 or newer and git 2.36 or newer.

```sh
cargo install ghtui
```

## Usage

```sh
ghtui                                   # home: your saved searches
ghtui ratatui/ratatui                   # a repository
ghtui ratatui/ratatui#1820              # an issue or pull request
ghtui @octocat                          # a profile
ghtui https://github.com/o/r/tree/main  # most github.com URLs
ghtui pr 1820                           # inside a clone: that PR of its GitHub remote
ghtui --theme light                     # override the color scheme
```

ghtui takes its token from `GH_TOKEN`, then `GITHUB_TOKEN`, then `gh auth
token`. If none works, it asks you to run `gh auth login`. On a GitHub
Enterprise Server (see [Configuration](#configuration)), it reads
`GH_ENTERPRISE_TOKEN`, then `GITHUB_ENTERPRISE_TOKEN`, then `gh auth token
--hostname HOST`, as gh does. The token stays in memory: it's never
logged, written to disk, or put on a command line. Reading needs read
access. Acting needs write access where you act (with a
classic token, the `repo` scope covers it all, but projects: reading them
takes `read:project` and changing them `project`, which `gh auth refresh
-s project` adds; gists take `gist`, which gh asks for; subscribing to a
thread takes `notifications`, which `gh auth refresh -s notifications`
adds; packages take `read:packages`, and deleting a version
`delete:packages`; saved replies take `read:user`; changing your
starred lists takes `user`; a repository's
traffic is shown only to people who can push to it). The Security tab's alerts take `security_events` (or
`repo`) with a classic token, and a fine-grained token's Dependabot, Code
scanning or Secret scanning alerts permission; GitHub shows them only to
people with write access (secrets: admins) and security managers. Where
something's missing, the page says what, and the `gh auth refresh`
command that adds a scope.
A wiki is pushed with the git credentials ghtui uses for its
clones.

## A short tour

The screen is laid out like GitHub's:
- **The header** shows where you are, a search field and your account.
- **A page's tabs** sit under the header: Code, Issues, Pull requests and
  so on.
- **The content** is in GitHub's boxes, with a sidebar on wide screens.
- **The status bar** names the keys that do something here.

`Space` lists everything you can do where you are. `:` opens a command
palette that also goes places (`owner/repo#123`, `@user`, a URL or its
path, like `owner/repo/pull/5/files`).

**Home** is your own saved searches: by default your review requests, your
open pull requests and your repositories. Save any list to Home from its
`Space` menu, and rename, reorder or remove sections in place. A review
request opens straight to its Files changed, and a pull request's row
says its review decision and how many approve it.

**A conversation** is GitHub's timeline, in GitHub's order: comments,
reviews and events. An event is one line (closed by whom and in which
commit or pull request, reopened, merged, mentioned from elsewhere and
whether that will close it, labels, assignees, milestone, projects,
reviewers asked, the title changed, ready for review or back to draft,
commits added, a force-push with a link to compare the two heads,
branches deleted or restored, locked, pinned, transferred, a duplicate),
and a run of one kind by one person reads as one line. A review shows its
state and its summary, then each thread it started: the lines it's on
(read from your clone, or the cache's, at the commit it was made on;
GitHub's diff hunk where there's no clone), its comments and replies, and
whether it's resolved or outdated. `↵` on a thread opens it in Files
changed. A review with more than three threads folds after the first
two: `↵` on its fold row shows the rest. Your own pending
review on GitHub shows as yours, not submitted, with its comments.

The sidebar has what GitHub's has: each reviewer's latest review (an
approval of an older head marked as before the last push, and the teams
it was for), who is asked and hasn't answered (code owners marked), what
a pull request will close, its merge commit, and the projects it's in
with its status there. An issue's has its type, its parent and
sub-issues with how many are done, and its Development: the pull
requests that will close it and the branches made for it. On a page too
narrow for a sidebar, the same is said above the conversation.

A commit's page lists its comments (those on a line open the diff
there, where they show as threads) and `c` writes one. A repository's
Issues tab starts with its pinned issues, and a discussion's answer is
shown at the top as well as where it is.

**Notifications** (`ctrl-n`, or the link at the top of Home) is your inbox,
as on GitHub: what each thread is about, why you got it and when, the
unread ones marked. `/` filters it as GitHub does (`is:unread`,
`reason:review-requested`, `repo:owner/name`), `t` shows unread, read or
all, and `↵` opens the thread at its latest comment. `m` marks a thread
read and `d` marks it done; `:mark all read` and `D` mark everything shown. A
thread marked done leaves the inbox until GitHub updates it again (a new
comment, say), and then it's back, as on github.com. A
filter GitHub can't apply (a reason, `is:read`) searches further pages
until a page fills. The header counts your unread notifications, kept
fresh as often as GitHub asks; click or follow it to open the inbox.

**Projects** are a tab of a profile and of a repository, as on GitHub.
A project opens at its first view, and its views are the page's tabs. A
board shows each column (each value of its Status, or the field it's
grouped by) as a box of cards, side by side as many as fit (the rest
below), or one above another on a narrow screen; `←` and `→` go between
columns. A board grouped by a field has swimlanes: each of the field's
values, with the board's columns of its own cards under it, and `←` and
`→` keep to the lane. A table shows the view's fields in columns, as many
as fit. A roadmap puts each item on a timeline from its start to its
target date (or across its iteration): a bar in its Status's color with
its dates, a diamond for a single day, today marked and the months
named; the page says how many days a column is. A view's filter applies,
and its sort and grouping (a board's columns too) cover all its items,
not only a page of them. A card opens what it is: an issue, a pull
request, or a draft's own page with its body and fields. Where GitHub
lets you change the project, `:checkout` moves the card to another column and
`:set field` sets one of its fields (a single select, an iteration, a text, a
number or a date); `:new issue` adds an issue or pull request (`owner/repo#123`
or its link) or a new draft, written in the comment editor; `e` edits a
draft's title and text; `:archive item` archives an item, asking first, and on one
under Archived (below the view), restores it. The board follows GitHub
until it shows the change.

An issue's or pull request's sidebar shows each project it's in with its
Status and its other fields' values, and `:set field` on its page sets one there,
with the board's pickers (first choosing the project, when it's in
several you may change), without opening the board.

**The Security tab** has GitHub's own menu: published advisories, then
Dependabot alerts (the vulnerable dependency, its manifest, the affected
and fixed versions, the advisory, and Dependabot's pull request or why it
couldn't make one), code scanning alerts (the rule, the tool's message, and
the file at the line the tool flagged, at the commit it read), and secret
scanning alerts (the kind of secret, whether it still works, and where it
is; never the secret itself). Each list filters with `/` as GitHub's does
(`is:open`, `severity:high`, `ecosystem:npm`, `package:`, `manifest:`,
`scope:runtime`; `tool:`, `branch:`; `secret-type:`, `validity:active`,
`resolution:revoked`) and `t` goes between open, closed and all. Where
GitHub lets you, `x` dismisses an open alert (for a secret, closes it):
pick GitHub's reason, after typing a comment if you like. On a closed one
it reopens it, after asking. A list whose alerts are off says so.

**A discussion** shows its answer at the top, as GitHub does, then its
comments with their replies and reactions. `c` writes an answer (or, in
a category without answers, a comment); on a comment or one of its
replies, it replies in that comment's thread, and `>` replies quoting
it. `:mark answer` marks the selected comment as the answer, or unmarks it, `+`
reacts, and `e` and `D` edit and delete, where GitHub lets you.

**Offline**, ghtui is the same ghtui. A page always shows what the cache
has first and GitHub refreshes it; when GitHub can't be reached (no
network, the server down), the header says `⚠ Offline`, once, and the
page stays as the cache had it, under a banner saying how old it is
("Offline: showing what was loaded 3h ago"). Nothing waits on GitHub
meanwhile: requests fail at once, and ghtui asks GitHub every few
seconds (then less often) whether it's back; `r` asks now. Files,
folders, history, blame and comparisons come from the clones, which
need no network: at the commit each branch or tag was at when GitHub
last listed them, which the page's first line says.

A comment, a reply, a review (with its comments), a reaction, labels,
assignees, a milestone, a title, an edit, closing or reopening, written
while GitHub can't be reached, is queued: the composer closes, and the
page it was written on shows it in a "Queued for GitHub" box, as does
the header (`⚠ Offline` and `2 queued`). When GitHub is back the queue is
sent in order, each write once, and the page follows GitHub until it
shows it. Merging and the like aren't queued: what they do depends on
GitHub's state when they're sent. A write GitHub refuses then (the issue
was locked or deleted meanwhile) stays, marked refused with GitHub's
reason and its text, and the header says `⚠ 1 not sent`: on its row,
`R` sends it again, `l` copies its text and `del` throws it away
(`ctrl-z` brings it back). The queue is kept on disk for each host and
account, so a restart keeps it. A write that may have reached GitHub
when the connection dropped (or ghtui stopped while sending it) is
never sent again on its own, unless sending it twice changes nothing (a
label, a reaction, closing): it's marked "Not known if sent", to check
and send again or throw away.

`:keep offline` on any of a repository's pages keeps it to read offline:
ghtui fetches ahead, into its cache, the repository's page, its Issues,
Pull requests and Discussions tabs, and its issues, pull requests and
discussions updated most recently (each as its page shows it), and
makes its clone (with the default branch's files). It's kept fresh every
half hour while GitHub can be reached, fetching again only what changed.
How much is plain, and set under `[offline]` in the config: by default
up to 100 of each kind, updated in the last 90 days, and nothing more
past 2 GiB of cache and clones. The Offline page (`:offline`, or the
header's link) lists the queue and the kept repositories: what each
keeps, its clone's size, when it was refreshed and what stopped it.

**Tabs** work like a browser's: `T` opens the selection in a new tab, `ctrl-pgdn`
and `ctrl-pgup` switch between them, and they come back next time you start.

**The diff viewer** (a pull request's Files changed) adds what GitHub's
doesn't:
- a comparison by syntax: only the code that changed stands out, not the
  lines that wrapping, reindenting or a new enclosing `if` touched;
- changed words within a line, and changed letters within a word;
- moved code, with a jump between its two ends;
- formatting-only changes folded away;
- scope names in hunk headers (`impl Doc › fn offset`);
- "changes since your last review", even across force-pushes and rebases;
- local per-change "reviewed" marks beside GitHub's per-file "viewed".

Comparing by syntax works as difftastic's does: both versions are parsed
and matched node by node, however they're laid out. It decides what's
emphasised, what folds away as formatting and which lines sit side by
side in split view; the lines, the +/− counts and where comments go stay
git's. `:by syntax` switches to comparing by lines and back, and the status bar
says which is on. A file without a grammar, or too big or slow to compare
that way, is compared by lines, and its header says why.

**A repository's code is read with git**: its files and folders, "Go to
file" (`f`), a branch's or a file's commits, a commit, blame, a
comparison, the branches and tags to switch to (`B`), the tags list, and
a code search in one repository (`repo:owner/name`, with `rev:` for a
branch, tag or commit, else the default branch). ghtui reads your clone
when it starts inside one, else a partial clone it makes in its cache, in
the background, the first time you open the repository. A branch or tag
is always GitHub's: ghtui asks GitHub where it is (with `git ls-remote`)
and fetches what the clone doesn't have. The page's first line says
where it was read: `At 1a2b3c4 · read by git from your clone`, or
GitHub's API and why (the clone is still being made, git failed, or a
limit was hit). Code search lists each file's matching lines, which
GitHub's API doesn't. It answers in milliseconds once the clone has what
it needs, and takes nothing from GitHub's rate limit.

**Reading code** goes beyond the file on screen, without a language
server. `:symbols` lists a file's functions, types, impls and modules, nested
as in the code, and goes to the one you pick (in Files changed, the
file under the cursor's). Across the repository, ghtui indexes the
definitions at the revision you're viewing, from a clone (yours, or
its cache's), in the background; in Files changed it's the pull
request's head, so definitions the PR adds are found. `g` goes to the
definition of the word under the cursor: in the file when it's there,
else from the index. `:repo symbols` lists the repository's symbols, nearest the
file first, and `:references` the lines that use the word (`git grep`, whole
words, at that revision). Symbols are read for Rust, TypeScript,
JavaScript, Python and Go. ghtui says when the index is still building
or stopped at a limit, and only then (or with no clone to read) asks
GitHub's code search instead.

Comments are kept on disk until you submit, so a crash never loses them.
Threads that GitHub calls outdated are moved to their line when it still
exists. A thread's comments are rendered Markdown, as in the
conversation: links follow (`i` letters them, `↵` on a link's line
follows it), and a suggested change shows as a small diff of the lines it
replaces.

**While you review**, ghtui keeps an eye on the pull request: while its
Files changed, its Review tab or its conversation is on screen, it asks
GitHub now and then for new review comments, a new push and check
results. It asks with ETags, so an answer that hasn't changed costs
nothing, and it asks less often while nothing changes or the terminal is
out of focus. What came in since you started looking (another's review,
comment, thread or reply) is marked New, and the status bar says what it
was ("alice approved"); after a push to a pull request you've reviewed,
it offers the changes since your last review.

**Other pages are watched the same way**, with the same pace:
- a running job: its steps as they start and end, then, once it ends,
  its log. The page follows the job the way `tail -f` follows a file,
  staying on the step running and ending at its log, until you move;
  then it says how the job ended. GitHub writes a job's log only when
  the job ends, so the lines themselves can't arrive sooner;
- the Actions tab and a workflow's runs: new runs appear (and are said),
  running ones update;
- a run: its outcome, its jobs' and its artifacts;
- an issue: new comments, edits, its state, labels and assignees;
- a discussion: new comments and replies, edits, its answer.

Another author's new comments are marked New; yours aren't. The first
answer after opening a page counts as a change when it shows something
the page doesn't, so what came in while the page loaded isn't lost.

**Why a check failed** is on the page that shows it:
- the Checks tab lists each check run's annotations (the failures,
  warnings and notices it attached to lines of files) under it, and a
  job's page lists all of its own; opening one goes to its lines, in the
  file at the commit that was checked;
- in Files changed, they sit on their lines, as on GitHub, named by
  their check;
- a run's page opens on its first failed job, and a job's page opens on
  its failing step's first error: a failure's log lines are one `↵`
  away;
- a run's artifacts are listed with their size and when they expire;
  `↵` on one asks where to save it (the current folder unless you type a
  file or folder), and a file already there is replaced only when you
  choose the row that says so.

**Running a workflow by hand** (`:run workflow` on a workflow's page or one of its
runs) opens GitHub's "Run workflow" form: the branch, then the inputs
its file declares on that branch (choices, booleans and text, with their
defaults). `↵` changes one; `:run workflow` (or the form's last row) runs it, and
the page follows the run it started.

**A pull request's Review tab** (`:review glance` from any of its tabs) shows where
its review stands, so you don't have to piece it together:
- your drafts that aren't submitted yet;
- the threads still unresolved, in every file;
- who has reviewed and what they said last, and who is asked and hasn't
  answered yet;
- every review, newest first;
- what still stops the merge.

Each row opens what it's about: a draft or a thread in Files changed at
its line, a review its own page. On a draft, `e` edits it (in Files
changed) and `D` or `del` deletes it, which `ctrl-z` brings back, as
they do on a draft in Files changed, where `c` comments on its line. A review's page has its summary and
every thread it started or replied in, then two ways into Files
changed: the changes it reviewed, as the pull request was at its commit
(from git), and what changed since, marked as "since your last review"
marks it. `e` edits a review's summary where GitHub lets you, and `D`
throws away your pending review.

**Acting on GitHub** asks first wherever it matters (merging, closing,
cancelling, updating a branch, deleting a comment). It asks GitHub what
you may do and says why when you can't. After a change, the page follows
GitHub until it shows it.

**Editing** works on what's selected: a comment (or, with none selected,
the description) opens in the comment editor in its own place, with its
text. Reactions show under each comment as GitHub shows them, yours
marked, and `+` picks one of GitHub's eight to give or take back. The
sidebar's fields are edited in pickers, as on GitHub: check and uncheck
labels, assignees or reviewers (people and teams), then apply them with
`ctrl-s`; a milestone is one choice. In Files changed, the same keys work
on the review comment under the cursor.

**A new issue** (`:new issue` from any of a repository's pages) starts from the
repository's templates, as on GitHub: choose one, and its title, text,
labels and assignees fill the form (a YAML form's fields become headings
to answer under). The comment editor opens in place of the text: the
first line is the title, the rest the description. Labels, assignees and
a milestone are rows of the form, each with its picker. `ctrl-s` (or
`ctrl-s` on the form) creates it, and the new issue opens.

**A new pull request** (`:new pull request`) proposes, inside a clone, the branch you're
on, into the default branch; Enter on either branch picks another. It's
filled in as GitHub fills it: one commit's message, or the branch's name
and a list of its commits, then the repository's template. A branch
GitHub doesn't have, or has behind yours, says so, and `:push branch` pushes it
with your own `git push` after asking. `ctrl-s` asks whether it's a draft,
then creates it.

**A new release** (`:new release` from a repository's pages) starts, as on GitHub,
with its tag: one GitHub has, or a new one, which GitHub makes when the
release is published, at the default branch or the branch or commit you
choose. ghtui asks GitHub about the tag first, and says when it has a
release already. The composer takes the title (the first line) and the
notes; "Generate release notes" has GitHub write them from the pull
requests since the last release. Pre-release and latest are rows to
toggle. `ctrl-s` asks whether to publish it or save a draft, then the
release's page (a draft's: the releases) opens and follows GitHub until
it shows it.

**A release's page** lists its assets with their sizes and how often
they were downloaded. `↵` on one asks where to save it (by its name in
the current folder unless you type a file or folder), and a file already
there is replaced only when you choose the row that says so. `:upload asset` uploads
a file from a path (a folder's path lists its files) and `D` on an asset
deletes it, after asking. `e` opens the release in the same form, to
change its title, notes, tag, pre-release or latest, and saving asks
whether to update it or make it a draft again (a draft's: publish it or
keep it a draft). `D` elsewhere on the page deletes the release, after
asking; then, asked apart, its tag too, or not. The page follows GitHub
until it shows each change.

**Gists**: `:new gist` writes a new one, file by file in the composer, with a
description; `ctrl-s` asks whether it's secret or public, as GitHub's two
buttons do. On your own gist, `e` opens it as the same form (rename,
add or remove files; only what changed is sent) and `D` deletes it
after asking.

**A wiki** is a git repository, and ghtui writes it with git: `e` on a
page opens it in the composer (or `$EDITOR`), `,W` writes a new one
(its title on the first line). Saving commits the page in the wiki's
clone in ghtui's cache, on the wiki's latest commit, and pushes it with
the git credentials ghtui uses. If the page changed on GitHub since you
opened it, nothing is overwritten: saving says so and keeps your text.

**Subscriptions**: `:watch` on an issue, a pull request or a discussion (or
its row in a list, your inbox included) subscribes to it, unsubscribes
or ignores it; anywhere else in a repository it sets how you watch the
repository (participating and @mentions, all activity, or ignore). The
current choice is marked.

**Insights** is a repository's last tab, as on GitHub, with GitHub's
menu: Contributors, Commit activity, Traffic and Dependency graph.
Contributors and commit activity are counted with git, in your clone or
the cache's (the page's first line says which, and at what commit): the
default branch's commits, merges left out, week by week since the
first, then each author's (by their git name, joined by the
repository's `.mailmap`; linked to their account when a GitHub no-reply
address names it), each with a sparkline of the same weeks. Commit
activity is the last 52 weeks, each day of the week, and the weeks
with commits, newest first. Until the clone is made, GitHub's own
statistics answer, and say so. Traffic is the last 14 days' views and
clones, day by day, the referring sites and the popular pages, for
those who can push to the repository. The dependency graph lists each
manifest GitHub read with what it depends on and the versions it
takes, each linked to its repository where GitHub knows it. Every
chart is bars or a sparkline beside the numbers they stand for, scaled
to the largest, and says what a bar counts.

**Explore** (`:explore`, or the Space menu's Go) is the nearest GitHub's
API comes to Trending: repositories made today, this week or this
month, most starred, in any language or the one picked, and GitHub's
featured topics. It says plainly that it isn't Trending, which ranks by
recent stars that the API doesn't give. A topic's page (`topics/rust`,
or a topic on a repository's page) has GitHub's description, its
stars, related topics, and its repositories, most starred first.

**Starred lists**: someone's Stars tab starts with their lists, and a
list's page has its repositories. `:lists` on a repository's pages lists
your lists, the ones it's in checked: `↵` puts it in a list or takes it
out, and a name no list has makes the list with it in. `D` on one of
your lists deletes it, after asking. Your lists are read again until
GitHub shows the change. Changing them takes a classic token's `user`
scope (`gh auth refresh -s user`), which ghtui names when it's
missing.

**Packages** are a profile's tab, and a link on a repository's page: an
owner's packages of every kind, or those a repository published. A
package's page says how to pull or install its newest version (or the
one picked from its versions), and `D` on a version deletes it, after
asking what goes with it; the page follows GitHub until it's gone.
Without your token's `read:packages` scope, the page says so and how to
add it.

**Saved replies**: in the comment editor, `ctrl-g` lists yours; typing
narrows them, and `↵` puts the one chosen where the cursor is.

**Settings** is a repository's last tab, shown, as on GitHub, only to
its admins. Its own menu goes between General, Collaborators, Branches,
Webhooks and Deploy keys. These change the repository for everyone, so
every change asks first and lists what it changes, line by line:
- **Branches** lists the branch protection rules (each with what it
  asks and how many branches it applies to) and the rulesets. `a`
  writes a new rule and `e` edits the selected one in the composer (or
  `$EDITOR`), as GitHub's form's settings, one a line: `Require linear
  history: yes`, `Required approvals: 2`, `Status checks: ci, lint`.
  `ctrl-s` asks, naming each setting that changes (`Required approvals:
  1 → 2`), and only those are sent, so what the form doesn't show stays
  as it is. If GitHub refuses, the form opens again with your text and
  GitHub's words. `D` deletes a rule; `x` turns a ruleset on or off
  (an organization's is the organization's to change).
- **Collaborators** lists who has access, with their role, and the
  invitations waiting. `a` invites someone: their login, then a role
  (Read, Triage, Write, Maintain or Admin, each saying what it allows);
  `e` changes a collaborator's or an invitation's role, and `D` removes
  them or cancels it. The owner's access can't change.
- **Webhooks** lists each with its events and what its last delivery
  got back; `↵` opens one with its recent deliveries. `a` adds one and
  `e` edits it as a form (`Payload URL`, `Content type`, `Secret`,
  `SSL verification`, `Events`, `Active`; its secret is never shown, and
  left empty keeps it), `x` turns it off or on, `D` deletes it, and on
  its page `R` redelivers the selected delivery.
- **Deploy keys**: `a` adds one (its title, the public key, and whether
  it may push, which the question warns about), `D` deletes one.
- **General** says how the repository is set and archives it (`x`),
  asking twice as GitHub does: for its name, typed, then once more.
  Unarchiving asks the same way. Deleting a repository is done on
  github.com: ghtui never deletes one.

**Checking out** a pull request (`:checkout` on it or its row), inside a clone
of its repository, fetches its head and switches to a branch for it: its
own branch's name (tracking it), or `owner/branch` for a fork's. It
refuses while you have changes that aren't committed, and says which
branch you're on.

**Managing** works as GitHub's sidebar and menus do, where GitHub lets
you (and says why where it doesn't):
- on a pull request, `:delete branch` deletes its branch once it's merged or closed,
  and restores it after; `:ready for review` converts it back to a draft (after asking);
  the menu changes its base (pick a branch, then confirm) and reverts a
  merged one: GitHub makes the revert's branch and pull request, which
  opens;
- `:lock` locks an issue's, a pull request's or a discussion's conversation,
  with one of GitHub's reasons or none, and unlocks it after asking;
- on an issue, the menu pins or unpins it and transfers it to another
  of its owner's repositories you can write to; `x` closes it as
  completed, not planned, or a duplicate of an issue you choose next
  (typed as `#12`, `owner/repo#12` or a link, or picked from its open
  issues);
- `:sub issues` lists an issue's sub-issues: `↵` on one removes it, `#12` (or a
  link) adds one, and `alt-↑` `alt-↓` move the one chosen; `:blocked by` does the
  same for the issues blocking it, which the sidebar lists under Blocked
  by (and Blocking);
- `:checkout` on an issue makes a branch for it on GitHub, named as GitHub
  names it (its number and title, which you can change), from the
  default branch and linked to it, then offers to check it out in your
  clone, as `:checkout` does a pull request;
- `:hide comment` hides the selected comment as spam, abuse, off-topic, outdated, a
  duplicate or resolved, and unhides a hidden one. A hidden comment shows
  folded, with GitHub's reason; `↵` shows it;
- in a repository's discussions, the menu starts a new one: pick its
  category, then write its title and text in the composer, which starts
  from the category's form where it has one. `x` closes a discussion as
  resolved, outdated or a duplicate, or reopens it.

**Labels and milestones** are reached from the Issues and Pull requests
tabs (`Labels · Milestones` above the list). The labels page lists each
label with its description and how many open issues and pull requests
have it; `↵` opens them. `:new here` makes a new label (its name, colour and
description are rows of a form, each set in a picker; GitHub's colours
are offered, or type `#rrggbb`), `,e` edits the selected one (renaming
it too) and `D` deletes it, asking first and saying how many it comes
off. On the milestones (and a milestone's page) `:new here` makes one, `e`
edits its title, due date and description (in the composer), `x`
closes or reopens it, and `D` deletes it after asking. Each page follows
GitHub until it shows the change.

**Branches and tags**: on the branches or tags list, `:new here` makes one: its
name, then where it starts (the branch selected, another branch, a tag,
or a commit's SHA typed). `D` deletes the selected one, asking first;
never the default branch, nor one GitHub's branch protection or a
ruleset keeps, and it says which. Inside a clone of the repository, git
does it, after asking: `git push` of the commit to the new branch or
tag, or `git push --delete`; your clone's own branches and tags aren't
touched. Elsewhere GitHub's API does it.

**Forks and new repositories**: `:fork` on any of a repository's pages
forks it, to your account or an organization that lets you make
repositories there, under the name you choose, with its default branch
only unless you say all. `:new repository` makes a new repository: its owner, name,
description, public or private, a README, and a `.gitignore` and licence
from GitHub's templates. Then its page opens and follows GitHub until
it's there (a fork can take GitHub a moment); for a new repository,
cloning it here (`git clone` into `./name`) is offered.

**Editing a file** (`e` on a file, as the website's pencil) reads it as
GitHub has it on the branch, and opens the composer on its text
(`ctrl-e` continues in `$EDITOR`). The form's rows name its path
(changing it renames the file), the branch the commit goes to (the one
you're on, or a new one, offered as a pull request after; a tag or a
commit is never a branch, so it's always a new one there) and the
commit's message (GitHub's own until you write one). `:new here` in a folder
makes a new file there, and `D` on a file deletes it, each the same
way. `ctrl-s` asks before committing. Inside a clone of the repository, git
makes the commit on GitHub's tip of the branch, from objects alone, and
pushes it, so your branches, index and working tree stay as they are;
elsewhere GitHub's API commits it (GitHub signs it). If the file changed
on GitHub since you opened it, nothing is overwritten: committing says
so, and keeps your text.

## Keys

The keys are the ones you already know: the arrows, Page Up and Down,
Home and End, `↵` to open, and `esc` or `⌫` to go back. On top of them, a
few single letters act on what's under the cursor, and the status bar
shows the ones that do something here. `Space` lists everything you can
do where you are, with its key if it has one, and `:` (the palette) finds
anything, goes anywhere (`owner/repo#123`, `@user`, a URL) and searches
GitHub; a number typed into it goes to that line of a file, a log or the
diff. Whatever has no key is in those two.

What a key does depends on where you are: the innermost of the contexts
below that binds it wins. A list adds its keys to a page's, which add
theirs to the ones that work everywhere; in Files changed, the file tree
has the focus or the diff does. A key that does nothing here says so, and
says where it works.

- `esc` cancels a search, a selection or a dialog, and with nothing to
  cancel goes back, as `⌫` and `alt-←` do; `alt-→` goes forward.
- A page's own tabs (a pull request's Conversation, Commits, Checks,
  Files changed and Review) aren't history: `⇥` and `⇧⇥` step through
  them and `1`–`9` go to one, as the tab bar numbers them, and each keeps
  where you were. In Files changed, `⇥` switches between the file tree
  and the diff.
- `/` searches what's on screen: the page, a file, the diff or a log, and
  `n` and `N` go through the matches. On a list it edits the list's
  filter instead, in place on the list. A URL or GitHub path typed or
  pasted into it or into the palette, even after a filter or a search's
  `repo:owner/name`, opens it.
- `i` letters every link on screen; typing a link's letters follows it.
- `c` comments (on a thread, it replies), `>` replies quoting the
  selected comment, `e` edits, `D` deletes, `+` reacts, `l` copies the
  link, `o` opens it on GitHub and `r` refreshes.
- Writes that change GitHub are capitals and ask first where it matters:
  `M` merges, `A` approves and `R` re-runs; `x` closes or reopens (on a
  running run, cancels it), and in Files changed resolves the thread
  under the cursor. `ctrl-s` creates or saves the form on screen, and in
  Files changed submits your review.
- `M` offers what GitHub would: merging now, merging when ready
  (auto-merge, offered first while checks run), or adding it to the
  merge queue where its base branch has one. Once it's set to merge or
  queued, the merge box says so (who turned auto-merge on and how it
  will merge; its place in the queue), and `M` takes that back.
- In files and logs, `←` and `→` scroll sideways and `ctrl-←` and
  `ctrl-→` move a word cursor along the line. `s` selects lines (then `l`
  copies `#L10-L20`), `b` blames the file at that line and `g` goes to
  the definition of the word under the cursor. In the blame, a run of
  lines opens its commit; a job's log lines are rows with their own
  links.
- In Files changed, `]` and `[` go to the next and previous change, `⇧↓`
  and `⇧↑` to the next and previous file and `U` to the next unviewed
  one. `v` marks a file viewed, `m` marks a change reviewed, and `⇧←`
  and `⇧→` pick the old or new half of a split row to comment on. `↵`
  opens a fold or a gap, folds a file at its header, and jumps between
  the ends of moved code. In the file tree, `←` and `→` fold and unfold
  folders, and the diff shows the file selected there.
- The diff's view: `w` wraps long lines, `W` ignores whitespace, `y`
  compares by syntax or by lines, `|` switches split and unified, `\`
  shows or hides the file tree, `z` folds or unfolds what's under the
  cursor and `Z` folds every file, `E` shows more context and `F` the
  whole file.
- On pages, `]` and `[` jump by section: a Home section, a box, a
  heading, a folder in the file tree.
- On lists, `t` shows open, closed or all and `S` changes the sort; in
  your notifications `m` marks one read and `d` done. On a project's
  board, `←` and `→` go between columns, `m` moves the selected card and
  `a` adds an item. In a repository's settings, `a` adds, `e` edits, `D`
  deletes and `x` turns a ruleset or webhook on or off.
- `h` goes home, `ctrl-n` to your notifications, `u` up a level, `f` to a
  file and `B` to another branch or tag. `T` opens the selection in a new
  tab, `ctrl-pgdn` and `ctrl-pgup` switch tabs, `alt-1`–`alt-9` go to
  one and `ctrl-w` closes it.
- `q` quits, as `:q` in the palette does; `ctrl-c` quits from anywhere,
  and in the comment editor asks before it throws away your text.

**Everywhere** (`global:`)

| Keys            | Action |
| --------------- | ------ |
| `↓`             | Move down |
| `↑`             | Move up |
| `pgdn`          | Page down |
| `pgup`          | Page up |
| `home`          | Go to the top |
| `end`           | Go to the bottom |
| `↵`             | Open the selection |
| `esc`           | Cancel a search or a selection; with nothing to cancel, go back |
| `⌫` `alt-←`     | Back |
| `alt-→`         | Forward |
| `u`             | Up a level |
| `⇥`             | Next tab of the page |
| `⇧⇥`            | Previous tab of the page |
| `1`–`9`         | Go to the page's tab 1 to 9 |
| `ctrl-pgdn`     | Next open tab |
| `ctrl-pgup`     | Previous open tab |
| `T`             | Open the selection in a new tab |
| `ctrl-w`        | Close the tab |
| `alt-1`–`alt-9` | Go to open tab 1 to 9 |
| `h`             | Go home |
| `ctrl-n`        | Your notifications |
| `/`             | Search what's on screen (on a list, filter it) |
| `n`             | Next search match |
| `N`             | Previous search match |
| `f`             | Go to file |
| `c`             | Comment (on a thread, reply) |
| `l`             | Copy the link (a queued write: its text) |
| `o`             | Open on GitHub |
| `r`             | Refresh |
| `space`         | Everything you can do here |
| `:` `ctrl-k`    | Command palette: go anywhere, search GitHub, run anything |
| `M`             | Merge the pull request |
| `A`             | Approve the pull request |
| `x`             | Close (as a duplicate too) or reopen (an alert: dismiss it); on a running run, cancel it |
| `R`             | Re-run the workflow run or job (a refused write: send it again) |
| `e`             | Edit the comment or description (a wiki page, your gist, a release, a label, a milestone, a file) |
| `D`             | Delete the comment (your gist, a release or its asset, a package's version, your list, a label, a milestone, a branch, a tag, a file) |
| `+`             | React to the comment, issue or pull request |
| `del`           | Delete the selection (a draft comment, a Home section, a write waiting for GitHub) |
| `ctrl-z`        | Bring back what was just deleted |
| `q`             | Quit |

**On pages** (`page:`)

| Keys     | Action |
| -------- | ------ |
| `]`      | Next section (a box, a Home section, a folder) |
| `[`      | Previous section |
| `>`      | Reply quoting the comment |
| `ctrl-s` | Create or save the form on screen (an issue, a release, a label, a file…) |
| `B`      | Switch branches or tags |
| `i`      | Follow a link by its letters |

**On lists** (`list:`)

| Keys | Action |
| ---- | ------ |
| `t`  | Open / closed / all (in the inbox, unread / read) |
| `S`  | Change the sort |
| `m`  | Mark the notification read |
| `d`  | Mark the notification done |

**In files** (`file:`)

| Keys     | Action |
| -------- | ------ |
| `b`      | Blame the file at this line, or back to the file |
| `←`      | Scroll left |
| `→`      | Scroll right |
| `ctrl-→` | Next word (to search or go to its definition) |
| `ctrl-←` | Previous word |
| `g`      | Go to the definition of the word under the cursor |
| `s`      | Select lines |

**In job logs** (`log:`)

| Keys     | Action |
| -------- | ------ |
| `←`      | Scroll left |
| `→`      | Scroll right |
| `ctrl-→` | Next word (to search or go to its definition) |
| `ctrl-←` | Previous word |

**In projects** (`project:`)

| Keys | Action |
| ---- | ------ |
| `←`  | Previous column of the board |
| `→`  | Next column of the board |
| `m`  | Move the item to another column |
| `a`  | Add an issue, a pull request or a draft |

**In a repository's settings** (`settings:`)

| Keys | Action |
| ---- | ------ |
| `a`  | Add a branch rule, a collaborator, a webhook, a deploy key, a secret or a variable; publish a Pages site |
| `e`  | Edit the rule, webhook, variable, Pages site or custom properties; set a secret; change the collaborator's role |
| `D`  | Delete the rule, webhook, deploy key, secret, variable or cache; remove the collaborator or invitation; unpublish the site |
| `x`  | Turn the ruleset or webhook on or off; archive or unarchive the repository |

**In Files changed** (`diff:`)

| Keys     | Action |
| -------- | ------ |
| `i`      | Follow a link by its letters |
| `←`      | Scroll left |
| `→`      | Scroll right |
| `ctrl-→` | Next word (to search or go to its definition) |
| `ctrl-←` | Previous word |
| `g`      | Go to the definition of the word under the cursor |
| `]`      | Next change |
| `[`      | Previous change |
| `⇧↓`     | Next file |
| `⇧↑`     | Previous file |
| `⇧←`     | Old side of a split row (for comments) |
| `⇧→`     | New side of a split row (for comments) |
| `U`      | Next unviewed file |
| `⇥`      | Switch between the file tree and the diff |
| `z`      | Fold or unfold the file, thread or review |
| `Z`      | Fold every file |
| `\`      | Show or hide the file tree |
| `\|`     | Split or unified view |
| `w`      | Wrap long lines |
| `W`      | Ignore whitespace changes |
| `y`      | Compare by syntax or by lines |
| `F`      | Show the whole file |
| `E`      | Show more context |
| `v`      | Mark the file viewed (syncs with GitHub) |
| `m`      | Mark the change reviewed |
| `s`      | Select lines |
| `x`      | Resolve or unresolve the thread |
| `ctrl-s` | Submit your review |

**In Files changed's file tree** (`tree:`)

| Keys | Action |
| ---- | ------ |
| `]`  | Next section (a box, a Home section, a folder) |
| `[`  | Previous section |
| `←`  | Fold the folder (on a file, go to its folder) |
| `→`  | Unfold the folder |

The comment editor opens where the comment will go: under the lines, the
thread or the file header in Files changed, and in place of the comment
box on a conversation (or of the comment it edits). In it, `ctrl-s` adds the comment, `ctrl-e`
continues in `$EDITOR`, `ctrl-g` lists your saved replies to insert
one, and `esc` cancels (keeping the lines selected).
Long lines wrap, and a paste arrives as text, newlines and all (in a
one-line box, as one line). Typing `@` offers the people taking part and
the repository's assignable users, `#` its recent issues and pull
requests, narrowed as you type: `⇥` or `↵` takes the one chosen, `↑` and
`↓` choose, and `esc` closes the list.

## Configuration

`~/.config/ghtui/config.toml`, or `--config <path>`. Everything is
optional, and an unknown key is an error, so typos don't go unnoticed.

```toml
[github]
host = "github.example.com"  # a GitHub Enterprise Server; default github.com

[theme]
mode = "auto"         # auto | light | dark
seed = "#3f6fb5"      # the whole color scheme is generated from this
color_depth = "auto"  # auto | truecolor | 256

[ui]
nerd_font = false
density = "auto"      # auto | compact | comfortable

[offline]             # what a repository kept offline keeps
items = 100           # of its issues, pull requests and discussions each
days = 90             # only those updated in this many days
disk = 2048           # MiB: nothing more is fetched past this (cache and clones)

[keys]                # each action listed replaces its default keys
down = ["j", "<Down>"]  # one key each: a character or a named key
sort = ["list:O"]     # a context first binds it there only
go_home = ["<A-h>"]   # modified keys too

[[home]]              # a Home section: a title and a GitHub search
title = "Bugs in our repos"
issues = "is:open label:bug org:my-org sort:updated-desc"
rows = 10
```

The host can also come from `GHTUI_HOST`, which wins over the config file,
or from gh's `GH_HOST`, which doesn't. On an Enterprise Server, ghtui asks
its API at `https://HOST/api/v3` and `https://HOST/api/graphql`, its links
and its clones' remotes open in ghtui, and links to github.com open in the
browser (and the other way round). ghtui asks the server its version once,
and leaves out of each query what that version doesn't have (from the
schemas GitHub publishes for 3.10 and later; what a server refuses
besides, it learns and asks again without), so its pages load with what
it has; the status bar says once what they go without ("GitHub
Enterprise Server 3.19 doesn't have project events in the timeline").
An API endpoint the server doesn't have (a repository's security
advisories) isn't asked, and its page says so. The cache, the tabs and the review
drafts are kept per host and per account, so switching accounts or hosts
never shows another's pages.

A section lists `pulls`, `issues` or `repos`, and shows up to 30 rows; its
last row opens the rest. Without any `[[home]]` tables, Home shows your
review requests, your open pull requests and your repositories. Action
names for `[keys]` are in the `actions!` table in
[`src/keymap.rs`](src/keymap.rs). A key bound twice in one context is an
error that names both, as is more than one key (`gg`): a binding is a
single key.

## Design

**An Elm-style app.** Keys and network replies become messages. A single
`update` changes the state and returns commands, and the view is a pure
function of the state. Network and git work runs in background tasks, so
the screen never waits on them.

**Wrong states are made hard to write.** Most of ghtui's bugs would be
GitHub's data read wrongly: a list cut short shown as whole, the oldest page
taken for the newest, a diff from the wrong base. So:
- a partial list is a type that knows what it left out;
- each GraphQL query is checked against GitHub's schema;
- an answer that doesn't add up is reported, not hidden;
- where two places must agree (what a page reserves and what fills it,
  what a change is about and what shows it), one of them owns the fact.

Non-test code can't panic: the lints reject `unwrap`, indexing and the like.

**Diffs come from git, not from GitHub.** ghtui uses your clone if you start
it inside one, and otherwise a bare partial clone in its cache. It diffs
from the merge base, as GitHub does, and can also rebuild the diff as it was
at your last review, or at any review's commit. Comments still anchor to
GitHub's own hunks, so they land where GitHub expects them. A review
thread's lines in the conversation come from the same clone, at the
commit the thread was started on; the conversation never clones for
them. Git never sees the token on a command line or in a config file.

**A conversation is GitHub's timeline.** Comments, reviews and events come
in one request (`timelineItems`), in GitHub's order, so what's shown and
what's left out ("120 earlier comments and events") are one list. Review
threads come in pages beside it, and each comment names the review it
was made in, which is how a review shows its threads.

**So does code.** Whatever git can answer, git answers: trees, files,
paths, histories, commits, blame, comparisons, branches and tags, code
search in a repository. The API keeps what only GitHub knows (issues,
pull requests, reviews, checks, notifications, search across GitHub, who
a commit's author is when their email doesn't say). One module,
`src/source.rs`, decides which answers each page, and says so on the
page. Your clone is only read: ghtui fetches commits into its object
store, as `git fetch` would, from its own GitHub remote, and never
touches its branches, index, working tree or config. Housekeeping writes
from it only what you ask for, after asking, with your own git: a
branch or tag pushed by its commit's ID, a deleted one pushed away, and
a file's commit made from objects alone (through an index of its own,
on GitHub's tip of the branch, fetched first) and pushed; so even then
your branches, index and working tree stay as they are. The cache's clone
is partial: a file's contents are fetched when a page reads it, all a
page needs in one batch. Limits keep a huge repository from turning a
page into a download, and the API answers past them, saying which:
- a clone over 512 MiB isn't made (and isn't tried again for a week);
- a file shows its first 1 MiB;
- "Go to file" lists 250,000 paths;
- line counts read at most 3,000 changed files, and blame 2,000 versions
  of a file;
- code search in ghtui's clone reads at most 25,000 files (in yours,
  any number);
- Insights counts the newest 200,000 commits of the default branch
  (their dates and authors only, which a blobless clone has);
- git gets 6 seconds for a page (20 for blame, comparisons, search and
  Insights, and 1 for the walk that finds each entry's latest commit, the rest of
  which GitHub finds), then the API answers while git carries on.

**Wikis are git repositories**, and ghtui reads and writes them as such:
a page is read from the wiki's clone in the cache, at the commit it was
fetched at, and saved as a commit on the wiki's latest, made without a
working tree and pushed as a fast-forward, so a page changed meanwhile
is never overwritten.

**Code navigation comes from git too.** The index of a repository's
definitions (`code_index`) is read from the same clone at a commit: a
branch is fetched first, so a stale local branch is never taken for
GitHub's. It's kept per commit in the cache (`code-index/`), and kept by
file contents, so when the revision moves only the changed files are
read. In the cache's partial clone, the files it reads are fetched in
batches of a thousand, never one by one, within limits: 20,000 files,
64 MiB and two minutes, nearest the file you asked from first. On
rust-lang/rust (39,000 Rust files), once cloned, that is about 5,000 files in 18
seconds, using about 130 MB.

**Where a page's data comes from.** Every piece of GitHub's data a page
shows has one key, a `DataKey`, and is one kind of `Data`; both are in
`src/data.rs`, where each line of the `data!` list is a kind of data and
how a page picks it out. One loader (`src/data/load.rs`) gets a key's
data: from git where git can answer (`src/source.rs` decides, and the
page says which), else from GitHub's API. It keeps what the API answered
in the cache under the same key (a list's first page), where
`DataKey::kept` allows, and reads it back by that key. In the state, each
key's data is a `Remote`: loading, failed and why, or there, with when it
was fetched. Pages read it through `State::get`, never on their own,
and so do pickers, the composer's `@` mentions, the header's unread
count and the search box's suggestions.

**Fast from the cache.** The first paint comes from a local cache (redb,
`src/store.rs`), then everything is refreshed in the background. What's
written to the cache is kept in memory, where reads see it, and committed
together in the background, so no reply waits on the disk. REST requests
revalidate with ETags (and `Last-Modified`), so unchanged data costs
nothing against the rate limit. Offline is that same path with GitHub
missing: one place (`api::reach`) knows whether GitHub can be reached (a
request that can't connect says it can't; any answer says it can), so a
request fails at once while it can't, the page keeps the cache's copy, and
the header says so. A failed connect never left, so only a write that
failed that way is queued (`offline::queue`): each is written down as
being sent before it goes, and its answer settles it, so none is sent
twice.

**A page says what it is in one place.** Pages come in families, one enum
each in `src/route/` (code, issues, pull requests, Actions' runs, releases,
people and so on; Insights and the like are `src/site.rs`'s, housekeeping's
forms `src/housekeeping.rs`'s). A family says, for each of its pages, its
URL and title, the page above it, what it needs fetched, what it loads
more of, how wide it is, its crumbs and tabs, how it's built, which form
it is if it's one, and how its URLs read (the `route::Family` trait).
`Route` is one variant per family, and only asks it. What a page needs is
the one list of what building it reads: a test builds every page the
fixtures reach and checks it reads nothing else. That is also what keeps
pages from being built for nothing: a page is built again only when the
data it needs changes, its form changes, or a time it shows relative to
now ("3m ago") reads differently. The pages are drawn by `src/ui/pages/`,
which, like `src/api/`, names no app module.

**A write is a job.** Everything ghtui writes, to GitHub, to git in
your clone or to Home's sections in your config, is a job (`src/work.rs`): a struct that says how it runs and
what its answer does (`impl Job`), listed once in `jobs!`, which makes the
`Work` a command carries and the `Done` its answer comes back as. A job
fails with one error type, `work::Failure`, which says GitHub's own words
when GitHub refused. A change to an issue, a pull request, a discussion, a
run, a notification, a project or a repository's settings is one job, a
`Change` (`act::Changing`), and what the offline queue keeps is made of
them. What an action does where you are is planned once by the feature
that owns it, as a step (`act::Step`: send it, ask first, choose in a
picker, write in the composer, start a job, or set a form's row), and
the key, the menu and the palette all ask that plan, so they can't
disagree; a picker's row is such a step too. Asking first is one
question (`forms::Ask`), about a form or about what an action changes,
whose answers carry what they're about.

**After a write, the page follows GitHub until it shows it**
(`src/follow.rs`). The feature that made the write says what the page
should show once GitHub has it, as an `Expectation`; one place polls the
page's data, says in the status bar that it's waiting, checks each answer
and gives up after a while. One write is followed at a time.

**Forms are kept in one map** (`src/forms.rs`), by their page, with what
was sent from them (`Sent`: sending, and GitHub's refusal): a new issue or
pull request, a release, a gist, a wiki page, housekeeping's, running a
workflow. A form is its fields (`forms::Fields`) and what it does
(`forms::Kind`: Enter on a row, writing its text, submitting it, filling
itself in from data as it arrives, its menu rows), which its page's
family names. Text is typed in one composer (`src/compose.rs`), for a
form's text as for a comment or a review's; what it's for says its title,
what ctrl-s does and where the text goes.

**What each feature adds is one table** (`src/features.rs`): the actions
it owns, the others it takes on its own pages, what it does with them,
the menu's rows it offers, and what it follows as data arrives or a page
opens. A key's action is done by the feature that takes it on the screen,
else by the one that owns it; tests check that no two features own one
action and that on no screen two take one, so the table's order is only
the menu's.

**Colors from one seed.** Light and dark schemes are generated with
Material 3's tonal palettes. Every foreground and background pair the UI
uses is tested for contrast in both schemes, in truecolor and 256 colors.

It's one crate. The top of `src/` is the app and the binary that runs it:
the state and `update` (`src/state/`), keys (`src/keymap.rs`), the view,
the runtime that runs commands (`src/runtime.rs`), and the pieces above
(`data`, `route`, `work`, `follow`, `forms`, `compose`, `features`), with
a module for each feature beside them. The rest are its modules:

| Module   | What it does                                                    |
| -------- | --------------------------------------------------------------- |
| `ui`     | Pages and widgets, pure functions of data                       |
| `api`    | GitHub over GraphQL (cynic) and REST: auth, retries, limits     |
| `schema` | GitHub's GraphQL schema, which the queries are typed against   |
| `store`  | The cache, and review drafts kept outside it                    |
| `git`    | The `git` CLI: clones, fetches, credentials, reads at a commit  |
| `diff`   | Hunks (imara-diff), syntax trees (tree-sitter), the diff model  |
| `theme`  | Color schemes, contrast, terminal detection                     |

## Development

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
scripts/check-tls-deps.sh
```

The tests come at GitHub's data from several sides:
- every query is checked against the schema (`src/api/query_check.rs`);
- recorded GitHub responses are replayed offline (`tests/corpus.rs`);
- the client is tested against a scripted GitHub (`tests/client.rs`):
  `github(vec![Reply::new(200, body).on(path)])` answers each request in
  turn, and the test checks what was asked and how the answer reads;
- git is the oracle for diffs, on real repositories over `file://`;
- behaviour is tested through `update` (`src/state/tests/`), with no
  network, runtime or terminal;
- every screen the fixtures reach is drawn in light and dark at a few
  sizes, tiny ones among them, to check nothing panics and every color
  pair drawn is declared (`src/state/tests/smoke.rs`);
- property tests throw random keys, resizes and replies at the app, and
  read every page's URL back as the page.

Nothing compares a whole screen: how a page looks may change when it
gets better. Where what a page says matters, a test looks for the text.

### Writing a test

A behaviour test starts from a builder in `src/state/tests/screens.rs`
(`with_repo()`, `with_pr()`, …, or `open(state, route, data)` with
`fixtures::sample(&key)`), presses keys (`press(&mut s, ",o")`), hands in
data (`fetched`) and a job's answer (`done`), and checks what was sent
(`sent::<Job>`, `asked`, `sent_change`) and what the page or status line
says (`says`, `warns`, the page's text). From
`src/state/tests/housekeeping.rs` (`with_labels` and `select` are that
file's own):

```rust
let mut s = with_labels();
select(&mut s, "good first issue");
press(&mut s, ",e");
let cmds = press(&mut s, ",s");
let work = fixtures::sent::<labels::Save>(&cmds);
assert_eq!(work.was.as_deref(), Some("good first issue"));
fixtures::done(&mut s, work, Ok(()));
says(&s, "Saved the label");
```

A write is tested once at each level: what it sends over HTTP in
`tests/client.rs`, the job the keys start (as above), its query against
the schema, and, for a new kind of write, one step of the live check
below. A new page needs a fixture its builder opens, listed in
`screens::pages()`, so the smoke test draws it and checks it reads only
what it needs.

### Adding a page

1. In its family's file in `src/route/`, a variant of the family's
   `Page`, and its arms in the family's `url`, `title`, `up`, `needs`
   (the page's own key last) and `build`, and where the defaults don't
   fit, `paged`, `width`, `chrome`. Its URLs are read in the family's
   `parse`, and listed in `tests/github_urls.txt`.
2. If it shows new data: a `DataKey`, its line in `data!`, whether it's
   kept (`DataKey::kept`), its arm in `data/load.rs`, and the request in
   `src/api/`.
3. Its drawing in `src/ui/pages/`, from the data alone.
4. A fixture for it (`fixtures::sample` for a new key), in
   `screens::pages()`.

A new family is one more file: its `Page` enum and `impl Family`, a
`Route` variant and its arm in `Route::family`, its parser's place in
`route.rs`'s `TOP` or `REPO` order, and its `arb()` for the property
tests.

### Adding a write or a form

1. A job: a struct, its `impl Job` (`run` sends it, `done` says what
   happened and what follows), and its line in `jobs!`. A change to an
   issue, a pull request and the like is a `Change` variant instead, sent
   with `act::send`.
2. If the page should show it: an `Expectation`, started from `done` with
   `follow::expect`.
3. The key: an `Action` (`src/keymap.rs`) in its feature's `owns` in
   `src/features.rs`, with an arm in the feature's `act` that plans a step
   or starts the job.
4. A form: its fields (`impl forms::Fields`), its `forms::Kind`, and its
   page in a family whose `form` names it; the page draws it with
   `ui::pages::write_form`.
5. Tests: the request in `tests/client.rs`, the job the keys start in
   `src/state/tests/`.

A contract suite checks ghtui's assumptions against live GitHub, read-only
(`cargo test --test contract -- --ignored contract`). Every
kind of github.com link is listed with what it must open in
`tests/github_urls.txt`.

### The live check

`cargo xtask live` runs ghtui for real, against real GitHub: it opens
each kind of page, and makes one write of each kind, checked against
GitHub's own answer (`gh api`), not only the screen. What a write sends
and what the app does with each key are tested above; this checks that
the real app does it against real GitHub. It's run by hand, not in CI,
and takes about five minutes. `cargo xtask live --list` names the
checks; `cargo xtask live issue offline` runs only those.

- **The checks:**
  - `pages` opens each kind of page on its own and checks it shows what
    GitHub says it has, finishes loading, and says "Couldn't load"
    nowhere: the repository, a folder, a file, its blame, main's
    commits, a commit, branches, tags, issues, issue #8, milestones,
    pull requests, PR #5's conversation and PR #11's files, Actions, a
    workflow and the latest run, releases, discussions and the first
    one, the wiki, security, code scanning and its alert #1, settings,
    projects, Insights, Explore, a topic, the owner's profile and
    gists, home and the inbox. It only reads.
  - `issue` creates an issue with `:new issue`, comments with `c`, sees a
    comment made on GitHub show without a refresh, and closes it with
    `x`.
  - `pull`, in a fresh clone, proposes a new branch (`:new pull request`), pushes it
    (`:push branch`), creates a draft pull request (`ctrl-s`) and marks it ready
    (`:ready for review`).
  - `release` makes a release with a new tag with `:new release`.
  - `labels` makes a label on the labels page with `:new here`, and deletes it
    with `D`.
  - `settings` writes a protection rule for `ghtui-live-` branches in
    its form (with `$EDITOR` set to a `sed` script) and saves it, then
    deletes it with `D`, and checks `main`'s rule ends as it began.
  - `actions` runs `actions.yml` by hand with `:run workflow`, with its `seconds`
    input set, and follows the run until it fails, as it does on
    purpose.
  - `offline` runs ghtui against a stand-in Enterprise Server (below)
    on port 8451, and stops it to make GitHub go away, without touching
    the machine's network: the page stays, marked offline; a comment
    written meanwhile is queued, still queued after ghtui restarts, and
    sent exactly once when the stand-in starts again.
- **What it needs:** `gh`, logged in as the owner of
  `gold-silver-copper/ghtui-test-writes`, with a classic token's `repo`
  and `workflow` scopes. Without `read:project` (or `project`),
  `pages` leaves the Projects tab out, and says so. It also needs git,
  openssl (for the stand-in's certificates), and a fux that speaks
  fuxth (`fux help-agents`): on `PATH`, `~/code/fux`'s release build,
  or `--fux PATH`. It builds `target/release/ghtui` unless given
  `--ghtui PATH`.
- **What the test repository needs:** issue #8 open; PR #5 open; PR #11
  with `bot-bot-1791523358.txt` in its files; `actions.yml`, a workflow
  run by hand with a `seconds` input whose job fails; a protection rule
  on `main`; and, for `pages`, code scanning alert #1 and a discussion
  if those pages are to be opened (they're left out otherwise).
- **Where it runs:** ghtui runs in a private fux server, one pane at a
  time, at 216 columns, with its own socket and `XDG_RUNTIME_DIR` and a
  scratch cache, config and clone, all under
  `~/.cache/claude-tmp/ghtui-live` (`GHTUI_LIVE_DIR`), which a run
  empties when it starts. A failing check leaves its last screen in
  `failures/`. A stand-in `xdg-open`, first on the server's `PATH`,
  writes down the URLs ghtui opens instead of opening a browser.
- **What it touches and cleans up:** it writes only to the test
  repository. Each check undoes what it made, pass or fail: the issues
  (deleted), the pull request (closed) and its branch, the release and
  its tag, the label, the protection rule and the workflow run; and the
  fux server and the stand-in are stopped. ctrl-c stops the run after
  cleaning up. A run that was killed is swept up by the next one:
  issues and pull requests titled `[ghtui live]`, and `ghtui-live-`
  branches, releases, tags, labels and protection rules.
- **Enterprise Server:** `cargo xtask ghes` serves a stand-in on
  `https://localhost:8443` that relays an Enterprise Server's paths to
  github.com (`/api/v3`, `/api/graphql`, `/raw`, git) and rewrites
  github.com's URLs in its answers to its own host. Its certificate
  authority is trusted only by the programs given `SSL_CERT_FILE` and
  `GIT_SSL_CAINFO`; it prints how to run ghtui against it. To stand in
  for an older server, `--version 3.N` is the version its `/meta` says,
  and `--strip FIELD` (or `Type.field`) refuses a query naming that
  field, as an older server does. To see what a published version's
  schema lacks of ghtui's queries, run `GHTUI_GHES_SDL=schema.graphql
  cargo test what_an_enterprise_schema_lacks -- --ignored --nocapture`
  (with `GHTUI_GHES_VERSION=3.N`, after ghtui leaves out what it knows
  that version lacks: nothing should be left).

Adding a check: write a `Check` (a name, what it shows, and a function)
in a file under `xtask/src/live/checks/`, and list it in `ALL` there. A
new kind of page is a line in `pages` instead. The function starts
ghtui (`cx.ghtui(&[…])`, or `cx.launch` for a directory or
environment), presses keys (`ghtui.keys(", m")`, `ghtui.type_text(…)`),
waits for the screen (`wait_for`, `wait_gone`, `wait_followed`,
`select`), asks GitHub (`gh::get`, `gh::send`, `gh::graphql`, retried
with `until`), and registers what to undo with `cx.defer` as soon as it
has made something. Name what it makes `[ghtui live] …` (`cx.mark`) or
`ghtui-live-…`, and give the check a `.sweep(…)` for what the run's own
sweep doesn't already find. Assert behaviour ("the comment shows",
"GitHub has it closed"), never whole screens. Add a check only for a
new kind of page or write: a variation of one is a test above.

`demo/record.sh` records a captioned tour of ghtui to a video, on a hidden
Hyprland monitor (it needs foot, grim and ffmpeg, and an unlocked session). It
uses a scratch config and cache, and only reads public GitHub.

Timing tests (`cargo test --release -- --ignored --nocapture`)
hold scrolling a 500-file pull request to under 8ms a frame (about 0.5ms
today), and diffing a 50,000-line file to under 100ms (about 32ms).

## Known limitations

- Explore isn't GitHub's Trending: GitHub's API doesn't say which
  repositories got stars lately, so Explore lists new repositories, most
  starred, and says so. A repository's dependents (who depends on it)
  aren't in GitHub's API either; the dependency graph links to them on
  github.com.
- Contributors and commit activity are counted with git, by git name:
  two names for one person are one only where the repository's
  `.mailmap` says so, and lines added and removed aren't counted (that
  would need every file's contents). ghtui counts the newest 200,000
  commits, and says when there were more. GitHub's statistics, used
  until the clone is made, list only its top 100 contributors.
- The dependency graph shows 20 manifests and the first 100
  dependencies of each, and says how many more GitHub has.
- Saved replies can be inserted, but not made, changed or deleted here:
  GitHub's API has no way to, so that's on github.com/settings/replies,
  which the list names. They take the `read:user` scope.
- Packages are listed 100 of each kind, and said when there are more.
  GitHub doesn't say beforehand who may delete a package's version (its
  admins), so `D` is offered on every version and GitHub's refusal is
  said. On an Enterprise Server, the install commands name its
  registries as `containers.HOST`, `npm.HOST` and so on (with subdomain
  isolation). A package's link opens under `/users/` (GitHub sends an
  organization's on).
- `:lists` reads up to 1,000 of a list's repositories to say whether one is
  in it, and says when a list is longer. A new list is public; its
  name, description and privacy are changed on github.com.

- A running job's log shows once the job ends: GitHub's API has no log
  until then (its website streams it over a connection only a signed-in
  browser can open). Its steps follow it meanwhile.
- A job's summary (what a workflow writes to `$GITHUB_STEP_SUMMARY`)
  isn't shown: GitHub keeps it out of its API, REST and GraphQL alike.
- A check's annotations are its first 50; the Checks tab shows each
  check's first five, and its page the rest.
- A discussion is watched over GraphQL, which has no ETags: each ask
  costs a point of the rate limit (the backing off and the slower pace
  out of focus keep that low).
- On an Enterprise Server that doesn't say which run a workflow run by
  hand started, the workflow's runs open instead of the run.
- One GitHub at a time: the host is chosen at start.
- Offline, a page opens only if it was read before (or its repository
  is kept offline), as it was then; a list shows its first page. A
  file's contents come from a clone only where it has them: the cache's
  partial clone fetches a file the first time it's read, so a file never
  opened isn't there, except in a repository kept offline (its default
  branch's files, within the disk limit). A pull request's Files changed
  fetches its commits from GitHub, so it doesn't open offline. Searching
  GitHub, the inbox's marks and merging aren't queued.
- Keeping a repository offline keeps its issues', pull requests' and
  discussions' pages (a pull request's conversation, not its Files
  changed, checks or commits), and the default branch's files; the disk
  limit is checked between pages, so a page or a clone may take it a
  little past. Stopping keeping one leaves what's cached until it's
  replaced.
- The queue, like the cache, is kept under the account GitHub last said
  the token is; a token never used before keeps it under its
  fingerprint until GitHub first answers.
- On an Enterprise Server, gists are taken to be at `HOST/gist`. A page
  goes without what the server's version doesn't have: who may assign,
  label or set a milestone isn't said by any Enterprise Server yet, so
  those are offered to whoever may edit (GitHub refuses the rest); before
  3.22, a project's archived items are told apart by each item, and
  before 3.20 a view's filter doesn't apply (all its items are shown).
- Long code lines are cut at the edge, and `←` and `→` scroll them
  sideways, unless Wrap long lines (in the menu) is on; a wrapped line
  takes at most 100 rows.
- Markdown in the terminal: images show their alt text, and HTML is reduced
  to its text and links.
- The first time a token is used, until GitHub says whose it is (a moment
  after starting), the cache and drafts are kept under the token's
  fingerprint, and don't carry over to its account's.
- Notifications filtered by a reason (`reason:mention`) or by `is:read`
  are filtered after fetching, as GitHub's API can't filter them: a load
  searches up to 300 notifications for a page of matches, and says when
  it stopped short. Marking done goes thread by thread, and only through
  those loaded. GitHub's API goes on listing a thread marked done, so
  ghtui keeps its own record of the threads you marked done here (per
  account, in its cache): one marked done on github.com or in another
  ghtui shows on, read. Fine-grained tokens can't read notifications at
  all; a classic token can.
- A pull request's Review tab and a review's page have no page on
  GitHub, so their links (`l`, `o`, a restored tab) are the
  conversation's (at the review).
- A conversation shows the newest 100 entries of its timeline and says
  how many earlier ones are on GitHub. A thread's lines are read from git
  only where a clone is already there (yours, or the cache's once Files
  changed made one), for up to 40 files; elsewhere, and for lines on the
  old side, it shows GitHub's diff hunk.
- A commit's comments are its first 100, and only a comment on the
  whole commit is written here (GitHub's REST API takes a line comment
  only by its place in the diff). An issue's or pull request's sidebar
  lists its first 10 projects.
- `g` and `:repo symbols` go by name: the index knows functions, types, traits,
  classes and modules, not imports, re-exports, names macros make, or
  what type a value has, so a name defined in several places offers
  them all. A repository bigger than the index's limits is indexed
  nearest the file you're in, and ghtui says so. Without a clone to
  read, or while the index is still building, `g` asks GitHub's code
  search, which reads only the default branch and searches text; it
  reads the first five files it finds.
- `:references` in the cache's partial clone searches only the files fetched for
  the index (Rust, TypeScript, JavaScript, Python and Go, within its
  limits), and says so; in your own clone it searches every text file.
  It lists the first 1,000 lines.
- `@` offers as taking part only who has written what's loaded (the
  newest 100 comments, and the review threads), and `#` the 100 issues and
  pull requests updated last.
- The sidebar's pickers list a repository's first 100 labels, people,
  open milestones and teams, and say how many more GitHub has. Teams are
  listed only where your token can read the organization.
- GitHub doesn't say who may set a pull request's milestone or ask for
  its reviews; ghtui offers them to whoever may label it (triage access).
- A new pull request can't set labels, assignees or a milestone (GitHub's
  `createPullRequest` doesn't take them), and a fork's branch is proposed
  only from a clone that's on it. Of several pull request templates, the
  first is used.
- A branch for an issue is made from its repository's default branch, in
  that repository (GitHub's dialog also lets you pick another source
  branch or a fork). A revert's pull request has GitHub's own title and
  description, and isn't a draft.
- An issue moves only to one of its owner's repositories (as GitHub
  allows); the first 100 of them, most recently pushed, are offered. The
  pickers for a duplicate, a sub-issue or a blocking issue list the
  repository's 50 open issues updated last; any other is typed
  (`#12`, `owner/repo#12` or its link).
- Hidden comments fold in conversations and discussions; a hidden review
  comment in Files changed shows as it is, and is hidden or unhidden on
  github.com.
- A wiki without pages has no git repository yet: GitHub makes it with
  the first page, which has to be made on github.com. A wiki page can't
  be renamed or deleted here, and its commit message is GitHub's own
  ("Updated Home (markdown)").
- GitHub's API can't change whether a gist is public: a secret gist is
  made public on github.com. A gist file over GitHub's size limit for
  its API can't be changed here.
- The labels page lists 100 labels at a time (the rest one row away),
  and the labels' issue counts are open issues and pull requests. A
  branch can't be renamed here, nor a repository deleted (that's the
  website's).
- A file is edited one at a time, as text: a binary file, or one over
  GitHub's API's size limit, can't be. A commit made with git in your
  clone isn't signed unless your git signs commits by default; one
  GitHub's API makes is signed by GitHub. Changing a workflow's file
  takes the token's `workflow` scope (`gh auth refresh -s workflow`).
  A branch protected so that it takes pull requests refuses a commit
  straight to it: commit to a new branch, and propose it.
- A new repository is cloned with your own git into the folder ghtui
  started in; a tag's protection by a ruleset is GitHub's to say, when
  deleting it.
- A repository's custom watch settings (only releases, issues…) and a
  thread's custom events can only be set on github.com; `:watch` offers
  participating, all activity and ignore. A new tag is made by GitHub
  when the release is published; a tag only in your clone has to be
  pushed first. A release's tag is deleted on GitHub, not in your clone.
  An asset is uploaded under its file's name, without a label.
- A repository's settings cover protection rules, rulesets, collaborators,
  webhooks, deploy keys and archiving; the rest of its settings (features,
  merge buttons, Actions, Pages, secrets, environments) are on
  github.com. A protection rule's form has GitHub's checkboxes; who may
  push, bypass or dismiss reviews, and required deployments, are set on
  github.com (and kept when a rule is edited here). A ruleset is turned
  on or off here, and written on github.com. A collaborator is invited
  by login (there's no search of people), and an organization's teams'
  access isn't listed. Each list shows its first 100, and says when
  GitHub has more; a webhook's deliveries are its latest 100. A
  webhook's secret can be changed, not removed.
- An issue form's dropdowns become a comment listing their options, to
  answer in your own words; its validations aren't checked.
- Security alerts are listed newest first, 30 at a time, without counts
  (GitHub's API doesn't count them); `S` doesn't sort them. A dismissal's
  comment is one line, and a code scanning alert shows its latest instance,
  not every branch it's on.
- A view that sorts or groups its items (a board does) loads them all
  up to 1,000, and says so when there are more. GitHub's API doesn't say
  which date fields a roadmap uses, or its zoom: ghtui takes a date field
  named for a start and one for a target (or end, or due), else the
  first two date fields, else the first iteration field, and says which;
  the timeline spans the items' dates and today. Archived items are
  listed 50 at a time, newest first, whatever the view's filter.
- A page read with git knows commits' authors by their git names: by
  their GitHub accounts only where the email is GitHub's no-reply
  address for one. A commit read with git doesn't say whether GitHub
  verified its signature. The branches list (each branch's pull request)
  still comes from GitHub's API.
- Code search runs with git only in one repository, for words, with
  `path:`, `extension:` and `filename:`; anything else (`language:`,
  `OR`, several repositories) goes to GitHub's code search. A word
  matches as text, without regard to case, not as a whole word.
