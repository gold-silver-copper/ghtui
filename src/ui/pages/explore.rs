//! Explore and topics, from what GitHub's API has: new repositories most
//! starred (said plainly as what it is, not as Trending), GitHub's
//! featured topics, and a topic's repositories.

use super::*;
use crate::api::explore::{Since, Topic, Topics};
use crate::route::Route;
use crate::site::Page as Site;

/// What Explore's list is, and why it isn't Trending.
fn not_trending(since: Since) -> String {
    format!(
        "GitHub's Trending ranks repositories by the stars they got lately, which its API doesn't give, so ghtui doesn't show it. Here instead: repositories made in {}, most starred, as GitHub's search finds them.",
        since.span()
    )
}

/// Explore: its period's tabs, its language filter, the new
/// repositories most starred, and the featured topics.
pub fn explore(
    page: &mut Page,
    language: Option<&str>,
    since: Since,
    repos: Fetched<'_, SearchResults>,
    topics: Fetched<'_, Topics>,
    cx: PageCtx<'_>,
) {
    let mut tabs = Vec::new();
    for s in Since::ALL {
        if !tabs.is_empty() {
            tabs.push(Seg::new(" · ", Role::Meta));
        }
        let target = Route::Site(Site::Explore {
            language: language.map(str::to_owned),
            since: s,
        });
        let role = if s == since { Role::Strong } else { Role::Link };
        tabs.push(link_seg(page, s.label(), target.url(), role));
    }
    page.wrapped(tabs, 0, Frame::None);
    page.blank();
    let shown = language.map_or_else(|| "Any language".to_owned(), |l| format!("language:{l}"));
    filter_field(page, &shown, cx.keys, "choose a language");
    page.blank();
    page.wrapped(vec![Seg::new(not_trending(since), Role::Meta)], 0, Frame::None);
    page.blank();
    let repos = repos.pick(|r| match r {
        SearchResults::Repos(r) => Some(r),
        _ => None,
    });
    let title = format!("New and most starred, {}", since.label().to_lowercase());
    paged_list(page, &title, repos, "No repositories made in that time.", |page, r| {
        repo_row(page, r, cx.now, true);
    });
    page.blank();
    let Some(t) = topics.show(page, "featured topics") else {
        return;
    };
    let count = match t.total.saturating_sub(t.topics.len() as u64) {
        0 => t.topics.len().to_string(),
        n => format!("{} of {}", t.topics.len(), t.topics.len() as u64 + n),
    };
    let title = vec![Seg::new(format!("Featured topics  {count}"), Role::Strong)];
    list_box(page, title, Vec::new(), &Whole(&t.topics), "None.", |page, topic| {
        topic_row(page, topic);
    });
}

/// A topic's row: its name and what it is.
fn topic_row(page: &mut Page, topic: &Topic) {
    let target = Route::Site(Site::Topic(topic.name.clone())).url();
    item(page, target, |page, link| {
        let mut segs = vec![Seg::linked(topic.title().to_owned(), Role::Link, link)];
        segs.push(Seg::new(format!("  #{}", topic.name), Role::Meta));
        page.box_line(segs, Vec::new(), 0);
        if let Some(d) = &topic.short_description {
            page.wrapped(vec![Seg::new(d.clone(), Role::Meta)], 2, Frame::Body);
        }
    });
}

/// A topic: what GitHub says of it, the topics it relates to, and its
/// repositories, most starred first.
pub fn topic(page: &mut Page, t: &Topic, repos: Fetched<'_, SearchResults>, now: u64) {
    let mut title = vec![Seg::new(t.title().to_owned(), Role::Title)];
    if t.featured {
        title.push(space());
        title.push(chip("Featured", Bg::SecondaryContainer));
    }
    page.line(title);
    if let Some(d) = t.description.as_ref().or(t.short_description.as_ref()) {
        page.wrapped(vec![Seg::new(d.clone(), Role::Body)], 0, Frame::None);
    }
    let mut facts = Vec::new();
    if let Some(by) = &t.created_by {
        facts.push(format!("Created by {by}"));
    }
    if let Some(when) = &t.released {
        facts.push(format!("Released {when}"));
    }
    if let Some(stars) = t.stars {
        facts.push(format!("☆ {}", plural(stars, "star")));
    }
    if !facts.is_empty() {
        page.wrapped(vec![Seg::new(facts.join(" · "), Role::Meta)], 0, Frame::None);
    }
    if !t.related.is_empty() {
        let mut segs = vec![Seg::new("Related  ", Role::Meta)];
        for (i, name) in t.related.iter().enumerate() {
            if i > 0 {
                segs.push(Seg::new(" · ", Role::Meta));
            }
            let target = Route::Site(Site::Topic(name.clone())).url();
            segs.push(link_seg(page, format!("#{name}"), target, Role::Link));
        }
        page.wrapped(segs, 0, Frame::None);
    }
    page.blank();
    let repos = repos.pick(|r| match r {
        SearchResults::Repos(r) => Some(r),
        _ => None,
    });
    paged_list(
        page,
        "Repositories",
        repos,
        "No repositories have this topic.",
        |page, r| repo_row(page, r, now, true),
    );
}
