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
  discussions, wikis, milestones, deployments, security advisories, gists,
  teams, profiles, projects and search, on github.com or a GitHub Enterprise
  Server. Every GitHub link opens in ghtui unless it can only work in a
  browser (a download, settings), and then it says why.
- **Reviewing** is where ghtui goes furthest. Diffs are computed locally
  with git, highlighted with tree-sitter, and shown unified or split. You
  can reply, resolve, comment on lines or ranges, suggest changes, and
  submit a review.
- **Acting** works from a page or from a row in a list: merge (now, when
  checks pass, or through a merge queue), approve, mark ready for review,
  update a branch, close or reopen, re-run or cancel a workflow run,
  comment and star, mark notifications read or done, one at a time or
  all at once, and move a project's cards or set their fields.
- **Editing** what's there: your comments and descriptions (or anyone's,
  where GitHub lets you), titles, reactions, and an issue's or pull
  request's labels, assignees, reviewers and milestone.
- **Creating**: open issues (from the repository's templates) and pull
  requests, and check a pull request out in your clone.

Not there yet: code navigation with a language server's precision,
structural diffs, and adding, archiving or editing a project's items.

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
-s project` adds).

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
`Space` menu, and rename, reorder or remove sections in place.

**Notifications** (`gn`, or the link at the top of Home) is your inbox,
as on GitHub: what each thread is about, why you got it and when, the
unread ones marked. `/` filters it as GitHub does (`is:unread`,
`reason:review-requested`, `repo:owner/name`), `t` shows unread, read or
all, and `↵` opens the thread at its latest comment. `,n` marks a thread
read and `,d` marks it done; `,N` and `,D` mark everything shown. A
filter GitHub can't apply (a reason, `is:read`) searches further pages
until a page fills. The header counts your unread notifications, kept
fresh as often as GitHub asks; click or follow it to open the inbox.

**Projects** are a tab of a profile and of a repository, as on GitHub.
A project opens at its first view, and its views are the page's tabs. A
board shows each column (each value of its Status, or the field it's
grouped by) as a box of cards, top to bottom; `h` and `l` go between
columns. A table shows the view's fields in columns, as many as fit. A
view's filter, sort and grouping apply. A card opens what it is: an
issue, a pull request, or a draft's own page with its body and fields.
Where GitHub lets you change the project, `,c` moves the card to another
column and `,f` sets one of its fields (a single select, an iteration, a
text, a number or a date), and the board follows GitHub until it shows
the change.

**Tabs** work like a browser's: `T` opens the selection in a new tab, `]b`
and `[b` switch between them, and they come back next time you start.

**The diff viewer** (a pull request's Files changed) adds what GitHub's
doesn't:
- changed words within a line, and changed letters within a word;
- moved code, with a jump between its two ends;
- formatting-only changes folded away;
- scope names in hunk headers (`impl Doc › fn offset`);
- "changes since your last review", even across force-pushes and rebases;
- local per-change "reviewed" marks beside GitHub's per-file "viewed".

**Reading code** goes beyond the file on screen, without a language
server. `gO` lists a file's functions, types, impls and modules, nested
as in the code, and goes to the one you pick (in Files changed, the
file under the cursor's). `gd` goes to the definition of the word under
the cursor: in the file when it's there, else in the files GitHub's
code search finds the word in, read at the revision you're viewing, and
it says when it can't tell. Symbols are read for Rust, TypeScript,
JavaScript, Python and Go.

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
out of focus. What came in since you started looking is marked New, and
the status bar says what it was; after a push to a pull request you've
reviewed, it offers the changes since your last review.

**A pull request's Review tab** (`gr` from any of its tabs) shows where
its review stands, so you don't have to piece it together:
- your drafts that aren't submitted yet;
- the threads still unresolved, in every file;
- who has reviewed and what they said last, and who is asked and hasn't
  answered yet;
- what still stops the merge.

Each row opens what it's about: a draft or a thread in Files changed at
its line, a review in the conversation.

**Acting on GitHub** asks first wherever it matters (merging, closing,
cancelling, updating a branch, deleting a comment). It asks GitHub what
you may do and says why when you can't. After a change, the page follows
GitHub until it shows it.

**Editing** works on what's selected: a comment (or, with none selected,
the description) opens in the comment editor in its own place, with its
text. Reactions show under each comment as GitHub shows them, yours
marked, and `,+` picks one of GitHub's eight to give or take back. The
sidebar's fields are edited in pickers, as on GitHub: check and uncheck
labels, assignees or reviewers (people and teams), then apply them with
`ctrl-s`; a milestone is one choice. In Files changed, the same keys work
on the review comment under the cursor.

**A new issue** (`,i` from any of a repository's pages) starts from the
repository's templates, as on GitHub: choose one, and its title, text,
labels and assignees fill the form (a YAML form's fields become headings
to answer under). The comment editor opens in place of the text: the
first line is the title, the rest the description. Labels, assignees and
a milestone are rows of the form, each with its picker. `ctrl-s` (or
`,s` on the form) creates it, and the new issue opens.

**A new pull request** (`,p`) proposes, inside a clone, the branch you're
on, into the default branch; Enter on either branch picks another. It's
filled in as GitHub fills it: one commit's message, or the branch's name
and a list of its commits, then the repository's template. A branch
GitHub doesn't have, or has behind yours, says so, and `,P` pushes it
with your own `git push` after asking. `,s` asks whether it's a draft,
then creates it.

**Checking out** a pull request (`,c` on it or its row), inside a clone
of its repository, fetches its head and switches to a branch for it: its
own branch's name (tracking it), or `owner/branch` for a fork's. It
refuses while you have changes that aren't committed, and says which
branch you're on.

## Keys

The keys are vim's where vim has one. A binding is a key sequence (`gg`,
`]c`), and digits before it are a count: `5j` moves five rows, `3gt` goes
to the page's third tab (the tab bar numbers them), `120gg` to line 120 of
a file. After the first key of a sequence, a panel lists what can follow
it here.

What a key does depends on where you are: the innermost of the contexts
below that binds it wins. A list adds its keys to a page's, which add
theirs to the ones that work everywhere; in Files changed, the file tree
has the focus or the diff does. A key that does nothing here says so, and
says where it works.

- `esc` only cancels: a search, a selection, an overlay or keys typed so
  far. It never leaves the page. Back is `ctrl-o` (or `⌫`), forward is
  `ctrl-i`, which terminals send as `⇥` (in Files changed `⇥` switches
  between the file tree and the diff, so forward there is `alt-→`).
- A page's own tabs (a pull request's Conversation, Commits, Checks,
  Files changed and Review) aren't history: `gt` and `gT` switch them in
  place, and each keeps where you were.
- `/` searches what's on screen: the page, a file, the diff or a log, and
  `n` and `N` go through the matches. On a list it edits the list's
  filter instead, in place on the list. `g/` searches GitHub, as does `:`.
- In a file, `j` and `k` move line by line, `y` copies the line's link,
  `V` selects lines (then `y` copies `#L10-L20`), and `gb` blames the
  file at that line. In the blame, a run of lines opens its commit. A
  job's log is the same: each line is a row with its own link.
- `h` and `l` scroll code sideways in files, logs and the diff; in the
  diff's file tree they fold and unfold folders, and the diff shows the
  file selected there.
- In the diff, `y` copies the link to the line, or to the lines `V`
  selected, as GitHub writes it; such a link opens back on those lines.
- What changes GitHub waits behind `,`: `,m` merges, `,a` approves, `,r`
  marks a draft ready, `,u` updates the branch, `,x` closes or reopens
  (or cancels a run), `,R` re-runs, `,*` stars, and in Files changed `,v`
  marks a file viewed, `,t` resolves a thread and `,s` submits your
  review. Each asks first where it matters.
- `,m` offers what GitHub would: merging now, merging when ready
  (auto-merge, offered first while checks run), or adding it to the
  merge queue where its base branch has one. Once it's set to merge or
  queued, the merge box says so (who turned auto-merge on and how it
  will merge; its place in the queue), and `,m` takes that back.
- Edits wait behind `,` too: `,e` edits the selected comment or the
  description, `,D` deletes a comment, `,+` reacts, and on an issue or
  pull request `,T` edits its title, `,l` its labels, `,A` its
  assignees, `,q` asks for reviews and `,M` sets its milestone.
- New things wait behind `,` as well: `,i` writes a new issue and `,p` a
  new pull request, which `,s` creates (and `,P` pushes its branch
  first); `,c` checks a pull request out in your clone.
- `q` closes the view: back a page, or the tab when there's nothing to go
  back to. `ZZ`, `:q` in the palette, or `ctrl-c` quits; `ctrl-c` also
  works in the comment editor, asking before it throws away your text.
- `]]` and `[[` jump by section: a Home section, a box, a heading, a
  folder in the file tree, a file in the diff. `}` and `{` go row by row,
  comment by comment, and change by change in the diff. `H` `M` `L` go to
  the top, middle and bottom of the screen.
- `?` searches backwards, and `n` and `N` follow its direction. `>`
  replies quoting the selected comment; Enter on a comment doesn't start
  writing. In Files changed the view toggles are under `z`: `zw`
  whitespace, `zs` split, `zf` the whole file, `zT` the file tree; `zc`
  `zo` `za` fold a thread or file, `zM` `zR` every file.
  In the file tree, `<` and `>` narrow and widen it.
- On a project's board `h` and `l` go between columns; `,c` moves the
  selected card to another column and `,f` sets one of its fields.
- `gn` opens your notifications (`,n` marks one read, `,d` done, `,N`
  and `,D` everything shown) and `gr` a pull request's Review tab.
- In files, logs and the diff, `w` and `b` move a word cursor along the
  line: `*` and `#` search its word as a whole word (vim's `\<word\>`;
  `/` takes `\<` and `\>` too), down and up, and `gx` opens a web
  address under it. A job log's lines are rows, as a file's are.
- In files and the diff, `gO` lists the file's symbols and `gd` goes to
  the definition of the word under the cursor.

**Everywhere** (`global:`)

| Keys                 | Action |
| -------------------- | ------ |
| `j` `↓` `ctrl-n`     | Move down |
| `k` `↑` `ctrl-p`     | Move up |
| `ctrl-d`             | Half page down |
| `ctrl-u`             | Half page up |
| `ctrl-f` `pgdn`      | Page down |
| `ctrl-b` `pgup`      | Page up |
| `ctrl-e`             | Scroll down a line |
| `ctrl-y`             | Scroll up a line |
| `zz`                 | Scroll the cursor to the middle |
| `zt`                 | Scroll the cursor to the top |
| `zb`                 | Scroll the cursor to the bottom |
| `H`                  | To the top of the screen |
| `M`                  | To the middle of the screen |
| `L`                  | To the bottom of the screen |
| `gg` `home`          | Go to the top (with a count, to that line) |
| `G` `end`            | Go to the bottom |
| `↵`                  | Open the selection |
| `esc`                | Cancel a search, a selection or keys typed so far |
| `ctrl-o` `⌫` `alt-←` | Back |
| `⇥` `alt-→`          | Forward |
| `q`                  | Close this view: back, or close the tab |
| `-` `u`              | Up a level |
| `gt`                 | Next tab of the page (with a count, that tab) |
| `gT`                 | Previous tab of the page |
| `]b` `ctrl-pgdn`     | Next open tab |
| `[b` `ctrl-pgup`     | Previous open tab |
| `T`                  | Open the selection in a new tab |
| `ctrl-w`             | Close the tab |
| `alt-1`–`alt-9`      | Go to open tab 1 to 9 |
| `gh`                 | Go home |
| `gn`                 | Your notifications |
| `gr`                 | The pull request's review at a glance |
| `/`                  | Search what's on screen (on a list, filter it) |
| `?`                  | Search what's on screen, backwards |
| `n`                  | Next search match |
| `N`                  | Previous search match |
| `*`                  | Search the word under the cursor |
| `#`                  | Search the word under the cursor, backwards |
| `g/`                 | Search GitHub or go to a page |
| `f`                  | Go to file |
| `c`                  | Comment (on a thread, reply) |
| `y`                  | Copy the link |
| `o`                  | Open on GitHub |
| `gx`                 | Open the link under the cursor in the browser |
| `r` `ctrl-l`         | Refresh |
| `space`              | Everything you can do here |
| `:` `ctrl-k`         | Command palette |
| `,m`                 | Merge the pull request |
| `,a`                 | Approve the pull request |
| `,r`                 | Mark the draft ready for review |
| `,u`                 | Update the branch from its base |
| `,x`                 | Close or reopen; on a running run, cancel it |
| `,R`                 | Re-run the workflow run or job |
| `,e`                 | Edit the comment or description |
| `,D`                 | Delete the comment |
| `,+`                 | React to the comment, issue or pull request |
| `,c`                 | Check out the pull request in your clone |
| `del`                | Delete the selection (a draft comment, a Home section) |
| `ctrl-z`             | Bring back what was just deleted |
| `ZZ` `ctrl-c`        | Quit |

**On pages** (`page:`)

| Keys    | Action |
| ------- | ------ |
| `]]`    | Next section (a box, a Home section, a folder) |
| `[[`    | Previous section |
| `}`     | Next comment or row |
| `{`     | Previous comment or row |
| `]C`    | Next commit of the pull request |
| `[C`    | Previous commit of the pull request |
| `>`     | Reply quoting the comment |
| `,*`    | Star / unstar |
| `,T`    | Edit the title |
| `,l`    | Edit the labels |
| `,A`    | Edit the assignees |
| `,q`    | Ask for reviews |
| `,M`    | Set the milestone |
| `,i`    | New issue |
| `,p`    | New pull request |
| `,s`    | Create the issue or pull request |
| `,P`    | Push the branch to GitHub |
| `B`     | Switch branches or tags |
| `i`     | Follow a link by its letters |
| `I`     | Follow a link, in the browser |
| `alt-↑` | Move the Home section up |
| `alt-↓` | Move the Home section down |

**On lists** (`list:`)

| Keys | Action |
| ---- | ------ |
| `t`  | Open / closed / all (in the inbox, unread / read) |
| `S`  | Change the sort |
| `,n` | Mark the notification read |
| `,d` | Mark the notification done |
| `,N` | Mark everything shown read |
| `,D` | Mark everything shown done |

**In files** (`file:`)

| Keys | Action |
| ---- | ------ |
| `gb` | Blame the file at this line, or back to the file |
| `h`  | Scroll left |
| `l`  | Scroll right |
| `w`  | Next word (for * and gx) |
| `b`  | Previous word |
| `gO` | Symbols in this file, to go to one |
| `gd` | Go to the definition of the word under the cursor |
| `V`  | Select lines |

**In job logs** (`log:`)

| Keys | Action |
| ---- | ------ |
| `h`  | Scroll left |
| `l`  | Scroll right |
| `w`  | Next word (for * and gx) |
| `b`  | Previous word |

**In projects** (`project:`)

| Keys | Action |
| ---- | ------ |
| `h`  | Previous column of the board |
| `l`  | Next column of the board |
| `,c` | Move the item to another column |
| `,f` | Set a field of the item |

**In Files changed** (`diff:`)

| Keys           | Action |
| -------------- | ------ |
| `i`            | Follow a link by its letters |
| `I`            | Follow a link, in the browser |
| `h`            | Scroll left |
| `l`            | Scroll right |
| `w`            | Next word (for * and gx) |
| `b`            | Previous word |
| `gO`           | Symbols in this file, to go to one |
| `gd`           | Go to the definition of the word under the cursor |
| `]c` `}`       | Next change |
| `[c` `{`       | Previous change |
| `]f` `]]` `⇧↓` | Next file |
| `[f` `[[` `⇧↑` | Previous file |
| `⇧←`           | Old side of a split row (for comments) |
| `⇧→`           | New side of a split row (for comments) |
| `U`            | Next unviewed file |
| `]t`           | Next unresolved thread |
| `[t`           | Previous unresolved thread |
| `%`            | Jump to the other end of moved code |
| `⇥`            | Switch between the file tree and the diff |
| `zc`           | Fold the file or thread |
| `zo`           | Unfold the file or thread |
| `za`           | Fold or unfold the file or thread |
| `zM`           | Fold every file |
| `zR`           | Unfold every file |
| `zT`           | Show or hide the file tree |
| `zs`           | Split or unified view |
| `zw`           | Ignore whitespace changes |
| `zf`           | Show the whole file |
| `e`            | Show more context |
| `,v`           | Mark the file viewed (syncs with GitHub) |
| `m`            | Mark the change reviewed |
| `V` `x`        | Select lines |
| `C`            | Comment on the whole file |
| `,t`           | Resolve or unresolve the thread |
| `,s`           | Submit your review |

**In Files changed's file tree** (`tree:`)

| Keys | Action |
| ---- | ------ |
| `]]` | Next section (a box, a Home section, a folder) |
| `[[` | Previous section |
| `h`  | Fold the folder (on a file, go to its folder) |
| `l`  | Unfold the folder |
| `<`  | Narrow the file tree |
| `>`  | Widen the file tree |

The comment editor opens where the comment will go: under the lines, the
thread or the file header in Files changed, and in place of the comment
box on a conversation (or of the comment it edits). In it, `ctrl-s` adds the comment, `ctrl-e`
continues in `$EDITOR`, and `esc` cancels (keeping the lines selected).
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

[keys]                # each action listed replaces its default keys
down = ["j", "<Down>"]  # without ctrl-n
sort = ["list:O"]     # a context first binds it there only
go_home = ["gh", "<A-h>"]   # sequences or modified keys

[[home]]              # a Home section: a title and a GitHub search
title = "Bugs in our repos"
issues = "is:open label:bug org:my-org sort:updated-desc"
rows = 10
```

The host can also come from `GHTUI_HOST`, which wins over the config file,
or from gh's `GH_HOST`, which doesn't. On an Enterprise Server, ghtui asks
its API at `https://HOST/api/v3` and `https://HOST/api/graphql`, its links
and its clones' remotes open in ghtui, and links to github.com open in the
browser (and the other way round). The cache, the tabs and the review
drafts are kept per host and per account, so switching accounts or hosts
never shows another's pages.

A section lists `pulls`, `issues` or `repos`, and shows up to 30 rows; its
last row opens the rest. Without any `[[home]]` tables, Home shows your
review requests, your open pull requests and your repositories. Action
names for `[keys]` are in the `actions!` table in
[`src/keymap.rs`](src/keymap.rs). A key bound twice in one context, or one
that starts another where both apply, is an error that names both.

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
at your last review. Comments still anchor to GitHub's own hunks, so they
land where GitHub expects them. Git never sees the
token on a command line or in a config file.

**Fast from the cache.** The first paint comes from a local cache (redb),
then everything is refreshed in the background. REST requests revalidate
with ETags (and `Last-Modified`), so unchanged data costs nothing against
the rate limit.

**Colors from one seed.** Light and dark schemes are generated with
Material 3's tonal palettes. Every foreground and background pair the UI
uses is tested for contrast in both schemes, in truecolor and 256 colors.

It's one crate. The top of `src/` is the app (routes, state, keys, view,
runtime) and the binary that runs it; the rest are its modules:

| Module   | What it does                                                    |
| -------- | --------------------------------------------------------------- |
| `ui`     | Pages and widgets, pure functions of data                       |
| `api`    | GitHub over GraphQL (cynic) and REST: auth, retries, limits     |
| `schema` | GitHub's GraphQL schema, which the queries are typed against   |
| `store`  | The cache, and review drafts kept outside it                    |
| `git`    | The `git` CLI: clones, fetches, credentials, blobs              |
| `diff`   | Hunks (imara-diff), highlighting (tree-sitter), the diff model  |
| `theme`  | Color schemes, contrast, terminal detection                     |

## Development

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
scripts/check-tls-deps.sh
```

The tests come at GitHub's data from several sides:
- every query is checked against the schema;
- recorded GitHub responses are replayed offline;
- git is the oracle for diffs, on real repositories over `file://`;
- full screens are snapshotted in light, dark and 256 colors
  (`INSTA_UPDATE=always cargo test` to accept changes, then review
  them);
- property tests throw random keys, resizes and replies at the app.

A contract suite checks ghtui's assumptions against live GitHub, read-only
(`cargo test --test contract -- --ignored contract`). Every
kind of github.com link is listed with what it must open in
`tests/github_urls.txt`.

`demo/record.sh` records a captioned tour of ghtui to a video, on a hidden
Hyprland monitor (it needs foot, grim and ffmpeg, and an unlocked session). It
uses a scratch config and cache, and only reads public GitHub.

Timing tests (`cargo test --release -- --ignored --nocapture`)
hold scrolling a 500-file pull request to under 8ms a frame (about 0.5ms
today), and diffing a 50,000-line file to under 100ms (about 32ms).

## Known limitations

- One GitHub at a time: the host is chosen at start.
- On an Enterprise Server, gists are taken to be at `HOST/gist`, and
  ghtui's queries are checked against github.com's schema: a server too
  old for a field ghtui asks for answers with an error instead.
- Long code lines are cut at the edge: `h` and `l` scroll them sideways,
  but they don't wrap.
- Markdown in the terminal: images show their alt text, and HTML is reduced
  to its text and links.
- The first time a token is used, until GitHub says whose it is (a moment
  after starting), the cache and drafts are kept under the token's
  fingerprint, and don't carry over to its account's.
- Notifications filtered by a reason (`reason:mention`) or by `is:read`
  are filtered after fetching, as GitHub's API can't filter them: a load
  searches up to 300 notifications for a page of matches, and says when
  it stopped short. Marking done goes thread by thread, and only through
  those loaded. Fine-grained tokens can't read notifications at all; a
  classic token can.
- A pull request's Review tab has no page on GitHub, so its link (`y`,
  `o`, a restored tab) is the conversation's.
- `gd` doesn't know types or imports: it goes by name. Outside the
  file, it searches with GitHub's code search API, which reads only the
  default branch and searches text, not symbols; it reads the first five
  files it finds at the revision you're viewing.
- `@` offers as taking part only who has written what's loaded (the
  newest 100 comments, and the review threads), and `#` the 100 issues and
  pull requests updated last.
- A review's summary can't be edited, and a discussion's comments show no
  reactions.
- The sidebar's pickers list a repository's first 100 labels, people,
  open milestones and teams, and say how many more GitHub has. Teams are
  listed only where your token can read the organization.
- GitHub doesn't say who may set a pull request's milestone or ask for
  its reviews; ghtui offers them to whoever may label it (triage access).
- A new pull request can't set labels, assignees or a milestone (GitHub's
  `createPullRequest` doesn't take them), and a fork's branch is proposed
  only from a clone that's on it. Of several pull request templates, the
  first is used.
- An issue form's dropdowns become a comment listing their options, to
  answer in your own words; its validations aren't checked.
- A project's board is its columns one above another, not side by side.
  A view's sort orders the items loaded (50 a page), not the whole
  project; a roadmap shows as a table, and a board's swimlanes aren't
  drawn.
