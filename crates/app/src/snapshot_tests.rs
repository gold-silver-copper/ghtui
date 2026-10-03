//! Full-screen snapshots through ratatui's `TestBackend`, in light and dark
//! schemes (and 256 colors). Snapshots include cell styles, so color changes
//! show up in review. Rendering also exercises the theme's debug assertion
//! that every fg/bg pair used is declared (and therefore contrast-tested).

use crossterm::event::KeyCode;
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

use crate::browse::{Data, DataKey};
use crate::fixtures;
use crate::keymap::Keymap;
use crate::route::{OPEN, Route};
use crate::state::{Msg, Overlay, State, new_palette, update};
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
    detail()
}

fn detail() -> PrDetail {
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
    update(&mut state, Msg::Pr(pr.clone(), Box::new(Ok(detail()))));
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
            fresh: true,
        },
    );
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
    update(&mut state, Msg::Key(key(KeyCode::Char('j'))));
    update(&mut state, Msg::Key(key(KeyCode::Char('j'))));
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
    update(&mut state, Msg::Key(key(KeyCode::Char('G'))));
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
    let (rev, path) = ("main".to_owned(), "src/main.rs".to_owned());
    state.push(Route::Blob {
        repo: ghtui(),
        rev: rev.clone(),
        path: path.clone(),
    });
    fetched(
        &mut state,
        DataKey::Blob(ghtui(), rev, path),
        Data::Blob(Box::new(fixtures::blob())),
    );
    insta::assert_snapshot!(render(&state));
}

#[test]
fn issues_dark() {
    let mut state = with_repo(Mode::Dark, ColorDepth::TrueColor);
    let route = Route::Issues {
        repo: ghtui(),
        query: OPEN.into(),
    };
    let (kind, query) = route.search().unwrap();
    state.push(route);
    fetched(
        &mut state,
        DataKey::Search(kind, query),
        Data::Search(Box::new(fixtures::issue_results(Some("c1")))),
    );
    insta::assert_snapshot!(render(&state));
}

#[test]
fn issue_light() {
    let mut state = state(Mode::Light, ColorDepth::TrueColor);
    state.push(Route::Issue {
        repo: ghtui(),
        number: 14,
    });
    fetched(
        &mut state,
        DataKey::Issue(ghtui(), 14),
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
    update(&mut state, Msg::Key(key(KeyCode::Char('2'))));
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
    state.push(Route::user("octocat"));
    fetched(
        &mut state,
        DataKey::Profile("octocat".into()),
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
    let (kind, query) = route.search().unwrap();
    state.push(route);
    fetched(
        &mut state,
        DataKey::Search(kind, query),
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
    let (kind, query) = route.search().unwrap();
    state.push(route);
    fetched(
        &mut state,
        DataKey::Search(kind, query),
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
    update(&mut state, Msg::Key(key(KeyCode::Char('f'))));
    insta::assert_snapshot!("hints_dark", render(&state));
    update(&mut state, Msg::Key(key(KeyCode::Esc)));
    update(&mut state, Msg::Key(key(KeyCode::Char('/'))));
    insta::assert_snapshot!("search_empty_dark", render(&state));
    for c in "oct".chars() {
        update(&mut state, Msg::Key(key(KeyCode::Char(c))));
    }
    insta::assert_snapshot!("search_typed_dark", render(&state));
    update(&mut state, Msg::Key(key(KeyCode::Esc)));
    update(&mut state, Msg::Key(key(KeyCode::Char('.'))));
    insta::assert_snapshot!("menu_dark", render(&state));
    update(&mut state, Msg::Key(key(KeyCode::Esc)));
    update(&mut state, Msg::Key(key(KeyCode::Char('g'))));
    insta::assert_snapshot!("which_key_dark", render(&state));
}

#[test]
fn go_to_file_light() {
    let mut state = with_repo(Mode::Light, ColorDepth::TrueColor);
    update(&mut state, Msg::Key(key(KeyCode::Char('t'))));
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
    for c in "main".chars() {
        update(&mut state, Msg::Key(key(KeyCode::Char(c))));
    }
    insta::assert_snapshot!(render(&state));
}

#[test]
fn list_filter_light() {
    let mut state = with_repo(Mode::Light, ColorDepth::TrueColor);
    update(&mut state, Msg::Key(key(KeyCode::Char('2'))));
    update(&mut state, Msg::Key(key(KeyCode::Char('/'))));
    insta::assert_snapshot!(render(&state));
}

#[test]
fn profile_stars_dark() {
    let mut state = state(Mode::Dark, ColorDepth::TrueColor);
    state.push(Route::User {
        login: "octocat".into(),
        tab: ghtui_ui::pages::ProfileTab::Stars,
    });
    fetched(
        &mut state,
        DataKey::Profile("octocat".into()),
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
fn help_overlay_dark() {
    let mut state = with_inbox(Mode::Dark, ColorDepth::TrueColor);
    state.overlay = Some(Overlay::Help);
    insta::assert_snapshot!(render(&state));
}

#[test]
fn palette_light() {
    let mut state = with_inbox(Mode::Light, ColorDepth::TrueColor);
    let mut palette = new_palette(&state.theme);
    palette.input.insert_str("ratatui");
    state.overlay = Some(Overlay::Palette(Box::new(palette)));
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
            state.overlay = Some(Overlay::Help);
            render(&state);
            let mut state = with_repo(mode, ColorDepth::TrueColor);
            update(&mut state, Msg::Resize(w, h));
            render(&state);
        }
    }
}

fn key(code: KeyCode) -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
}

// ---- diff screen -----------------------------------------------------------

pub(crate) fn diff_fixture() -> crate::diff_screen::DiffState {
    diff::diff_state()
}

pub(crate) mod diff {
    use std::collections::HashSet;
    use std::sync::Arc;

    use ghtui_api::model::PrRef;
    use ghtui_diff::FileDiff;
    use ghtui_git::files::{ChangedFile, FileStatus, ZERO_OID};
    use ghtui_git::repo::PrRefs;
    use ghtui_theme::{ColorDepth, Mode};
    use ghtui_ui::diff_doc::{Doc, Pos};

    use super::{render, state};
    use crate::diff_screen::{DiffScreen, DiffState, Pane};
    use crate::state::Screen;

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
        let mut state = DiffState::loading();
        state.set_files(
            PrRefs {
                head: "h".into(),
                base: "b".into(),
                merge_base: "m".into(),
            },
            doc,
        );
        state
    }

    fn screen_state(
        mode: Mode,
        depth: ColorDepth,
        cursor: Pos,
        focus: Pane,
    ) -> crate::state::State {
        let mut s = state(mode, depth);
        s.size = (110, 34);
        let pr = PrRef::parse("o/r#7").unwrap();
        s.diffs.insert(pr.clone(), diff_state());
        let mut screen = DiffScreen::new(pr, 110);
        screen.cursor = cursor;
        screen.focus = focus;
        s.screens.push(Screen::Diff(Box::new(screen)));
        let content = s.content_area();
        let crate::state::State { screens, diffs, .. } = &mut s;
        if let Some(Screen::Diff(screen)) = screens.last_mut() {
            crate::diff_screen::settle(screen, diffs.get_mut(&screen.pr).unwrap(), content);
        }
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
        settle(&mut s);
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
        let pr = PrRef::parse("o/r#7").unwrap();
        let diff = s.diffs.get_mut(&pr).unwrap();
        let hash = diff.doc.files[0].blocks()[0].hash.clone();
        diff.set_review(ghtui_store::ReviewState {
            reviewed_hunks: vec![hash],
            ..Default::default()
        });
        diff.doc.set_viewed(1, ghtui_ui::diff_doc::Viewed::Viewed);
        settle(&mut s);
        insta::assert_snapshot!(render(&s));
    }

    fn settle(s: &mut crate::state::State) {
        let content = s.content_area();
        let crate::state::State { screens, diffs, .. } = s;
        if let Some(Screen::Diff(screen)) = screens.last_mut() {
            crate::diff_screen::settle(screen, diffs.get_mut(&screen.pr).unwrap(), content);
        }
    }

    fn with_threads(mode: Mode) -> crate::state::State {
        use ghtui_api::model::{ReviewComment, ReviewThread, Side};
        let mut s = screen_state(
            mode,
            ColorDepth::TrueColor,
            Pos { file: 0, row: 0 },
            Pane::Diff,
        );
        let pr = PrRef::parse("o/r#7").unwrap();
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
        let diff = s.diffs.get_mut(&pr).unwrap();
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
        settle(&mut s);
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
    fn better_than_github(mode: Mode) -> crate::state::State {
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
            .files
            .iter()
            .map(|f| f.text().unwrap().clone())
            .collect();
        let moves = ghtui_diff::moves::detect_moves(&[
            (0, &texts[0], texts[0].lines(ghtui_diff::Whitespace::Exact)),
            (1, &texts[1], texts[1].lines(ghtui_diff::Whitespace::Exact)),
        ]);
        assert_eq!(moves.len(), 1);
        doc.set_moves(moves);
        let mut diff = DiffState::loading();
        diff.set_files(
            PrRefs {
                head: "h".into(),
                base: "b".into(),
                merge_base: "m".into(),
            },
            doc,
        );
        let mut s = state(mode, ColorDepth::TrueColor);
        s.size = (110, 30);
        let pr = PrRef::parse("o/r#7").unwrap();
        s.diffs.insert(pr.clone(), diff);
        let mut screen = DiffScreen::new(pr, 110);
        screen.cursor = Pos { file: 0, row: 0 };
        s.screens.push(Screen::Diff(Box::new(screen)));
        settle(&mut s);
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
        let pr = PrRef::parse("o/r#7").unwrap();
        let mut diff = DiffState::loading();
        diff.progress = Some("Receiving objects:  42% (420/1000), 1.2 MiB | 3.4 MiB/s".into());
        s.diffs.insert(pr.clone(), diff);
        s.screens
            .push(Screen::Diff(Box::new(DiffScreen::new(pr, 100))));
        insta::assert_snapshot!(render(&s));
    }
}

/// Scrolling stays cheap on a big PR: rendering only touches visible rows.
/// Run with `cargo test --release -p ghtui -- --ignored scroll_timing`.
#[test]
#[ignore = "timing; run in release"]
fn scroll_timing_on_500_files_20k_lines() {
    use std::collections::HashSet;
    use std::sync::Arc;

    use ghtui_api::model::PrRef;
    use ghtui_diff::FileDiff;
    use ghtui_git::files::{ChangedFile, FileStatus};
    use ghtui_git::repo::PrRefs;
    use ghtui_ui::diff_doc::Doc;

    use crate::diff_screen::{DiffScreen, DiffState};
    use crate::state::{Screen, update};

    let files: Vec<ChangedFile> = (0..500)
        .map(|i| ChangedFile {
            status: FileStatus::Modified,
            old_path: Some(format!("src/m{i}.rs")),
            new_path: Some(format!("src/m{i}.rs")),
            old_mode: 0o100644,
            new_mode: 0o100644,
            old_oid: "1".repeat(40),
            new_oid: "2".repeat(40),
            similarity: None,
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
        .files
        .iter()
        .map(|f| f.diff.as_ref().unwrap().additions)
        .sum();
    assert!(changed >= 20_000, "{changed}");

    let mut s = state(Mode::Dark, ColorDepth::TrueColor);
    s.size = (160, 50);
    let pr = PrRef::parse("o/r#1").unwrap();
    let mut diff_state = DiffState::loading();
    diff_state.set_files(
        PrRefs {
            head: "h".into(),
            base: "b".into(),
            merge_base: "m".into(),
        },
        doc,
    );
    s.diffs.insert(pr.clone(), diff_state);
    s.screens
        .push(Screen::Diff(Box::new(DiffScreen::new(pr, 160))));

    let mut terminal = Terminal::new(TestBackend::new(160, 50)).unwrap();
    let frames = 2000;
    let start = std::time::Instant::now();
    for i in 0..frames {
        let key = if i % 100 < 95 { 'j' } else { 'd' };
        let event = if key == 'd' {
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('d'),
                crossterm::event::KeyModifiers::CONTROL,
            )
        } else {
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('j'),
                crossterm::event::KeyModifiers::NONE,
            )
        };
        update(&mut s, Msg::Key(event));
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
    use std::collections::HashSet;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use ghtui_api::model::PrRef;
    use ghtui_diff::FileDiff;
    use ghtui_git::files::{ChangedFile, FileStatus};
    use ghtui_git::repo::PrRefs;
    use ghtui_ui::diff_doc::Doc;

    use crate::diff_screen::{DiffScreen, DiffState};
    use crate::state::{Screen, update};

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

    let file = ChangedFile {
        status: FileStatus::Modified,
        old_path: Some("big.rs".into()),
        new_path: Some("big.rs".into()),
        old_mode: 0o100644,
        new_mode: 0o100644,
        old_oid: "1".repeat(40),
        new_oid: "2".repeat(40),
        similarity: None,
    };
    let mut s = state(Mode::Dark, ColorDepth::TrueColor);
    s.size = (160, 50);
    let pr = PrRef::parse("o/r#1").unwrap();
    let mut diff_state = DiffState::loading();
    diff_state.set_files(
        PrRefs {
            head: "h".into(),
            base: "b".into(),
            merge_base: "m".into(),
        },
        Doc::new(vec![file], &HashSet::new()),
    );
    s.diffs.insert(pr.clone(), diff_state);
    s.screens
        .push(Screen::Diff(Box::new(DiffScreen::new(pr.clone(), 160))));
    let mut terminal = Terminal::new(TestBackend::new(160, 50)).unwrap();

    let start = Instant::now();
    update(&mut s, Msg::FileDiff(pr.clone(), 0, diff));
    terminal.draw(|frame| view(&s, frame, NOW)).unwrap();
    let first_screen = start.elapsed();

    // Full-file mode on 50k lines must stay interactive too.
    let start = Instant::now();
    update(
        &mut s,
        Msg::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('F'),
            crossterm::event::KeyModifiers::NONE,
        )),
    );
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
