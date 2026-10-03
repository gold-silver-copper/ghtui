Build `ghtui`: a keyboard-driven terminal client for GitHub, written in Rust, meant to eventually replace the GitHub website for daily use. The centerpiece is a pull request diff viewer that is better than GitHub's. It should have a clean, flat Material 3 aesthetic adapted to the terminal. Create the project at ~/Desktop/code/ghtui.

## Stack
- Rust (stable), Cargo workspace, tokio.
- TUI: ratatui + crossterm. tui-textarea for short inline input; $EDITOR for long-form text (comments, PR bodies, suggestions). Handing off to $EDITOR must leave the alternate screen and raw mode, wait for the editor to exit, then restore both and force a full redraw. The same restore path runs on panic and on error exit.
- GitHub API: GraphQL via `cynic` (typed against GitHub's public schema) for most reads; REST via `octocrab` for notifications, Actions, per-file PR patches, and conditional (ETag) requests. GitHub's schema is large: register it once in a dedicated schema crate (following cynic's guidance for large schemas) so query changes don't recompile the schema, and keep an eye on build times.
- Auth: use `GH_TOKEN` or `GITHUB_TOKEN` if set; otherwise run `gh auth token`. If neither yields a token, exit with a clear message telling me to run `gh auth login`. Never log or persist the token.
- TLS: rustls with the `ring` crypto provider (not `aws-lc-rs`, not native-tls/OpenSSL). Configure every HTTP client (octocrab, any reqwest use) accordingly, and install the `ring` provider explicitly as the process default at startup. Transitive dependencies often pull in `aws-lc-rs` or OpenSSL anyway: add a CI check that `cargo tree -i aws-lc-rs` and `cargo tree -i openssl-sys` find nothing.
- Cache: `redb` (pure-Rust embedded key-value store) under the platform cache dir (e.g. ~/Library/Caches/ghtui on macOS). Store ETags and honor 304s. Version the on-disk schema; on a version mismatch, discard the cache rather than migrating.
- Diffing: `imara-diff` (histogram by default, Myers available). Syntax highlighting and tokenization: tree-sitter. Bundle an initial grammar set (Rust, TypeScript/TSX, JavaScript, Python, Go, JSON, YAML, TOML, Markdown, shell) with plain-text fallback for everything else; keep grammars behind cargo features so the set can grow without bloating every build.
- Git: use the `git` CLI for all network operations and for reading blobs (a long-lived `git cat-file --batch` process) so partial-clone fetching works. `gix` may be used for read-only object/ref access where it's verified to work with partial clones. Check the installed git version at startup and state the minimum required version in the README.
- Before using any crate, check its current version and API rather than assuming; note any substitutions in the README.

## Workspace layout
- `api`: GraphQL + REST clients, auth, rate-limit tracking (GraphQL points and REST headers), retries.
- `schema`: the registered GitHub GraphQL schema for cynic (nothing else, so it rarely rebuilds).
- `store`: redb cache, ETags, review-state persistence (last-reviewed head SHA per PR, hunk review marks).
- `git`: repo cache management, credentials, fetching PR refs, blob prefetch, merge-base, file lists with rename detection, blob reading.
- `diff`: diff computation, intra-line token diff, moved-code detection, comment anchoring. Pure logic, no UI, no I/O, heavily unit tested.
- `theme`: Material 3 color scheme generation, semantic style roles, color-depth quantization, contrast checks.
- `ui`: ratatui views and widgets.
- `app`: binary; state, message loop, keymap, command palette.

Use an Elm-style architecture: input events and async results become messages sent over a channel; a single update function mutates state; the view is a pure function of state. All network and git work runs in background tokio tasks; the UI thread never blocks.

## Local git model (non-negotiable: diffs are computed locally and GitHub's patches are never rendered)
GitHub's per-file `patch` is used for exactly one thing: deciding which lines GitHub will accept comments on (see Comment anchoring). Everything shown on screen comes from local git objects.

Repository selection:
- If the current directory is a clone with any remote (not just `origin`) pointing at the PR's base repo, use it. This covers fork clones where `origin` is the fork and `upstream` is the base. In that repo, ghtui only writes refs under `refs/ghtui/` and never touches branches, HEAD, the index, or the working tree.
- Otherwise maintain a bare partial clone at <cache>/repos/<owner>/<repo>.git (`git clone --bare --filter=blob:none`). Serialize git operations per repo, since concurrent fetches into one repo conflict.

Credentials for the cache clone (private repos must work):
- Supply the token to git via a credential helper: `gh auth git-credential` when gh is available, otherwise a `GIT_ASKPASS` helper that reads the token from ghtui's environment. Pass config through `GIT_CONFIG_COUNT`/`GIT_CONFIG_KEY_n`/`GIT_CONFIG_VALUE_n` env vars. Never put the token on a command line (visible in `ps`), in a remote URL, or in any git config file on disk. Set `GIT_TERMINAL_PROMPT=0` so git never blocks on a prompt behind the TUI.
- In the user's own clone, use whatever credentials that clone already has.

Fetching:
- Fetch `+refs/pull/<N>/head:refs/ghtui/pr/<N>/head` plus the base branch. Diff = merge-base(base, head) → head, matching GitHub's three-dot semantics.
- File list with rename detection from `git diff -z --name-status -M <mergebase> <head>`.
- Blob prefetch (critical for performance): in a partial clone, every missing blob read through `cat-file --batch` triggers its own network round trip, so a 500-file PR would take ~1000 sequential fetches. Once the file list is known, collect the old and new blob OIDs for all changed files and fetch the missing ones in a single batched request (newer git versions provide `git backfill`; otherwise one `git fetch` for those object IDs). Show the file tree immediately, prefetch in the background, and compute per-file diffs with imara-diff as blobs arrive, with the currently viewed file first. Rename detection also needs blob contents; make sure it doesn't fall back to one-at-a-time lazy fetches.
- Handle: renames (with similarity), copies, mode changes, binary files, submodules, symlinks, deleted/added files, huge files (render progressively, never block).

Keeping old heads available:
- Whenever I submit a review, and whenever a PR is opened, pin the current head as `refs/ghtui/pr/<N>/seen/<sha>` so that later force-pushes and `git gc` can't remove the commit "changes since my last review" needs. Prune pins for PRs that are closed or merged and older than a configurable age.
- If a needed old SHA is missing locally (for example, reviewed on another machine), try to fetch it by SHA. If GitHub no longer serves it, say so inline and fall back to the full PR diff.

## Diff viewer requirements (the core of v1)
Rendering:
- Unified and split views; auto-pick by terminal width, toggle with a key.
- Syntax highlighting via tree-sitter, cached per blob OID.
- Intra-line highlighting at token granularity (tree-sitter tokens, falling back to words).
- Expand context incrementally around any hunk, or toggle full-file view with changes marked.
- Virtualized rendering: only visible lines are laid out. Smooth scrolling on a 500-file / 20k-line PR (see Quality bar for the measurable target).
- Gutter: old/new line numbers, comment-thread markers, viewed/reviewed state.

Navigation (vim-style, all rebindable via a TOML config):
- j/k lines, ctrl-d/ctrl-u half pages, ]h/[h next/prev hunk, ]f/[f next/prev file, ]u next unviewed file, ]c/[c next/prev unresolved thread, gf fuzzy file finder, / search within diff, tab toggles file tree pane, s split/unified, w ignore whitespace, x expand context, F full file, v toggle viewed, V visual line selection, o open current location in browser, : command palette, ? help overlay generated from the keymap.

File tree:
- Directory tree with per-file +/- counts and viewed state.
- Collapse generated/vendored files and lockfiles by default (`linguist-generated` / `linguist-vendored` in .gitattributes, plus common lockfile names).

Review state:
- Sync per-file "viewed" with GitHub (GraphQL `markFileAsViewed` / `unmarkFileAsViewed`, read `viewerViewedState`). Apply changes optimistically and roll back with an inline error if the mutation fails.
- Local per-hunk "reviewed" marks persisted in redb, keyed by path + a hash of the hunk's normalized content (not line numbers), so a mark survives the hunk moving and clears when its content changes.

Comments and reviews:
- Show existing review threads inline (collapsed markers in the gutter, expand in place), including resolved/outdated state; reply, resolve/unresolve.
- New comments: single-line and multi-line (visual line selection), written in tui-textarea or $EDITOR. Suggested changes edited in $EDITOR with a preview of the applied result.
- Pending comments accumulate into one review; submit as Comment / Approve / Request changes with a summary body. Persist unsent pending comments in redb so a crash or quit doesn't lose them.
- COMMENT ANCHORING (critical):
  - Anchor every comment by path + line + side (+ start_line/start_side for ranges) + head commit_id, using real file line numbers, never rendered-row positions.
  - GitHub only accepts line comments inside its own diff hunks, which may differ from ours (different algorithm and context). Get commentable ranges from GitHub's per-file `patch` (REST pulls/files). That field is missing for large or binary files and the endpoint is paginated and capped, so the fallback is common, not rare: a local Myers diff with 3 lines of context.
  - A multi-line range must start and end in the same GitHub hunk. Clamp or reject visual selections that cross hunk boundaries, and explain why.
  - Visibly mark non-commentable lines. Offer a file-level comment (`subject_type: file`) when a line isn't commentable or the API rejects the anchor, keeping the drafted text.
  - Map outdated comments forward to current lines where possible, using the comment's original commit and line and a diff from that commit to the current head; leave them marked outdated when the mapping is ambiguous.

## Visual design: flat Material (Material 3 adapted to the terminal)
Principles:
- Flat surfaces, not boxes. Separate regions with background tone (Material 3 surface container levels: surface, surface-container-low, surface-container, surface-container-high), not box-drawing borders. Borders only where tone alone is ambiguous, and then a single subtle outline-variant line.
- Elevation = tone. Popups, the command palette, and dialogs sit on a higher surface-container tone with 1-cell vertical / 2-cell horizontal padding. No shadows, no double lines.
- Spacing on a grid: 1 cell vertical, 2 cells horizontal as the base unit. Consistent padding inside panes, list rows, and the status bar. Never cram text against a pane edge. On narrow terminals, reduce padding before truncating content.
- Typography hierarchy without font sizes: titles = bold + on-surface; body = on-surface; secondary metadata = on-surface-variant; disabled = reduced-contrast tone (the one role exempt from the contrast rule, per WCAG). Use italics sparingly. Avoid dim (SGR 2) as the only signal.
- State layers: hover/focus/selection shown as a full-width tinted background (primary at low opacity blended into the surface), not by reverse video or arrows. Focused pane gets a slightly raised surface tone; unfocused panes stay flat.
- Components:
  - Status bar and top bar: flat filled bars on surface-container, content left/right aligned with padding.
  - Tabs: active tab is a filled secondary-container pill; inactive tabs are plain text.
  - Labels, review states, check statuses: chips with filled tonal backgrounds. Use GitHub label colors but adjust their tone to meet contrast against the chip text.
  - Buttons/actions in dialogs: filled (primary) for the main action, tonal or text-only for others.
  - Lists: full-width rows, 1-cell padding, selected row tinted, metadata right-aligned in on-surface-variant.
- Icons: Nerd Font glyphs when enabled in config; otherwise clean Unicode fallbacks (●, ○, ✓, ✗, ↗). Never rely on an icon alone to carry meaning. Rounded chip ends via Nerd Font/Powerline glyphs only when enabled.
- Motion: none or minimal. No spinners that redraw the whole screen; a small inline progress indicator in the status bar.

Color system:
- Generate full light and dark schemes from a single seed color using Material 3's HCT tonal palettes (primary, secondary, tertiary, neutral, neutral-variant, error). Use an existing Rust port of Material Color Utilities if a maintained one exists (check, e.g. the `material-colors` crate); otherwise implement HCT palette generation in the `theme` crate.
- Auto-detect the terminal's background with an OSC 11 query, sent before entering the alternate screen, with a short timeout (~100ms) and a fallback to dark. Handle tmux and terminals that don't reply. Overridable in config. Seed color configurable in TOML; ship a tasteful default.
- Color depth: detect truecolor (`COLORTERM`, config override). In 256-color mode, quantize every role to the nearest xterm-256 color and check which adjacent surface levels collapsed into the same index. Wherever separation by tone is lost, switch that boundary to an outline-variant border automatically. Do the same for diff backgrounds: if added/removed/context tints collapse, fall back to a colored gutter marker so changes stay visible.
- Diff colors derived from the scheme, not hardcoded: added/removed lines get low-chroma tinted backgrounds (harmonized green/red toward the seed), token-level changes get a stronger tint of the same hue, moved code uses a tertiary tint, context lines sit on plain surface.
- Syntax highlighting theme generated from the same palette so code harmonizes with the UI (keywords primary, strings tertiary, comments on-surface-variant, etc.).
- Contrast: every foreground/background pair the UI can produce must meet WCAG 4.5:1 (3:1 for bold titles); disabled text is exempt. The pairs most likely to fail are syntax colors on diff backgrounds, so explicitly cover every syntax role × every diff background (context, added, removed, strong added, strong removed, moved), each with and without the selection state layer, in both schemes and at both color depths. When a pair fails, adjust the foreground's tone, not the background's.

Put all styling behind semantic roles in the `theme` crate (e.g. `theme.surface_container_high`, `theme.diff_added_bg`); views never use raw colors. The theme crate exposes the full list of fg/bg pairs it can emit, so the contrast test can't drift from the real usage.

## Milestones (stop after each, summarize, and wait for my go-ahead)
M0 Scaffolding: `git init`, workspace, `.github/workflows/ci.yml` running `cargo fmt --check`, `clippy -D warnings`, `test`, and the TLS dependency check; config loading; auth (env var, then `gh auth token`); logging to a file (never stdout while the TUI runs); panic hook that restores the terminal; theme crate with light/dark schemes from a seed color, terminal background detection, color-depth quantization, and the contrast test.
M1 Minimal entry: `ghtui pr <owner>/<repo>#<N>`, a PR URL, or `ghtui pr <N>` inside a clone (repo inferred from remotes) opens the PR; a basic list of my open PRs and review requests to pick from. Fetch PR metadata via GraphQL with caching.
M2a Local git + unified diff: repo selection, credentials, fetch + merge-base, file list with renames, batched blob prefetch, file tree, unified view with syntax highlighting, virtualized scrolling, core navigation.
M2b Diff viewer complete: split view, context expansion, full-file mode, whitespace toggle, search, fuzzy file finder, remaining navigation keys, viewed sync, per-hunk reviewed marks, performance targets met.
M3 Review: read/write threads, multi-line comments, suggestions, pending review submission and persistence, the full anchoring logic above.
M4 Better-than-GitHub: token-level intra-line highlights, moved-code detection (like `git diff --color-moved`; render moved blocks with the tertiary tint and "moved from path:line" plus a jump key), formatting-only hunk detection and collapse, "changes since my last review" that survives force-pushes/rebases (use the pinned head SHA from my last review; compute via `git range-diff` or diff-of-diffs so rebase noise is excluded), arbitrary commit/range selection.
M5 Later, do not start without asking: structural (difftastic-style) diff mode, tree-sitter go-to-definition then LSP on a per-PR git worktree, blame gutter, stacked-PR base detection, image diffs via kitty graphics, then the rest of the GitHub surface (notifications inbox, issues, Actions, code browsing, releases, discussions, projects).

In each milestone summary, list anything you deviated from, stubbed, or couldn't verify, and the current build time for a clean and an incremental build.

## Quality bar
- `diff` and `git` crates: unit tests using temporary git repos created in tests (renames, binary files, rebased PRs, force-pushed PRs, moved blocks, CRLF, no trailing newline, huge files, submodules, symlinks). Partial-clone behavior (prefetch, lazy fetch) is tested against a local bare repo served with `uploadpack.allowFilter`, with no network access needed.
- UI: snapshot tests with ratatui's TestBackend + `insta`, rendered in both light and dark themes and in 256-color mode.
- Comment anchoring: dedicated tests covering commentable-range derivation from patches, the local fallback, cross-hunk range rejection, and outdated-comment mapping.
- Theme: the exhaustive contrast test described above.
- Performance targets, each backed by a benchmark or timing test in the repo:
  - First paint from cache < 200ms.
  - Opening an uncached 500-file PR shows the file tree before blob prefetch and diffs finish.
  - Scrolling a synthetic 20k-line diff: update + render per frame < 8ms on a release build.
  - Opening a single 50k-line file diff never blocks input; first screen < 100ms.
- Never block the UI on network or git. Show loading and error states inline; surface rate-limit status in the status bar.
- README: install, auth (including how the git cache clone authenticates), minimum git version, keymap, architecture overview, theming, crate substitutions, known limitations.

Start with M0 and M1, then pause for review.
