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
    let tabs = Since::ALL
        .into_iter()
        .map(|s| (s.label(), url::explore(language, s), s == since));
    let tabs = nav_strip(page, &meta(" · "), &Role::Link, tabs);
    page.wrapped(tabs, 0, Frame::None);
    languages(page, language, since);
    page.blank();
    say(page, not_trending(since), Role::Meta);
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
    let title = vec![strong(format!("Featured topics  {count}"))];
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
    let here = |l: Option<&str>| match (l, language) {
        (None, None) => true,
        (Some(l), Some(on)) => l.eq_ignore_ascii_case(on),
        _ => false,
    };
    let other = language.filter(|l| !LANGUAGES.iter().any(|k| k.eq_ignore_ascii_case(l)));
    let tabs = std::iter::once(None)
        .chain(LANGUAGES.iter().copied().map(Some))
        .chain(other.map(Some))
        .map(|l| {
            let target = url::explore(l.map(str::to_lowercase).as_deref(), since);
            (l.unwrap_or("Any"), target, here(l))
        });
    let mut segs = vec![meta("Language  ")];
    segs.extend(nav_strip(page, &meta(" · "), &Role::Link, tabs));
    segs.push(meta(" · another: g/ github.com/trending/<language>"));
    page.wrapped(segs, 0, Frame::None);
}

/// A topic's row: its name and what it is.
fn topic_row(page: &mut Page, topic: &Topic) {
    let target = url::topic(&topic.name);
    item(page, target, |page, link| {
        let mut segs = vec![Seg::linked(topic.title().to_owned(), Role::Link, link)];
        segs.push(meta(format!("  #{}", topic.name)));
        page.box_line(segs, Vec::new(), 0);
        if let Some(d) = &topic.short_description {
            page.wrapped(vec![meta(d.clone())], 2, Frame::Body);
        }
    });
}

/// A topic: what GitHub says of it, the topics it relates to, and its
/// repositories, most starred first.
pub fn topic(page: &mut Page, t: &Topic, repos: Fetched<'_, SearchResults>, now: u64) {
    let mut title = vec![Seg::new(t.title().to_owned(), Role::Title)];
    if t.featured {
        tag(&mut title, "Featured", Bg::SecondaryContainer);
    }
    page.line(title);
    if let Some(d) = t.description.as_ref().or(t.short_description.as_ref()) {
        say(page, d.clone(), Role::Body);
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
        say(page, facts.join(" · "), Role::Meta);
    }
    if !t.related.is_empty() {
        let tabs = t
            .related
            .iter()
            .map(|name| (format!("#{name}"), url::topic(name), false));
        let mut segs = vec![meta("Related  ")];
        segs.extend(nav_strip(page, &meta(" · "), &Role::Link, tabs));
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
