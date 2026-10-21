//! Notifications: your inbox, over REST (GraphQL has none). The inbox
//! filters as GitHub's does (`is:unread reason:mention repo:o/r`); REST
//! filters by repository, read state and participation itself, and the
//! rest is filtered here.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::api::browse::{Results, filter_words};
use crate::api::model::RepoId;

/// What a notification is about, as GitHub types it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubjectKind {
    Issue,
    PullRequest,
    Release,
    Discussion,
    Commit,
    CheckSuite,
    /// A Dependabot or vulnerability alert.
    Alert,
    /// Another type, by GitHub's name for it.
    Other(String),
}

impl SubjectKind {
    fn of(kind: &str) -> SubjectKind {
        match kind {
            "Issue" => SubjectKind::Issue,
            "PullRequest" => SubjectKind::PullRequest,
            "Release" => SubjectKind::Release,
            "Discussion" => SubjectKind::Discussion,
            "Commit" => SubjectKind::Commit,
            "CheckSuite" => SubjectKind::CheckSuite,
            "RepositoryVulnerabilityAlert" | "RepositoryDependabotAlertsThread" => {
                SubjectKind::Alert
            }
            other => SubjectKind::Other(other.to_owned()),
        }
    }
}

/// A thread in your inbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notification {
    /// The thread's ID, which marking it read or done takes.
    pub id: String,
    pub repo: RepoId,
    pub kind: SubjectKind,
    pub title: String,
    /// Why you got it, in the API's word (`review_requested`).
    pub reason: String,
    pub updated_at: String,
    pub unread: bool,
    /// Where it opens: its subject's page, at the latest comment.
    pub url: String,
}

/// The inbox a filter lists, as far as it's loaded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inbox {
    pub list: Results<Notification>,
    /// How many notifications were looked through for these: more than
    /// the list holds when the filter applies after fetching.
    pub searched: u64,
    /// The last fetch stopped at its limit, short of a full page and of
    /// the end of the inbox.
    pub cut: bool,
}

/// A thread as it was when marked: by its ID, and when GitHub said it
/// was last updated.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ThreadAt {
    pub id: String,
    pub updated_at: String,
}

impl ThreadAt {
    pub fn of(n: &Notification) -> ThreadAt {
        ThreadAt {
            id: n.id.clone(),
            updated_at: n.updated_at.clone(),
        }
    }
}

/// The threads you marked done, kept per account: GitHub's API goes on
/// listing a thread marked done, read, though it has left github.com's
/// inbox. ghtui leaves such a thread out until GitHub updates it again,
/// and then it's back, as on github.com.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Done {
    /// When GitHub said each was last updated, as it was marked done.
    threads: BTreeMap<String, String>,
}

impl Done {
    /// Where it's kept, in the account's cache.
    pub const KEY: &'static str = "notifications:done";
    /// How many it remembers: the most recently updated.
    const KEEP: usize = 2000;

    pub fn mark(&mut self, threads: &[ThreadAt]) {
        for t in threads {
            self.threads.insert(t.id.clone(), t.updated_at.clone());
        }
        let excess = self.threads.len().saturating_sub(Self::KEEP);
        if excess > 0 {
            let mut times: Vec<String> = self.threads.values().cloned().collect();
            times.sort_unstable();
            if let Some(oldest) = times.get(excess) {
                self.threads.retain(|_, at| *at >= *oldest);
            }
        }
    }

    /// Whether `n` was marked done since GitHub last updated it.
    pub fn hides(&self, n: &Notification) -> bool {
        (self.threads.get(&n.id)).is_some_and(|at| n.updated_at.as_str() <= at.as_str())
    }

    /// Forgets the threads among `seen` that GitHub updated since they
    /// were marked done; whether any were.
    pub fn forget_updated(&mut self, seen: &[Notification]) -> bool {
        let before = self.threads.len();
        for n in seen {
            if !self.hides(n) {
                self.threads.remove(&n.id);
            }
        }
        self.threads.len() != before
    }
}

/// How many notifications are unread, as polled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unread {
    /// The unread on the one page asked for.
    pub count: u64,
    /// More are unread past that page.
    pub more: bool,
    /// When GitHub asks to be asked again, in seconds.
    pub poll_secs: u64,
}

/// Why GitHub notifies, as the API names it, and as its inbox says it.
pub const REASONS: [(&str, &str); 15] = [
    ("approval_requested", "Approval requested"),
    ("assign", "Assigned"),
    ("author", "Author"),
    ("ci_activity", "CI activity"),
    ("comment", "Comment"),
    ("invitation", "Invitation"),
    ("manual", "Subscribed"),
    ("member_feature_requested", "Feature requested"),
    ("mention", "Mentioned"),
    ("review_requested", "Review requested"),
    ("security_advisory_credit", "Advisory credit"),
    ("security_alert", "Security alert"),
    ("state_change", "State change"),
    ("subscribed", "Watching"),
    ("team_mention", "Team mentioned"),
];

/// A reason as the inbox says it.
pub fn reason_label(reason: &str) -> String {
    REASONS.iter().find(|(r, _)| *r == reason).map_or_else(
        || reason.replace('_', " "),
        |(_, label)| (*label).to_owned(),
    )
}

/// The inbox's filter in GitHub's words, as REST takes it.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct InboxFilter {
    /// `is:unread` (true) or `is:read`; the last one wins.
    pub unread: Option<bool>,
    /// `reason:participating`: threads you take part in.
    pub participating: bool,
    /// Any of these reasons, in the API's words.
    pub reasons: Vec<String>,
    pub repo: Option<RepoId>,
    /// Words the inbox can't be filtered by.
    pub ignored: Vec<String>,
}

impl InboxFilter {
    pub fn parse(query: &str) -> InboxFilter {
        let mut filter = InboxFilter::default();
        for word in filter_words(query) {
            match word.split_once(':').unwrap_or_default() {
                ("is", "unread") => filter.unread = Some(true),
                ("is", "read") => filter.unread = Some(false),
                ("reason", "participating") => filter.participating = true,
                ("reason", reason) => {
                    // The website writes `review-requested`.
                    let reason = reason.replace('-', "_");
                    if REASONS.iter().any(|(r, _)| *r == reason) {
                        filter.reasons.push(reason);
                    } else {
                        filter.ignored.push(word);
                    }
                }
                ("repo", repo) => match RepoId::parse(repo) {
                    Some(repo) => filter.repo = Some(repo),
                    None => filter.ignored.push(word),
                },
                _ => filter.ignored.push(word),
            }
        }
        filter
    }

    /// The REST path of page `page`, `per_page` threads a page.
    pub fn path(&self, page: u64, per_page: u64) -> String {
        let list = match &self.repo {
            Some(repo) => format!("/repos/{}/{}/notifications", repo.owner, repo.name),
            None => "/notifications".to_owned(),
        };
        // Read threads too, unless only unread ones are asked for.
        let all = if self.unread == Some(true) {
            ""
        } else {
            "all=true&"
        };
        let participating = if self.participating {
            "participating=true&"
        } else {
            ""
        };
        format!("{list}?{all}{participating}per_page={per_page}&page={page}")
    }

    /// Whether the filter applies after fetching (a reason, `is:read`),
    /// as REST can't apply it.
    pub fn after_fetching(&self) -> bool {
        !self.reasons.is_empty() || self.unread == Some(false)
    }

    /// Whether a thread REST sent is one the filter keeps.
    pub fn keeps(&self, n: &Notification) -> bool {
        self.unread.is_none_or(|unread| n.unread == unread)
            && (self.reasons.is_empty() || self.reasons.contains(&n.reason))
    }
}

/// Where a notification opens: its subject's page on GitHub (its API
/// URL turned into the website's), at its latest comment when it has one.
/// What has no API URL (a discussion, a check suite, an alert) opens the
/// list it's in.
pub fn web_url(
    repo: &RepoId,
    kind: &SubjectKind,
    subject: Option<&str>,
    latest: Option<&str>,
) -> String {
    let base = crate::ui::pages::url::repo(repo);
    let page = match api_path(subject).as_slice() {
        ["issues", n] => Some(format!("{base}/issues/{n}")),
        ["pulls", n] => Some(format!("{base}/pull/{n}")),
        ["commits", sha] => Some(format!("{base}/commit/{sha}")),
        ["discussions", n] => Some(format!("{base}/discussions/{n}")),
        _ => None,
    };
    let Some(page) = page else {
        return match kind {
            SubjectKind::Release => format!("{base}/releases"),
            SubjectKind::Discussion => format!("{base}/discussions"),
            SubjectKind::CheckSuite => format!("{base}/actions"),
            SubjectKind::Alert => format!("{base}/security/dependabot"),
            // Where it is accepted.
            SubjectKind::Other(k) if k == "RepositoryInvitation" => format!("{base}/invitations"),
            _ => base,
        };
    };
    let anchor = match api_path(latest.filter(|l| Some(*l) != subject)).as_slice() {
        ["issues", "comments", id] => format!("#issuecomment-{id}"),
        ["pulls", "comments", id] => format!("#discussion_r{id}"),
        ["pulls", _, "reviews", id] => format!("#pullrequestreview-{id}"),
        ["comments", id] => format!("#commitcomment-{id}"),
        _ => String::new(),
    };
    format!("{page}{anchor}")
}

/// The segments of an API URL after `/repos/<owner>/<name>/`.
fn api_path(url: Option<&str>) -> Vec<&str> {
    let Some((_, rest)) = url.and_then(|u| u.split_once("/repos/")) else {
        return Vec::new();
    };
    rest.split('/').filter(|s| !s.is_empty()).skip(2).collect()
}

/// REST's notifications.
pub(crate) mod rest {
    use serde::Deserialize;

    use super::{Notification, SubjectKind, web_url};
    use crate::api::model::RepoId;

    #[derive(Deserialize)]
    pub struct Thread {
        pub id: String,
        pub unread: bool,
        pub reason: String,
        pub updated_at: String,
        pub subject: Subject,
        pub repository: Repository,
    }

    #[derive(Deserialize)]
    pub struct Subject {
        pub title: String,
        #[serde(rename = "type")]
        pub kind: String,
        pub url: Option<String>,
        pub latest_comment_url: Option<String>,
    }

    pub use crate::api::browse::rest_search::Repo as Repository;

    impl Thread {
        /// The thread, if its repository's name is one.
        pub fn into_notification(self) -> Option<Notification> {
            let repo = RepoId::parse(&self.repository.full_name)?;
            let kind = SubjectKind::of(&self.subject.kind);
            let url = web_url(
                &repo,
                &kind,
                self.subject.url.as_deref(),
                self.subject.latest_comment_url.as_deref(),
            );
            Some(Notification {
                id: self.id,
                repo,
                kind,
                title: self.subject.title,
                reason: self.reason,
                updated_at: self.updated_at,
                unread: self.unread,
                url,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A subject's API URL opens its page on github.com, at its latest
    /// comment; what has none opens its list.
    #[test]
    fn subjects_open_their_pages() {
        let repo = RepoId::new("o", "r");
        let api = |path: &str| format!("https://api.github.com/repos/o/r/{path}");
        let open = |kind: SubjectKind, subject: Option<&str>, latest: Option<&str>| {
            web_url(&repo, &kind, subject, latest)
        };
        let (issue, pull) = (api("issues/3"), api("pulls/7"));
        let commit = api("commits/abc123");
        for (kind, subject, latest, url) in [
            (SubjectKind::Issue, Some(&issue), None, "issues/3"),
            (SubjectKind::Issue, Some(&issue), Some(&issue), "issues/3"),
            (
                SubjectKind::Issue,
                Some(&issue),
                Some(&api("issues/comments/11")),
                "issues/3#issuecomment-11",
            ),
            (
                SubjectKind::PullRequest,
                Some(&pull),
                Some(&api("issues/comments/12")),
                "pull/7#issuecomment-12",
            ),
            (
                SubjectKind::PullRequest,
                Some(&pull),
                Some(&api("pulls/comments/13")),
                "pull/7#discussion_r13",
            ),
            (
                SubjectKind::PullRequest,
                Some(&pull),
                Some(&api("pulls/7/reviews/14")),
                "pull/7#pullrequestreview-14",
            ),
            (
                SubjectKind::Commit,
                Some(&commit),
                Some(&api("comments/15")),
                "commit/abc123#commitcomment-15",
            ),
            (
                SubjectKind::Release,
                Some(&api("releases/99")),
                None,
                "releases",
            ),
            (SubjectKind::Discussion, None, None, "discussions"),
            (SubjectKind::CheckSuite, None, None, "actions"),
            (SubjectKind::Alert, None, None, "security/dependabot"),
        ] {
            let expected = format!("https://github.com/o/r/{url}");
            assert_eq!(
                open(
                    kind,
                    subject.map(String::as_str),
                    latest.map(String::as_str)
                ),
                expected
            );
        }
        let invitation = SubjectKind::Other("RepositoryInvitation".into());
        assert_eq!(
            open(invitation, None, None),
            "https://github.com/o/r/invitations"
        );
        let other = SubjectKind::Other("TeamDiscussion".into());
        assert_eq!(open(other, None, None), "https://github.com/o/r");
    }
}
