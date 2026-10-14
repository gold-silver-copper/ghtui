//! Explore and topics, from what GitHub's API has: new repositories most
//! starred (said plainly as what it is, not as Trending), GitHub's
//! featured topics, and a topic's repositories.

use super::*;
use crate::api::explore::{Since, Topic, Topics};

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
        let role = if s == since { Role::Strong } else { Role::Link };
        tabs.push(link_seg(page, s.label(), url::explore(language, s), role));
    }
    page.wrapped(tabs, 0, Frame::None);
    languages(page, language, since);
    page.blank();
    page.wrapped(
        vec![Seg::new(not_trending(since), Role::Meta)],
        0,
        Frame::None,
    );
    page.blank();
    let repos = repos.pick(|r| match r {
        SearchResults::Repos(r) => Some(r),
        _ => None,
    });
    let title = format!("New and most starred, {}", since.label().to_lowercase());
    paged_list(
        page,
        &title,
        repos,
        "No repositories made in that time.",
        |page, r| {
            repo_row(page, r, cx.now, true);
        },
    );
    page.blank();
    let Some(t) = topics.show(page, "featured topics") else {
        return;
    };
    let count = match t.total.saturating_sub(t.topics.len() as u64) {
        0 => t.topics.len().to_string(),
        n => format!("{} of {}", t.topics.len(), t.topics.len() as u64 + n),
    };
    let title = vec![Seg::new(format!("Featured topics  {count}"), Role::Strong)];
    list_box(
        page,
        title,
        Vec::new(),
        &Whole(&t.topics),
        "None.",
        |page, topic| {
            topic_row(page, topic);
        },
    );
}

/// Languages to narrow it to, as Trending's menu offers the common
/// ones; another is a `g/` away (`github.com/trending/zig`).
const LANGUAGES: [&str; 12] = [
    "C",
    "C#",
    "C++",
    "Go",
    "Java",
    "JavaScript",
    "Kotlin",
    "Python",
    "Ruby",
    "Rust",
    "Swift",
    "TypeScript",
];

fn languages(page: &mut Page, language: Option<&str>, since: Since) {
    let mut segs = vec![Seg::new("Language  ", Role::Meta)];
    let here = |l: Option<&str>| match (l, language) {
        (None, None) => true,
        (Some(l), Some(on)) => l.eq_ignore_ascii_case(on),
        _ => false,
    };
    let add = |page: &mut Page, segs: &mut Vec<Seg>, l: Option<&str>| {
        if segs.len() > 1 {
            segs.push(Seg::new(" · ", Role::Meta));
        }
        let target = url::explore(l.map(str::to_lowercase).as_deref(), since);
        let role = if here(l) { Role::Strong } else { Role::Link };
        segs.push(link_seg(page, l.unwrap_or("Any"), target, role));
    };
    add(page, &mut segs, None);
    for l in LANGUAGES {
        add(page, &mut segs, Some(l));
    }
    if let Some(other) = language.filter(|l| !LANGUAGES.iter().any(|k| k.eq_ignore_ascii_case(l))) {
        add(page, &mut segs, Some(other));
    }
    segs.push(Seg::new(
        " · another: g/ github.com/trending/<language>",
        Role::Meta,
    ));
    page.wrapped(segs, 0, Frame::None);
}

/// A topic's row: its name and what it is.
fn topic_row(page: &mut Page, topic: &Topic) {
    let target = url::topic(&topic.name);
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
        page.wrapped(
            vec![Seg::new(facts.join(" · "), Role::Meta)],
            0,
            Frame::None,
        );
    }
    if !t.related.is_empty() {
        let mut segs = vec![Seg::new("Related  ", Role::Meta)];
        for (i, name) in t.related.iter().enumerate() {
            if i > 0 {
                segs.push(Seg::new(" · ", Role::Meta));
            }
            let target = url::topic(name);
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
