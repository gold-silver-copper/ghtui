# ghtui

A keyboard-driven terminal client for GitHub, written in Rust. The goal is to
replace the GitHub website for daily use, built around a pull request diff
viewer that's better than GitHub's. The look is flat Material 3 adapted to
the terminal.

**Status: milestones M0–M2.** You can browse your open pull requests and
review requests, open any pull request's overview, and review its diff. The
diff is computed locally from git and syntax-highlighted, unified or split,
with expandable context, a full-file mode, whitespace-insensitive comparison,
search, a fuzzy file finder, "viewed" synced with GitHub, and local per-change
"reviewed" marks. Next: review threads and comments (M3).

## Install

Requires Rust stable (the repo pins it in `rust-toolchain.toml`) and git
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
ghtui                                   # inbox: review requests and your open PRs
ghtui pr ratatui/ratatui#1820           # open a pull request
ghtui pr https://github.com/o/r/pull/7  # PR URLs work too, including /files etc.
ghtui pr 1820                           # inside a clone: uses the `upstream` remote,
                                        # then `origin`, then any github.com remote
ghtui --theme light                     # override the color scheme
```

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

All keys can be rebound (see [Configuration](#configuration)). Press `?` in the
app for this list, generated from your actual keymap.

| Keys            | Action                                                                        |
| --------------- | ----------------------------------------------------------------------------- |
| `j` `<Down>`    | Move down                                                                     |
| `k` `<Up>`      | Move up                                                                       |
| `<C-d>` `<C-u>` | Half page down / up                                                           |
| `gg` `G`        | Go to top / bottom                                                            |
| `<Enter>`       | Open: the selected PR, a PR's diff, a file from the tree, a collapsed file    |
| `<Esc>` `<BS>`  | Back                                                                          |
| `q`             | Close the view (quits from inbox)                                             |
| `<C-c>`         | Quit                                                                          |
| `r`             | Refresh (on the diff: fetch and recompute)                                    |
| `o`             | Open on GitHub in the browser                                                 |
| `:`             | Command palette                                                               |
| `?`             | Keyboard shortcuts                                                            |
| `]h` `[h`       | Next / previous hunk                                                          |
| `]f` `[f`       | Next / previous file                                                          |
| `]u`            | Next file not marked viewed                                                   |
| `<Tab>`         | Show or hide the file tree                                                    |
| `<C-w>`         | Switch focus between the file tree and the diff                               |
| `s`             | Split or unified (automatic: split when the diff pane is 160+ columns wide)   |
| `w`             | Ignore whitespace changes (like `git diff -w`)                                |
| `x`             | Show 20 more lines of context (on a `⋯` gap: open it; in a hunk: widen it)    |
| `F`             | Show the whole file, changes marked                                           |
| `v`             | Mark the file viewed / unviewed, synced with GitHub; viewed files collapse    |
| `m`             | Mark the change under the cursor reviewed (local, survives restarts)          |
| `/` `n` `N`     | Search the diff (smart case), next / previous match                           |
| `gf`            | Find a file by fuzzy path                                                     |

In the file tree, moving the selection scrolls the diff to that file;
`<Enter>` returns focus to the diff.

The command palette fuzzy-matches action names and descriptions. Type
`owner/repo#123` or a PR URL to open a pull request. On a PR screen, a bare
number like `42` opens that PR in the same repository.

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

[keys]
# Each action listed here replaces its default bindings. Vim notation:
# j, G, gg, ]h, <C-d>, <Enter>, <Esc>, <Tab>, <S-Tab>, <BS>, <Up>, <lt> for "<".
down = ["j", "<Down>", "<C-n>"]
up = ["k", "<Up>", "<C-p>"]
```

Action names: `down`, `up`, `half_page_down`, `half_page_up`, `top`,
`bottom`, `open`, `back`, `close`, `quit`, `refresh`, `open_in_browser`,
`command_palette`, `help`, `next_hunk`, `prev_hunk`, `next_file`,
`prev_file`, `toggle_tree`, `switch_pane`, `toggle_split`,
`ignore_whitespace`, `expand_context`, `full_file`, `toggle_viewed`,
`next_unviewed`, `mark_reviewed`, `search`, `search_next`, `search_prev`,
`find_file`. A binding that duplicates another, or is a prefix
of another (`g` next to `gg`), is rejected at startup.

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
| `app`    | Binary: CLI, config, keymap, state/update, view, runtime loop            |
| `ui`     | Ratatui widgets (bars, chips, PR list, PR overview, overlays); pure      |
| `theme`  | Scheme generation, semantic roles, quantization, contrast, detection     |
| `api`    | Auth, GraphQL (cynic) and REST over one octocrab client, retries, limits |
| `schema` | GitHub's GraphQL schema compiled once (see below)                        |
| `store`  | redb cache: GraphQL results, REST bodies with ETags, review state        |
| `git`    | `git` CLI: repo selection, credentials, fetch, prefetch, `cat-file`      |
| `diff`   | Line hunks (imara-diff), tree-sitter highlighting, per-file diff model   |

Notes:

- **GraphQL** queries are typed against GitHub's public schema
  (`crates/schema/github.graphql`, from
  `https://docs.github.com/public/fpt/schema.docs.graphql`). The generated
  schema module lives in its own crate so query edits never recompile it.
- **HTTP**: a single octocrab client is used for both GraphQL and REST. It
  uses rustls with the `ring` provider, which is installed explicitly as the
  process default. `scripts/check-tls-deps.sh`, which runs in CI, fails if
  `aws-lc-rs`, `openssl-sys` or `native-tls` ever enters the dependency tree.
  The client is built lazily on a blocking thread, because loading macOS root
  certificates takes over 100ms and would delay the first paint.
- **Retries**: transport errors and 5xx responses are retried up to 3 times
  with backoff. On a 403/429 rate limit, ghtui waits out a `Retry-After` of
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
  via `GHTUI_LOG`, e.g. `GHTUI_LOG=debug`). Nothing is written to the
  terminal while the TUI runs. Panics restore the terminal before printing.

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
| `octocrab.graphql()` | octocrab's raw `_post`  | The convenience method hides response headers (needed for rate limits) and turns partial GraphQL results into errors                                                         |
| octocrab defaults | `rustls-ring` + `jwt-rust-crypto`, no default features | Keeps `aws-lc-rs` out. Octocrab requires a JWT backend even when it isn't used                                                                         |
| directories       | `etcetera`                 | XDG-style config dir on macOS (`~/.config`), native cache dir (`~/Library/Caches`)                                                                                            |

## Known limitations

- github.com only; GitHub Enterprise Server isn't supported yet.
- The inbox shows the 25 most recently updated PRs per section, with GitHub's
  total count. Larger pages that include check status make GitHub's search
  time out (HTTP 502) for busy accounts.
- The cache isn't separated per GitHub account. After switching accounts, the
  previous account's cached inbox shows until the first refresh completes.
- PR descriptions are shown as wrapped plain text; Markdown isn't rendered.
- Long diff lines are cut at the pane edge (marked `…`); there's no
  horizontal scrolling or wrapping yet.
- `]c` / `[c` (next/previous unresolved thread) arrive with review threads in
  M3.
- Search highlights the matching line (the cursor moves to it), not the
  matched characters.
- Expansion windows reset when you toggle whitespace mode, because they
  index the alignment that changed.
- Markdown highlighting covers block structure only (no inline emphasis or
  code fences).
- First paint from cache is about 85ms on macOS, measured in tmux. About 40–80ms of that is
  `gh auth token`, which has to finish before the TUI starts in case it
  fails. Setting `GH_TOKEN` avoids running `gh`.
- A cold `cargo clippy` takes about 8½ minutes, almost all of it linting the
  generated GitHub schema module. A cold `cargo build` takes about 1 minute,
  and incremental builds 1–2s. CI caches build output with `rust-cache`.
- No reviews or comments yet (M3).
