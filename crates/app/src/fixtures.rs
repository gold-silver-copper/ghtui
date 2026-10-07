//! Page data and key presses for tests and snapshots.

use crossterm::event::KeyEvent;
use ghtui_api::browse::{
    Advisory, AdvisoryPackage, Asset, Blame, BlameRange, Blob, BranchInfo, CheckItem, CheckOutcome,
    Checks, CodeHit, Comment, CommitDetail, CommitHit, CommitInfo, Comparison, Contributed,
    Contributions, DeploymentInfo, DeploymentList, DiscussionCategory, DiscussionComment,
    DiscussionDetail, DiscussionHit, DiscussionList, DiscussionSummary, EntryKind, Gist, GistFile,
    GistSummary, IssueDetail, IssueState, IssueSummary, Job, JobSummary, MilestoneDetail,
    MilestoneInfo, MilestoneList, MonthActivity, PrActivity, Profile, Readme, Release,
    RepoOverview, RepoSummary, Results, ReviewSummary, RunSummary, SearchResults, Step, TagInfo,
    TeamDetail, TeamRepo, TeamSummary, TreeEntry, UserSummary, Week, WikiPage, Workflow,
    WorkflowRun,
};
use ghtui_api::model::{Label, NodeId, PrRef, RepoId, ReviewComment, ReviewThread, Side};

use crate::browse::{Data, DataKey};
use crate::diff_screen::DiffOf;
use crate::state::{Cmd, DiffMsg, Msg, State, update};

/// Presses `keys`, in vim notation (`"jj"`, `"<Esc>/"`, `"<C-d>"`).
pub(crate) fn press(state: &mut State, keys: &str) -> Vec<Cmd> {
    let mut cmds = Vec::new();
    let mut rest = keys;
    while let Some(c) = rest.chars().next() {
        // One key: `<…>`, or a character.
        let len = match rest.find('>') {
            Some(end) if c == '<' && end > 1 => end + 1,
            _ => c.len_utf8(),
        };
        let (key, tail) = rest.split_at(len);
        let key = crate::keymap::parse_key(key).unwrap();
        cmds.extend(update(state, Msg::Key(KeyEvent::new(key.code, key.mods))));
        rest = tail;
    }
    cmds
}

/// Delivers freshly fetched page data.
pub(crate) fn fetched(state: &mut State, key: DataKey, data: Data) -> Vec<Cmd> {
    let result = Ok(data);
    let cached_at = None;
    update(
        state,
        Msg::Fetched {
            key,
            result,
            cached_at,
        },
    )
}

/// Delivers a message for `pr`'s diff.
pub(crate) fn diff_msg(state: &mut State, pr: &PrRef, msg: DiffMsg) -> Vec<Cmd> {
    update(state, Msg::Diff(DiffOf::Pr(pr.clone()), msg))
}

/// A review thread on `src/point.rs`, with one comment.
pub(crate) fn thread(id: &str, line: Option<u32>, resolved: bool, outdated: bool) -> ReviewThread {
    ReviewThread {
        id: NodeId::new(id),
        path: "src/point.rs".into(),
        side: Side::Right,
        start_side: None,
        line,
        start_line: None,
        original_line: Some(3),
        original_start_line: None,
        outdated,
        resolved,
        file_level: false,
        can_reply: true,
        can_resolve: true,
        can_unresolve: true,
        comments: vec![ReviewComment {
            id: NodeId::new(format!("{id}-c")),
            author: "alice".into(),
            body: format!("Thread {id}"),
            created_at: "2026-10-03T09:00:00Z".into(),
            url: String::new(),
            original_commit: Some("abc".into()),
            pending: false,
        }]
        .into(),
    }
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
        topics: vec!["tui".into(), "github".into(), "rust".into()].into(),
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
        has_discussions: true,
        has_wiki: true,
        branches: 14,
        tags: 9,
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

pub fn commit() -> CommitDetail {
    CommitDetail {
        oid: "0a1b2c3d4e5f60718293a4b5c6d7e8f901234567".into(),
        headline: "Hunk headers name the enclosing scope".into(),
        body: "Hunk headers were a bare range. They now name where the first change\nis, from the syntax tree.\n\nFixes #12.".into(),
        author: "octocat".into(),
        authored_at: "2026-01-14T10:00:00Z".into(),
        committer: Some("hubot".into()),
        committed_at: "2026-01-15T09:30:00Z".into(),
        parents: vec!["1b2c3d4e5f60718293a4b5c6d7e8f9012345678a".into()].into(),
        additions: 445,
        deletions: 71,
        changed_files: Some(20),
        verified: Some(true),
    }
}

/// A page of a branch's history.
pub fn history(next: Option<&str>) -> Results<CommitInfo> {
    let commits = activity().commits;
    Results {
        total: 40,
        items: commits,
        next: next.map(str::to_owned),
    }
}

/// Checks on a commit: a failure, a run still going, passes, a skip, and
/// a commit status from another service.
pub fn checks() -> Checks {
    let run = |name: &str,
               group: &str,
               outcome,
               took: Option<(&str, &str)>,
               summary: Option<&str>,
               job: u32| {
        CheckItem {
            name: name.into(),
            group: group.into(),
            outcome,
            started_at: took.map(|(s, _)| format!("2026-10-03T12:{s}Z")),
            completed_at: took
                .map(|(_, e)| format!("2026-10-03T12:{e}Z"))
                .filter(|_| outcome != CheckOutcome::Pending),
            summary: summary.map(str::to_owned),
            url: Some(format!(
                "https://github.com/gold-silver-copper/ghtui/actions/runs/7/job/{job}"
            )),
        }
    };
    let mut items = vec![
        run(
            "test (ubuntu)",
            "CI",
            CheckOutcome::Failure,
            Some(("00:00", "04:12")),
            Some("Process completed with exit code 101."),
            1,
        ),
        run(
            "test (macos)",
            "CI",
            CheckOutcome::Pending,
            Some(("01:00", "01:00")),
            None,
            2,
        ),
        run(
            "clippy",
            "CI",
            CheckOutcome::Success,
            Some(("00:00", "01:31")),
            None,
            3,
        ),
        run(
            "fmt",
            "CI",
            CheckOutcome::Success,
            Some(("00:00", "00:14")),
            None,
            4,
        ),
        run("deploy", "Release", CheckOutcome::Skipped, None, None, 5),
    ];
    items.push(CheckItem {
        name: "codecov/patch".into(),
        group: "Statuses".into(),
        outcome: CheckOutcome::Success,
        started_at: Some("2026-10-03T12:05:00Z".into()),
        completed_at: None,
        summary: Some("92.31% of diff hit (target 80.00%)".into()),
        url: Some("https://app.codecov.io/gh/gold-silver-copper/ghtui/pull/12".into()),
    });
    Checks {
        oid: "0a1b2c3d4e5f60718293a4b5c6d7e8f901234567".into(),
        total: items.len() as u64,
        items,
    }
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

/// [`blob`]'s blame: two commits.
pub fn blame() -> Blame {
    let range = |start, end, oid: char, author: &str, date: &str| BlameRange {
        start,
        end,
        age: 1,
        oid: oid.to_string().repeat(40),
        headline: "Say hello".into(),
        author: author.into(),
        date: date.into(),
    };
    Blame {
        ranges: vec![
            range(1, 4, 'a', "octocat", "2025-01-01T00:00:00Z"),
            range(
                5,
                5,
                'b',
                "monalisa-with-a-long-name",
                "2026-10-01T00:00:00Z",
            ),
            range(6, 7, 'a', "octocat", "2025-01-01T00:00:00Z"),
        ],
    }
}
/// A gist with a Rust file and a README.
pub fn gist() -> Gist {
    let file = |name: &str, language: &str, text: &str| GistFile {
        name: name.into(),
        language: Some(language.into()),
        size: text.len() as u64,
        text: Some(text.into()),
        truncated: false,
    };
    Gist {
        id: "6cad326836d38bd3a7ae".into(),
        owner: Some("octocat".into()),
        description: "Hello world!".into(),
        public: true,
        created_at: "2025-01-01T00:00:00Z".into(),
        updated_at: "2026-10-02T12:00:00Z".into(),
        comments: 3,
        files: vec![
            file(
                "hello.rs",
                "Rust",
                "fn main() {\n    println!(\"hello\");\n}\n",
            ),
            file("README.md", "Markdown", "# Hello\n\nSays *hello*."),
        ],
    }
}

/// Two of someone's gists.
pub fn gists() -> Results<GistSummary> {
    let gist = |id: &str, file: &str, description: &str| GistSummary {
        id: id.into(),
        description: description.into(),
        files: vec![file.into()],
        updated_at: "2026-10-02T12:00:00Z".into(),
        stars: 26,
        comments: 3,
    };
    Results {
        total: 8,
        items: vec![
            gist("6cad326836d38bd3a7ae", "hello.rs", "Hello world!"),
            gist("9257657", ".gitignore", ""),
        ],
        next: Some("g1".into()),
    }
}
fn team_summary(slug: &str, name: &str, secret: bool) -> TeamSummary {
    TeamSummary {
        slug: slug.into(),
        name: name.into(),
        description: format!("The {name} team"),
        secret,
        members: 7,
        repos: 2,
    }
}

/// Three teams.
pub fn teams() -> Results<TeamSummary> {
    Results {
        total: 3,
        items: vec![
            team_summary("core", "Core", false),
            team_summary("security", "Security", true),
            team_summary("triage", "Triage", false),
        ],
        next: None,
    }
}

/// A team with a parent, two members, a repository and a child team.
pub fn team() -> TeamDetail {
    let member = |login: &str, name: Option<&str>| UserSummary {
        login: login.into(),
        name: name.map(Into::into),
        bio: None,
        is_org: false,
    };
    TeamDetail {
        team: team_summary("core", "Core", false),
        parent: Some(team_summary("maintainers", "Maintainers", false)),
        members: vec![
            member("octocat", Some("The Octocat")),
            member("hubot", None),
        ]
        .into(),
        repos: vec![TeamRepo {
            repo: RepoId::new("gold-silver-copper", "ghtui"),
            description: "GitHub in the terminal".into(),
            stars: 1200,
        }]
        .into(),
        children: vec![team_summary("core-docs", "Core docs", false)].into(),
    }
}
/// A high-severity advisory with one package.
pub fn advisory() -> Advisory {
    Advisory {
        ghsa: "GHSA-6r3q-mjv7-xr8m".into(),
        cve: Some("CVE-2026-41510".into()),
        summary: "Silent argument drop allows bypassing rules".into(),
        description: "Requests with many arguments skip the rules that read them.".into(),
        severity: "high".into(),
        published_at: Some("2026-10-02T12:00:00Z".into()),
        updated_at: Some("2026-10-02T12:00:00Z".into()),
        withdrawn_at: None,
        cvss: Some((
            "7.2".into(),
            "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:C/C:N/I:L/A:L".into(),
        )),
        cwes: vec!["CWE-693 Protection Mechanism Failure".into()],
        packages: vec![AdvisoryPackage {
            ecosystem: "go".into(),
            name: "github.com/corazawaf/coraza/v3".into(),
            vulnerable: Some(">= 3.0.0, < 3.8.1".into()),
            patched: Some("3.8.1".into()),
        }],
        credits: vec!["octocat".into()],
        references: vec!["https://nvd.nist.gov/vuln/detail/CVE-2026-41510".into()],
    }
}

/// A repository's three advisories.
pub fn advisories() -> Vec<Advisory> {
    let mut low = advisory();
    low.ghsa = "GHSA-6gcq-wc29-5xf2".into();
    low.severity = "low".into();
    low.summary = "Deep JSON bodies can crash the process".into();
    let mut critical = advisory();
    critical.ghsa = "GHSA-3c6w-j9xm-8h2h".into();
    critical.severity = "critical".into();
    critical.summary = "Unbounded recursion exhausts the CPU".into();
    vec![critical, advisory(), low]
}
/// A wiki's Home, with links to its other pages.
pub fn wiki() -> WikiPage {
    WikiPage {
        title: Some("Home".into()),
        text: Some(
            "Welcome to the wiki.\n\nSee [Getting started](Getting-started) and [[FAQ]].".into(),
        ),
        markdown: true,
        pages: vec!["FAQ".into(), "Getting started".into(), "Home".into()],
        sidebar: Some("**[Home](Home)** · [FAQ](FAQ)".into()),
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
            vec![label("bug", "d73a4a")].into()
        } else {
            Vec::new().into()
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
        labels: vec![label("bug", "d73a4a"), label("ui", "1d76db")].into(),
        assignees: vec!["hubot".into()].into(),
        milestone: Some(ghtui_api::model::MilestoneRef {
            number: 3,
            title: "Links".into(),
        }),
        comments: vec![Comment {
            id: Some(1_000_001),
            author: "hubot".into(),
            body: "Agreed. 120 columns, centered?".into(),
            created_at: "2026-10-02T12:00:00Z".into(),
        }],
        total_comments: 1,
        id: NodeId::new("I_14"),
    }
}

pub fn activity() -> PrActivity {
    PrActivity {
        id: NodeId::new("PR_12"),
        total_comments: 1,
        total_reviews: 2,
        total_commits: 2,
        comments: vec![Comment {
            id: Some(1_000_001),
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
        star_count: 120,
        readme: Some("### Hi there 👋\n\nI'm the Octocat. I like **terminals**.\n".into()),
        status: Some("🐙 Reviewing pull requests".into()),
        pronouns: Some("they/them".into()),
        socials: vec![(
            "@octocat@hachyderm.io".into(),
            "https://hachyderm.io/@octocat".into(),
        )],
        orgs: vec!["github".into(), "ratatui".into()].into(),
        verified: false,
        people: Vec::new(),
        people_count: 0,
        contributions: Some(contributions()),
    }
}

/// A year of contributions, shaded in a repeating pattern, and two months
/// of activity.
pub fn contributions() -> Contributions {
    const DAYS: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let (mut year, mut month, mut day) = (2025u32, 10u32, 5u32);
    let mut weeks = Vec::new();
    for w in 0..53u32 {
        let days = (0..7u32)
            .map(|d| match (w * 7 + d) * 37 % 11 {
                0..=4 => 0,
                5 | 6 => 1,
                7 | 8 => 2,
                9 => 3,
                _ => 4,
            })
            .take(if w == 52 { 3 } else { 7 })
            .collect();
        weeks.push(Week {
            start: format!("{year}-{month:02}-{day:02}"),
            days,
        });
        day += 7;
        let length = DAYS.get(month as usize - 1).copied().unwrap_or(30);
        if day > length {
            day -= length;
            month += 1;
            if month > 12 {
                (month, year) = (1, year + 1);
            }
        }
    }
    let item = |repo: &str, number: u64, title: &str| Contributed {
        repo: repo.into(),
        number,
        title: title.into(),
    };
    Contributions {
        total: 1234,
        weeks,
        activity: vec![
            MonthActivity {
                month: "2026-10".into(),
                commits: vec![
                    ("gold-silver-copper/ghtui".into(), 42),
                    ("ratatui/ratatui".into(), 3),
                ],
                pulls: vec![item(
                    "gold-silver-copper/ghtui",
                    12,
                    "Theme: generate syntax palette from seed",
                )],
                issues: Vec::new(),
                reviews: vec![item("ratatui/ratatui", 1900, "Add a scrollbar widget")],
            },
            MonthActivity {
                month: "2026-09".into(),
                commits: vec![("gold-silver-copper/ghtui".into(), 17)],
                pulls: Vec::new(),
                issues: vec![item(
                    "ratatui/ratatui",
                    1888,
                    "Unicode width of emoji in tables",
                )],
                reviews: Vec::new(),
            },
        ],
        // More reviews than were fetched, from October back.
        short: ghtui_api::browse::Short {
            reviews: Some("2026-10".into()),
            ..Default::default()
        },
    }
}

/// The first page of a profile's repositories.
pub fn profile_repos() -> Results<RepoSummary> {
    Results {
        total: 8,
        items: profile().repos,
        next: Some("p1".into()),
    }
}

/// An organization: verified, with members and a README.
pub fn org_profile() -> Profile {
    let mut repos = vec![
        repo_summary("ratatui/ratatui", 22_900),
        repo_summary("ratatui/templates", 300),
        repo_summary("ratatui/website", 120),
    ];
    if let Some(r) = repos.get_mut(2) {
        r.language = Some("TypeScript".into());
    }
    Profile {
        login: "ratatui".into(),
        name: Some("Ratatui".into()),
        bio: Some("Rust library for cooking up terminal user interfaces".into()),
        company: None,
        location: None,
        website: Some("https://ratatui.rs".into()),
        followers: None,
        following: None,
        is_org: true,
        pinned: Vec::new(),
        repos,
        repo_count: 24,
        star_count: 0,
        readme: Some(
            "Welcome to **Ratatui**. Start with the [tutorial](https://ratatui.rs).\n".into(),
        ),
        status: None,
        pronouns: None,
        socials: Vec::new(),
        orgs: Vec::new().into(),
        verified: true,
        people: vec!["joshka".into(), "orhun".into(), "kdheepak".into()],
        people_count: 14,
        contributions: None,
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

/// A page of people, with more to load.
pub fn people() -> Results<UserSummary> {
    let person = |login: &str, name: Option<&str>, bio: Option<&str>| UserSummary {
        login: login.into(),
        name: name.map(str::to_owned),
        bio: bio.map(str::to_owned),
        is_org: false,
    };
    Results {
        total: 1234,
        items: vec![
            person("octocat", Some("The Octocat"), Some("GitHub's mascot.")),
            person("hubot", None, None),
            person("monalisa", Some("Mona Lisa Octocat"), None),
        ],
        next: Some("u1".into()),
    }
}

/// A page of a repository's forks.
pub fn forks() -> Results<RepoSummary> {
    Results {
        total: 87,
        items: vec![
            repo_summary("octocat/ghtui", 12),
            repo_summary("hubot/ghtui", 3),
        ],
        next: Some("f1".into()),
    }
}

/// A release, with notes and assets when it's on its own page.
pub fn release(full: bool) -> Release {
    let asset = |name: &str, size: u64, downloads: u64| Asset {
        name: name.into(),
        size,
        downloads,
        url: format!("https://github.com/gold-silver-copper/ghtui/releases/download/v0.2.0/{name}"),
    };
    Release {
        name: "ghtui 0.2.0".into(),
        tag: "v0.2.0".into(),
        published_at: Some("2026-09-30T12:00:00Z".into()),
        prerelease: false,
        draft: false,
        latest: true,
        author: Some("octocat".into()),
        notes: full.then(|| {
            "## Highlights\n\n- Scope names in hunk headers\n- Commits open in ghtui\n".into()
        }),
        assets: if full {
            vec![
                asset("ghtui-x86_64-linux.tar.gz", 4_200_000, 1_312),
                asset("ghtui-aarch64-macos.tar.gz", 3_900_000, 845),
            ]
            .into()
        } else {
            Vec::new().into()
        },
    }
}

/// A page of releases: the latest, and a pre-release before it.
pub fn releases() -> Results<Release> {
    let mut pre = release(false);
    pre.name = "ghtui 0.2.0-rc.1".into();
    pre.tag = "v0.2.0-rc.1".into();
    pre.latest = false;
    pre.prerelease = true;
    pre.published_at = Some("2026-09-20T12:00:00Z".into());
    Results {
        total: 9,
        items: vec![release(false), pre],
        next: Some("r1".into()),
    }
}

pub fn tags() -> Results<TagInfo> {
    let tag = |name: &str, oid: &str, date: &str| TagInfo {
        name: name.into(),
        oid: Some(oid.into()),
        date: Some(date.into()),
    };
    Results {
        total: 2,
        items: vec![
            tag(
                "v0.2.0",
                "0a1b2c3d4e5f60718293a4b5c6d7e8f901234567",
                "2026-09-30T12:00:00Z",
            ),
            tag(
                "v0.1.0",
                "1b2c3d4e5f60718293a4b5c6d7e8f9012345678a",
                "2026-06-01T12:00:00Z",
            ),
        ],
        next: None,
    }
}

/// A workflow run with a failed job, one still going, and passes.
pub fn workflow_run() -> WorkflowRun {
    let job = |id, name: &str, outcome, end: Option<&str>| JobSummary {
        id,
        name: name.into(),
        outcome,
        started_at: Some("2026-10-03T12:00:00Z".into()),
        completed_at: end.map(|e| format!("2026-10-03T12:{e}Z")),
    };
    WorkflowRun {
        id: 7,
        name: "CI".into(),
        title: "Theme: generate syntax palette from seed".into(),
        number: 412,
        attempt: 2,
        event: "pull_request".into(),
        branch: Some("syntax-palette".into()),
        sha: "0a1b2c3d4e5f60718293a4b5c6d7e8f901234567".into(),
        outcome: CheckOutcome::Failure,
        actor: Some("octocat".into()),
        started_at: Some("2026-10-03T12:00:00Z".into()),
        updated_at: Some("2026-10-03T12:04:12Z".into()),
        path: ".github/workflows/ci.yml".into(),
        jobs: vec![
            job(1, "fmt", CheckOutcome::Success, Some("00:14")),
            job(2, "test (ubuntu)", CheckOutcome::Failure, Some("04:12")),
            job(3, "test (macos)", CheckOutcome::Pending, None),
        ],
    }
}

/// A job whose test step failed, and its log.
pub fn job() -> (Job, ghtui_api::browse::JobLog) {
    let step = |number, name: &str, outcome, start: &str, end: &str| Step {
        number,
        name: name.into(),
        outcome,
        started_at: Some(format!("2026-10-03T12:{start}Z")),
        completed_at: Some(format!("2026-10-03T12:{end}Z")),
    };
    let job = Job {
        id: 2,
        run_id: 7,
        name: "test (ubuntu)".into(),
        outcome: CheckOutcome::Failure,
        started_at: Some("2026-10-03T12:00:00Z".into()),
        completed_at: Some("2026-10-03T12:04:12Z".into()),
        steps: vec![
            step(1, "Set up job", CheckOutcome::Success, "00:00", "00:02"),
            step(
                2,
                "Run actions/checkout@v4",
                CheckOutcome::Success,
                "00:02",
                "00:05",
            ),
            step(3, "cargo test", CheckOutcome::Failure, "00:05", "04:11"),
            step(
                4,
                "Post Run actions/checkout@v4",
                CheckOutcome::Success,
                "04:11",
                "04:12",
            ),
        ],
    };
    let log = [
        ("00:00:01.1", "Current runner version: '2.319.1'"),
        ("00:02:01.0", "##[group]Run actions/checkout@v4"),
        ("00:02:02.0", "Syncing repository: gold-silver-copper/ghtui"),
        ("00:04:00.0", "##[endgroup]"),
        ("00:05:00.5", "##[group]Run cargo test"),
        ("00:05:01.0", "   Compiling ghtui v0.1.0"),
        (
            "04:10:00.0",
            "test theme::tests::heat_levels_stay_apart ... FAILED",
        ),
        (
            "04:10:00.1",
            "##[error]Process completed with exit code 101.",
        ),
        ("04:11:00.0", "Post job cleanup."),
    ]
    .map(|(at, line)| format!("2026-10-03T12:{at}000000Z {line}\n"))
    .concat();
    let log = ghtui_api::browse::JobLog {
        text: log,
        ..Default::default()
    };
    (job, log)
}

/// A workflow's runs.
pub fn workflow_runs() -> (Workflow, Results<RunSummary>) {
    let run = |id, number, title: &str, outcome| RunSummary {
        id,
        title: title.into(),
        number,
        event: "push".into(),
        branch: Some("main".into()),
        outcome,
        actor: Some("octocat".into()),
        created_at: Some("2026-10-03T12:00:00Z".into()),
    };
    let workflow = Workflow {
        name: "CI".into(),
        path: ".github/workflows/ci.yml".into(),
        state: "active".into(),
    };
    let runs = Results {
        total: 412,
        items: vec![
            run(
                7,
                412,
                "Theme: generate syntax palette from seed",
                CheckOutcome::Failure,
            ),
            run(
                6,
                411,
                "Hunk headers name the enclosing scope",
                CheckOutcome::Success,
            ),
        ],
        next: Some("2".into()),
    };
    (workflow, runs)
}

/// Three branches by name: the default and two others, one with a merged
/// pull request.
pub fn branches() -> Results<BranchInfo> {
    let branch = |name: &str, default, headline: &str, pr| BranchInfo {
        name: name.into(),
        default,
        oid: Some("8d1a76430406c877b35d0b627e7f796dcf0dfeca".into()),
        headline: Some(headline.into()),
        author: Some("octocat".into()),
        date: Some("2026-10-02T12:00:00Z".into()),
        pr,
    };
    Results {
        total: 14,
        items: vec![
            branch(
                "links",
                false,
                "Discussions are pages",
                Some((6, IssueState::Open)),
            ),
            branch(
                "main",
                true,
                "Checks show each check's latest run once",
                None,
            ),
            branch(
                "tabs",
                false,
                "Tabs keep their history",
                Some((5, IssueState::Merged)),
            ),
        ],
        next: Some("b1".into()),
    }
}
fn milestone_info(number: u64, title: &str, closed: bool) -> MilestoneInfo {
    MilestoneInfo {
        number,
        title: title.into(),
        description: "Every GitHub link opens in ghtui.\n\nOr says why it doesn't.".into(),
        due_on: Some("2026-11-05T00:00:00Z".into()),
        closed,
        closed_at: closed.then(|| "2026-09-30T00:00:00Z".into()),
        updated_at: "2026-10-02T12:00:00Z".into(),
        open: 4,
        done: 6,
    }
}

/// Three open milestones.
pub fn milestones() -> MilestoneList {
    let mut later = milestone_info(4, "1.0", false);
    later.due_on = None;
    later.description = String::new();
    later.open = 9;
    later.done = 0;
    MilestoneList {
        open: 3,
        closed: 3,
        results: Results {
            total: 3,
            items: vec![
                milestone_info(3, "Links", false),
                milestone_info(5, "Pages", false),
                later,
            ],
            next: None,
        },
    }
}

/// A milestone with an issue and a pull request.
pub fn milestone() -> MilestoneDetail {
    MilestoneDetail {
        info: milestone_info(3, "Links", false),
        items: Results {
            total: 10,
            items: vec![
                issue_summary(42, false, IssueState::Open),
                issue_summary(6, true, IssueState::Merged),
            ],
            next: Some("m1".into()),
        },
        unsearchable: false,
    }
}
/// Three deployments to two environments, one failed.
pub fn deployments() -> DeploymentList {
    let deployment = |env: &str, outcome, state: &str, url: Option<&str>| DeploymentInfo {
        environment: env.into(),
        outcome,
        state: state.into(),
        created_at: "2026-10-02T12:00:00Z".into(),
        creator: Some("octocat".into()),
        branch: Some("main".into()),
        oid: "8d1a76430406c877b35d0b627e7f796dcf0dfeca".into(),
        log_url: Some("https://github.com/gold-silver-copper/ghtui/actions/runs/9/job/12".into()),
        environment_url: url.map(Into::into),
    };
    DeploymentList {
        environments: vec!["production".into(), "staging".into()].into(),
        results: Results {
            total: 3,
            items: vec![
                deployment(
                    "production",
                    CheckOutcome::Success,
                    "active",
                    Some("https://ghtui.example.com"),
                ),
                deployment("staging", CheckOutcome::Failure, "failure", None),
                deployment("staging", CheckOutcome::Neutral, "inactive", None),
            ],
            next: None,
        },
    }
}
/// Two tags compared: the first of many commits.
pub fn comparison() -> Comparison {
    let commit = |oid: char, headline: &str, date: &str| CommitInfo {
        oid: oid.to_string().repeat(40),
        headline: headline.into(),
        author: "octocat".into(),
        date: date.into(),
    };
    Comparison {
        status: "ahead".into(),
        ahead: 120,
        behind: 0,
        total_commits: 120,
        commits: vec![
            commit('a', "Branches are a page", "2026-10-01T10:00:00Z"),
            commit('b', "Milestones are pages", "2026-10-01T12:00:00Z"),
            commit('c', "Deployments are a page", "2026-10-02T12:00:00Z"),
        ],
        from: "1".repeat(40),
        to: "c".repeat(40),
        files: 14,
        files_capped: false,
        additions: 812,
        deletions: 40,
    }
}
/// A page of discussions with two categories.
pub fn discussions() -> DiscussionList {
    let summary = |number, title: &str, category: &str, answered| DiscussionSummary {
        number,
        title: title.into(),
        author: "octocat".into(),
        category: category.into(),
        comments: 3,
        answered,
        upvotes: 12,
        updated_at: "2026-10-03T12:00:00Z".into(),
    };
    DiscussionList {
        categories: vec![
            DiscussionCategory {
                name: "Ideas".into(),
                slug: "ideas".into(),
            },
            DiscussionCategory {
                name: "Q&A".into(),
                slug: "q-a".into(),
            },
        ]
        .into(),
        results: Results {
            total: 48,
            items: vec![
                summary(14603, "How do I review a PR offline?", "Q&A", true),
                summary(14590, "Show the contribution graph", "Ideas", false),
            ],
            next: Some("d1".into()),
        },
    }
}

/// A question with an answer and a reply.
pub fn discussion() -> DiscussionDetail {
    let comment = |id, author: &str, body: &str| Comment {
        id: Some(id),
        author: author.into(),
        body: body.into(),
        created_at: "2026-10-03T12:00:00Z".into(),
    };
    DiscussionDetail {
        repo: RepoId::new("cli", "cli"),
        number: 14603,
        title: "How do I review a PR offline?".into(),
        body: "I'm on a plane a lot. Can I review without a connection?".into(),
        author: "octocat".into(),
        created_at: "2026-10-02T12:00:00Z".into(),
        category: "Q&A".into(),
        answered: true,
        upvotes: 12,
        comments: vec![
            DiscussionComment {
                comment: comment(
                    18765024,
                    "hubot",
                    "Fetch the PR first; the diff then works offline.",
                ),
                upvotes: 5,
                answer: true,
                replies: vec![comment(18765030, "octocat", "That works, thanks!")],
                // More than came: the page says how many more.
                total_replies: 3,
            },
            DiscussionComment {
                comment: comment(18765100, "monalisa", "Drafts are saved locally too."),
                upvotes: 1,
                answer: false,
                replies: Vec::new(),
                total_replies: 0,
            },
        ],
        total_comments: 2,
    }
}
/// A discussion search's result.
pub fn discussion_results() -> SearchResults {
    let summary = discussions().results.items.remove(0);
    SearchResults::Discussions(Results {
        total: 6065,
        items: vec![DiscussionHit {
            repo: RepoId::new("cli", "cli"),
            summary,
        }],
        next: Some("2".into()),
    })
}

/// A commit search's results.
pub fn commit_results() -> SearchResults {
    let hit = |oid: char, headline: &str| CommitHit {
        repo: RepoId::new("ratatui", "ratatui"),
        commit: CommitInfo {
            oid: oid.to_string().repeat(40),
            headline: headline.into(),
            author: "octocat".into(),
            date: "2026-10-01T12:00:00Z".into(),
        },
    };
    SearchResults::Commits(Results {
        total: 60,
        items: vec![
            hit('a', "perf(terminal): skip a redundant clear"),
            hit('b', "feat: a tui widget"),
        ],
        next: Some("2".into()),
    })
}

/// A code search's results.
pub fn code_results() -> SearchResults {
    let hit = |path: &str| CodeHit {
        repo: RepoId::new("ratatui", "ratatui"),
        path: path.into(),
        sha: "f60d17e29bb58b7e699e9befbb97298ec22aac14".into(),
    };
    SearchResults::Code(Results {
        total: 158,
        items: vec![hit("ratatui-termina/src/lib.rs"), hit("src/terminal.rs")],
        next: Some("2".into()),
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
