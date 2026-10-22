//! Every screen the fixtures reach, drawn in both schemes at a few
//! sizes, tiny ones among them. Drawing never panics,
//! and in debug builds every color pair drawn is one the theme declares,
//! so its contrast is tested.

use super::*;
use crate::api::security::AlertKind;
use crate::api::settings::SettingsPage;
use crate::ui::pages::InsightsTab;

fn pressed(mut s: State, keys: &str) -> State {
    press(&mut s, keys);
    s
}

/// The screens and overlays [`pages`] and the keys' screens leave out.
pub(super) fn more() -> Vec<(&'static str, State)> {
    use super::diff::screen::{better_than_github, diff_at, diff_pr, open_diff, with_threads};
    let mut loading = state();
    let _ = loading.load_visible(false);
    let mut palette = with_home();
    let _ = palette.open_picker(crate::picker::Kind::Commands);
    let mut go_to_file = pressed(with_repo(), "f");
    let files = vec!["src/main.rs".into(), "crates/ui/src/page.rs".into()];
    let files = Data::Files(std::sync::Arc::new(files), false);
    fetched(
        &mut go_to_file,
        DataKey::Files(ghtui(), "main".into()),
        files,
    );
    let mut submit = with_threads();
    let dialog = crate::review::SubmitDialog::new(&submit.theme, diff_pr(&submit));
    submit.overlay = Some(Overlay::Submit(Box::new(dialog)));
    let mut diff_loading = state();
    let at = crate::ui::diff_doc::Pos::default();
    open_diff(&mut diff_loading, DiffState::fresh(), at, Pane::Diff);
    let mut overflowing = with_tabs(12);
    let _ = overflowing.switch_to_tab(6);
    let mut menu = with_repo();
    crate::nav::open_menu(&mut menu);
    let merge = pressed(with_pr(), "M");
    let mut search_box = with_repo();
    super::act(&mut search_box, crate::keymap::Action::SearchGithub);
    press(&mut search_box, "oct");
    vec![
        ("home loading", loading),
        ("palette", palette),
        ("go to file", go_to_file),
        ("menu", menu),
        ("hints", pressed(with_repo(), "i")),
        ("search box", search_box),
        ("find on the page", pressed(with_repo(), "/oct")),
        ("list filter", pressed(with_repo(), "2/")),
        ("log search", pressed(with_job(None), "/")),
        ("comment on an issue", pressed(with_issue(), "c")),
        ("merge confirm", merge),
        ("tabs", with_tabs(3)),
        ("tabs overflowing", overflowing),
        ("organization", with_org()),
        ("followers", with_profile(ProfileTab::Followers)),
        ("diff search", pressed(diff_at(0, 4), "/point")),
        ("threads", with_threads()),
        ("reply", pressed(with_threads(), "c")),
        ("comment on lines", pressed(diff_at(0, 4), "s<Down>c")),
        ("submit", submit),
        ("moves", better_than_github()),
        ("diff loading", diff_loading),
        ("settings", super::settings::on(SettingsPage::Branches)),
        ("webhook", super::settings::on(SettingsPage::Hook(11))),
        ("secrets", super::settings::on(SettingsPage::Secrets)),
        ("variables", super::settings::on(SettingsPage::Variables)),
        ("pages", super::settings::on(SettingsPage::Pages)),
        ("caches", super::settings::on(SettingsPage::Caches)),
        ("properties", super::settings::on(SettingsPage::Properties)),
        (
            "alerts",
            super::security::with_alerts(AlertKind::Dependabot),
        ),
        (
            "alert",
            super::security::with_alert(AlertKind::CodeScanning, false),
        ),
        ("insights", super::site::insights(InsightsTab::Contributors)),
        ("explore", super::site::explore()),
        ("package", super::site::package()),
    ]
}

#[test]
fn every_screen_draws_at_every_size_in_both_schemes() {
    let tiny = (0..=12)
        .step_by(2)
        .flat_map(|w| (0..=12).step_by(3).map(move |h| (w, h)));
    let sizes: Vec<(u16, u16)> = tiny
        .chain([(1, 200), (200, 1), (80, 24), (160, 50)])
        .collect();
    for mode in [Mode::Light, Mode::Dark] {
        let keys = super::keys::screens().map(|(name, make)| (name, make()));
        for (name, mut s) in pages().into_iter().chain(keys).chain(more()) {
            s.theme = Theme::new(DEFAULT_SEED, mode, ColorDepth::TrueColor);
            for &(w, h) in &sizes {
                update(&mut s, Msg::Resize(w, h));
                let _ = s.settle_diff();
                let drawn = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| draw(&s)));
                assert!(drawn.is_ok(), "{name} panicked at {w}x{h} in {mode:?}");
            }
        }
    }
}

/// Building a page reads only what the page says it needs, so a page
/// whose needs haven't changed shows the same thing.
#[test]
fn every_page_reads_only_what_it_needs() {
    let keys = super::keys::screens().map(|(name, make)| (name, make()));
    let mut wrong = Vec::new();
    for (name, s) in pages().into_iter().chain(keys).chain(more()) {
        let Screen::Page(p) = s.screen() else { continue };
        let route = &p.route;
        let (_, reads) = crate::browse::reads::during(|| s.build_page(route, 100, NOW));
        let needs = s.needs(route);
        for key in reads {
            if !needs.contains(&key) && !route.family().also_reads(&key) {
                wrong.push(format!("{name} ({route:?}) reads {key:?}"));
            }
        }
    }
    wrong.dedup();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
