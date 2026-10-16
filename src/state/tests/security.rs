use crate::api::security::{AlertChange, AlertKind, AlertRef};
use crate::route::security;

use super::*;
use crate::act::{By, Subject};
use crate::security::fixtures;

fn alert_ref(kind: AlertKind, number: u64) -> AlertRef {
    AlertRef {
        repo: ghtui(),
        kind,
        number,
    }
}

/// A repository's alerts of `kind`, open ones first.
pub(super) fn with_alerts(kind: AlertKind) -> State {
    let (repo, query) = (ghtui(), OPEN.into());
    at(
        with_repo(),
        Route::Security(security::Alerts { repo, kind, query }),
    )
}

/// An alert's page.
pub(super) fn with_alert(kind: AlertKind, dismissed: bool) -> State {
    let mut s = with_repo();
    let route = Route::Security(security::Alert {
        repo: ghtui(),
        kind,
        number: 2,
    });
    let alert = if dismissed {
        fixtures::dismissed(kind, 2)
    } else {
        fixtures::alert(kind, 2)
    };
    open(&mut s, route, Data::Alert(Box::new(alert)));
    s
}

/// Each kind's page says what matters about it: the fixed version and
/// Dependabot's pull request, the file and line a tool flagged (opening
/// it at that commit), where a secret is; and who closed it and why.
#[test]
fn an_alert_says_what_fixes_it_and_where_it_is() {
    let s = with_alert(AlertKind::Dependabot, true);
    let text = page_text(&s);
    for part in [
        "Patched 0.8.6",
        "#12 Bump rand from 0.8.5 to 0.8.6",
        "Closed by octocat",
        "A fix has already been started",
        "Tracked in #40.",
    ] {
        assert!(text.contains(part), "{part} in {text}");
    }
    let s = with_alert(AlertKind::CodeScanning, false);
    let text = page_text(&s);
    assert!(text.contains("src/main.rs:10 on main at 75c61df"), "{text}");
    let s = with_alert(AlertKind::SecretScanning, false);
    let text = page_text(&s);
    assert!(text.contains("config/dev.env:3"), "{text}");
    assert!(text.contains("Found in: issue comment"), "{text}");
}

/// The Security tab's own menu leads to each kind's list, and `t`
/// toggles a list between open, closed and all, as other lists do.
#[test]
fn alert_lists_are_reached_from_the_security_tab_and_filter() {
    let s = super::with_advisories();
    let text = page_text(&s);
    assert!(
        text.contains("Advisories · Dependabot · Code scanning · Secret scanning"),
        "{text}"
    );
    let mut s2 = with_alerts(AlertKind::CodeScanning);
    press(&mut s2, "t");
    assert_eq!(
        s2.route(),
        Some(&Route::Security(security::Alerts {
            repo: ghtui(),
            kind: AlertKind::CodeScanning,
            query: "is:closed".into(),
        }))
    );
    let mut s3 = with_alerts(AlertKind::Dependabot);
    press(&mut s3, "S");
    says(&s3, "Alerts are newest first");
}

/// `,x` on an open alert lists GitHub's reasons; what's typed is the
/// comment. The choice goes to GitHub, and the page follows it until the
/// alert shows dismissed.
#[test]
fn an_alert_is_dismissed_with_a_reason_and_followed() {
    let mut s = with_alert(AlertKind::CodeScanning, false);
    assert!(press(&mut s, ",x").is_empty());
    let Some(Overlay::Picker(picker)) = &s.overlay else {
        panic!("no picker");
    };
    let labels: Vec<String> = (s.picker_rows(picker).into_iter())
        .map(|(r, _)| r.label)
        .collect();
    assert_eq!(labels, ["False positive", "Won't fix", "Used in tests"]);
    let sent = press(&mut s, "only in fixtures<Down><Down><Enter>");
    let a = alert_ref(AlertKind::CodeScanning, 2);
    let change = Change::Alert(AlertChange {
        alert: a.clone(),
        dismiss: Some(("used in tests".into(), "only in fixtures".into())),
    });
    let about = Subject::Alert(a.clone());
    assert_eq!(sent, vec![crate::fixtures::change(change, By::Act(about))]);
    answer(&mut s, &sent, Ok(()));
    says(&s, "Alert dismissed: Used in tests");
    let key = DataKey::Alert(a);
    let open = fixtures::alert(AlertKind::CodeScanning, 2);
    fetched(&mut s, key.clone(), Data::Alert(Box::new(open)));
    assert!(s.following.is_some());
    let dismissed = fixtures::dismissed(AlertKind::CodeScanning, 2);
    fetched(&mut s, key, Data::Alert(Box::new(dismissed)));
    assert!(s.following.is_none());
    assert!(page_text(&s).contains("Dismissed"));
}

/// A closed alert reopens after asking; a secret's reasons are its own.
#[test]
fn a_closed_alert_reopens_after_asking() {
    let mut s = with_alert(AlertKind::SecretScanning, true);
    press(&mut s, ",x");
    let Some(Overlay::Ask(confirm)) = &s.overlay else {
        panic!("no confirmation");
    };
    assert_eq!(confirm.title, "Reopen secret scanning alert #2?");
    let sent = press(&mut s, "<Enter>");
    let change = Change::Alert(AlertChange {
        alert: alert_ref(AlertKind::SecretScanning, 2),
        dismiss: None,
    });
    assert_eq!(*crate::fixtures::sent_change(&sent).0, change);
    let mut s = with_alert(AlertKind::SecretScanning, false);
    press(&mut s, ",x");
    let Some(Overlay::Picker(picker)) = &s.overlay else {
        panic!("no picker");
    };
    let first = s
        .picker_rows(picker)
        .into_iter()
        .next()
        .map(|(r, _)| r.label);
    assert_eq!(first.as_deref(), Some("Revoked"));
}

/// On a list, `,x` is about the selected row's alert, loaded first.
#[test]
fn a_rows_alert_loads_then_is_dismissed() {
    let mut s = with_alerts(AlertKind::Dependabot);
    let cmds = press(&mut s, ",x");
    let key = DataKey::Alert(alert_ref(AlertKind::Dependabot, 2));
    assert!(crate::fixtures::asks(&cmds, &key), "{cmds:?}");
    let alert = fixtures::alert(AlertKind::Dependabot, 2);
    fetched(&mut s, key, Data::Alert(Box::new(alert)));
    assert!(matches!(s.overlay, Some(Overlay::Picker(_))));
}
