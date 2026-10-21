use super::*;
use crate::api::mention::Mentionable;

fn composer(s: &State) -> &Compose {
    overlay!(s, Compose)
}

/// A comment box open on a line of o/r#7, with a thread alice wrote.
fn composing() -> (State, PrRef, Vec<Cmd>) {
    let (mut s, pr) = diff_state(120);
    let thread = thread("t", Some(1), false, false);
    diff_msg(&mut s, &pr, DiffMsg::ThreadsLoaded(Ok(vec![thread])));
    press(&mut s, "/origin<Enter>");
    let cmds = press(&mut s, "c");
    (s, pr, cmds)
}

fn completions(s: &State) -> Vec<String> {
    let choices = crate::typing::completions(s).unwrap_or_default();
    choices.into_iter().map(|c| c.label).collect()
}

/// `@` offers the people taking part first, then the repository's, as
/// you type; `#` its issues and pull requests. Tab or Enter takes one.
#[test]
fn completion_offers_participants_and_inserts_the_mention() {
    let (mut s, pr, cmds) = composing();
    let repo = pr.repo;
    let key = DataKey::Mentionable(repo);
    assert!(fixtures::asks(&cmds, &key));
    let known = Mentionable {
        users: vec!["alicia".into(), "bob".into()],
        issues: vec![(12, "Fix the parser".into()), (9, "Docs".into())],
    };
    fixtures::fetched(&mut s, key, Data::from(known));

    press(&mut s, "cc @al");
    assert_eq!(completions(&s), ["@alice", "@alicia"]);
    press(&mut s, "<Down><Tab>");
    assert_eq!(composer(&s).text(), "cc @alicia ");
    assert!(completions(&s).is_empty());

    press(&mut s, "and #pars");
    assert_eq!(completions(&s), ["#12 Fix the parser"]);
    press(&mut s, "<Enter>");
    assert_eq!(composer(&s).text(), "cc @alicia and #12 ");

    // Esc closes the list, and only the list; Enter is a new line again.
    press(&mut s, "@b<Esc>");
    assert!(completions(&s).is_empty());
    press(&mut s, "<Enter>");
    assert_eq!(composer(&s).text(), "cc @alicia and #12 @b\n");
}

/// A paste arrives as text: with its lines in the comment box, as one
/// line in a one-line box.
#[test]
fn a_paste_is_text() {
    let (mut s, _, _) = composing();
    update(&mut s, Msg::Paste("fn a() {\r\n    b()\r\n}".into()));
    assert_eq!(composer(&s).input.lines(), ["fn a() {", "    b()", "}"]);

    let (mut s, _) = diff_state(120);
    press(&mut s, "/");
    update(&mut s, Msg::Paste("pub fn\norigin".into()));
    assert_eq!(overlay!(s, Find).lines(), ["pub fn origin"]);
}

/// A long line in the comment box wraps instead of scrolling sideways:
/// all of it shows.
#[test]
fn the_comment_box_wraps() {
    use ratatui::widgets::Widget;
    let (mut s, _, _) = composing();
    let words = "every word of this comment stays in view as it wraps";
    update(&mut s, Msg::Paste(words.into()));
    let compose = composer(&s);
    let area = ratatui::layout::Rect::new(0, 0, 30, compose.rows());
    let mut buf = ratatui::buffer::Buffer::empty(area);
    crate::ui::review_sheets::ComposeSheet {
        ctx: s.ctx(0),
        title: "Comment",
        input: &compose.input,
        preview: None,
        note: None,
        error: None,
        sending: false,
        save: "add to review",
        completions: None,
    }
    .render(area, &mut buf);
    let shown: String = (buf.content.iter())
        .map(ratatui::buffer::Cell::symbol)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(shown.contains(words), "{shown}");
}

/// GitHub's answer for your saved replies.
fn saved_replies(s: &mut State, result: Result<Vec<(&str, &str)>, ApiError>) {
    use crate::api::replies::SavedReply;
    let result = result.map(|replies| {
        let replies = (replies.into_iter())
            .map(|(title, body)| SavedReply {
                title: title.into(),
                body: body.into(),
            })
            .collect::<Vec<_>>();
        Data::from(crate::api::model::Capped::from(replies))
    });
    let key = DataKey::SavedReplies;
    update(s, Msg::fetched(key, result));
}

/// ctrl-g lists your saved replies (asking GitHub for them), typing
/// narrows them, and ↵ puts the one chosen where the cursor is.
#[test]
fn a_saved_reply_goes_in_at_the_cursor() {
    let (mut s, ..) = composing();
    press(&mut s, "Hi. ");
    let cmds = press(&mut s, "<C-g>");
    let key = DataKey::SavedReplies;
    assert!(crate::fixtures::asks(&cmds, &key), "{cmds:?}");
    saved_replies(
        &mut s,
        Ok(vec![
            ("Thanks", "Thanks for the report!"),
            ("Duplicate", "Closing as a duplicate of #1."),
        ]),
    );
    let (rows, _) = crate::site::replies::shown(&s).unwrap();
    assert!(rows.contains(&"Thanks".to_owned()), "{rows:?}");
    press(&mut s, "dup");
    let (rows, selected) = crate::site::replies::shown(&s).unwrap();
    assert_eq!(rows.get(selected).map(String::as_str), Some("Duplicate"));
    press(&mut s, "<Enter>");
    assert!(crate::site::replies::shown(&s).is_none());
    assert_eq!(composer(&s).text(), "Hi. Closing as a duplicate of #1.");
}

/// Without the token's scope, the list says which, and where saved
/// replies are made (GitHub's API can't make them).
#[test]
fn saved_replies_say_what_the_token_lacks() {
    let (mut s, ..) = composing();
    press(&mut s, "<C-g>");
    let why = "Saved replies take your token's read:user scope, which it lacks: `gh auth refresh -s read:user` adds it";
    saved_replies(&mut s, Err(ApiError::Denied(why.into())));
    let (rows, _) = crate::site::replies::shown(&s).unwrap();
    assert!(
        rows.iter()
            .any(|r| r.contains("gh auth refresh -s read:user")),
        "{rows:?}"
    );
    assert!(
        rows.iter()
            .any(|r| r.contains("github.com/settings/replies")),
        "{rows:?}"
    );
    // esc closes the list, not the composer.
    press(&mut s, "<Esc>");
    assert!(crate::site::replies::shown(&s).is_none());
    assert_eq!(composer(&s).text(), "");
}
