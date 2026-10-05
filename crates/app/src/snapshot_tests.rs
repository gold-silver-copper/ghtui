//! Full-screen snapshots through ratatui's `TestBackend`, in light and dark
//! schemes (and 256 colors). Snapshots include cell styles, so color changes
//! show up in review. Rendering also exercises the theme's debug assertion
//! that every fg/bg pair used is declared (and therefore contrast-tested).

use ghtui_api::browse::SearchKind;
use ghtui_api::model::{
    ChecksState, Inbox, Label, Mergeable, PrDetail, PrRef, PrState, PrSummary, RepoId,
    ReviewDecision,
};
use ghtui_api::rate_limit::{Bucket, RateLimits};
use ghtui_theme::{ColorDepth, DEFAULT_SEED, Mode, Theme};
use ghtui_ui::Icons;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use ghtui_ui::pages::PrTab;

use crate::browse::{Data, DataKey, Need, needs};
use crate::fixtures::{self, press};
use crate::keymap::Keymap;
use crate::route::{OPEN, Route};
use crate::state::{Msg, Overlay, State, update};
use crate::view::view;

/// 2026-10-03T12:00:00Z
const NOW: u64 = 1_791_028_800;

fn summary(
    pr: &str,
    title: &str,
    state: PrState,
    review: Option<ReviewDecision>,
    checks: Option<ChecksState>,
) -> PrSummary {
    PrSummary {
        pr: PrRef::parse(pr).unwrap(),
        title: title.into(),
        author: "octocat".into(),
        state,
        updated_at: "2026-10-03T09:00:00Z".into(),
        additions: 120,
        deletions: 34,
        comments: 3,
        review,
        checks,
    }
}

fn inbox() -> Inbox {
    Inbox {
        review_requested: vec![
            summary(
                "ratatui/ratatui#1820",
                "Add a virtualized list widget for very long collections",
                PrState::Open,
                Some(ReviewDecision::ReviewRequired),
                Some(ChecksState::Passing),
            ),
            summary(
                "tokio-rs/tokio#7001",
                "Fix waker leak in the multi-thread scheduler when tasks are cancelled during shutdown",
                PrState::Draft,
                None,
                Some(ChecksState::Pending),
            ),
        ],
        authored: vec![
            summary(
                "gold-silver-copper/ghtui#12",
                "Theme: generate syntax palette from seed",
                PrState::Open,
                Some(ReviewDecision::Approved),
                Some(ChecksState::Failing),
            ),
            summary(
                "gold-silver-copper/ghtui#9",
                "Cache GraphQL responses in redb",
                PrState::Open,
                Some(ReviewDecision::ChangesRequested),
                None,
            ),
        ],
        review_requested_total: 2,
        authored_total: 31,
    }
}

pub(crate) fn pr_detail() -> PrDetail {
    PrDetail {
        summary: summary(
            "gold-silver-copper/ghtui#12",
            "Theme: generate syntax palette from seed",
            PrState::Open,
            Some(ReviewDecision::Approved),
            Some(ChecksState::Failing),
        ),
        body: "Derives keyword, string and type colors from the seed's HCT palettes.\n\n\
               Every syntax role is now checked against every diff background, in both \
               schemes and at both color depths."
            .into(),
        created_at: "2026-09-30T12:00:00Z".into(),
        base_ref: "main".into(),
        head_ref: "syntax-palette".into(),
        base_oid: "0123456789abcdef0123456789abcdef01234567".into(),
        head_oid: "fedcba9876543210fedcba9876543210fedcba98".into(),
        head_repo: Some("octocat/ghtui".into()),
        changed_files: 7,
        mergeable: Mergeable::Conflicting,
        labels: vec![
            Label {
                name: "enhancement".into(),
                color: "a2eeef".into(),
            },
            Label {
                name: "theme".into(),
                color: "7057ff".into(),
            },
        ],
    }
}

fn state(mode: Mode, depth: ColorDepth) -> State {
    let mut state = State::new(
        Theme::new(DEFAULT_SEED, mode, depth),
        Icons::default(),
        Keymap::default(),
        (100, 30),
    );
    state.viewer = Some("octocat".into());
    state.clock = || NOW;
    state.rate_limits = RateLimits {
        graphql: Some(Bucket {
            remaining: 4987,
            limit: 5000,
            reset_at: NOW + 3600,
        }),
        rest: None,
    };
    state
}

fn with_inbox(mode: Mode, depth: ColorDepth) -> State {
    let mut state = state(mode, depth);
    update(&mut state, Msg::Inbox(Ok(inbox())));
    fetched(
        &mut state,
        DataKey::ViewerRepos,
        Data::Repos(vec![
            fixtures::repo_summary("gold-silver-copper/ghtui", 1234),
            fixtures::repo_summary("gold-silver-copper/fux", 87),
        ]),
    );
    state
}

fn with_pr(mode: Mode) -> State {
    let mut state = with_inbox(mode, ColorDepth::TrueColor);
    let pr = PrRef::parse("gold-silver-copper/ghtui#12").unwrap();
    state.push(Route::Pr {
        pr: pr.clone(),
        tab: PrTab::Conversation,
    });
    update(&mut state, Msg::Pr(pr.clone(), Box::new(Ok(pr_detail()))));
    fetched(
        &mut state,
        DataKey::PrActivity(pr),
        Data::PrActivity(Box::new(fixtures::activity())),
    );
    state
}

fn fetched(state: &mut State, key: DataKey, data: Data) {
    update(
        state,
        Msg::Fetched {
            key,
            result: Ok(data),
            cached_at: None,
        },
    );
}

/// Pushes `route` and delivers `data` for its page's own fetch (its last).
fn open(state: &mut State, route: Route, data: Data) {
    let Some(Need::Data(key)) = needs(&route).pop() else {
        panic!("{route:?} doesn't end in a data fetch");
    };
    state.push(route);
    fetched(state, key, data);
}

fn ghtui() -> RepoId {
    RepoId::new("gold-silver-copper", "ghtui")
}

fn with_repo(mode: Mode, depth: ColorDepth) -> State {
    let mut state = state(mode, depth);
    state.push(Route::Repo(ghtui()));
    fetched(
        &mut state,
        DataKey::Repo(ghtui()),
        Data::Repo(Box::new(fixtures::overview())),
    );
    state
}

fn render(state: &State) -> String {
    let (w, h) = state.size;
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|frame| view(state, frame, NOW)).unwrap();
    format!("{:?}", terminal.backend().buffer())
}

#[test]
fn home_dark() {
    insta::assert_snapshot!(render(&with_inbox(Mode::Dark, ColorDepth::TrueColor)));
}

#[test]
fn home_light() {
    insta::assert_snapshot!(render(&with_inbox(Mode::Light, ColorDepth::TrueColor)));
}

#[test]
fn home_dark_256() {
    insta::assert_snapshot!(render(&with_inbox(Mode::Dark, ColorDepth::Ansi256)));
}

#[test]
fn home_third_row_selected_light() {
    let mut state = with_inbox(Mode::Light, ColorDepth::TrueColor);
    press(&mut state, "jj");
    insta::assert_snapshot!(render(&state));
}

#[test]
fn home_loading_dark() {
    let mut state = state(Mode::Dark, ColorDepth::TrueColor);
    state.load_visible(false);
    insta::assert_snapshot!(render(&state));
}

#[test]
fn home_error_light() {
    let mut state = state(Mode::Light, ColorDepth::TrueColor);
    state.load_visible(false);
    update(
        &mut state,
        Msg::Inbox(Err(ghtui_api::ApiError::Network(
            "connection refused".into(),
        ))),
    );
    insta::assert_snapshot!(render(&state));
}

#[test]
fn repo_dark() {
    insta::assert_snapshot!(render(&with_repo(Mode::Dark, ColorDepth::TrueColor)));
}

#[test]
fn repo_light() {
    insta::assert_snapshot!(render(&with_repo(Mode::Light, ColorDepth::TrueColor)));
}

#[test]
fn repo_readme_dark_256() {
    let mut state = with_repo(Mode::Dark, ColorDepth::Ansi256);
    press(&mut state, "G");
    insta::assert_snapshot!(render(&state));
}

#[test]
fn repo_wide_terminal_centers_light() {
    let mut state = with_repo(Mode::Light, ColorDepth::TrueColor);
    update(&mut state, Msg::Resize(160, 30));
    insta::assert_snapshot!(render(&state));
}

#[test]
fn file_light() {
    let mut state = with_repo(Mode::Light, ColorDepth::TrueColor);
    let route = Route::Blob {
        repo: ghtui(),
        rev: "main".into(),
        path: "src/main.rs".into(),
    };
    open(&mut state, route, Data::Blob(Box::new(fixtures::blob())));
    insta::assert_snapshot!(render(&state));
}

#[test]
fn issues_dark() {
    let mut state = with_repo(Mode::Dark, ColorDepth::TrueColor);
    let route = Route::Issues {
        repo: ghtui(),
        query: OPEN.into(),
    };
    let results = fixtures::issue_results(Some("c1"));
    open(&mut state, route, Data::Search(Box::new(results)));
    insta::assert_snapshot!(render(&state));
}

#[test]
fn issue_light() {
    let mut state = state(Mode::Light, ColorDepth::TrueColor);
    let route = Route::Issue {
        repo: ghtui(),
        number: 14,
    };
    open(
        &mut state,
        route,
        Data::Issue(Some(Box::new(fixtures::issue()))),
    );
    insta::assert_snapshot!(render(&state));
}

#[test]
fn pr_dark() {
    insta::assert_snapshot!(render(&with_pr(Mode::Dark)));
}

#[test]
fn pr_light() {
    insta::assert_snapshot!(render(&with_pr(Mode::Light)));
}

#[test]
fn pr_commits_dark() {
    let mut state = with_pr(Mode::Dark);
    press(&mut state, "2");
    insta::assert_snapshot!(render(&state));
}

#[test]
fn pr_refresh_error_dark() {
    let mut state = with_pr(Mode::Dark);
    let pr = PrRef::parse("gold-silver-copper/ghtui#12").unwrap();
    update(
        &mut state,
        Msg::Pr(pr, Box::new(Err(ghtui_api::ApiError::RateLimited(42)))),
    );
    insta::assert_snapshot!(render(&state));
}

#[test]
fn profile_light() {
    let mut state = state(Mode::Light, ColorDepth::TrueColor);
    open(
        &mut state,
        Route::user("octocat"),
        Data::Profile(Box::new(fixtures::profile())),
    );
    insta::assert_snapshot!(render(&state));
}

#[test]
fn search_dark() {
    let mut state = state(Mode::Dark, ColorDepth::TrueColor);
    let route = Route::Search {
        kind: SearchKind::Repos,
        query: "terminal file manager".into(),
    };
    open(
        &mut state,
        route,
        Data::Search(Box::new(fixtures::repo_results())),
    );
    insta::assert_snapshot!(render(&state));
}

#[test]
fn search_users_light() {
    let mut state = state(Mode::Light, ColorDepth::TrueColor);
    let route = Route::Search {
        kind: SearchKind::Users,
        query: "octocat".into(),
    };
    open(
        &mut state,
        route,
        Data::Search(Box::new(fixtures::user_results())),
    );
    insta::assert_snapshot!(render(&state));
}

#[test]
fn quick_ways_around_dark() {
    let mut state = with_repo(Mode::Dark, ColorDepth::TrueColor);
    state.visits = vec![crate::nav::Visit {
        url: "https://github.com/octocat".into(),
        title: "@octocat".into(),
        count: 3,
        last: NOW - 600,
    }];
    press(&mut state, "l");
    insta::assert_snapshot!("hints_dark", render(&state));
    press(&mut state, "<Esc>/");
    insta::assert_snapshot!("search_empty_dark", render(&state));
    press(&mut state, "oct");
    insta::assert_snapshot!("search_typed_dark", render(&state));
    press(&mut state, "<Esc><Space>");
    insta::assert_snapshot!("menu_dark", render(&state));
    press(&mut state, "<Esc>g");
    insta::assert_snapshot!("which_key_dark", render(&state));
}

#[test]
fn go_to_file_light() {
    let mut state = with_repo(Mode::Light, ColorDepth::TrueColor);
    press(&mut state, "f");
    fetched(
        &mut state,
        DataKey::Files(ghtui(), "main".into()),
        Data::Files(
            std::sync::Arc::new(vec![
                "src/main.rs".into(),
                "crates/app/src/main.rs".into(),
                "crates/ui/src/page.rs".into(),
            ]),
            false,
        ),
    );
    press(&mut state, "main");
    insta::assert_snapshot!(render(&state));
}

#[test]
fn list_filter_light() {
    let mut state = with_repo(Mode::Light, ColorDepth::TrueColor);
    press(&mut state, "2/");
    insta::assert_snapshot!(render(&state));
}

#[test]
fn profile_stars_dark() {
    let mut state = state(Mode::Dark, ColorDepth::TrueColor);
    let route = Route::User {
        login: "octocat".into(),
        tab: ghtui_ui::pages::ProfileTab::Stars,
    };
    open(
        &mut state,
        route,
        Data::Profile(Box::new(fixtures::profile())),
    );
    insta::assert_snapshot!(render(&state));
}

#[test]
fn repo_wide_with_about_dark() {
    let mut state = with_repo(Mode::Dark, ColorDepth::TrueColor);
    update(&mut state, Msg::Resize(150, 36));
    insta::assert_snapshot!(render(&state));
}

#[test]
fn menu_overlay_dark() {
    let mut state = with_inbox(Mode::Dark, ColorDepth::TrueColor);
    crate::nav::open_menu(&mut state);
    insta::assert_snapshot!(render(&state));
}

#[test]
fn palette_light() {
    let mut state = with_inbox(Mode::Light, ColorDepth::TrueColor);
    state.open_picker(crate::picker::Kind::Commands);
    if let Some(Overlay::Picker(p)) = &mut state.overlay {
        p.input.insert_str("ratatui");
    }
    insta::assert_snapshot!(render(&state));
}

#[test]
fn narrow_terminal_does_not_panic() {
    for (w, h) in [(1, 1), (9, 2), (20, 5), (40, 10)] {
        for mode in [Mode::Light, Mode::Dark] {
            let mut state = with_pr(mode);
            update(&mut state, Msg::Resize(w, h));
            render(&state);
            state.screens.truncate(1);
            update(&mut state, Msg::Resize(w, h));
            render(&state);
            crate::nav::open_menu(&mut state);
            render(&state);
            let mut state = with_repo(mode, ColorDepth::TrueColor);
            update(&mut state, Msg::Resize(w, h));
            render(&state);
        }
    }
}

// ---- diff screen -----------------------------------------------------------

pub(crate) use diff::diff_state as diff_fixture;

pub(crate) mod diff {
    use std::collections::HashSet;
    use std::sync::Arc;

    use ghtui_api::model::PrRef;
    use ghtui_diff::FileDiff;
    use ghtui_git::files::{ChangedFile, FileStatus, ZERO_OID};
    use ghtui_git::repo::PrRefs;
    use ghtui_theme::{ColorDepth, Mode};
    use ghtui_ui::diff_doc::{Doc, Pos};

    use super::{NOW, Terminal, TestBackend, press, render, state, view};
    use crate::diff_screen::{DiffScreen, DiffState, Pane};
    use crate::state::{Msg, Screen, State, update};

    fn pr() -> PrRef {
        PrRef::parse("o/r#7").unwrap()
    }

    fn file(
        status: FileStatus,
        old: Option<&str>,
        new: Option<&str>,
        modes: (u32, u32),
    ) -> ChangedFile {
        ChangedFile {
            status,
            old_path: old.map(Into::into),
            new_path: new.map(Into::into),
            old_mode: modes.0,
            new_mode: modes.1,
            old_oid: "1".repeat(40),
            new_oid: if new.is_some() {
                "2".repeat(40)
            } else {
                ZERO_OID.into()
            },
            similarity: (status == FileStatus::Renamed).then_some(94),
        }
    }

    const OLD_RS: &str = "use std::fmt;\n\n/// A point.\npub struct Point {\n    x: i32,\n    y: i32,\n}\n\nimpl Point {\n    pub fn new(x: i32, y: i32) -> Self {\n        Point { x, y }\n    }\n}\n";
    const NEW_RS: &str = "use std::fmt;\n\n/// A point in 2D.\npub struct Point {\n    x: i32,\n    y: i32,\n}\n\nimpl Point {\n    pub fn new(x: i32, y: i32) -> Self {\n        Point { x, y }\n    }\n\n    pub fn origin() -> Self {\n        Self::new(0, \"0\".len() as i32 - 1)\n    }\n}\n";

    fn loaded(doc: Doc) -> DiffState {
        let mut diff = DiffState::loading();
        let refs = PrRefs {
            head: "h".into(),
            base: "b".into(),
            merge_base: "m".into(),
        };
        diff.set_files(refs, doc);
        diff
    }

    /// Opens a diff screen on `diff`, as wide as the terminal.
    fn open_diff(s: &mut State, diff: DiffState, cursor: Pos, focus: Pane) {
        s.diffs.insert(pr(), diff);
        let mut screen = DiffScreen::new(pr(), s.size.0);
        screen.cursor = cursor;
        screen.focus = focus;
        s.screens.push(Screen::Diff(Box::new(screen)));
        s.settle_diff();
    }

    pub(crate) fn diff_state() -> DiffState {
        let regular = (0o100644, 0o100644);
        let files = vec![
            file(
                FileStatus::Modified,
                Some("src/point.rs"),
                Some("src/point.rs"),
                regular,
            ),
            file(
                FileStatus::Renamed,
                Some("docs/old.md"),
                Some("docs/new.md"),
                regular,
            ),
            file(FileStatus::Added, None, Some("notes.txt"), (0, 0o100644)),
            file(FileStatus::Deleted, Some("gone.py"), None, (0o100644, 0)),
            file(
                FileStatus::Modified,
                Some("logo.png"),
                Some("logo.png"),
                regular,
            ),
            file(
                FileStatus::Modified,
                Some("run.sh"),
                Some("run.sh"),
                (0o100644, 0o100755),
            ),
            file(
                FileStatus::Modified,
                Some("Cargo.lock"),
                Some("Cargo.lock"),
                regular,
            ),
            file(
                FileStatus::Modified,
                Some("zz/pending.rs"),
                Some("zz/pending.rs"),
                regular,
            ),
        ];
        let mut doc = Doc::new(files, &HashSet::new());
        let diffs = [
            FileDiff::compute(
                "src/point.rs",
                Some(OLD_RS.as_bytes()),
                Some(NEW_RS.as_bytes()),
            ),
            FileDiff::compute("docs/new.md", Some(b"# Title\n"), Some(b"# Title\n")),
            FileDiff::compute("notes.txt", None, Some(b"first\nsecond, no newline")),
            FileDiff::compute("gone.py", Some(b"def f():\n    return 1\n"), None),
            FileDiff::compute("logo.png", Some(b"\x89PNG\0a"), Some(b"\x89PNG\0bc")),
            FileDiff::compute("run.sh", Some(b"echo hi\n"), Some(b"echo hi\n")),
            FileDiff::compute("Cargo.lock", Some(b"a\n"), Some(b"b\n")),
        ];
        for (i, d) in diffs.into_iter().enumerate() {
            doc.set_diff(i, Arc::new(d));
        }
        loaded(doc)
    }

    fn screen_state(mode: Mode, depth: ColorDepth, cursor: Pos, focus: Pane) -> State {
        let mut s = state(mode, depth);
        s.size = (110, 34);
        open_diff(&mut s, diff_state(), cursor, focus);
        s
    }

    #[test]
    fn diff_dark() {
        let s = screen_state(
            Mode::Dark,
            ColorDepth::TrueColor,
            Pos { file: 0, row: 4 },
            Pane::Diff,
        );
        insta::assert_snapshot!(render(&s));
    }

    #[test]
    fn diff_light() {
        let s = screen_state(
            Mode::Light,
            ColorDepth::TrueColor,
            Pos { file: 0, row: 4 },
            Pane::Diff,
        );
        insta::assert_snapshot!(render(&s));
    }

    #[test]
    fn diff_dark_256_tree_focused() {
        let s = screen_state(
            Mode::Dark,
            ColorDepth::Ansi256,
            Pos { file: 2, row: 0 },
            Pane::Tree,
        );
        insta::assert_snapshot!(render(&s));
    }

    #[test]
    fn diff_scrolled_shows_sticky_header_light() {
        let s = screen_state(
            Mode::Light,
            ColorDepth::TrueColor,
            Pos { file: 7, row: 1 },
            Pane::Diff,
        );
        insta::assert_snapshot!(render(&s));
    }

    #[test]
    fn diff_split_wide_light() {
        let mut s = screen_state(
            Mode::Light,
            ColorDepth::TrueColor,
            Pos { file: 0, row: 6 },
            Pane::Diff,
        );
        s.size = (200, 30);
        s.settle_diff();
        insta::assert_snapshot!(render(&s));
    }

    #[test]
    fn diff_viewed_and_reviewed_dark() {
        let mut s = screen_state(
            Mode::Dark,
            ColorDepth::TrueColor,
            Pos { file: 0, row: 3 },
            Pane::Diff,
        );
        let diff = s.diffs.get_mut(&pr()).unwrap();
        let hash = diff.doc.files()[0].blocks()[0].hash.clone();
        diff.set_review(ghtui_store::ReviewState {
            reviewed_hunks: vec![hash],
            ..Default::default()
        });
        diff.doc.set_viewed(1, ghtui_ui::diff_doc::Viewed::Viewed);
        s.settle_diff();
        insta::assert_snapshot!(render(&s));
    }

    fn with_threads(mode: Mode) -> State {
        use ghtui_api::model::{ReviewComment, ReviewThread, Side};
        let mut s = screen_state(
            mode,
            ColorDepth::TrueColor,
            Pos { file: 0, row: 0 },
            Pane::Diff,
        );
        let comment = |id: &str, author: &str, body: &str, pending: bool| ReviewComment {
            id: id.into(),
            author: author.into(),
            body: body.into(),
            created_at: "2026-10-03T08:00:00Z".into(),
            url: String::new(),
            original_commit: Some("abc".into()),
            pending,
        };
        let thread = |id: &str, line: Option<u32>, resolved: bool, file_level: bool, comments| {
            ReviewThread {
                id: id.into(),
                path: "src/point.rs".into(),
                side: Side::Right,
                start_side: None,
                line,
                start_line: None,
                original_line: line,
                original_start_line: None,
                outdated: false,
                resolved,
                file_level,
                can_reply: true,
                can_resolve: true,
                can_unresolve: true,
                comments,
            }
        };
        let diff = s.diffs.get_mut(&pr()).unwrap();
        diff.set_threads(vec![
            thread(
                "t1",
                Some(3),
                false,
                false,
                vec![
                    comment("c1", "alice", "Should this say \"2D\" or \"two-dimensional\"? The rest of the docs spell it out.", false),
                    comment("c2", "octocat", "Good point, I'll spell it out.", true),
                ],
            ),
            thread("t2", Some(10), true, false, vec![comment("c3", "bob", "Looks fine now.", false)]),
            thread("t3", None, false, true, vec![comment("c4", "carol", "Please add tests for this file.", false)]),
        ]);
        diff.set_review(ghtui_store::ReviewState {
            pending: vec![ghtui_store::DraftComment {
                id: 1,
                path: "src/point.rs".into(),
                body: "Prefer `Self::default()` here.".into(),
                side: ghtui_store::DraftSide::Right,
                line: Some(15),
                start_line: None,
                start_side: None,
                commit: "h".into(),
                error: Some("pull_request_review_thread.line must be part of the diff".into()),
            }],
            ..Default::default()
        });
        s.settle_diff();
        s
    }

    #[test]
    fn threads_dark() {
        insta::assert_snapshot!(render(&with_threads(Mode::Dark)));
    }

    #[test]
    fn threads_light() {
        insta::assert_snapshot!(render(&with_threads(Mode::Light)));
    }

    #[test]
    fn compose_suggestion_dark() {
        use crate::review::{Compose, ComposeTarget, Preview};
        use ghtui_diff::anchor::{LinePos, Side};
        let mut s = with_threads(Mode::Dark);
        let target = ComposeTarget::Line {
            path: "src/point.rs".into(),
            start: LinePos {
                side: Side::Right,
                line: 14,
            },
            end: LinePos {
                side: Side::Right,
                line: 14,
            },
        };
        let mut compose = Compose::new(
            &s.theme,
            target,
            "```suggestion\n    pub fn zero() -> Self {\n```",
        );
        compose.preview = Some(Preview {
            start_line: 14,
            original: vec!["    pub fn origin() -> Self {".into()],
            suggested: vec!["    pub fn zero() -> Self {".into()],
        });
        s.overlay = Some(crate::state::Overlay::Compose(Box::new(compose)));
        insta::assert_snapshot!(render(&s));
    }

    #[test]
    fn submit_dialog_light() {
        let mut s = with_threads(Mode::Light);
        let mut dialog = crate::review::SubmitDialog::new(&s.theme);
        dialog.cycle(true);
        dialog.input.insert_str("Nice work overall.");
        s.overlay = Some(crate::state::Overlay::Submit(Box::new(dialog)));
        insta::assert_snapshot!(render(&s));
    }

    /// Intra-line emphasis, code moved between files, and a
    /// formatting-only change folded away.
    fn better_than_github(mode: Mode) -> State {
        let regular = (0o100644, 0o100644);
        let files = vec![
            file(
                FileStatus::Modified,
                Some("src/math.rs"),
                Some("src/math.rs"),
                regular,
            ),
            file(
                FileStatus::Modified,
                Some("src/util.rs"),
                Some("src/util.rs"),
                regular,
            ),
        ];
        let helper = "pub fn clamp(value: i32, low: i32, high: i32) -> i32 {\n    value.max(low).min(high)\n}\n";
        let math_old = format!(
            "pub fn area(width: u32, height: u32) -> u32 {{\n    width * height\n}}\n\n{helper}\nlet total = compute(alpha, beta);\n// a\n// b\n// c\n// d\ncall(\n    one,\n    two\n);\n"
        );
        let math_new = "pub fn area(width: u32, height: u32) -> u32 {\n    width * height\n}\n\nlet total = compute(alpha, gamma);\n// a\n// b\n// c\n// d\ncall(one, two);\n";
        let util_old = "// helpers\n";
        let util_new = format!("// helpers\n{helper}");
        let mut doc = Doc::new(files, &HashSet::new());
        doc.set_diff(
            0,
            Arc::new(FileDiff::compute(
                "src/math.rs",
                Some(math_old.as_bytes()),
                Some(math_new.as_bytes()),
            )),
        );
        doc.set_diff(
            1,
            Arc::new(FileDiff::compute(
                "src/util.rs",
                Some(util_old.as_bytes()),
                Some(util_new.as_bytes()),
            )),
        );
        let texts: Vec<_> = doc
            .files()
            .iter()
            .map(|f| f.text().unwrap().clone())
            .collect();
        let moves = ghtui_diff::moves::detect_moves(&[
            (0, &texts[0], texts[0].lines(ghtui_diff::Whitespace::Exact)),
            (1, &texts[1], texts[1].lines(ghtui_diff::Whitespace::Exact)),
        ]);
        assert_eq!(moves.len(), 1);
        doc.set_moves(moves);
        let mut s = state(mode, ColorDepth::TrueColor);
        s.size = (110, 30);
        open_diff(&mut s, loaded(doc), Pos::default(), Pane::Diff);
        s
    }

    #[test]
    fn better_than_github_dark() {
        insta::assert_snapshot!(render(&better_than_github(Mode::Dark)));
    }

    #[test]
    fn better_than_github_light() {
        insta::assert_snapshot!(render(&better_than_github(Mode::Light)));
    }

    #[test]
    fn diff_loading_dark() {
        let mut s = state(Mode::Dark, ColorDepth::TrueColor);
        let mut diff = DiffState::loading();
        diff.progress = Some("Receiving objects:  42% (420/1000), 1.2 MiB | 3.4 MiB/s".into());
        open_diff(&mut s, diff, Pos::default(), Pane::Diff);
        insta::assert_snapshot!(render(&s));
    }

    /// A long git error wraps instead of being cut off.
    #[test]
    fn diff_long_error_dark() {
        let mut s = state(Mode::Dark, ColorDepth::TrueColor);
        s.size = (80, 24);
        let mut diff = DiffState::loading();
        diff.progress = None;
        diff.error = Some(
            "git fetch failed: fatal: unable to access 'https://github.com/o/r.git/': \
             Could not resolve host: github.com (is the network up?)"
                .into(),
        );
        open_diff(&mut s, diff, Pos::default(), Pane::Diff);
        let screen = render(&s);
        assert!(screen.contains("network up?)"), "{screen}");
        insta::assert_snapshot!(screen);
    }

    /// Every screen and overlay, at every tiny size and a few extreme
    /// ones: rendering never panics.
    #[test]
    fn every_screen_survives_every_size() {
        use super::{
            DataKey, Overlay, Route, SearchKind, fetched, fixtures, ghtui, open, with_inbox,
            with_pr, with_repo,
        };
        use crate::browse::Data;
        let tc = ColorDepth::TrueColor;
        let pressed = |mut s: State, keys: &str| {
            press(&mut s, keys);
            s
        };
        type Build = Box<dyn Fn() -> State>;
        let builders: Vec<(&str, Build)> = vec![
            ("home", Box::new(move || with_inbox(Mode::Dark, tc))),
            ("home loading", Box::new(move || state(Mode::Dark, tc))),
            ("repo", Box::new(move || with_repo(Mode::Light, tc))),
            (
                "file",
                Box::new(move || {
                    let mut s = with_repo(Mode::Light, tc);
                    let route = Route::Blob {
                        repo: ghtui(),
                        rev: "main".into(),
                        path: "src/main.rs".into(),
                    };
                    open(&mut s, route, Data::Blob(Box::new(fixtures::blob())));
                    s
                }),
            ),
            (
                "issues",
                Box::new(move || {
                    let mut s = with_repo(Mode::Dark, tc);
                    let route = Route::Issues {
                        repo: ghtui(),
                        query: crate::route::OPEN.into(),
                    };
                    let results = fixtures::issue_results(Some("c1"));
                    open(&mut s, route, Data::Search(Box::new(results)));
                    s
                }),
            ),
            (
                "issue",
                Box::new(move || {
                    let mut s = state(Mode::Light, tc);
                    let route = Route::Issue {
                        repo: ghtui(),
                        number: 14,
                    };
                    open(
                        &mut s,
                        route,
                        Data::Issue(Some(Box::new(fixtures::issue()))),
                    );
                    s
                }),
            ),
            ("pr", Box::new(|| with_pr(Mode::Dark))),
            (
                "pr commits",
                Box::new(move || pressed(with_pr(Mode::Dark), "2")),
            ),
            (
                "profile",
                Box::new(move || {
                    let mut s = state(Mode::Light, tc);
                    let profile = Data::Profile(Box::new(fixtures::profile()));
                    open(&mut s, Route::user("octocat"), profile);
                    s
                }),
            ),
            (
                "search",
                Box::new(move || {
                    let mut s = state(Mode::Dark, tc);
                    let route = Route::Search {
                        kind: SearchKind::Repos,
                        query: "terminal".into(),
                    };
                    open(
                        &mut s,
                        route,
                        Data::Search(Box::new(fixtures::repo_results())),
                    );
                    s
                }),
            ),
            (
                "hints",
                Box::new(move || pressed(with_repo(Mode::Dark, tc), "l")),
            ),
            (
                "search box",
                Box::new(move || pressed(with_repo(Mode::Dark, tc), "/oct")),
            ),
            (
                "menu",
                Box::new(move || pressed(with_repo(Mode::Dark, tc), "<Space>")),
            ),
            (
                "which key",
                Box::new(move || pressed(with_repo(Mode::Dark, tc), "g")),
            ),
            (
                "list filter",
                Box::new(move || pressed(with_repo(Mode::Light, tc), "2/")),
            ),
            (
                "palette",
                Box::new(move || {
                    let mut s = with_inbox(Mode::Light, tc);
                    s.open_picker(crate::picker::Kind::Commands);
                    s
                }),
            ),
            (
                "go to file",
                Box::new(move || {
                    let mut s = pressed(with_repo(Mode::Light, tc), "f");
                    let files = vec!["src/main.rs".into(), "crates/ui/src/page.rs".into()];
                    fetched(
                        &mut s,
                        DataKey::Files(ghtui(), "main".into()),
                        Data::Files(Arc::new(files), false),
                    );
                    s
                }),
            ),
            (
                "diff",
                Box::new(move || screen_state(Mode::Dark, tc, Pos { file: 0, row: 4 }, Pane::Diff)),
            ),
            (
                "diff tree",
                Box::new(move || screen_state(Mode::Dark, tc, Pos { file: 2, row: 0 }, Pane::Tree)),
            ),
            (
                "diff search",
                Box::new(move || {
                    let s = screen_state(Mode::Dark, tc, Pos { file: 0, row: 4 }, Pane::Diff);
                    pressed(s, "/point")
                }),
            ),
            ("threads", Box::new(|| with_threads(Mode::Dark))),
            (
                "submit",
                Box::new(|| {
                    let mut s = with_threads(Mode::Light);
                    let dialog = crate::review::SubmitDialog::new(&s.theme);
                    s.overlay = Some(Overlay::Submit(Box::new(dialog)));
                    s
                }),
            ),
            ("moves", Box::new(|| better_than_github(Mode::Dark))),
            (
                "diff loading",
                Box::new(move || {
                    let mut s = state(Mode::Dark, tc);
                    open_diff(&mut s, DiffState::loading(), Pos::default(), Pane::Diff);
                    s
                }),
            ),
        ];
        let tiny = (0..=12).flat_map(|w| (0..=12).map(move |h| (w, h)));
        let sizes: Vec<(u16, u16)> = tiny.chain([(1, 200), (200, 1), (80, 24)]).collect();
        for (name, build) in &builders {
            for &(w, h) in &sizes {
                let mut s = build();
                update(&mut s, Msg::Resize(w, h));
                s.settle_diff();
                let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
                    terminal.draw(|frame| view(&s, frame, NOW)).unwrap();
                }));
                assert!(caught.is_ok(), "{name} panicked at {w}x{h}");
            }
        }
    }

    /// Scrolling stays cheap on a big PR: rendering only touches visible rows.
    /// Run with `cargo test --release -p ghtui -- --ignored scroll_timing`.
    #[test]
    #[ignore = "timing; run in release"]
    fn scroll_timing_on_500_files_20k_lines() {
        let regular = (0o100644, 0o100644);
        let files: Vec<ChangedFile> = (0..500)
            .map(|i| {
                let path = format!("src/m{i}.rs");
                file(FileStatus::Modified, Some(&path), Some(&path), regular)
            })
            .collect();
        let mut doc = Doc::new(files, &HashSet::new());
        let old: String = (0..60)
            .map(|i| format!("    let v{i} = compute({i}, \"x\");\n"))
            .collect();
        let new: String = old.replace("compute(", "compute_fast(");
        let diff = Arc::new(FileDiff::compute(
            "m.rs",
            Some(old.as_bytes()),
            Some(new.as_bytes()),
        ));
        for i in 0..500 {
            doc.set_diff(i, diff.clone());
        }
        let changed: u32 = doc
            .files()
            .iter()
            .map(|f| f.diff.as_ref().unwrap().additions)
            .sum();
        assert!(changed >= 20_000, "{changed}");

        let mut s = state(Mode::Dark, ColorDepth::TrueColor);
        s.size = (160, 50);
        open_diff(&mut s, loaded(doc), Pos::default(), Pane::Diff);

        let mut terminal = Terminal::new(TestBackend::new(160, 50)).unwrap();
        let frames = 2000;
        let start = std::time::Instant::now();
        for i in 0..frames {
            press(&mut s, if i % 100 < 95 { "j" } else { "<C-d>" });
            terminal.draw(|frame| view(&s, frame, NOW)).unwrap();
        }
        let per_frame = start.elapsed() / frames;
        eprintln!("update + render per frame: {per_frame:?}");
        assert!(
            per_frame < std::time::Duration::from_millis(8),
            "{per_frame:?} per frame"
        );
    }

    /// A single 50k-line file: diffing and highlighting run on the diff job's
    /// workers (never the UI task); once its result arrives, the first screen
    /// must be ready within 100ms. This measures the worker's compute plus the
    /// UI's update and first render.
    /// Run with `cargo test --release -p ghtui -- --ignored big_file_timing`.
    #[test]
    #[ignore = "timing; run in release"]
    fn big_file_timing_50k_lines() {
        use std::time::{Duration, Instant};

        let old: String = (0..50_000)
            .map(|i| format!("    let value_{i} = compute({i}, \"label\");\n"))
            .collect();
        let new = old
            .replace("compute(100,", "compute_fast(100,")
            .replace("compute(25000,", "compute_fast(25000,")
            .replace("compute(49999,", "compute_fast(49999,");

        let start = Instant::now();
        let diff = Arc::new(FileDiff::compute(
            "big.rs",
            Some(old.as_bytes()),
            Some(new.as_bytes()),
        ));
        let compute = start.elapsed();

        let regular = (0o100644, 0o100644);
        let big = file(
            FileStatus::Modified,
            Some("big.rs"),
            Some("big.rs"),
            regular,
        );
        let mut s = state(Mode::Dark, ColorDepth::TrueColor);
        s.size = (160, 50);
        let doc = Doc::new(vec![big], &HashSet::new());
        open_diff(&mut s, loaded(doc), Pos::default(), Pane::Diff);
        let mut terminal = Terminal::new(TestBackend::new(160, 50)).unwrap();

        let start = Instant::now();
        let job = s.diffs[&pr()].job;
        update(
            &mut s,
            Msg::Job(pr(), job, crate::diff_job::JobMsg::File(0, diff)),
        );
        terminal.draw(|frame| view(&s, frame, NOW)).unwrap();
        let first_screen = start.elapsed();

        // Full-file mode on 50k lines must stay interactive too.
        let start = Instant::now();
        press(&mut s, "F");
        terminal.draw(|frame| view(&s, frame, NOW)).unwrap();
        let full_file = start.elapsed();

        eprintln!("compute {compute:?}, first screen {first_screen:?}, full file {full_file:?}");
        assert!(
            compute + first_screen < Duration::from_millis(100),
            "{:?}",
            compute + first_screen
        );
        assert!(full_file < Duration::from_millis(100), "{full_file:?}");
    }
}

// ---- screenshots for design review ----------------------------------------------

/// Writes the main screens as HTML (exact cells and colors) to
/// `$GHTUI_SHOTS`, for looking at the design in a browser.
#[test]
#[ignore = "design review: GHTUI_SHOTS=dir cargo test -p ghtui screenshots -- --ignored"]
fn screenshots() {
    let Some(dir) = std::env::var_os("GHTUI_SHOTS") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let shot = |name: &str, state: &State| {
        let (w, h) = state.size;
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|frame| view(state, frame, NOW)).unwrap();
        std::fs::write(
            dir.join(format!("{name}.html")),
            to_html(terminal.backend().buffer()),
        )
        .unwrap();
    };
    let sized = |mut s: State, w: u16, h: u16| {
        update(&mut s, Msg::Resize(w, h));
        s
    };
    for (mode, tag) in [(Mode::Dark, "dark"), (Mode::Light, "light")] {
        shot(
            &format!("home_{tag}"),
            &sized(with_inbox(mode, ColorDepth::TrueColor), 120, 36),
        );
        let mut repo = sized(with_repo(mode, ColorDepth::TrueColor), 150, 40);
        let commit = |headline: &str, date: &str| ghtui_api::browse::CommitInfo {
            oid: "abc1234".into(),
            headline: headline.into(),
            author: "octocat".into(),
            date: date.into(),
        };
        fetched(
            &mut repo,
            DataKey::LastCommits(ghtui(), "HEAD".into(), String::new()),
            Data::LastCommits(std::sync::Arc::new(
                [
                    (
                        ".github".to_owned(),
                        commit("ci: run clippy on stable", "2026-09-20T10:00:00Z"),
                    ),
                    (
                        "crates".to_owned(),
                        commit("Browse GitHub like the website", "2026-10-03T10:00:00Z"),
                    ),
                    (
                        "Cargo.toml".to_owned(),
                        commit("Update dependencies", "2026-09-01T10:00:00Z"),
                    ),
                    (
                        "README.md".to_owned(),
                        commit("Document single-key navigation", "2026-10-02T10:00:00Z"),
                    ),
                ]
                .into_iter()
                .collect(),
            )),
        );
        shot(&format!("repo_{tag}"), &repo);
        shot(
            &format!("repo_narrow_{tag}"),
            &sized(with_repo(mode, ColorDepth::TrueColor), 90, 36),
        );
        let mut issues = sized(with_repo(mode, ColorDepth::TrueColor), 130, 36);
        let route = Route::Issues {
            repo: ghtui(),
            query: OPEN.into(),
        };
        let results = fixtures::issue_results(Some("c"));
        open(&mut issues, route, Data::Search(Box::new(results)));
        shot(&format!("issues_{tag}"), &issues);
        let mut issue = sized(state(mode, ColorDepth::TrueColor), 130, 40);
        let route = Route::Issue {
            repo: ghtui(),
            number: 14,
        };
        open(
            &mut issue,
            route,
            Data::Issue(Some(Box::new(fixtures::issue()))),
        );
        fetched(
            &mut issue,
            DataKey::Repo(ghtui()),
            Data::Repo(Box::new(fixtures::overview())),
        );
        shot(&format!("issue_{tag}"), &issue);
        shot(&format!("pr_{tag}"), &sized(with_pr(mode), 130, 44));
        let mut commits = sized(with_pr(mode), 120, 30);
        press(&mut commits, "2");
        shot(&format!("pr_commits_{tag}"), &commits);
        let mut file = sized(with_repo(mode, ColorDepth::TrueColor), 120, 24);
        let route = Route::Blob {
            repo: ghtui(),
            rev: "main".into(),
            path: "src/main.rs".into(),
        };
        open(&mut file, route, Data::Blob(Box::new(fixtures::blob())));
        shot(&format!("file_{tag}"), &file);
        let mut profile = sized(state(mode, ColorDepth::TrueColor), 120, 36);
        let data = Data::Profile(Box::new(fixtures::profile()));
        open(&mut profile, Route::user("octocat"), data);
        shot(&format!("profile_{tag}"), &profile);
        let mut search = sized(state(mode, ColorDepth::TrueColor), 120, 30);
        let route = Route::Search {
            kind: SearchKind::Repos,
            query: "terminal file manager".into(),
        };
        open(
            &mut search,
            route,
            Data::Search(Box::new(fixtures::repo_results())),
        );
        shot(&format!("search_{tag}"), &search);
        let mut s = sized(with_repo(mode, ColorDepth::TrueColor), 120, 36);
        s.visits = vec![crate::nav::Visit {
            url: "https://github.com/octocat".into(),
            title: "@octocat".into(),
            count: 3,
            last: NOW,
        }];
        press(&mut s, "l");
        shot(&format!("hints_{tag}"), &s);
        press(&mut s, "<Esc>/oct");
        shot(&format!("search_box_{tag}"), &s);
        press(&mut s, "<Esc><Space>");
        shot(&format!("menu_{tag}"), &s);
        press(&mut s, "<Esc>?");
        shot(&format!("help_{tag}"), &s);
    }
}

fn to_html(buf: &ratatui::buffer::Buffer) -> String {
    use ratatui::style::{Color, Modifier};
    let css = |c: Color, fallback: &str| match c {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        _ => fallback.to_owned(),
    };
    let mut out = String::from(
        "<!doctype html><meta charset=utf-8><style>body{margin:0;background:#000}pre{margin:0;font:14px 'JetBrains Mono','SF Mono',Menlo,monospace;letter-spacing:0}div{height:19px;line-height:19px;white-space:pre}span{display:inline-block;height:19px;vertical-align:top}</style><pre>",
    );
    let area = buf.area;
    for y in area.top()..area.bottom() {
        out.push_str("<div>");
        let mut skip = 0;
        for x in area.left()..area.right() {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let cell = &buf[(x, y)];
            let sym = cell.symbol();
            let w = ghtui_ui::text::width(sym).max(1);
            skip = w - 1;
            let mut style = format!(
                "color:{};background:{}",
                css(cell.fg, "#ccc"),
                css(cell.bg, "#000")
            );
            if cell.modifier.contains(Modifier::BOLD) {
                style.push_str(";font-weight:700");
            }
            if cell.modifier.contains(Modifier::ITALIC) {
                style.push_str(";font-style:italic");
            }
            if cell.modifier.contains(Modifier::UNDERLINED) {
                style.push_str(";text-decoration:underline");
            }
            let text = sym
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            let width = if w > 1 {
                format!(";display:inline-block;width:{w}ch")
            } else {
                String::new()
            };
            out.push_str(&format!("<span style=\"{style}{width}\">{text}</span>"));
        }
        out.push_str("</div>");
    }
    out.push_str("</pre>");
    out
}
