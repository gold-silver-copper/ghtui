//! Notifications.

use super::*;

/// Your notifications, as GitHub's inbox lists them: what each is about,
/// why you got it and when, the unread ones marked. Words the inbox can't
/// be filtered by are said, and so is how far a filter applied after
/// fetching searched.
pub fn notifications(page: &mut Page, query: &str, inbox: Fetched<'_, Inbox>, cx: PageCtx<'_>) {
    let PageCtx { icons, keys, now } = cx;
    filter_field(page, query, keys, "filter");
    let ignored = InboxFilter::parse(query).ignored;
    let knows = "notifications filter by is:unread, is:read, reason: and repo:";
    ignored_note(page, &ignored, knows);
    page.blank();
    let Some(inbox) = inbox.show(page, "your notifications") else { return };
    let l = &inbox.list;
    let shown = list_state(query);
    let tabs = [("all", "All"), ("unread", "Unread"), ("read", "Read")]
        .map(|(state, label)| (label, Link::State(state.to_owned()), state == shown));
    let title = nav_strip(page, &Seg::new("   ", Role::Body), &Role::Meta, tabs);
    let empty = if query.is_empty() {
        "All caught up: no notifications."
    } else {
        "No notifications match this filter."
    };
    let count = vec![meta(count_of(l))];
    list_box(page, title, count, l, empty, |page, n| {
        item(page, n.url.clone(), |page, link| {
            let (mark, role) = if n.unread {
                ("● ", Role::Strong)
            } else {
                ("  ", Role::Body)
            };
            page.box_line(
                vec![
                    Seg::new(mark, Role::Accent),
                    meta(format!("{} ", subject_icon(&n.kind, icons))),
                    Seg::linked(n.title.clone(), role, link),
                ],
                vec![meta(time::ago_iso(&n.updated_at, now))],
                0,
            );
            let subline = format!("{} · {}", n.repo, reason_label(&n.reason));
            page.box_line(vec![meta(subline)], Vec::new(), 4);
        });
    });
    // The filter applies after fetching, and the search stopped short.
    if inbox.cut {
        let text = format!(
            "Searched {} notifications, {} match: Load more searches further.",
            inbox.searched,
            l.items.len()
        );
        page.blank();
        say(page, text, Role::Meta);
    }
}

/// What a notification is about, as an icon.
pub(super) fn subject_icon(kind: &SubjectKind, icons: Icons) -> &'static str {
    let (nerd, plain) = match kind {
        SubjectKind::PullRequest => return icons.pr_state(PrState::Open),
        SubjectKind::Discussion => return icons.comment(),
        SubjectKind::Issue => ("\u{f41b}", "◉"),
        SubjectKind::Release => ("\u{f412}", "◇"),
        SubjectKind::Commit => ("\u{f417}", "◦"),
        SubjectKind::CheckSuite => ("\u{f04b}", "▷"),
        SubjectKind::Alert => ("\u{f421}", "△"),
        SubjectKind::Other(_) => ("·", "·"),
    };
    if icons.nerd_font { nerd } else { plain }
}
