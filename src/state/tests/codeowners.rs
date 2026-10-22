//! Who owns a file: on its page, from its revision's CODEOWNERS, and in a
//! pull request's files, from its base branch's.

use crate::api::codeowners::{Codeowners, OwnersError};
use crate::diff_screen::Pane;
use crate::ui::diff_doc::Pos;

use super::*;

fn blob(path: &str) -> Route {
    Route::Code(code::Blob {
        repo: ghtui(),
        rev: "main".into(),
        path: path.into(),
        lines: None,
    })
}

/// A file's page names its owners, by the rule that owns it; the
/// CODEOWNERS file's page shows the problems GitHub found in it.
#[test]
fn a_file_names_its_owners_and_the_file_its_problems() {
    let s = at(with_repo(), blob("src/main.rs"));
    let text = page_text(&s);
    assert!(
        text.contains("Owned by @octo-org/core, @hubot  (.github/CODEOWNERS, line 2)"),
        "{text}"
    );

    let mut s = at(with_repo(), blob(".github/CODEOWNERS"));
    assert!(page_text(&s).contains("GitHub found no problems in this file"));
    let wrong = Codeowners {
        errors: Some(vec![OwnersError {
            line: 3,
            column: 13,
            kind: "Unknown owner".into(),
            message: "@octo-org/docs isn't a team".into(),
            suggestion: Some("make sure the team exists".into()),
        }]),
        ..fixtures::codeowners()
    };
    let key = DataKey::Codeowners(ghtui(), "main".into());
    fetched(&mut s, key, Data::from(wrong));
    let text = page_text(&s);
    assert!(
        text.contains("GitHub found 1 problem in this file"),
        "{text}"
    );
    assert!(
        text.contains(
            "Line 3, column 13 · Unknown owner: @octo-org/docs isn't a team (make sure the team exists)"
        ),
        "{text}"
    );
}

/// A pull request's files read its base branch's owners once it's
/// loaded, and each file's header names them.
#[test]
fn a_pull_requests_files_name_their_owners() {
    let mut s = screen_state(Pos::default(), Pane::Diff);
    let pr = s.screen().diff().and_then(|d| d.of.pr()).unwrap().clone();
    let cmds = fetched(
        &mut s,
        DataKey::Pr(pr.clone()),
        Data::from(fixtures::pr_detail()),
    );
    let key = DataKey::Codeowners(pr.repo, "main".into());
    assert!(fixtures::asks(&cmds, &key), "{cmds:?}");
    fetched(&mut s, key, Data::from(fixtures::codeowners()));
    let screen = screen_text(&s);
    let header = screen.lines().find(|l| l.contains("src/point.rs"));
    assert!(
        header.is_some_and(|l| l.contains("@octo-org/core @hubot")),
        "{screen}"
    );
}
