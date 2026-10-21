//! Starred lists: someone's lists, on their Stars tab, and a list's own
//! page with its repositories.

use super::*;
use crate::api::lists::{ListPage, StarList};

/// A list's page, as a link.
fn list_url(login: &str, list: &StarList) -> String {
    url::star_list(login, &list.slug)
}

/// What a list is: its count, and whether it's private.
fn about(list: &StarList) -> Vec<Seg> {
    let mut segs = vec![meta(format!("  {}", plural(list.count, "repository")))];
    if list.private {
        tag(&mut segs, "Private", Bg::SecondaryContainer);
    }
    segs
}

/// Someone's lists, above what they starred, when they have any.
pub fn star_lists(page: &mut Page, login: &str, lists: Fetched<'_, Capped<StarList>>) {
    // Nothing while they load: most people have none.
    let failed = |page: &mut Page, why: &str| {
        empty_box(page, strong("Lists"), why);
        page.blank();
    };
    let Some(lists) = lists.show_with(page, "the lists", |_| {}, failed) else {
        return;
    };
    if lists.is_empty() {
        return;
    }
    let title = counted("Lists", lists);
    list_box(page, title, Vec::new(), lists, "", |page, list| {
        item(page, list_url(login, list), |page, link| {
            let mut segs = vec![Seg::linked(list.name.clone(), Role::Link, link)];
            segs.extend(about(list));
            page.box_line(segs, Vec::new(), 0);
            if let Some(d) = &list.description {
                page.wrapped(vec![meta(d.clone())], 2, Frame::Body);
            }
        });
    });
    page.blank();
}

/// A list: its name and description, and its repositories.
pub fn star_list(page: &mut Page, login: &str, l: &ListPage, now: u64) {
    let mut title = vec![Seg::new(l.list.name.clone(), Role::Title)];
    title.extend(about(&l.list));
    page.line(title);
    if let Some(d) = &l.list.description {
        say(page, d.clone(), Role::Body);
    }
    let by = vec![
        meta("A list of "),
        login_seg(page, login, Role::Link),
        meta("'s stars"),
    ];
    page.wrapped(by, 0, Frame::None);
    page.blank();
    let title = counted("Repositories", &l.repos);
    list_box(
        page,
        title,
        Vec::new(),
        &l.repos,
        "No repositories in this list yet.",
        |page, r| repo_row(page, r, now, true),
    );
}
