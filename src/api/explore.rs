//! Explore, from what GitHub's API has of it: topics, and searches for
//! what's new and starred. GitHub's Trending page ranks repositories by
//! the stars they got lately, which its API doesn't give, so ghtui
//! doesn't show that list (and says so) rather than read the website.

use serde::{Deserialize, Serialize};

use crate::api::{ApiError, GitHub};

/// A topic, as GitHub describes it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Topic {
    /// Its name in searches (`rust`).
    pub name: String,
    /// As GitHub titles it (`Rust`).
    pub display_name: Option<String>,
    pub short_description: Option<String>,
    /// Its longer description, as text.
    pub description: Option<String>,
    pub created_by: Option<String>,
    pub released: Option<String>,
    /// GitHub features it on its Topics page.
    pub featured: bool,
    /// GitHub wrote its description.
    pub curated: bool,
    /// How many starred it, once GraphQL says.
    pub stars: Option<u64>,
    /// The topics GitHub relates to it.
    pub related: Vec<String>,
}

impl Topic {
    /// Its title: GitHub's name for it, else its own.
    pub fn title(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.name)
    }
}

/// GitHub's featured topics.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Topics {
    pub topics: Vec<Topic>,
    /// How many GitHub features.
    pub total: u64,
}

words! {
    /// How far back "new" goes, as GitHub's Trending periods.
    #[derive(Hash, Default, Serialize, Deserialize)]
    pub enum Since {
        /// As GitHub's URLs say it (`?since=weekly`).
        pub fn slug -> pub fn from_slug,
        /// As its tab says it.
        pub fn label,
        /// In a sentence: "made in the past week".
        pub fn span,
    }
    /// As GitHub's Trending starts.
    #[default]
    Daily   "daily"   "Today"      "the past day";
    Weekly  "weekly"  "This week"  "the past week";
    Monthly "monthly" "This month" "the past month";
}

impl Since {
    fn days(self) -> u64 {
        match self {
            Since::Daily => 1,
            Since::Weekly => 7,
            Since::Monthly => 30,
        }
    }
}

/// The search for repositories made since `since`, in `language` if
/// given, most starred first: the nearest GitHub's API comes to
/// Trending. `now` in unix seconds; the date is a day's, so the search
/// (and its cache) holds for the day.
pub fn new_and_starred(language: Option<&str>, since: Since, now: u64) -> String {
    let from = now.saturating_sub(since.days() * 86_400);
    let day = crate::ui::time::iso8601(from);
    let day = day.get(..10).unwrap_or(&day);
    let language = language.map(str::trim).filter(|l| !l.is_empty());
    let language = match language {
        Some(l) if l.contains(' ') => format!(" language:\"{l}\""),
        Some(l) => format!(" language:{l}"),
        None => String::new(),
    };
    format!("created:>{day}{language} sort:stars")
}

/// The search for a topic's repositories, most starred first.
pub fn topic_repos(topic: &str) -> String {
    format!("topic:{topic} sort:stars")
}

impl GitHub {
    /// GitHub's featured topics.
    pub async fn featured_topics(&self) -> Result<Topics, ApiError> {
        let path = "/search/topics?q=is:featured&per_page=100";
        let found: wire::Found = self.rest_json(path, "GitHub's featured topics").await?;
        let mut topics: Vec<Topic> = (found.items.into_iter())
            .map(wire::Topic::into_topic)
            .collect();
        topics.sort_by_cached_key(|t| t.title().to_lowercase());
        Ok(Topics {
            topics,
            total: found.total_count,
        })
    }

    /// A topic: what GitHub says of it, its stars and related topics. A
    /// topic nobody described is only its name.
    pub async fn topic(&self, name: &str) -> Result<Topic, ApiError> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Graph {
            stargazer_count: u64,
            related_topics: Vec<crate::api::browse::wire::Name>,
        }
        let path = format!(
            "/search/topics?q={}&per_page=10",
            url::form_urlencoded::byte_serialize(name.as_bytes()).collect::<String>()
        );
        let vars = serde_json::json!({ "name": name });
        let (found, graph) = tokio::join!(
            self.rest_json::<wire::Found>(&path, name),
            self.graphql_json::<Graph>(crate::api::raw::TOPIC, vars, name, "/topic"),
        );
        let mut topic = (found?.items.into_iter())
            .find(|t| t.name.eq_ignore_ascii_case(name))
            .map_or_else(
                || Topic {
                    name: name.to_owned(),
                    ..Topic::default()
                },
                wire::Topic::into_topic,
            );
        match graph {
            Ok(g) => {
                topic.stars = Some(g.stargazer_count);
                topic.related = g.related_topics.into_iter().map(|t| t.name).collect();
            }
            Err(err) => tracing::debug!(%err, "no stars or related topics for the topic"),
        }
        Ok(topic)
    }
}

/// HTML reduced to its text, as a topic's description comes.
fn text_of(html: &str) -> String {
    let mut out = String::new();
    let mut tag = false;
    for c in html.chars() {
        match c {
            '<' => tag = true,
            '>' => tag = false,
            c if !tag => out.push(c),
            _ => {}
        }
    }
    let out = out
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

mod wire {
    use serde::Deserialize;

    #[derive(Deserialize)]
    pub struct Found {
        pub total_count: u64,
        pub items: Vec<Topic>,
    }

    #[derive(Deserialize)]
    pub struct Topic {
        pub name: String,
        pub display_name: Option<String>,
        pub short_description: Option<String>,
        pub description: Option<String>,
        pub created_by: Option<String>,
        pub released: Option<String>,
        #[serde(default)]
        pub featured: bool,
        #[serde(default)]
        pub curated: bool,
    }

    impl Topic {
        pub fn into_topic(self) -> super::Topic {
            let some = |s: Option<String>| s.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
            super::Topic {
                name: self.name,
                display_name: some(self.display_name),
                short_description: some(self.short_description),
                description: some(self.description.map(|d| super::text_of(&d))),
                created_by: some(self.created_by),
                released: some(self.released),
                featured: self.featured,
                curated: self.curated,
                stars: None,
                related: Vec::new(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_and_starred_searches_from_a_day() {
        let now = crate::ui::time::parse_iso8601("2026-10-12T15:00:00Z").unwrap();
        assert_eq!(
            new_and_starred(Some("rust"), Since::Weekly, now),
            "created:>2026-10-05 language:rust sort:stars"
        );
        assert_eq!(
            new_and_starred(Some("Visual Basic .NET"), Since::Daily, now),
            "created:>2026-10-11 language:\"Visual Basic .NET\" sort:stars"
        );
        assert_eq!(
            new_and_starred(None, Since::Monthly, now),
            "created:>2026-09-12 sort:stars"
        );
    }

    #[test]
    fn a_description_is_its_text() {
        assert_eq!(
            text_of("<p>Rust is a <em>systems</em> language &amp; more.</p>\n"),
            "Rust is a systems language & more."
        );
    }
}
