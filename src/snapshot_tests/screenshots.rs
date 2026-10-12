//! Screenshots for design review, and every screen at odd sizes.

use super::*;

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
            &sized(with_home(mode, ColorDepth::TrueColor), 120, 36),
        );
        let mut repo = sized(with_repo(mode, ColorDepth::TrueColor), 150, 40);
        let commit = |headline: &str, date: &str| crate::api::browse::CommitInfo {
            oid: "abc1234".into(),
            headline: headline.into(),
            author: crate::api::browse::Person::User(crate::api::browse::Login::unchecked(
                "octocat",
            )),
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
        shot(&format!("issues_{tag}"), &sized(with_issues(mode), 130, 36));
        shot(&format!("issue_{tag}"), &sized(with_issue(mode), 130, 40));
        shot(&format!("pr_{tag}"), &sized(with_pr(mode), 130, 44));
        let mut commits = sized(with_pr(mode), 120, 30);
        press(&mut commits, "2gt");
        shot(&format!("pr_commits_{tag}"), &commits);
        shot(&format!("file_{tag}"), &sized(with_file(mode), 120, 24));
        let profile = with_profile(mode, ProfileTab::Overview);
        shot(&format!("profile_{tag}"), &sized(profile, 120, 36));
        shot(
            &format!("search_{tag}"),
            &sized(with_repo_search(mode), 120, 30),
        );
        let mut s = sized(with_repo(mode, ColorDepth::TrueColor), 120, 36);
        s.visits = vec![crate::nav::Visit {
            url: "https://github.com/octocat".into(),
            title: "@octocat".into(),
            count: 3,
            last: NOW,
        }];
        press(&mut s, "i");
        shot(&format!("hints_{tag}"), &s);
        press(&mut s, "<Esc>g/oct");
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
            let w = crate::ui::text::width(sym).max(1);
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

/// Text at its worst: long, wide (CJK), with emoji, ZWJ sequences, flags,
/// stacked combining marks and a right-to-left override.
fn awful(n: usize) -> String {
    "漢字かな 👩‍👩‍👧‍👦 🇯🇵 e\u{301}\u{302}\u{303} \u{202e}rtl\u{202c} Ｆｕｌｌ ".repeat(n)
}

/// Stretches what's on the screens' data to its worst.
fn stretch(state: &mut State) {
    for remote in state.data.values_mut() {
        match &mut remote.data {
            Some(Data::Issue(Some(d))) => {
                d.title = awful(40);
                d.body = format!("{}\n\n```\n{}\n```", awful(30), "x".repeat(5000));
                for i in d.timeline.iter_mut() {
                    if let crate::api::timeline::Item::Comment(c) = i {
                        c.author = "a".repeat(39);
                        c.body = awful(20);
                    }
                }
            }
            Some(Data::Branches(r)) => {
                for b in &mut r.items {
                    b.name = format!("feature/{}", awful(10));
                    b.headline = Some(awful(20));
                }
            }
            Some(Data::Blob(b)) => {
                b.text = Some(format!("{}\n{}\n", "y".repeat(10_000), awful(50)));
            }
            Some(Data::Repo(o)) => {
                o.summary.description = Some(awful(30));
                o.topics = vec![awful(2); 30].into();
            }
            _ => {}
        }
    }
    state.data_gen += 1;
}

/// Every screen, with ordinary and with awful data, renders at any size
/// (one cell, a phone, a classic terminal) without panicking or drawing
/// outside the screen.
#[test]
fn every_screen_renders_at_odd_sizes() {
    type Build = fn() -> State;
    let builders: Vec<(&str, Build)> = vec![
        ("home", || with_home(Mode::Dark, ColorDepth::TrueColor)),
        ("pr", || with_pr(Mode::Dark)),
        ("repo", || with_repo(Mode::Dark, ColorDepth::TrueColor)),
        ("file", || with_file(Mode::Dark)),
        ("blame", || with_blame(Mode::Dark)),
        ("gist", || with_gist(Mode::Dark)),
        ("teams", || with_team(Mode::Dark)),
        ("advisory", || with_advisory(Mode::Dark)),
        ("wiki", || with_wiki(Mode::Dark)),
        ("issues", || with_issues(Mode::Dark)),
        ("commit", || with_commit(Mode::Dark)),
        ("checks", || with_pr_checks(Mode::Dark)),
        ("actions", || with_actions(Mode::Dark)),
        ("tabs", || with_tabs(Mode::Dark, 9)),
        ("org", || with_org(Mode::Dark)),
        ("discussion", || with_discussion(Mode::Dark)),
        ("run", || with_run(Mode::Dark)),
        ("job", || with_job(Mode::Dark, Some((3, 2)))),
        ("releases", || with_releases(Mode::Dark)),
        ("branches", || with_branches(Mode::Dark)),
        ("milestone", || with_milestone(Mode::Dark)),
        ("deployments", || with_deployments(Mode::Dark)),
        ("compare", || with_compare(Mode::Dark)),
        ("history", || with_history(Mode::Dark)),
        ("issue", || with_issue(Mode::Dark)),
        ("profile", || with_profile(Mode::Dark, ProfileTab::Overview)),
        ("search", || with_repo_search(Mode::Dark)),
    ];
    for (name, build) in builders {
        for awful_data in [false, true] {
            for (w, h) in [(1, 1), (20, 5), (80, 24), (3, 40), (200, 2)] {
                let mut state = build();
                if awful_data {
                    stretch(&mut state);
                }
                update(&mut state, Msg::Resize(w, h));
                let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
                terminal
                    .draw(|frame| view(&state, frame, NOW))
                    .unwrap_or_else(|e| panic!("{name} at {w}x{h}: {e}"));
                assert_eq!(terminal.backend().buffer().area.width, w, "{name}");
            }
        }
    }
}
