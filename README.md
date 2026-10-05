# ghtui

A keyboard-driven terminal client for GitHub, written in Rust. The goal is to
replace the GitHub website for daily use, built around a pull request diff
viewer that's better than GitHub's. The look is flat Material 3 adapted to
the terminal.

**Status: milestones M0–M4, plus browsing.** You can browse GitHub much as you
would on the website. That covers your home page (review requests, your pull
requests, your repositories), any repository, issue and pull request lists,
issues, pull requests, profiles, and search. Repository pages show files, stats
and the rendered README. You can also comment and star. Reviewing a pull
request's diff is where ghtui goes furthest. The
diff is computed locally from git and syntax-highlighted, unified or split,
with expandable context, a full-file mode, whitespace-insensitive comparison,
search, a fuzzy file finder, "viewed" synced with GitHub, and local per-change
"reviewed" marks. Review threads show inline. You can reply, resolve, write
single- and multi-line comments and suggested changes, and submit a review as
Comment, Approve or Request changes. Beyond what GitHub shows, ghtui
highlights changed tokens within lines, marks moved code, folds
formatting-only changes, shows only what changed since your last review
(even across force-pushes and rebases), and lets you view any commit or
commit range. M5 (structural diffs, code navigation, Actions, wikis and the
rest of GitHub) is planned but not started.

## Install

Requires Rust 1.97 or newer (the repo builds with stable, pinned in
`rust-toolchain.toml`; CI also checks the minimum) and git
2.36 or newer. Two optimizations need newer git and are skipped otherwise:
2.40+ reads `linguist-generated`/`linguist-vendored` attributes in the cache
clone, and 2.44+ (`GIT_NO_LAZY_FETCH`) lets the blob prefetch skip blobs you
already have.

```sh
cargo install --path crates/app
```

That installs the `ghtui` binary.

## Usage

```sh
ghtui                                   # home: review requests, your PRs and repositories
ghtui ratatui/ratatui                   # a repository
ghtui ratatui/ratatui#1820              # an issue or pull request
ghtui @octocat                          # a profile
ghtui https://github.com/o/r/tree/main  # most github.com URLs
ghtui pr ratatui/ratatui#1820           # open a pull request
ghtui pr https://github.com/o/r/pull/7  # PR URLs work too, including /files etc.
ghtui pr 1820                           # inside a clone: uses the `upstream` remote,
                                        # then `origin`, then any github.com remote
ghtui --theme light                     # override the color scheme
```

## Browsing

ghtui is laid out like the website:
- **The header** shows where you are (`owner / repo`), a search field ("Type /
  to search"), your review requests and your account.
- **Tabs** sit under the header, with the active tab underlined: Code, Issues,
  Pull requests and Actions on a repository; Conversation, Commits, Checks and
  Files changed on a pull request; Overview, Repositories and Stars on a
  profile.
- **The content** is in GitHub's boxes: the file list under the latest
  commit, the README, issue lists, and comments joined into a timeline. Wide
  terminals also get the sidebar (About, Assignees, Labels, Reviewers).
- **The status bar** names the keys that do something here.

**Moving.** Arrows move, `Enter` opens, `Esc` goes back, and each letter is
one verb that means the same thing on every screen. The status bar always
shows the keys that do something here, and `Space` (or `?`) lists everything you can
do.
- `↑`/`↓` (or `j`/`k`) move the selection. On lists that's row by row; in a
  conversation it's comment by comment, and a long comment scrolls before
  the next one is selected.
- `←`/`→` switch tabs (Code, Issues, Pull requests; Conversation, Commits,
  Files changed).
- `Enter` opens the selected row; on a comment it starts a quote reply.
- `Esc` (or `⌫`, or `alt-←`) goes back, `alt-→` goes forward; `u` goes up
  a level.
- `PgUp`/`PgDn` and `Home`/`End` scroll.
- The mouse works too: click a link, a row, a tab or the search field;
  scroll with the wheel; right-click for the actions menu.

**Verbs.**
- `/` searches: GitHub from anywhere, the list's filter on a list. It
  suggests pages you've visited, your repositories and live matches.
- `f` finds a file in the repository (GitHub's "Go to file"); `b` switches
  branches or tags.
- `c` comments (on a review thread, it replies); `s` stars.
- `o` opens in the browser; `y` copies the link.
- `l` puts letters on every link on screen; type them to follow one (`L`
  follows it in the browser).
- `h` goes home; `r` refreshes; `q` quits.
- Errors show in the status bar; `Esc` dismisses one, and "Recent messages
  and errors" in the `Space` menu lists them all (`Enter` copies one).
  Problems that last, such as a rejected token or drafts that can't be
  saved, stay in a banner above the status bar until they're fixed.
- `:` (or `ctrl-k`) is the command palette. It also takes `owner/repo`,
  `owner/repo#123`, `@user`, or a URL.

**The pages.**
- **Home** shows review requests, your open pull requests and your
  repositories.
- **Repository** has the Code tab:
  - A title with Star, Fork and Watch.
  - About.
  - The branch and "Go to file" buttons.
  - The file list.
  - The README rendered from Markdown, with relative links resolved as on
    GitHub.
- **Directories and files** open in place. Files are syntax-highlighted with
  line numbers, and `.md` files are rendered.
- **Issues and pull requests** lists are newest first, under a filter field,
  with Open/Closed counts and a sort (click them, or use the `Space` menu).
  Long lists end in "Load more".
- **Issue** shows the conversation as a timeline of comment boxes. Issue
  numbers that are pull requests redirect, as on GitHub.
- **Pull request**:
  - A sticky title.
  - Conversation, with a merge box for checks, reviews and conflicts.
  - Commits, grouped by day.
  - Files changed, which is the diff viewer below.
- **Profile** has its Overview (pinned repositories), Repositories and Stars.
- **Search** has tabs for repositories, issues, pull requests and users.
- **Writing**: `c` comments on the issue or pull request on screen, and the
  "Add a comment" box at the end does the same. `s` stars or unstars.

Links ghtui doesn't show itself (Actions, wikis, releases, external sites)
open in your browser. Pages show cached data first and refresh in the
background. Pages are at most 140 columns wide and centered.

## Authentication

ghtui looks for a token in this order:

1. `GH_TOKEN`
2. `GITHUB_TOKEN`
3. `gh auth token` (the [GitHub CLI](https://cli.github.com/)'s stored login)

If none of these works, ghtui exits and tells you to run `gh auth login`. The
token is held only in memory. It's never logged or written to disk, and its
`Debug` output is redacted.

**Git.** If you start ghtui inside a clone of the PR's repository (any remote
pointing at it counts, so fork clones work), diffs are computed there with
your own git credentials. ghtui only adds refs under `refs/ghtui/`; it never
touches your branches, HEAD, index or working tree. Otherwise it keeps a bare
partial clone (`--filter=blob:none`) per repository in the cache. That clone
authenticates with `gh auth git-credential` when the token came from `gh`.
Otherwise git runs the `ghtui` binary itself as `GIT_ASKPASS`, and it answers
from an environment variable that is set only on the git process. Either way,
the configuration goes through `GIT_CONFIG_COUNT` environment variables. The
token never appears on a command line, in a remote URL, or in a git config
file. Git never prompts: `GIT_TERMINAL_PROMPT=0`, and SSH runs in batch mode
unless you set your own `GIT_SSH_COMMAND`.

## Keys

Arrows move, `Enter` opens, `Esc` goes back, and every letter means the same
thing on every screen. A verb that doesn't apply where you are says so; it
never does something else. Less common actions have no key: they're in the
`Space` menu (and the `:` palette). All keys can be rebound (see
[Configuration](#configuration)).

| Keys                  | Action                                                        |
| --------------------- | ------------------------------------------------------------- |
| `↑` `↓` (`j` `k`)     | Move                                                          |
| `←` `→`               | Previous / next tab                                           |
| `Enter`               | Open the selection (on a comment: quote reply)                |
| `Esc` `⌫` `alt-←`     | Back (in the diff, an active search is cleared first)         |
| `alt-→`               | Forward                                                       |
| `PgUp` `PgDn`         | Page up / down                                                |
| `Home` `End` (`g` `G`) | Top / bottom                                                 |
| `1`–`4`               | Tab by number                                                 |
| `Space` `?`           | Everything you can do here, with its keys                     |
| `/`                   | Search (on a list: filter it; in the diff: search the diff)   |
| `f`                   | Find a file                                                   |
| `b`                   | Switch branches or tags                                       |
| `c`                   | Comment (on a review thread: reply)                           |
| `s`                   | Star / unstar                                                 |
| `o`                   | Open on GitHub in the browser                                 |
| `y`                   | Copy the link                                                 |
| `l`                   | Follow a link by its letters                                  |
| `L`                   | Follow a link by its letters, in the browser                  |
| `u`                   | Up a level (file → folder, PR → list, Files changed → PR)     |
| `h`                   | Home                                                          |
| `r`                   | Refresh                                                       |
| `:` `ctrl-k`          | Command palette                                               |
| `q` `ctrl-c`          | Quit                                                          |

Reviewing, in a pull request's Files changed tab:

| Keys        | Action                                                                       |
| ----------- | ---------------------------------------------------------------------------- |
| `n` `p`     | Next / previous change (after a search: next / previous match)               |
| `N` `P`     | Next / previous unresolved thread                                            |
| `⇧↓` `⇧↑`   | Next / previous file                                                         |
| `⇥`         | Switch between the file tree and the diff                                    |
| `U`         | Next unviewed file                                                           |
| `v`         | Mark the file viewed, synced with GitHub; viewed files collapse              |
| `m`         | Mark the change under the cursor reviewed (local, survives restarts)         |
| `c`         | Comment on the line or selection; on a thread, reply                         |
| `C`         | Comment on the whole file                                                    |
| `R`         | Resolve or unresolve the thread                                              |
| `x`         | Select lines (for multi-line comments)                                       |
| `e`         | Show more context                                                            |
| `F`         | Show the whole file                                                          |
| `S`         | Split or unified view                                                        |
| `t`         | Show or hide the file tree                                                   |
| `M`         | Jump to the other end of moved code                                          |
| `w`         | Ignore whitespace changes                                                    |
| `a`         | Submit your review (Comment / Approve / Request changes)                     |
| `Delete`    | Delete the draft comment                                                     |
| `ctrl-z`    | Bring back the draft just deleted                                            |

In the `Space` menu: changes since your last review, picking commits,
suggesting a change, and open/closed and sort on lists.

In the comment editor: `<C-s>` adds the comment to your review (or posts a
reply or a conversation comment), `<C-e>` continues in `$EDITOR`, and `Esc` cancels (press it twice if
there's text).

In the file tree, moving the selection scrolls the diff to that file;
`<Enter>` returns focus to the diff.

The command palette fuzzy-matches action names and descriptions. It also goes
places: `owner/repo`, `owner/repo#123`, `@user`, or a github.com URL. On a
repository's pages, a bare `#42` opens that issue or pull request. Anything
else can be searched on GitHub.

`y` copies with OSC 52, which most terminals support; in tmux, `set -g
set-clipboard on`.

## Configuration

`~/.config/ghtui/config.toml` (or `$XDG_CONFIG_HOME/ghtui/config.toml`, or
`--config <path>`). Every setting is optional, and unknown keys are reported as
errors so typos don't go unnoticed.

```toml
[theme]
mode = "auto"         # auto | light | dark
seed = "#3f6fb5"      # any #rrggbb; the whole scheme is generated from it
color_depth = "auto"  # auto | truecolor | 256

[ui]
nerd_font = false     # Nerd Font icons and rounded chip ends
density = "auto"      # auto | compact | comfortable: one row per list item,
                      # or two as on GitHub; auto is compact under 30 rows

[keys]
# Each action listed here replaces its default bindings. Vim notation:
# j, G, <C-d>, <Enter>, <Esc>, <Tab>, <S-Tab>, <BS>, <Up>, <lt> for "<".
# Sequences such as "gi" work too, and "page:" or "diff:" limits a binding
# to pages or the diff. Actions without a default key can be bound here.
down = ["j", "<Down>", "<C-n>"]
up = ["k", "<Up>", "<C-p>"]
sort = ["O"]
```

Action names are listed in the `actions!` table in
[`crates/app/src/keymap.rs`](crates/app/src/keymap.rs), with their default keys;
an unknown name in `[keys]` is an error that lists them all.

## Theming

- **One seed, two schemes.** Light and dark schemes come from the seed color
  using Material 3's HCT tonal palettes, via the
  [`material-colors`](https://crates.io/crates/material-colors) crate
  (TonalSpot variant). The diff tints (green and red harmonized toward the
  seed, tertiary for moved code) and the syntax colors come from the same
  palettes.
- **Light or dark.** With `mode = "auto"`, ghtui asks the terminal for its
  background (OSC 11) before entering the alternate screen. It waits at most
  100ms, and falls back to dark if the terminal doesn't answer.
- **Color depth.** Truecolor when `COLORTERM` is `truecolor` or `24bit`,
  otherwise 256 colors. In 256-color mode every color is quantized to the
  xterm palette (indices 16–255, so your terminal theme's 16 colors are never
  used). Lightness errors are weighted heavily so contrast holds. Where two
  surface tones become identical, ghtui draws outline-variant rules instead.
  Selection tints are strengthened until they're visible.
- **Contrast.** Views never use raw colors: they request
  `theme.style(fg_role, bg_role)`. Every pair the UI may use is declared in
  one table (`ghtui_theme::requirement`), and debug builds panic on an
  undeclared pair. A test checks every declared pair (all syntax roles on all
  diff backgrounds, with and without selection, in both schemes, at both
  color depths, for 31 seeds) against WCAG 4.5:1. When a pair fails, the
  foreground's tone moves; backgrounds never change. Disabled text is the
  only exempt role. GitHub label colors keep their hue and get a
  contrast-checked chip.

## Architecture

Cargo workspace (`crates/`), with an Elm-style app: input events and async
results become messages on a channel, a single `update` mutates state and
returns commands, and the view is a pure function of state. Network and git
work runs in tokio tasks, so the UI task never waits on them.

| Crate    | Role                                                                     |
| -------- | ------------------------------------------------------------------------ |
| `app`    | Binary: CLI, config, keymap, routes, state/update, view, runtime loop    |
| `ui`     | Widgets and pages (Markdown, repository, issue, PR, profile...); pure    |
| `theme`  | Scheme generation, semantic roles, quantization, contrast, detection     |
| `api`    | Auth, GraphQL (cynic) and REST over one reqwest client, retries, limits  |
| `schema` | GitHub's GraphQL schema compiled once (see below)                        |
| `store`  | redb cache (GraphQL results, REST bodies with ETags); review state files |
| `git`    | `git` CLI: repo selection, credentials, fetch, prefetch, `cat-file`      |
| `diff`   | Line hunks (imara-diff), tree-sitter highlighting, per-file diff model   |

Notes:

- **GraphQL** queries are typed against GitHub's public schema
  (`crates/schema/github.graphql`, from
  `https://docs.github.com/public/fpt/schema.docs.graphql`). The generated
  schema module lives in its own crate so query edits never recompile it.
- **HTTP**: a single reqwest client is used for both GraphQL and REST. It
  uses rustls with the platform's root certificates and the `ring` provider,
  which is installed explicitly as the process default. `scripts/check-tls-deps.sh`, which runs in CI, fails if
  `aws-lc-rs`, `openssl-sys` or `native-tls` ever enters the dependency tree.
  The client is built lazily on a blocking thread, because loading macOS root
  certificates takes over 100ms and would delay the first paint.
- **Retries**: transport errors and 5xx responses are retried up to 3 times
  with backoff. Mutations are the exception: they're retried only when the
  connection was refused, since a 5xx or timeout may mean GitHub already
  applied them. On a 403/429 rate limit, ghtui waits out a `Retry-After` of
  up to 10s inline; longer waits are reported. Rate-limit headers (REST and
  GraphQL) feed the `API remaining/limit` indicator in the status bar.
- **Caching**: the first paint comes from the cache, then data is
  revalidated in the background. GraphQL results are cached by key, and REST
  GETs revalidate with `If-None-Match`, so a 304 serves the cached body and
  doesn't count against the rate limit. The on-disk format is versioned: a
  mismatched or corrupt cache is deleted and rebuilt. If another ghtui
  process holds the cache, this one runs without a cache instead of failing.
- **Files**: cache at `~/Library/Caches/ghtui/cache.redb` on macOS
  (`$XDG_CACHE_HOME/ghtui` on Linux); log at `ghtui.log` next to it (level
  via `GHTUI_LOG`, e.g. `GHTUI_LOG=debug`). Review drafts and reviewed marks
  live outside the cache, one JSON file per PR under
  `~/Library/Application Support/ghtui/reviews` (`$XDG_DATA_HOME/ghtui/reviews`
  on Linux), so deleting the cache never loses them. If they can't be
  written, a banner says so until they can. Nothing is written to the
  terminal while the TUI runs. A panic in the UI restores the terminal before
  printing; a panic in background work is logged and reported in the status
  bar, and the session carries on.

## How a diff is built

1. **Repository.** Your clone if you're in one, otherwise the cache's bare
   partial clone (made on first use; an interrupted clone leaves nothing
   behind). Git operations on the same repository are serialized.
2. **Fetch.** `refs/pull/<N>/head` and the base branch are fetched into
   `refs/ghtui/pr/<N>/{head,base}`. The diff runs from
   `merge-base(base, head)` to `head`, matching GitHub's three-dot
   comparison. The head is pinned as `refs/ghtui/pr/<N>/seen/<sha>` so later
   force-pushes and `git gc` can't remove it.
3. **File list.** `git diff --raw -z -M` gives statuses, modes, object IDs
   and renames. Generated, vendored and lockfiles are collapsed.
4. **Prefetch.** In a partial clone, the first file's blobs are fetched in one
   request, then all remaining blobs in a second. Workers wait for the batch
   that covers their file, so git never falls back to fetching blobs one at a
   time. The file tree is on screen before any of this finishes.
5. **Diff.** Blobs are read through one long-lived `git cat-file --batch`
   process. They are diffed with imara-diff (histogram, 3 lines of context;
   hunk headers are tested against `git diff` output) and highlighted with
   tree-sitter. Per-file work is done by a small worker pool, starting with
   the files on screen.

6. **View.** Each file keeps its full line alignment, both exact and
   whitespace-insensitive. Rows are rebuilt from it when you switch between
   split and unified, toggle whitespace, expand context or show the full file.
   An anchor keeps the cursor on the same source line through every change.

**Viewed and reviewed.** "Viewed" is GitHub's per-file state, read with
`viewerViewedState` and changed with `markFileAsViewed` /
`unmarkFileAsViewed`. A toggle shows immediately and is rolled back with an
error if GitHub refuses it. A file that changed after you viewed it shows
"Changed since viewed". "Reviewed" marks are local, per change block (a
maximal run of changed lines). They're stored in the cache, keyed by a hash of
the file path and the block's changed lines, so a mark follows its change when
line numbers shift and clears when the change is edited.

## Beyond GitHub's diff

- **Changed tokens within lines** get a stronger tint. Lines are split at
  word and syntax-token boundaries and diffed token by token. A removed line
  is paired with the most similar added line in its block (character-bigram
  similarity, in order), so an edit finds its counterpart even when lines
  were added or removed around it. Lines that mostly changed get no emphasis.
- **Moved code** (like `git diff --color-moved`): a run of at least 3
  removed lines (30+ characters) that reappears as added lines, in the same
  file or another, ignoring indentation. It's shown in the tertiary tint, with
  "moved to/from path:line" and `M` to jump between the two ends.
- **Formatting-only changes** (removed and added text identical once
  whitespace and line breaks are ignored) fold into one row. `<Enter>` shows
  them.
- **Changes since your last review** (in the `Space` menu). The reviewed head comes from
  your latest submitted review on GitHub, or ghtui's own record. ghtui
  rebuilds the PR's diff as it was at that head (fetching the commit by SHA
  if it was force-pushed away) and compares change blocks by content hash.
  A change you already saw stays hidden however lines shifted, and changes
  that came from rebasing onto a newer base were never in either diff, so
  they don't show up. Seen blocks next to new ones fold into one row; files
  with nothing new say so.
- **Commits** (in the `Space` menu): view one commit, or mark a start with Space and pick
  an end for a range. Comments need the whole PR (their line numbers anchor
  there), so commenting is off in a range view.

## Reviewing

- **Anchoring.** Comments are anchored by file path, line number and side
  (plus start line and side for ranges) on the head commit, never by
  position on screen. GitHub only accepts line comments inside the hunks of
  *its* diff, which can differ from ghtui's. Commentable ranges therefore
  come from GitHub's per-file `patch` (REST `pulls/{n}/files`). When GitHub
  omits the patch (large or binary files, or past its file cap), ghtui
  reconstructs GitHub's view with a Myers diff and 3 lines of context. Line
  numbers outside those ranges are dimmed.
- **Where comments go.** Commenting on a line GitHub won't accept offers a
  file-level comment instead, and says why. A selection that crosses two of
  GitHub's hunks is refused with an explanation.
- **Pending comments** are kept locally (outside the cache, so a crash, a
  quit or clearing the cache doesn't lose them) until you submit. On submit, ghtui reuses your pending
  review on GitHub (or starts one on the head commit), adds each comment as
  its own thread, and submits only if GitHub accepted every comment. A
  rejected comment keeps its text, shows GitHub's reason, and can be turned
  into a file comment with `C`. Accepted ones stay in GitHub's pending review
  until the next submit.
- **Threads.** Threads show inline, with resolved and outdated state. Open
  threads are expanded and resolved ones collapse to a gutter marker (◆ open,
  ◇ resolved, ✎ draft). Lines with threads are always shown, even outside the
  normal context. File-level comments sit under the file header.
- **Outdated threads.** GitHub has no current line for these. ghtui reads the
  file at the thread's original commit (fetching the commit by SHA if it was
  force-pushed away), diffs it against the head, and moves the thread to its
  line if that line survived unchanged. Otherwise the thread stays outdated
  under the file header. Left-side (deleted-line) threads aren't mapped.
- **Suggestions** open the selected lines in `$EDITOR`. The result becomes a
  ```` ```suggestion ```` block, with a preview of the change before you add
  it.
- **The editor.** `$VISUAL` or `$EDITOR` (default `vi`) runs through `sh`, so
  it may include arguments. ghtui stops reading the keyboard, leaves the
  alternate screen and raw mode, waits for the editor, then restores the
  screen and redraws fully.

**Bundled grammars** (each behind a `lang-*` cargo feature of `ghtui-diff`):
Rust, TypeScript/TSX, JavaScript/JSX, Python, Go, JSON, YAML, TOML, Markdown
and shell. Other files, and files over 1 MiB, are shown as plain text. Binary
files (a NUL byte in the first 8000 bytes, like git), submodules, files over
16 MiB, mode changes, symlinks, CRLF changes (marked `␍`) and missing final
newlines are all shown explicitly.

## Development

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
scripts/check-tls-deps.sh
```

UI snapshot tests (`crates/app/src/snapshots/`) render full screens through
ratatui's `TestBackend`, in light, dark and 256-color modes. They include cell
styles, so color changes appear in review. To accept intentional changes:
`INSTA_UPDATE=always cargo test -p ghtui`, then review the diff.

The API client is tested against a scripted local HTTP server
(`crates/api/tests/client.rs`): ETag revalidation, retries, rate limits, and
error mapping. Git operations are tested against throwaway repositories
served over `file://` with filtering enabled (`crates/git/tests/repo.rs`). Those
tests cover partial clones, prefetch, lazy fetch, every file status, and a
check that your clone only gains `refs/ghtui/*`. Neither suite needs network
access.

Panics are kept out by construction: workspace lints reject `unwrap`,
`expect`, indexing, string slicing, truncating casts and ratatui's unclipped
buffer writes outside tests. Property tests (proptest) check that arbitrary
Markdown and diffs lay out, render at any size and stay in bounds, that every
page's URL leads back to it, and that key notation round-trips; a test renders
every screen at every size up to 12×12. Fuzz targets for the Markdown renderer
and the diff pipeline live in `fuzz/` (`cargo +nightly fuzz run markdown`,
`cargo +nightly fuzz run file_diff`; needs `cargo-fuzz`).

Performance targets, as timing tests
(`cargo test --release -p ghtui -- --ignored --nocapture`):

| Target                                             | Budget  | Measured |
| -------------------------------------------------- | ------- | -------- |
| Scroll a 500-file PR with 20k+ changed lines       | 8ms     | ~0.5ms per frame |
| 50k-line file: diff + first screen                 | 100ms   | ~32ms (31ms diff on a worker, <1ms render) |
| 50k-line file: toggle full-file view               | 100ms   | <1ms     |
| First paint from cache (measured in the log)       | 200ms   | ~85ms    |

## Crate substitutions

| Planned           | Used                       | Why                                                                                                                                                                           |
| ----------------- | -------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `tui-textarea`    | `ratatui-textarea` 0.9     | `tui-textarea` 0.7 depends on ratatui 0.29; `ratatui-textarea` is its maintained fork for ratatui 0.30                                                                        |
| OSC 11 by hand    | `terminal-colorsaurus` 1.0 | Handles the query with a timeout, detects terminals that can't answer (DA1), and avoids GNU Screen's broken replies                                                           |
| `octocrab`        | `reqwest` 0.13 (`rustls-no-provider`) | ghtui used octocrab only as an HTTP transport, with its retries off, and it brought a JWT backend, chrono and tower along; reqwest does the job with 60 fewer crates |
| directories       | `etcetera`                 | XDG-style config dir on macOS (`~/.config`), native cache dir (`~/Library/Caches`)                                                                                            |

## Known limitations

- github.com only; GitHub Enterprise Server isn't supported yet.
- The home page shows the 25 most recently updated PRs per section, with
  GitHub's total count (the "see all" links list them all). Larger pages that include check status make GitHub's search
  time out (HTTP 502) for busy accounts.
- The cache isn't separated per GitHub account. After switching accounts, the
  previous account's cached pages show until the first refresh completes.
- Actions, wikis, discussions, releases, notifications and code search
  aren't pages yet; their links open in the browser.
- Profiles list the 30 most recently pushed (and starred) repositories;
  profile READMEs and the contribution graph aren't shown.
- "Go to file" lists what GitHub's tree API returns in one request; for very
  large repositories GitHub cuts it short, and ghtui says so.
- Live search suggestions use GitHub's search, which can take a second or two.
- Markdown is rendered for the terminal: images show their alt text (badges
  without it are left out), and HTML is reduced to its text, links and
  structure.
- Relative times on pages ("3h ago") update when the page's data does.
- Long diff lines are cut at the pane edge (marked `…`); there's no
  horizontal scrolling or wrapping yet.
- Comments in diff threads are shown as plain wrapped Markdown, not rendered.
- Suggestions apply to new-side lines only (as on GitHub). Threads on
  deleted lines aren't mapped forward when outdated.
- Replies, resolve and submit go to GitHub immediately; there's no undo
  beyond GitHub's own (edit or delete on the web).
- Search highlights the matching line (the cursor moves to it), not the
  matched characters.
- Expansion windows reset when you toggle whitespace mode, because they
  index the alignment that changed.
- Moved code is detected on the exact comparison only, not while ignoring
  whitespace.
- "Since your last review" compares change blocks. If a block you reviewed
  was later edited, the whole block counts as new, not just the edited line.
- Markdown highlighting covers block structure only (no inline emphasis or
  code fences).
- First paint from cache is about 85ms on macOS, measured in tmux. About 40–80ms of that is
  `gh auth token`, which has to finish before the TUI starts in case it
  fails. Setting `GH_TOKEN` avoids running `gh`.
- A cold `cargo clippy` takes about 8½ minutes, almost all of it linting the
  generated GitHub schema module. A cold `cargo build` takes about 1 minute,
  and incremental builds 1–2s. CI caches build output with `rust-cache`.
