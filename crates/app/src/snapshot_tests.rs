//! Full-screen snapshots through ratatui's `TestBackend`, in light and dark
//! schemes (and 256 colors). Snapshots include cell styles, so color changes
//! show up in review. Rendering also exercises the theme's debug assertion
//! that every fg/bg pair used is declared (and therefore contrast-tested).

use ghtui_api::model::{
    ChecksState, Inbox, Label, Mergeable, PrDetail, PrRef, PrState, PrSummary, ReviewDecision,
};
use ghtui_api::rate_limit::{Bucket, RateLimits};
use ghtui_theme::{ColorDepth, DEFAULT_SEED, Mode, Theme};
use ghtui_ui::Icons;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use crate::keymap::Keymap;
use crate::state::{Msg, Overlay, Remote, State, new_palette, update};
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
    state
}

fn with_pr(mode: Mode) -> State {
    let mut state = with_inbox(mode, ColorDepth::TrueColor);
    let pr = PrRef::parse("gold-silver-copper/ghtui#12").unwrap();
    state.prs.insert(pr.clone(), Remote::cached(Some(detail())));
    state.open_pr(pr.clone());
    update(&mut state, Msg::Pr(pr, Box::new(Ok(detail()))));
    state
}

fn render(state: &State) -> String {
    let (w, h) = state.size;
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|frame| view(state, frame, NOW)).unwrap();
    format!("{:?}", terminal.backend().buffer())
}

#[test]
fn inbox_dark() {
    insta::assert_snapshot!(render(&with_inbox(Mode::Dark, ColorDepth::TrueColor)));
}

#[test]
fn inbox_light() {
    insta::assert_snapshot!(render(&with_inbox(Mode::Light, ColorDepth::TrueColor)));
}

#[test]
fn inbox_dark_256() {
    insta::assert_snapshot!(render(&with_inbox(Mode::Dark, ColorDepth::Ansi256)));
}

#[test]
fn inbox_second_row_selected_light() {
    let mut state = with_inbox(Mode::Light, ColorDepth::TrueColor);
    state.screens[0] = crate::state::Screen::Inbox {
        selected: 1,
        scroll: 0,
    };
    insta::assert_snapshot!(render(&state));
}

#[test]
fn inbox_loading_dark() {
    let mut state = state(Mode::Dark, ColorDepth::TrueColor);
    state.load_visible(false);
    insta::assert_snapshot!(render(&state));
}

#[test]
fn inbox_error_light() {
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
fn pr_dark() {
    insta::assert_snapshot!(render(&with_pr(Mode::Dark)));
}

#[test]
fn pr_light() {
    insta::assert_snapshot!(render(&with_pr(Mode::Light)));
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
fn help_overlay_dark() {
    let mut state = with_inbox(Mode::Dark, ColorDepth::TrueColor);
    state.overlay = Some(Overlay::Help);
    insta::assert_snapshot!(render(&state));
}

#[test]
fn palette_light() {
    let mut state = with_inbox(Mode::Light, ColorDepth::TrueColor);
    let mut palette = new_palette(&state.theme);
    palette.input.insert_str("op");
    state.overlay = Some(Overlay::Palette(Box::new(palette)));
    insta::assert_snapshot!(render(&state));
}

#[test]
fn narrow_terminal_does_not_panic() {
    for (w, h) in [(1, 1), (9, 2), (20, 5), (40, 10)] {
        for mode in [Mode::Light, Mode::Dark] {
            let mut state = with_pr(mode);
            state.size = (w, h);
            render(&state);
            state.screens.truncate(1);
            render(&state);
            state.overlay = Some(Overlay::Help);
            render(&state);
        }
    }
}

// ---- diff screen -----------------------------------------------------------

mod diff {
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

    pub(super) fn diff_state() -> DiffState {
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
