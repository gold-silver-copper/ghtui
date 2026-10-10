# ghtui

A keyboard-driven terminal client for GitHub, written in Rust.

The aim is simple: never open github.com for day-to-day work. You browse
repositories, issues, pull requests, Actions and profiles the way you would
on the website, and you review pull requests in a diff viewer that does more
than GitHub's. It looks like GitHub, in flat Material 3 colors adapted to
the terminal.

## Where it stands

ghtui is pre-release, but it's usable every day.

- **Browsing** covers most of GitHub: your home page, repositories (files,
  README, branches, tags, releases, commits, blame, comparisons), issues,
  pull requests, Actions runs and job logs, discussions, wikis, milestones,
  deployments, security advisories, gists, teams, profiles and search. Every
  github.com link opens in ghtui unless it can only work in a browser (a
  download, settings, notifications), and then it says why.
- **Reviewing** is where ghtui goes furthest. Diffs are computed locally
  with git, highlighted with tree-sitter, and shown unified or split. You
  can reply, resolve, comment on lines or ranges, suggest changes, and
  submit a review.
- **Acting** works from a page or from a row in a list: merge, approve,
  mark ready for review, update a branch, close or reopen, re-run or cancel
  a workflow run, comment and star.

Not there yet: notifications, projects, GitHub Enterprise Server, code
navigation and structural diffs.

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
cargo install --path crates/app
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
token`. If none works, it asks you to run `gh auth login`. The token stays
in memory: it's never logged, written to disk, or put on a command line.
Reading needs read access. Acting needs write access where you act (with a
classic token, the `repo` scope covers it all).

## A short tour

The screen is laid out like GitHub's:
- **The header** shows where you are, a search field and your account.
- **A page's tabs** sit under the header: Code, Issues, Pull requests and
  so on.
- **The content** is in GitHub's boxes, with a sidebar on wide screens.
- **The status bar** names the keys that do something here.

`Space` lists everything you can do where you are. `:` opens a command
palette that also goes places (`owner/repo#123`, `@user`, a URL).

**Home** is your own saved searches: by default your review requests, your
open pull requests and your repositories. Save any list to Home from its
`Space` menu, and rename, reorder or remove sections in place.

**Tabs** work like a browser's: `T` opens the selection in a new tab, `[`
and `]` switch between them, and they come back next time you start.

**The diff viewer** (a pull request's Files changed) adds what GitHub's
doesn't:
- changed words within a line, and changed letters within a word;
- moved code, with a jump between its two ends;
- formatting-only changes folded away;
- scope names in hunk headers (`impl Doc › fn offset`);
- "changes since your last review", even across force-pushes and rebases;
- local per-change "reviewed" marks beside GitHub's per-file "viewed".

Comments are kept on disk until you submit, so a crash never loses them.
Threads that GitHub calls outdated are moved to their line when it still
exists.

**Acting on GitHub** asks first wherever it matters (merging, closing,
cancelling, updating a branch). It asks GitHub what you may do and says
why when you can't. After a change, the page follows GitHub until it shows
it.

## Keys

Arrows move, `Enter` opens, `Esc` goes back, and each letter is one verb.
Less common actions have no key; they're in the `Space` menu. Every key can
be rebound (see [Configuration](#configuration)).

| Keys                  | Action                                                        |
| --------------------- | ------------------------------------------------------------- |
| `↑` `↓` (`j` `k`)     | Move                                                          |
| `←` `→` (`h` `l`)     | Previous / next tab                                           |
| `Enter`               | Open the selection (on a comment: quote reply)                |
| `Esc` `⌫` `alt-←`     | Back (clears a search or selection first)                     |
| `alt-→`               | Forward                                                       |
| `PgUp` `PgDn`         | Page up / down                                                |
| `Home` `End` (`g` `G`) | Top / bottom                                                 |
| `1`–`9`               | Tab by number                                                 |
| `Space` `?`           | Everything you can do here                                    |
| `/`                   | Search (on a list, the diff or a log: search that)            |
| `f`                   | Find a file                                                   |
| `b`                   | Switch branches or tags                                       |
| `c`                   | Comment (on a thread: reply)                                  |
| `s`                   | Star / unstar                                                 |
| `M`                   | Merge                                                         |
| `A`                   | Approve                                                       |
| `W`                   | Mark ready for review                                         |
| `B`                   | Update the branch from its base                               |
| `X`                   | Close or reopen; cancel a running workflow run                |
| `ctrl-r`              | Re-run the workflow run, its failed jobs, or the job          |
| `alt-↑` `alt-↓`       | Move a Home section up / down                                 |
| `Delete`              | Delete a Home section or a draft comment                      |
| `ctrl-z`              | Bring back what was just deleted                              |
| `n` `p`               | Next / previous search match in a job's log                   |
| `o`                   | Open in the browser                                           |
| `y`                   | Copy the link                                                 |
| `i`                   | Follow a link by its letters                                  |
| `I`                   | Follow a link by its letters, in the browser                  |
| `u`                   | Up a level                                                    |
| `T`                   | Open the selection in a new tab                               |
| `[` `]`               | Previous / next open tab                                      |
| `alt-1`–`alt-9`       | Open tab by number                                            |
| `ctrl-w`              | Close the tab                                                 |
| `H`                   | Home                                                          |
| `r`                   | Refresh                                                       |
| `:` `ctrl-k`          | Command palette                                               |
| `q` `ctrl-c`          | Quit                                                          |

Reviewing, in a pull request's Files changed:

| Keys        | Action                                                          |
| ----------- | --------------------------------------------------------------- |
| `n` `p`     | Next / previous change                                          |
| `N` `P`     | Next / previous unresolved thread                               |
| `⇧↓` `⇧↑`   | Next / previous file                                            |
| `⇧←` `⇧→`   | Split view: the old / new half of the row                       |
| `⇥`         | Switch between the file tree and the diff                       |
| `U`         | Next unviewed file                                              |
| `v`         | Mark the file viewed (synced with GitHub)                       |
| `m`         | Mark the change reviewed (local)                                |
| `c`         | Comment on the line or selection; on a thread, reply            |
| `C`         | Comment on the whole file                                       |
| `R`         | Resolve or unresolve the thread                                 |
| `x`         | Select lines                                                    |
| `e`         | Show more context                                               |
| `F`         | Show the whole file                                             |
| `S`         | Split or unified view                                           |
| `t`         | Show or hide the file tree                                      |
| `J`         | Jump to the other end of moved code                             |
| `w`         | Ignore whitespace changes                                       |
| `a`         | Submit your review                                              |
| `Delete`    | Delete the draft comment                                        |
| `ctrl-z`    | Bring back the draft just deleted                               |

In the comment editor, `ctrl-s` adds the comment, `ctrl-e` continues in
`$EDITOR`, and `Esc` cancels.

## Configuration

`~/.config/ghtui/config.toml`, or `--config <path>`. Everything is
optional, and an unknown key is an error, so typos don't go unnoticed.

```toml
[theme]
mode = "auto"         # auto | light | dark
seed = "#3f6fb5"      # the whole color scheme is generated from this
color_depth = "auto"  # auto | truecolor | 256

[ui]
nerd_font = false
density = "auto"      # auto | compact | comfortable

[keys]                # each action listed replaces its default keys
down = ["j", "<Down>", "<C-n>"]
sort = ["O"]

[[home]]              # a Home section: a title and a GitHub search
title = "Bugs in our repos"
issues = "is:open label:bug org:my-org sort:updated-desc"
rows = 10
```

A section lists `pulls`, `issues` or `repos`, and shows up to 30 rows; its
last row opens the rest. Without any `[[home]]` tables, Home shows your
review requests, your open pull requests and your repositories. Action
names for `[keys]` are in the `actions!` table in
[`crates/app/src/keymap.rs`](crates/app/src/keymap.rs).

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
with ETags, so unchanged data costs nothing against the rate limit.

**Colors from one seed.** Light and dark schemes are generated with
Material 3's tonal palettes. Every foreground and background pair the UI
uses is tested for contrast in both schemes, in truecolor and 256 colors.

| Crate    | What it does                                                    |
| -------- | --------------------------------------------------------------- |
| `app`    | The binary: routes, state, keys, view, runtime                  |
| `ui`     | Pages and widgets, pure functions of data                       |
| `api`    | GitHub over GraphQL (cynic) and REST: auth, retries, limits     |
| `schema` | GitHub's GraphQL schema, compiled once                          |
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
  (`INSTA_UPDATE=always cargo test -p ghtui` to accept changes, then review
  them);
- property tests throw random keys, resizes and replies at the app.

A contract suite checks ghtui's assumptions against live GitHub, read-only
(`cargo test -p ghtui-api --test contract -- --ignored contract`). Every
kind of github.com link is listed with what it must open in
`crates/app/tests/github_urls.txt`.

Timing tests (`cargo test --release -p ghtui -- --ignored --nocapture`)
hold scrolling a 500-file pull request to under 8ms a frame (about 0.5ms
today), and diffing a 50,000-line file to under 100ms (about 32ms).

## Known limitations

- github.com only.
- Long diff lines are cut at the pane's edge; there's no wrapping yet.
- Comments in diff threads show as plain text, not rendered Markdown.
- Markdown in the terminal: images show their alt text, and HTML is reduced
  to its text and links.
- The cache isn't kept per account. After switching accounts, the old
  account's pages show until they refresh.
- Merging doesn't offer auto-merge or merge queues.
