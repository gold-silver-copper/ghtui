//! Starred lists: someone's lists, on their Stars tab, and a list's own
//! page with its repositories.

use super::*;
use crate::api::lists::{ListPage, StarList};
use crate::route::Route;
use crate::site::Page as Site;

/// A list's page, as a link.
fn list_url(login: &str, list: &StarList) -> String {
    Route::Site(Site::StarList {
        login: login.to_owned(),
        slug: list.slug.clone(),
    })
    .url()
}

/// What a list is: its count, and whether it's private.
fn about(list: &StarList) -> Vec<Seg> {
    let mut segs = vec![Seg::new(
        format!("  {}", plural(list.count, "repository")),
        Role::Meta,
    )];
    if list.private {
        segs.push(space());
        segs.push(chip("Private", Bg::SecondaryContainer));
    }
    segs
}

/// Someone's lists, above what they starred, when they have any.
pub fn star_lists(page: &mut Page, login: &str, lists: Fetched<'_, Capped<StarList>>) {
    #[expect(clippy::disallowed_methods, reason = "said in the box")]
    let lists = match lists.text("the lists") {
        Ok(lists) if lists.is_empty() => return,
        Ok(lists) => lists,
        Err(why) => {
            empty_box(page, Seg::new("Lists", Role::Strong), &why);
            page.blank();
            return;
        }
    };
    let title = vec![Seg::new(format!("Lists  {}", count_of(lists)), Role::Strong)];
    list_box(page, title, Vec::new(), lists, "", |page, list| {
        item(page, list_url(login, list), |page, link| {
            let mut segs = vec![Seg::linked(list.name.clone(), Role::Link, link)];
            segs.extend(about(list));
            page.box_line(segs, Vec::new(), 0);
            if let Some(d) = &list.description {
                page.wrapped(vec![Seg::new(d.clone(), Role::Meta)], 2, Frame::Body);
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
        page.wrapped(vec![Seg::new(d.clone(), Role::Body)], 0, Frame::None);
    }
    let by = vec![
        Seg::new("A list of ", Role::Meta),
        login_seg(page, login, Role::Link),
        Seg::new("'s stars", Role::Meta),
    ];
    page.wrapped(by, 0, Frame::None);
    page.blank();
    let title = vec![Seg::new(
        format!("Repositories  {}", count_of(&l.repos)),
        Role::Strong,
    )];
    list_box(
        page,
        title,
        Vec::new(),
        &l.repos,
        "No repositories in this list yet.",
        |page, r| repo_row(page, r, now, true),
    );
}
