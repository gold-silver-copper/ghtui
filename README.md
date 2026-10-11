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

Comments are kept on disk until you submit, so a crash never loses them.
Threads that GitHub calls outdated are moved to their line when it still
exists.

**Acting on GitHub** asks first wherever it matters (merging, closing,
cancelling, updating a branch). It asks GitHub what you may do and says
why when you can't. After a change, the page follows GitHub until it shows
it.

## Keys

The keys are vim's where vim has one. A binding is a key sequence (`gg`,
`]c`), and digits before it are a count: `5j` moves five rows, `3gt` goes
to the page's third tab, `120gg` to line 120 of a file. After the first key
of a sequence, the status bar lists what can follow it.

What a key does depends on where you are: the innermost of the contexts
below that binds it wins. A list adds its keys to a page's, which add
theirs to the ones that work everywhere; in Files changed, the file tree
has the focus or the diff does. A key that does nothing here says so, and
says where it works.

- `esc` only cancels: a search, a selection, an overlay or keys typed so
  far. It never leaves the page. Back is `ctrl-o` (or `⌫`), forward is
  `ctrl-i`, which terminals send as `⇥`.
- A page's own tabs (a pull request's Conversation, Commits, Checks and
  Files changed) aren't history: `gt` and `gT` switch them in place, and
  each keeps where you were.
- `/` searches what's on screen: the page, a file, the diff or a log, and
  `n` and `N` go through the matches. On a list it filters the list
  instead. `g/` searches GitHub, as does `:`.
- In a file, `j` and `k` move line by line, `y` copies the line's link,
  `V` selects lines (then `y` copies `#L10-L20`), and `gb` blames the
  file at that line. In the blame, a run of lines opens its commit.
- `h` and `l` scroll code sideways in files, logs and the diff; in the
  diff's file tree they fold and unfold folders.

**Everywhere** (`global:`)

| Keys                 | Action |
| -------------------- | ------ |
| `j` `↓`              | Move down |
| `k` `↑`              | Move up |
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
| `B`     | Switch branches or tags |
| `i`     | Follow a link by its letters |
| `I`     | Follow a link, in the browser |
| `alt-↑` | Move the Home section up |
| `alt-↓` | Move the Home section down |

**On lists** (`list:`)

| Keys | Action |
| ---- | ------ |
| `t`  | Open / closed / all |
| `S`  | Change the sort |

**In files** (`file:`)

| Keys | Action |
| ---- | ------ |
| `gb` | Blame the file at this line, or back to the file |
| `h`  | Scroll left |
| `l`  | Scroll right |
| `w`  | Next word (for * and gx) |
| `b`  | Previous word |
| `V`  | Select lines |

**In job logs** (`log:`)

| Keys | Action |
| ---- | ------ |
| `h`  | Scroll left |
| `l`  | Scroll right |
| `w`  | Next word (for * and gx) |
| `b`  | Previous word |

**In Files changed** (`diff:`)

| Keys           | Action |
| -------------- | ------ |
| `h`            | Scroll left |
| `l`            | Scroll right |
| `w`            | Next word (for * and gx) |
| `b`            | Previous word |
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
| `v`            | Mark the file viewed (syncs with GitHub) |
| `m`            | Mark the change reviewed |
| `V` `x`        | Select lines |
| `C`            | Comment on the whole file |
| `R`            | Resolve or unresolve the thread |
| `,s`           | Submit your review |

**In Files changed's file tree** (`tree:`)

| Keys | Action |
| ---- | ------ |
| `]]` | Next section (a box, a Home section, a folder) |
| `[[` | Previous section |
| `h`  | Fold the folder (on a file, go to its folder) |
| `l`  | Unfold the folder |

The comment editor opens where the comment will go: under the lines, the
thread or the file header in Files changed, and in place of the comment
box on a conversation. In it, `ctrl-s` adds the comment, `ctrl-e`
continues in `$EDITOR`, and `esc` cancels (keeping the lines selected).

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
sort = ["list:O"]     # a context first binds it there only
go_home = ["gh", "<A-h>"]   # sequences or modified keys

[[home]]              # a Home section: a title and a GitHub search
title = "Bugs in our repos"
issues = "is:open label:bug org:my-org sort:updated-desc"
rows = 10
```

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
with ETags, so unchanged data costs nothing against the rate limit.

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

- github.com only.
- Long code lines are cut at the edge: `h` and `l` scroll them sideways,
  but they don't wrap.
- Comments in diff threads show as plain text, not rendered Markdown.
- Markdown in the terminal: images show their alt text, and HTML is reduced
  to its text and links.
- The cache isn't kept per account. After switching accounts, the old
  account's pages show until they refresh.
- Merging doesn't offer auto-merge or merge queues.
