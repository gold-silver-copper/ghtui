//! Page data and key presses for tests and snapshots.

use crossterm::event::KeyEvent;
use ghtui_api::browse::{
    Blob, Comment, CommitInfo, EntryKind, IssueDetail, IssueState, IssueSummary, PrActivity,
    Profile, Readme, RepoOverview, RepoSummary, Results, ReviewSummary, SearchResults, TreeEntry,
    UserSummary,
};
use ghtui_api::model::{Label, NodeId, RepoId};

use crate::state::{Cmd, Msg, State, update};

/// Presses `keys`, in vim notation (`"jj"`, `"<Esc>/"`, `"<C-d>"`).
pub(crate) fn press(state: &mut State, keys: &str) -> Vec<Cmd> {
    let mut cmds = Vec::new();
    for key in crate::keymap::parse_sequence(keys).unwrap() {
        cmds.extend(update(state, Msg::Key(KeyEvent::new(key.code, key.mods))));
    }
    cmds
}

pub fn repo_summary(repo: &str, stars: u64) -> RepoSummary {
    RepoSummary {
        repo: RepoId::parse(repo).unwrap(),
        description: Some("A keyboard-driven terminal client for GitHub".into()),
        stars,
        forks: stars / 20,
        language: Some("Rust".into()),
        language_color: Some("#dea584".into()),
        pushed_at: Some("2026-10-02T12:00:00Z".into()),
        private: false,
        fork: false,
        archived: false,
    }
}

fn entry(path: &str, kind: EntryKind, size: Option<u64>) -> TreeEntry {
    TreeEntry {
        name: path.rsplit('/').next().unwrap().into(),
        path: path.into(),
        kind,
        size,
    }
}

pub const README: &str = "# ghtui\n\nA **keyboard-driven** terminal client for [GitHub](https://github.com), \
with a diff viewer that goes [beyond GitHub's](docs/diff.md).\n\n\
## Install\n\n```sh\ncargo install ghtui\n```\n\n\
- [x] Browse repositories, issues and pull requests\n- [ ] Actions\n\n\
> Everything is a link: press `Enter` to follow it.\n";

pub fn overview() -> RepoOverview {
    RepoOverview {
        summary: repo_summary("gold-silver-copper/ghtui", 1234),
        homepage: Some("https://ghtui.dev".into()),
        watchers: 18,
        open_issues: 7,
        open_prs: 3,
        closed_issues: 41,
        closed_prs: 112,
        license: Some("MIT".into()),
        topics: vec!["tui".into(), "github".into(), "rust".into()],
        default_branch: Some("main".into()),
        last_commit: Some(CommitInfo {
            oid: "fedcba9876543210fedcba9876543210fedcba98".into(),
            headline: "Browse GitHub like the website".into(),
            author: "octocat".into(),
            date: "2026-10-03T10:00:00Z".into(),
        }),
        commits: 412,
        parent: None,
        starred: false,
        id: NodeId::new("R_ghtui"),
        has_issues: true,
        entries: vec![
            entry(".github", EntryKind::Dir, None),
            entry("crates", EntryKind::Dir, None),
            entry("Cargo.toml", EntryKind::File, Some(1530)),
            entry("README.md", EntryKind::File, Some(2210)),
        ],
        readme: Some(Readme {
            path: "README.md".into(),
            text: README.into(),
        }),
    }
}

/// The latest commit touching each of [`overview`]'s entries.
pub fn last_commits() -> std::collections::HashMap<String, CommitInfo> {
    let commit = |headline: &str, date: &str| CommitInfo {
        oid: "0123456789abcdef0123456789abcdef01234567".into(),
        headline: headline.into(),
        author: "octocat".into(),
        date: date.into(),
    };
    [
        (
            ".github",
            commit("Run CI on macOS too", "2026-09-20T10:00:00Z"),
        ),
        (
            "crates",
            commit("Browse GitHub like the website", "2026-10-03T10:00:00Z"),
        ),
        (
            "Cargo.toml",
            commit("Bump ratatui to 0.30", "2026-09-28T10:00:00Z"),
        ),
        (
            "README.md",
            commit("Document the keys", "2026-10-01T10:00:00Z"),
        ),
    ]
    .into_iter()
    .map(|(name, c)| (name.to_owned(), c))
    .collect()
}

pub fn tree() -> Vec<TreeEntry> {
    vec![
        entry("crates/api", EntryKind::Dir, None),
        entry("crates/ui", EntryKind::Dir, None),
        entry("crates/README.md", EntryKind::File, Some(120)),
    ]
}

pub fn blob() -> Blob {
    Blob {
        path: "src/main.rs".into(),
        text: Some(
            "use std::io;\n\n/// Says hello.\nfn main() -> io::Result<()> {\n    println!(\"hello, {}\", 42);\n    Ok(())\n}\n"
                .into(),
        ),
        size: 96,
        truncated: false,
    }
}

fn label(name: &str, color: &str) -> Label {
    Label {
        name: name.into(),
        color: color.into(),
    }
}

pub fn issue_summary(number: u64, is_pr: bool, state: IssueState) -> IssueSummary {
    IssueSummary {
        repo: RepoId::new("gold-silver-copper", "ghtui"),
        number,
        title: format!("Issue {number}: pages wrap at the terminal width"),
        is_pr,
        state,
        author: "octocat".into(),
        updated_at: "2026-10-03T09:00:00Z".into(),
        comments: number % 3,
        labels: if number.is_multiple_of(2) {
            vec![label("bug", "d73a4a")]
        } else {
            Vec::new()
        },
        created_at: "2026-09-29T09:00:00Z".into(),
        review: None,
        checks: None,
    }
}

pub fn issue_results(next: Option<&str>) -> SearchResults {
    SearchResults::Issues(Results {
        total: 7,
        items: vec![
            issue_summary(14, false, IssueState::Open),
            issue_summary(11, false, IssueState::Open),
            issue_summary(8, false, IssueState::Open),
        ],
        next: next.map(str::to_owned),
    })
}

pub fn issue() -> IssueDetail {
    IssueDetail {
        repo: RepoId::new("gold-silver-copper", "ghtui"),
        number: 14,
        title: "Pages wrap at the terminal width".into(),
        body: "On a 200-column terminal README lines get **very** long.\n\nGitHub caps the width; we should too.".into(),
        state: IssueState::Open,
        author: "octocat".into(),
        created_at: "2026-10-01T12:00:00Z".into(),
        labels: vec![label("bug", "d73a4a"), label("ui", "1d76db")],
        assignees: vec!["hubot".into()],
        comments: vec![Comment {
            author: "hubot".into(),
            body: "Agreed. 120 columns, centered?".into(),
            created_at: "2026-10-02T12:00:00Z".into(),
        }],
        id: NodeId::new("I_14"),
    }
}

pub fn activity() -> PrActivity {
    PrActivity {
        id: NodeId::new("PR_12"),
        comments: vec![Comment {
            author: "hubot".into(),
            body: "Does this cover the 256-color fallback?".into(),
            created_at: "2026-10-01T09:00:00Z".into(),
        }],
        reviews: vec![
            ReviewSummary {
                author: "monalisa".into(),
                state: "approved".into(),
                body: String::new(),
                submitted_at: "2026-10-02T09:00:00Z".into(),
            },
            ReviewSummary {
                author: "hubot".into(),
                state: "commented".into(),
                body: "Yes: every role is checked at both depths.".into(),
                submitted_at: "2026-10-01T10:00:00Z".into(),
            },
        ],
        commits: vec![
            CommitInfo {
                oid: "0a1b2c3d4e5f60718293a4b5c6d7e8f901234567".into(),
                headline: "Theme: derive syntax colors from the seed".into(),
                author: "octocat".into(),
                date: "2026-09-30T12:00:00Z".into(),
            },
            CommitInfo {
                oid: "1b2c3d4e5f60718293a4b5c6d7e8f9012345678a".into(),
                headline: "Check every syntax role against diff backgrounds".into(),
                author: "octocat".into(),
                date: "2026-10-01T12:00:00Z".into(),
            },
        ],
    }
}

pub fn profile() -> Profile {
    Profile {
        login: "octocat".into(),
        name: Some("The Octocat".into()),
        bio: Some("Eight arms, one keyboard.".into()),
        company: Some("@github".into()),
        location: Some("San Francisco".into()),
        website: Some("https://github.blog".into()),
        followers: Some(21_400),
        following: Some(9),
        is_org: false,
        pinned: vec![repo_summary("octocat/Hello-World", 3100)],
        repos: vec![
            repo_summary("octocat/Spoon-Knife", 13_000),
            repo_summary("octocat/linguist", 640),
        ],
        repo_count: 8,
        stars: vec![repo_summary("ratatui/ratatui", 22_900)],
        star_count: 120,
    }
}

pub fn repo_results() -> SearchResults {
    SearchResults::Repos(Results {
        total: 1243,
        items: vec![
            repo_summary("sxyazi/yazi", 42_600),
            repo_summary("jarun/nnn", 22_000),
        ],
        next: Some("c2".into()),
    })
}

pub fn user_results() -> SearchResults {
    SearchResults::Users(Results {
        total: 1,
        items: vec![UserSummary {
            login: "octocat".into(),
            name: Some("The Octocat".into()),
            bio: None,
            is_org: false,
        }],
        next: None,
    })
}
