use super::*;
use crate::api::mention::Mentionable;
use crate::review::Compose;

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
    assert!(cmds.contains(&Cmd::Api(Api::FetchMentionable(repo.clone()))));
    let known = Mentionable {
        users: vec!["alicia".into(), "bob".into()],
        issues: vec![(12, "Fix the parser".into()), (9, "Docs".into())],
    };
    update(&mut s, Msg::Mentionable(repo, Ok(known)));

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
