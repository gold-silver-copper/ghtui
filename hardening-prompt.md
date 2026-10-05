Harden and polish `ghtui`, the Rust GitHub TUI in this repository. Its feature set is already broad. This pass is about making it impossible to crash, making it safe with the user's data and with GitHub, making the code more idiomatic, and fixing the UX rough edges found in a review. Read `README.md` and `github-tui-prompt.md` (the original brief) first: every design decision there still holds unless this document overrides it.

The author values concision. Prefer deleting code to adding it. Match the existing style, naming and comment density. Line references below come from commit `035c444` and may drift, so find the code by name rather than by number.

## Ground rules
- Work milestone by milestone. After each one, stop, summarize what changed and what you found, and wait for my go-ahead.
- Make one commit per logical fix, with a plain-prose message in the style of the existing history.
- Every bug fix comes with a test that fails before the fix and passes after it: a unit test, an insta snapshot, an api client test against the local server, or a git integration test.
- `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --check` must pass at the end of every milestone.
- Don't change user-visible behaviour beyond what's listed here without asking. If a fix needs a design decision this document doesn't settle, ask.
- Keep the README accurate. Whenever behaviour, keys or known limitations change, update it in the same commit.

## M1: bugs that break the terminal, lose data, or touch GitHub twice
1. **Panic containment.** `main.rs` installs a logging panic hook and then calls `ratatui::init`, which wraps it with a hook that restores the terminal. That hook fires for panics on *any* thread. A panic inside `tokio::spawn` or `spawn_blocking` is caught by tokio, so the main loop keeps running on a restored (cooked, non-alternate) terminal, and the reply never arrives, leaving a spinner that never stops. `diff_job.rs` tries to recover from a panic in `FileDiff::compute`, but the hook breaks the terminal first. Fix:
   - Restore the terminal from the hook only when the panic is on the main thread. Always log.
   - Spawn all background work through one helper that `catch_unwind`s, using `FutureExt::catch_unwind` or by inspecting the `JoinError`, and turns a panic into an error message for the screen that asked. No command may leave a screen loading forever: every `Cmd` that expects a reply must get either the result or an error.
   - Replace `ratatui::init()` with `try_init()`, and replace the tokio runtime builder's `expect`, so running without a TTY prints `ghtui: …` and exits non-zero.
2. **Palette crash on short terminals.** At 3–5 rows tall, `overlays.rs` (`list_top = inner.y + 2`, then `list_row` in `ui/src/lib.rs`) writes the selection stripe below the buffer, and ratatui panics with "index outside of buffer". Limit the list to `inner.bottom()`. Add a regression test that renders every widget and overlay (header, tabs, status bar, palette and every picker, menu, help, review sheets, diff view, file tree, page) at every size from 0×0 to 12×12, plus 1×200, 200×1 and 80×24. Use fixtures with content (a diff with threads, a markdown page, a long list). The test must never panic.
3. **No retrying mutations.** `api/src/client.rs` `send` retries `Request::Post` on network errors and 5xx. That duplicates comments, replies and review submissions when GitHub applied the first attempt. Retry POSTs and GraphQL mutations only when the request provably never reached the server (connection refused, DNS failure). Never retry after a timeout or a 5xx. GraphQL queries sent as POST should keep retrying, so tell queries and mutations apart explicitly rather than by HTTP method.
4. **Stale diff results.** `JobControl` aborting only stops the outer `diff_job::run` task. Its worker tasks and the prefetch task are detached `tokio::spawn`s that keep sending `Msg::FileDiff(pr, index, …)`. After a Refresh or a commit-range change, an old job's file can land at the same index in the new doc. Fix both halves:
   - Tag every message the diff job sends (`DiffFiles`, `FileDiff`, `MovesDetected`, `DiffProgress`, `DiffFailed`) with a job generation number, and drop messages whose generation doesn't match.
   - Own the workers in a `JoinSet` inside the job so aborting the job aborts them.
   - Make sure `BlobReader::read` stays correct if its future is dropped mid-reply. Either never cancel it mid-read, or discard the process once a read has been cancelled.
5. **Review drafts must survive.**
   - Pending review drafts live in the cache redb, which is deleted on a schema-version mismatch or when the file is unreadable. Move them to their own store, either a second redb or a JSON file per PR written atomically (temp file + rename) under the platform data dir rather than the cache dir. Wiping the cache must never wipe drafts. Migrate existing drafts once.
   - `SaveReview` writes run as unordered `spawn_blocking` calls, so an older snapshot can commit after a newer one. Serialize them through a single writer task, or attach a sequence number and drop older ones.
   - When the store can't be opened (for example a second ghtui instance holds the redb lock), say so persistently on screen: "Drafts are not being saved: …". Don't silently drop writes.
6. **Untrusted links.** Links from GitHub content (markdown in READMEs, issues and comments) reach `xdg-open`/`open` with any scheme, and on Windows `cmd /C start` is open to command injection. Content hrefs starting with `ghtui:` also reach the internal action handlers in `nav.rs` `follow`. Fix:
   - Open only `http`, `https` and `mailto` links externally. Show anything else as plain text with a notice.
   - Make internal actions impossible to express as a content link. M4 replaces the fake `ghtui:` URLs with a typed enum; until then, reject `ghtui:` hrefs during markdown rendering.
   - On Windows use `explorer` (or `rundll32 url.dll,FileProtocolHandler`), never `cmd`.
7. **Smaller safety fixes:**
   - Handle SIGTERM, SIGHUP and SIGQUIT with `tokio::signal::unix` as a clean quit that restores the terminal and mouse mode.
   - After the main loop exits, restore the terminal and then `shutdown_timeout` the runtime (or exit) so quitting never hangs on running `spawn_blocking` work.
   - Give git network operations a stall timeout: set `GIT_HTTP_LOW_SPEED_LIMIT`/`GIT_HTTP_LOW_SPEED_TIME` in the `git()` env, plus an outer `tokio::time::timeout` on `fetch`, `clone` and blob prefetch. A stall should show an error with "r tries again", not a progress bar that never moves.
   - Create the `$EDITOR` temp file with `tempfile`: exclusive creation, mode 0600, never following symlinks. Move `tempfile` from dev-dependencies to dependencies.
   - Move the synchronous `store.http_get` in `client.rs` `rest_get` off the async worker (`spawn_blocking`, matching how writes already work).
   - Fix `rust-version` in `Cargo.toml` to match the locked dependencies (`material-colors` 0.5 needs 1.97), and add a CI job that runs `cargo check` on that exact toolchain.
8. **Home page error.** `browse.rs` `build_page` skips the `missing()` helper for `Route::Home`, so a failed first load with an empty cache shows empty boxes and a spinner forever. `home_error_light.snap` currently looks the same as the loading snapshot. Show "Couldn't load …: … r tries again" like every other page, and update the snapshot.

## M2: make panics structurally impossible
"Impossible" here means: the compiler rejects panicking constructs in non-test code, buffer writes can't go out of bounds, and anything that still panics inside a dependency is contained by the M1 boundary instead of killing the session.
1. **Workspace lints.** Add a `[workspace.lints]` table and `[lints] workspace = true` to every crate:
   - `rust`: `unsafe_code = "forbid"`.
   - `clippy`, denied: `unwrap_used`, `expect_used`, `panic`, `unreachable`, `todo`, `unimplemented`, `indexing_slicing`, `string_slice`, `cast_possible_truncation`.
   - `clippy`, warned: `redundant_clone`, `needless_pass_by_value`, `manual_let_else`, `must_use_candidate`.
   - Allow the panicking lints in tests, using `#[cfg_attr(test, allow(...))]` on test modules or `clippy.toml`'s `allow-unwrap-in-tests` / `allow-expect-in-tests` / `allow-indexing-slicing-in-tests`.
   - Never silence a lint with a bare `#[allow]` in non-test code. If something really can't fail, encode that in the types instead.
2. **Fix the roughly 120 sites properly, not mechanically.** Use `.get()` with an early return, `let … else`, `split_at_checked`, `str::get`, `checked_sub`/`saturating_sub`, and `u16::try_from(..).unwrap_or(u16::MAX)` behind a single `cols(usize) -> u16` helper in `ui`. Where an index is guarded by an invariant, prefer a type that makes the invariant hold:
   - A 1-based `LineNo(NonZeroU32)` with `fn index(self) -> usize`, replacing every `n as usize - 1` in `diff/src/file.rs`, `intraline.rs` and `ui/src/diff_view.rs`.
   - Private fields on `Doc` (`files`, `annotations`, `opts` are `pub` today), so rows can't go stale without a rebuild. The same goes for any other struct whose fields carry an invariant.
   - `Text::new` refusing inputs over `u32::MAX` bytes. `app/src/diff_job.rs` currently passes uncapped blobs to it, unlike the paths that apply `MAX_DIFF_BYTES`.
   - Collapsing the `expect` + `unreachable!` pair in `diff_doc.rs` into one `let Some(Content::Text(text)) = … else { … }`.
   - Removing the `unreachable!` in `runtime.rs` by splitting `Cmd` (see M4.3) instead of matching around it.
   - Making `theme::color::xterm_color` total: take a typed index, or return `Option`.
3. **Clipped drawing only.** In `clippy.toml`, put `ratatui::buffer::Buffer::set_string`, `set_stringn`, `set_line`, `set_span`, and indexing into `Buffer`, under `disallowed_methods`. Route every write through one helper, for example `ui::put(buf, x, y, text, style)`, that returns early when the position is outside `buf.area`.
4. **Arithmetic in layout code.** Deny `clippy::arithmetic_side_effects` in the modules that do u16 geometry (`ui/src/lib.rs`, `chrome.rs`, `overlays.rs`, `bars.rs`, `review_sheets.rs`, `page.rs`, `file_tree.rs`) and use saturating ops there. Also use `saturating_add` when parsing hunk headers in `diff/src/anchor.rs`: GitHub patch line numbers are untrusted u32s.
5. **Property tests and fuzzing:**
   - Add `proptest` round-trips: `Target::from_url(&route.url()) == Target::Page(route)` (include branches with slashes and percent-encoded paths), and `format_sequence(parse_sequence(s)) == s` for the keymap.
   - Add no-panic properties: `markdown::render` on arbitrary input never panics and every line fits `page.width`; for arbitrary old/new text, `FileDiff::compute` + `Doc::new` never panics, `to_pos(to_global(p))` round-trips, and `search` never panics; diff code spans on arbitrary Unicode with tabs, control characters, wide characters and emphasis ranges never exceed `room`.
   - Add `cargo-fuzz` targets for `markdown::render`, `FileDiff::compute` and `route::parse_input`. Run each for a few minutes and fix whatever turns up.
6. **Safe rendering of untrusted Unicode.** ratatui already drops C0/C1 controls. Additionally strip or visibly mark bidi overrides and isolates (U+202A–U+202E, U+2066–U+2069) and zero-width characters in titles, branch names, logins, labels and comment text, so text can't be visually spoofed. Do this in one place in the text layer.

## M3: UX fixes
1. **README and keymap agree.** Today the README documents `S` (cycle state), `O` (cycle sort) and `gm` (jump across moved code), none of which are bound. It says `*` stars, but the key is `s`. It says `f` turns a comment into a file comment, but `f` is Go to file. Its known limitations still say there's no branch picker, but `b` exists. Bind the missing actions or fix the docs, whichever is right for each. Add a test that every key the README mentions in its key tables resolves to the documented action, generated from the keymap if that's simpler.
2. **Default keys for daily diff actions.** These have no binding today: resolve/unresolve thread, next/previous unresolved thread, next unviewed file, split/unified toggle, file tree toggle, full-file mode, and Forward (history). Propose bindings that fit the existing "one meaning per key, everywhere" scheme (`R`, `N`/`P`, `U`, `F`, `t`, `<C-o>`/`<C-i>` are free or close to it) and check them with me before committing. Thread hints must never fall back to `:resolve resolve`.
3. **The Space menu fits on any screen.** It overflows silently: Quit is cut off at 100×30, and about a third is lost at 80×24. Make it scroll, show a count, and filter as you type, reusing the palette's behaviour.
4. **Pending review visibility.** While drafts exist, the diff status bar shows "N pending · a submit" (using the real key).
5. **Errors you can read and recall:**
   - Status-bar errors currently expire after 10s and can't be dismissed or recalled. Keep a bounded message history, viewable from the menu or a key, and let Esc dismiss the current one.
   - The diff error banner (`view.rs`) wraps instead of truncating long git errors.
   - `Unauthorized` becomes a persistent banner with the fix ("run `gh auth login`, or set GH_TOKEN"), not a notice that expires after 10s.
6. **Offline and stale indication.** When showing cached data because the network is unavailable, or before a refresh has finished, show the cache age up front ("cached 2h ago"). Keep the existing pattern for a failed refresh.
7. **Undo for deleting a draft.** `<Delete>` on a draft removes it immediately. Keep the last deleted draft and offer "Draft deleted · u undo" until the next action.
8. **Link hints don't hide text.** Hint labels currently overwrite the first character of the link ("shtui", "uctocat" in `hints_dark.snap`). Draw the label as a badge in front of the link.
9. **Icons:**
   - The non-Nerd-Font comment glyph `💬` is double-width; use a single-width glyph.
   - `●`/`✓`/`✗` mean both PR state and check status on the same row. Give them distinct glyphs or colours so the two can be told apart without colour too.
10. **Small terminals:**
    - Add a compact list density, one row per item, as `[ui] density = "compact" | "comfortable" | "auto"`, where `auto` goes compact below about 30 rows. At 80×24, home currently shows about 3.5 PRs and the issues list 3.
    - Fold the PR ref into the diff header and show the PR title, which is absent today.
    - Add 80×24 snapshots for home, an issue list, a PR conversation, the diff, and the menu.
11. **Hints for what's under the cursor.** The status bar takes the first 7 actions for the screen. Order them by what's under the cursor instead: on a thread reply/resolve, on a draft edit/delete, on a file viewed/comment. Drop the least relevant ones first when the bar is narrow.
12. **Spinner honesty.** "⠋ Loading" remains in many page snapshots after the content has rendered. Find out whether the fixtures leave a fetch pending or the indicator ignores what's on screen, and fix whichever is wrong.

## M4: idiomatic code and performance
1. **Rebuild once per batch:**
   - `update` calls `sync_page()` for every message, and almost every message bumps `data_gen`, including `RateLimits`, sent after every API call, and `Notice`/`DiffProgress`. One fetch currently causes two full page rebuilds (markdown + layout). Bump `data_gen` only where page data actually changes, and run `sync_page`/`settle_diff` once after the runtime drains the channel, not per message.
   - `layout()` in `app/src/chrome.rs` builds the full `Chrome` (formatted strings, routes) just to check whether a title and tabs exist, 5–8 times per keypress. Compute it once per frame, or add cheap predicates.
2. **No allocations per visible row:**
   - `diff_view.rs` creates a `String` per character (`c.to_string()` in the code-span loop); push into the existing buffer instead.
   - Replace the `.to_vec()` inside `flat_map` and the per-segment `vec![s, e]` with iterators.
   - `Keymap::keys_in` builds `Vec<Vec<Key>>` to take the first entry, roughly 20 times per frame. Use `find`.
3. **Split messages and commands by screen:**
   - Group the 17 `Msg` variants that start with a `PrRef` into `Msg::Diff(PrRef, DiffMsg)`, handled by a `diff_screen::update`.
   - Split `Cmd` into kinds (API, git, terminal/editor) so the runtime's two dispatch sites each match only what they handle.
   - Move the review flow out of `state.rs` into the existing `review.rs`: `review_action`, `start_since_review`, `apply_commit_choice`, compose/submit key handling, `save_compose`, `on_submitted`, `on_edited`.
   - Add `fn info(..)` / `fn error(..)` helpers for the 43 hand-written `state.notice = Some(Notice::…)` sites.
   - Stop cloning the whole `Screen` in `load_visible` and the annotation under the cursor on every `review_action`.
4. **Typed links:**
   - Replace the fake `ghtui:state:…`, `ghtui:quote:{author}\n{body}`, etc. URLs in `Page::links` with `enum Link { Url(String), Internal(Action) }` or similar. Quoting carries a comment index, not the comment body.
   - Make `Page::link` deduplication O(1), using a `HashMap` or by construction. It's an O(n²) linear scan today.
   - This finishes the security fix from M1.6.
5. **Types over strings and tuples:**
   - Introduce `NodeId` and `Oid` newtypes for GraphQL node IDs and git object IDs.
   - Replace `last_review: Option<Option<String>>` with an enum.
   - Turn `Cmd::MapOutdated`'s `(String, String, String, u32)` and `commits: Vec<(String, String)>` into named structs.
   - Replace `Result<_, String>` in messages with a small error type that keeps the cause for the log.
   - Put `#[must_use]` on every function returning `Vec<Cmd>`, since a dropped vector silently loses effects.
   - Replace the five `#[allow(clippy::too_many_arguments)]` with parameter structs.
   - Make `load_more` stop reusing `loading`, so loading more results doesn't block a refresh or show the global "Loading".
6. **Dependencies.** octocrab is used only as an HTTP transport: its retries are disabled and `client.rs` has its own retry loop. It pulls in jsonwebtoken, chrono, snafu, tower and a second base64. Replace it with `reqwest` (rustls + ring, no default features), keeping the existing ETag, rate-limit and retry behaviour and the api client tests passing. Keep the CI check that `aws-lc-rs` and `openssl-sys` are absent. Report the dependency count and clean build time before and after.

## M5: features (propose a plan first; don't start without my go-ahead)
Ranked by how much a daily github.com or gh-dash user misses them:
1. A notifications inbox (the route exists and currently opens the browser).
2. A Checks tab inside ghtui: failing check names, their summaries, a link to the log, re-run.
3. PR actions with confirmation: merge (method picker), close/reopen, mark ready for review, request reviewers.
4. Check out a PR locally, when inside a clone.
5. Configurable home sections with saved filters, like gh-dash.
6. Markdown rendering in diff threads, and horizontal scrolling or soft wrapping for long diff lines.

## Quality bar
- No regressions in startup time or diff latency. Measure both before M1 and after M4 and report the numbers: time to first frame, time to first rendered file on a 500-file PR, and frame time while scrolling a large diff.
- Every decision this document leaves open gets a one-line note in the milestone summary.
- At the end, summarize what remains unsafe or unhandled. Be honest rather than claiming completeness.
