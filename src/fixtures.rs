//! Page data and key presses for tests.

#![allow(
    unused_must_use,
    reason = "tests drive the state, and check only the commands they care about"
)]

use crate::api::ApiError;
use crate::api::browse::{
    Advisory, AdvisoryPackage, Asset, Blame, BlameRange, Blob, BranchInfo, CheckItem, CheckOutcome,
    Checks, CodeHit, Comment, CommitDetail, CommitHit, CommitInfo, Comparison, Contributed,
    Contributions, DeploymentInfo, DeploymentList, DiscussionCategory, DiscussionComment,
    DiscussionDetail, DiscussionHit, DiscussionList, DiscussionSummary, EntryKind, Gist, GistFile,
    GistSummary, IssueDetail, IssueKind, IssueState, IssueSummary, Job, JobSummary, Login,
    MilestoneDetail, MilestoneInfo, MilestoneList, MonthActivity, Person, PrActivity, PrState,
    Profile, Readme, Release, RepoOverview, RepoSummary, Results, Reviewer, RunSummary, SearchKind,
    SearchResults, Status, Step, TagInfo, TeamDetail, TeamRepo, TeamSummary, TreeEntry,
    UserSummary, Week, WikiPage, Workflow, WorkflowRun,
};
use crate::api::model::{
    Capped, Label, NodeId, PrDetail, PrRef, RepoId, ReviewComment, ReviewState, ReviewThread,
};
use crate::api::notifications::{Inbox, Notification};
use crate::api::timeline::{Event, Item, Latest, Linked, LinkedState, Requested, Review, What};
use crossterm::event::KeyEvent;
use std::sync::Arc;

use crate::store::DraftComment;
use crate::ui::bars::Notice;

use crate::act::By;
use crate::api::change::Change;
use crate::data::{Data, DataKey};
use crate::diff_screen::DiffOf;
use crate::state::{Api, Cmd, Cmds, DiffMsg, Msg, State, update};
use crate::work::{Failure, Sent, Work};

/// Asserts the status bar says `part`, as information.
#[track_caller]
pub(crate) fn says(state: &State, part: &str) {
    shows(state, Notice::Info, part);
}

/// Asserts the status bar says `part`, as an error.
#[track_caller]
pub(crate) fn warns(state: &State, part: &str) {
    shows(state, Notice::Error, part);
}

#[track_caller]
fn shows(state: &State, kind: fn(String) -> Notice, part: &str) {
    let shown = state.notices.shown();
    let fits = shown.is_some_and(|n| match n {
        Notice::Info(text) | Notice::Error(text) => *n == kind(text.clone()) && text.contains(part),
    });
    assert!(fits, "{shown:?} doesn't say “{part}”");
}

/// Presses `keys` one after another, each a character or a named key
/// (`"<Down><Down>"`, `"<Esc>/"`, `"<C-s>"`).
pub(crate) fn press(state: &mut State, keys: &str) -> Cmds {
    let mut cmds = Cmds::new();
    let mut rest = keys;
    while let Some(c) = rest.chars().next() {
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

/// GitHub's answer to the one change `sent` holds.
pub(crate) fn answer(state: &mut State, sent: &[Cmd], result: Result<(), ApiError>) -> Cmds {
    let (change, by) = sent_change(sent);
    let (change, by) = (change.clone(), by.clone());
    changed(state, change, by, result)
}

/// Sending `change` to GitHub, from `by`.
pub(crate) fn change(change: Change, by: By) -> Cmd {
    crate::act::send(change, by)
}

/// The changes `cmds` send.
pub(crate) fn changes(cmds: &[Cmd]) -> Vec<(&Change, &By)> {
    (cmds.iter())
        .filter_map(|c| match c {
            Cmd::Work(Work::Change(c)) => Some((&c.change, &c.by)),
            _ => None,
        })
        .collect()
}

/// The one change `cmds` hold, and nothing else.
#[track_caller]
pub(crate) fn sent_change(cmds: &[Cmd]) -> (&Change, &By) {
    match (changes(cmds).as_slice(), cmds.len()) {
        ([one], 1) => *one,
        _ => panic!("not one change: {cmds:?}"),
    }
}

/// Moves the selection down from the top to the first row of the page
/// where `is`.
pub(crate) fn select_where(state: &mut State, is: impl Fn(&crate::nav::PageScreen) -> bool) {
    press(state, "<Home>");
    for _ in 0..200 {
        if let crate::state::Screen::Page(p) = state.screen()
            && is(p)
        {
            return;
        }
        press(state, "<Down>");
    }
    panic!("no row is what's wanted");
}

/// Moves the selection down from the top to the first row whose link
/// `is`.
pub(crate) fn select_by(state: &mut State, is: impl Fn(&crate::ui::page::Link) -> bool) {
    select_where(state, |p| p.selected_link().is_some_and(&is));
}

/// Selects the comment `author` wrote that starts with `text`.
pub(crate) fn select_comment(state: &mut State, author: &str, text: &str) {
    use crate::ui::page::Link;
    select_by(
        state,
        |l| matches!(l, Link::Quote { author: a, body, .. } if a == author && body.starts_with(text)),
    );
}

/// Selects the row on screen whose link is `link`.
pub(crate) fn select_link(state: &mut State, link: &crate::ui::page::Link) {
    let height = state.page_height();
    let crate::state::Screen::Page(p) = state.screen_mut() else {
        panic!("not on a page")
    };
    let page = p.page();
    let i = (0..page.items.len())
        .find(|&i| page.target(page.items[i].link) == Some(link))
        .unwrap_or_else(|| panic!("no {link:?} row"));
    p.select(i, height);
}

/// The open composer.
pub(crate) fn composer(state: &mut State) -> &mut crate::compose::Compose {
    match &mut state.overlay {
        Some(crate::state::Overlay::Compose(c)) => c,
        _ => panic!("no composer"),
    }
}

/// GitHub's answer to `change`, sent from `by`.
pub(crate) fn changed(
    state: &mut State,
    change: Change,
    by: By,
    result: Result<(), ApiError>,
) -> Cmds {
    let job = crate::act::Changing { change, by };
    done(state, job, result.map_err(Failure::from))
}

/// Starting `job`.
pub(crate) fn start(job: impl Into<Work>) -> Cmd {
    Cmd::Work(job.into())
}

/// The jobs of `J`'s kind `cmds` start.
pub(crate) fn jobs<J: Sent>(cmds: &[Cmd]) -> Vec<J> {
    (cmds.iter())
        .filter_map(|c| match c {
            Cmd::Work(work) => J::of(work).cloned(),
            _ => None,
        })
        .collect()
}

/// The one job of `J`'s kind `cmds` start.
#[track_caller]
pub(crate) fn sent<J: Sent>(cmds: &[Cmd]) -> J {
    match jobs::<J>(cmds).as_slice() {
        [one] => one.clone(),
        _ => panic!("not one {}: {cmds:?}", std::any::type_name::<J>()),
    }
}

/// What came of `job`, delivered.
pub(crate) fn done<J: Sent>(state: &mut State, job: J, result: Result<J::Out, Failure>) -> Cmds {
    update(state, Msg::Done(job.answered(result)))
}

/// Why a job failed, as tests say it.
pub(crate) fn failure(why: &str) -> Failure {
    Failure::msg(why.to_owned())
}

/// Delivers freshly fetched page data.
pub(crate) fn fetched(state: &mut State, key: DataKey, data: Data) -> Cmds {
    update(state, Msg::fetched(key, Ok(data)))
}

/// What `cmds` fetch, by key.
pub(crate) fn asked(cmds: &[Cmd]) -> Vec<DataKey> {
    (cmds.iter())
        .filter_map(|c| match c {
            Cmd::Api(Api::Fetch { key, .. }) => Some(key.clone()),
            _ => None,
        })
        .collect()
}

/// Whether `cmds` fetch `key`.
pub(crate) fn asks(cmds: &[Cmd], key: &DataKey) -> bool {
    asked(cmds).contains(key)
}

/// Whether `cmds` fetch pull request `pr`.
pub(crate) fn asks_pr(cmds: &[Cmd], pr: &PrRef) -> bool {
    asks(cmds, &DataKey::Pr(pr.clone()))
}

/// Fetching pull request `pr` for the first time.
pub(crate) fn fetch_pr(pr: &PrRef) -> Cmd {
    let key = DataKey::Pr(pr.clone());
    Cmd::Api(Api::Fetch { key, cached: true })
}

/// Delivers GitHub's answer for pull request `pr`.
pub(crate) fn pr_fetched(
    state: &mut State,
    pr: &PrRef,
    result: Result<PrDetail, ApiError>,
) -> Cmds {
    let key = DataKey::Pr(pr.clone());
    update(state, Msg::fetched(key, result.map(Data::from)))
}

/// Puts pull request `pr` in the state as the disk cache had it.
pub(crate) fn pr_cached(state: &mut State, pr: &PrRef, detail: PrDetail) {
    let cached = crate::store::Cached {
        value: Data::from(detail),
        fetched_at: 0,
    };
    let key = DataKey::Pr(pr.clone());
    (state.data).insert(key, crate::state::Remote::cached(Some(cached)));
}

/// Fills Home's section `i` with a search's results.
pub(crate) fn section(state: &mut State, i: usize, results: SearchResults) -> Cmds {
    let search = state.home[i].search.clone().unwrap();
    fetched(state, search.key(), Data::from(results))
}

/// Pull requests as a search finds them, of `total`.
pub(crate) fn found_prs(items: Vec<IssueSummary>, total: u64) -> SearchResults {
    SearchResults::Issues(page(total, items, None))
}

/// A pull request a search found.
pub(crate) fn found_pr(pr: &str, title: &str, state: PrState) -> IssueSummary {
    let pr = PrRef::parse(pr).unwrap();
    IssueSummary {
        kind: IssueKind::Pr {
            id: NodeId::new(format!("PR_{pr}")),
            state,
            review: None,
            approvals: 0,
            checks: None,
        },
        repo: pr.repo,
        number: pr.number,
        title: title.into(),
        author: "octocat".into(),
        updated_at: "2026-10-03T09:00:00Z".into(),
        comments: 3,
        labels: Capped::default(),
        created_at: "2026-10-01T09:00:00Z".into(),
    }
}

/// Delivers a message for `pr`'s diff.
pub(crate) fn diff_msg(state: &mut State, pr: &PrRef, msg: DiffMsg) -> Cmds {
    update(state, Msg::Diff(DiffOf::Pr(pr.clone()), msg))
}

/// GitHub's review threads for `pr`'s diff.
pub(crate) fn threads_loaded(state: &mut State, pr: &PrRef, threads: Vec<ReviewThread>) -> Cmds {
    diff_msg(state, pr, DiffMsg::ThreadsLoaded(Ok(threads)))
}

/// The diff job `cmds` start.
#[track_caller]
pub(crate) fn diff_job(cmds: &[Cmd]) -> crate::diff_job::JobId {
    let job = cmds.iter().find_map(|c| match c {
        Cmd::Git(crate::state::Git::LoadDiff { job, .. }) => Some(*job),
        _ => None,
    });
    job.unwrap_or_else(|| panic!("no diff job in {cmds:?}"))
}

/// git's listing of a pull request's diff: at `head`, with no files.
pub(crate) fn no_files(job: crate::diff_job::JobId, head: &str) -> DiffMsg {
    let base = crate::git::Oid::parse(&"b".repeat(40)).unwrap();
    let refs = crate::git::repo::PrRefs {
        head: crate::git::Oid::parse(head).unwrap(),
        base: base.clone(),
        merge_base: base,
    };
    let files = crate::diff_job::DiffFiles {
        refs,
        files: Vec::new(),
        generated: std::collections::HashSet::new(),
    };
    DiffMsg::Job(job, crate::diff_job::JobMsg::Files(Box::new(files)))
}

/// A draft comment on new line `line` of `src/point.rs`.
pub(crate) fn draft(id: u64, line: u32, body: &str) -> DraftComment {
    DraftComment {
        id,
        path: "src/point.rs".into(),
        body: body.into(),
        line: Some(line),
        commit: "e".repeat(40),
        ..Default::default()
    }
}

/// A review thread on `src/point.rs`, with one comment.
pub(crate) fn thread(id: &str, line: Option<u32>, resolved: bool, outdated: bool) -> ReviewThread {
    ReviewThread {
        id: NodeId::new(id),
        path: "src/point.rs".into(),
        line,
        original_line: Some(3),
        outdated,
        resolved,
        can_reply: true,
        // As GitHub says: only what applies now.
        can_resolve: !resolved,
        can_unresolve: resolved,
        comments: vec![ReviewComment {
            id: NodeId::new(format!("{id}-c")),
            author: "alice".into(),
            body: format!("Thread {id}"),
            created_at: "2026-10-03T09:00:00Z".into(),
            original_commit: Some("abc".into()),
            editable: editable(&format!("{id}-c")),
            ..Default::default()
        }]
        .into(),
        ..Default::default()
    }
}

/// A comment or description you may edit, delete and react to, without
/// reactions yet.
pub fn editable(id: &str) -> crate::api::edit::Editable {
    crate::api::edit::Editable {
        id: NodeId::new(id),
        can_update: true,
        can_delete: true,
        can_react: true,
        ..Default::default()
    }
}

fn named(id: &str, name: &str) -> crate::api::edit::Named {
    crate::api::edit::Named {
        id: NodeId::new(id),
        name: name.into(),
    }
}

/// The sidebar of [`issue`] (or a pull request), with what its
/// repository offers.
pub fn sidebar() -> crate::api::edit::Sidebar {
    crate::api::edit::Sidebar {
        labels: vec![named("L_bug", "bug"), named("L_ui", "ui")].into(),
        assignees: vec![named("U_hubot", "hubot")].into(),
        milestone: Some(named("M_3", "Links")),
        requested: crate::api::edit::Requested {
            users: vec![named("U_defunkt", "defunkt")],
            teams: vec![named("T_core", "octo-org/core")],
            ..Default::default()
        },
        author: Some("octocat".into()),
        all_labels: vec![
            named("L_bug", "bug"),
            named("L_docs", "documentation"),
            named("L_good", "good first issue"),
            named("L_ui", "ui"),
        ]
        .into(),
        users: vec![
            named("U_defunkt", "defunkt"),
            named("U_hubot", "hubot"),
            named("U_monalisa", "monalisa"),
            named("U_octocat", "octocat"),
        ]
        .into(),
        milestones: vec![named("M_3", "Links"), named("M_4", "1.0")].into(),
        teams: Some(Ok(vec![
            named("T_core", "octo-org/core"),
            named("T_docs", "octo-org/docs"),
        ]
        .into())),
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
        ..Default::default()
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

/// What a new issue or pull request in the repository can have: labels,
/// people, a milestone, two templates (a form and a Markdown one) and a
/// pull request template.
pub fn new_form() -> crate::api::create::NewForm {
    let choice =
        |id: &str, name: &str, about: &str, color: Option<&str>| crate::api::create::Choice {
            id: NodeId::new(id),
            name: name.into(),
            about: about.into(),
            color: color.map(str::to_owned),
        };
    let file = |name: &str, text: &str| (name.to_owned(), text.to_owned());
    crate::api::create::NewForm {
        repo_id: NodeId::new("R_ghtui"),
        default_branch: Some("main".into()),
        can_create_issues: true,
        has_issues: true,
        archived: false,
        can_triage: true,
        labels: vec![
            choice("LA_bug", "bug", "Something isn't working", Some("d73a4a")),
            choice("LA_docs", "documentation", "Docs", Some("0075ca")),
        ]
        .into(),
        assignees: vec![choice("U_octo", "octocat", "The Octocat", None)].into(),
        milestones: vec![choice("MI_1", "v1.0", "due 2026-12-01", None)].into(),
        issue_templates: vec![
            file(
                "bug.yml",
                "name: Bug report\ndescription: Something doesn't work\ntitle: \"[Bug]: \"\nlabels: [bug]\nassignees: octocat\nbody:\n  - type: textarea\n    attributes:\n      label: What happened?\n  - type: input\n    attributes:\n      label: Version\n",
            ),
            file(
                "feature.md",
                "---\nname: Feature request\nabout: Suggest an idea\nlabels: documentation\n---\n## The idea\n",
            ),
        ],
        pr_template: Some("## Checklist\n\n- [ ] Tests".into()),
    }
}

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
        last_commit: Some(commit_info(
            "fedcba9876543210fedcba9876543210fedcba98",
            "Browse GitHub like the website",
            "2026-10-03T10:00:00Z",
        )),
        commits: 412,
        can_write: true,
        id: NodeId::new("R_ghtui"),
        has_issues: true,
        has_discussions: true,
        has_wiki: true,
        branches: 14,
        tags: 9,
        pinned_issues: vec![Linked {
            repo: RepoId::new("gold-silver-copper", "ghtui"),
            number: 2,
            title: "Roadmap".into(),
            state: LinkedState::Issue(IssueState::Open),
        }]
        .into(),
        entries: vec![
            entry(".github", EntryKind::Dir, None),
            entry("crates", EntryKind::Dir, None),
            entry("Cargo.toml", EntryKind::File, Some(1530)),
            entry("README.md", EntryKind::File, Some(2210)),
        ],
        ..Default::default()
    }
}

/// [`overview`]'s README.
pub fn readme() -> Readme {
    Readme {
        path: "README.md".into(),
        text: README.into(),
    }
}

/// The latest commit touching each of [`overview`]'s entries.
pub fn last_commits() -> std::collections::HashMap<String, CommitInfo> {
    let commit =
        |headline, date| commit_info("0123456789abcdef0123456789abcdef01234567", headline, date);
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
        author: Person::User(Login::unchecked("octocat")),
        authored_at: "2026-01-14T10:00:00Z".into(),
        committer: Some(Person::User(Login::unchecked("hubot"))),
        committed_at: "2026-01-15T09:30:00Z".into(),
        parents: vec!["1b2c3d4e5f60718293a4b5c6d7e8f9012345678a".into()].into(),
        additions: 445,
        deletions: 71,
        changed_files: Some(20),
        verified: Some(true),
    }
}

/// A comment on a line of [`commit`].
pub fn commit_comment() -> crate::api::commit_comments::CommitComment {
    crate::api::commit_comments::CommitComment {
        id: 7,
        node_id: "CC_7".into(),
        author: "hubot".into(),
        body: "This one broke the 256-color theme.".into(),
        created_at: "2026-10-02T12:00:00Z".into(),
        path: Some("src/theme.rs".into()),
        line: Some(12),
        ..Default::default()
    }
}

/// A page of a branch's history.
pub fn history(next: Option<&str>) -> Results<CommitInfo> {
    let commits = activity().commits;
    page(40, commits.items, next)
}

/// Checks on a commit: a failure, a run still going, passes, a skip, and
/// a commit status from another service.
pub fn checks() -> Checks {
    use CheckOutcome::{Failure, Pending, Skipped, Success};
    let failed = Some("Process completed with exit code 101.");
    let runs = [
        (
            "test (ubuntu)",
            "CI",
            Failure,
            Some(("00:00", "04:12")),
            failed,
        ),
        (
            "test (macos)",
            "CI",
            Pending,
            Some(("01:00", "01:00")),
            None,
        ),
        ("clippy", "CI", Success, Some(("00:00", "01:31")), None),
        ("fmt", "CI", Success, Some(("00:00", "00:14")), None),
        ("deploy", "Release", Skipped, None, None),
    ];
    let mut items: Vec<_> = (runs.into_iter().zip(1u32..))
        .map(|((name, group, outcome, took, summary), job)| CheckItem {
            name: name.into(),
            group: group.into(),
            outcome,
            started_at: took.map(|(s, _)| format!("2026-10-03T12:{s}Z")),
            completed_at: took
                .map(|(_, e)| format!("2026-10-03T12:{e}Z"))
                .filter(|_| outcome != Pending),
            summary: summary.map(str::to_owned),
            url: Some(format!(
                "https://github.com/gold-silver-copper/ghtui/actions/runs/7/job/{job}"
            )),
            annotations: match outcome {
                Failure => vec![check_annotation()].into(),
                _ => Default::default(),
            },
        })
        .collect();
    items.push(CheckItem {
        name: "codecov/patch".into(),
        group: "Statuses".into(),
        outcome: CheckOutcome::Success,
        started_at: Some("2026-10-03T12:05:00Z".into()),
        completed_at: None,
        summary: Some("92.31% of diff hit (target 80.00%)".into()),
        url: Some("https://app.codecov.io/gh/gold-silver-copper/ghtui/pull/12".into()),
        annotations: Default::default(),
    });
    Checks {
        oid: "0a1b2c3d4e5f60718293a4b5c6d7e8f901234567".into(),
        items: items.into(),
    }
}

pub fn blob() -> Blob {
    file(
        "src/main.rs",
        "use std::io;\n\n/// Says hello.\nfn main() -> io::Result<()> {\n    println!(\"hello, {}\", 42);\n    Ok(())\n}\n",
    )
}

/// The file at `path`, all of it.
pub fn file(path: &str, text: &str) -> Blob {
    Blob {
        path: path.into(),
        text: Some(text.into()),
        size: text.len() as u64,
        truncated: false,
    }
}

/// [`blob`]'s blame: two commits.
pub fn blame() -> Blame {
    let range = |start, end, oid: char, author: &str, date: &str| BlameRange {
        start,
        end,
        age: 1,
        commit: CommitInfo {
            oid: oid.to_string().repeat(40),
            headline: "Say hello".into(),
            author: Person::User(Login::unchecked(author)),
            date: date.into(),
        },
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
        ..Default::default()
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
    page(
        8,
        vec![
            gist("6cad326836d38bd3a7ae", "hello.rs", "Hello world!"),
            gist("9257657", ".gitignore", ""),
        ],
        Some("g1"),
    )
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
    page(
        3,
        vec![
            team_summary("core", "Core", false),
            team_summary("security", "Security", true),
            team_summary("triage", "Triage", false),
        ],
        None,
    )
}

/// A team with a parent, two of its seven members, one of its two
/// repositories and a child team.
pub fn team() -> TeamDetail {
    let member = |login: &str, name: Option<&str>| UserSummary {
        login: login.into(),
        name: name.map(Into::into),
        ..Default::default()
    };
    TeamDetail {
        team: team_summary("core", "Core", false),
        parent: Some(team_summary("maintainers", "Maintainers", false)),
        members: Capped::new(
            vec![
                member("octocat", Some("The Octocat")),
                member("hubot", None),
            ],
            7,
        ),
        repos: Capped::new(
            vec![TeamRepo {
                repo: RepoId::new("gold-silver-copper", "ghtui"),
                description: "GitHub in the terminal".into(),
                stars: 1200,
            }],
            2,
        ),
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
        severity: crate::api::browse::Severity::High,
        published_at: Some("2026-10-02T12:00:00Z".into()),
        updated_at: Some("2026-10-02T12:00:00Z".into()),
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
        ..Default::default()
    }
}

/// The first page of an inbox: two unread threads, two read.
pub fn notifications() -> Inbox {
    use crate::api::notifications::SubjectKind;
    let thread =
        |id: &str, repo: &str, kind, title: &str, reason: &str, unread, url: &str| Notification {
            id: id.into(),
            repo: RepoId::parse(repo).unwrap_or_else(|| RepoId::new("o", "r")),
            kind,
            title: title.into(),
            reason: reason.into(),
            // Newest first, as GitHub lists them.
            updated_at: match id {
                "101" => "2026-10-03T11:40:00Z",
                "102" => "2026-10-03T09:00:00Z",
                "103" => "2026-10-02T12:00:00Z",
                _ => "2026-09-28T12:00:00Z",
            }
            .into(),
            unread,
            url: format!("https://github.com/{repo}/{url}"),
        };
    let items = vec![
        thread(
            "101",
            "gold-silver-copper/ghtui",
            SubjectKind::PullRequest,
            "Theme: generate syntax palette from seed",
            "review_requested",
            true,
            "pull/12",
        ),
        thread(
            "102",
            "ratatui/ratatui",
            SubjectKind::Issue,
            "Add a virtualized list widget for very long collections",
            "mention",
            true,
            "issues/1820#issuecomment-7",
        ),
        thread(
            "103",
            "gold-silver-copper/ghtui",
            SubjectKind::CheckSuite,
            "CI workflow run failed for main branch",
            "ci_activity",
            false,
            "actions",
        ),
        thread(
            "104",
            "tokio-rs/tokio",
            SubjectKind::Release,
            "Tokio v1.48.0",
            "subscribed",
            false,
            "releases",
        ),
    ];
    Inbox {
        searched: items.len() as u64,
        list: page(items.len() as u64, items, Some("2")),
        cut: false,
    }
}

/// A repository's three advisories.
pub fn advisories() -> Vec<Advisory> {
    let mut low = advisory();
    low.ghsa = "GHSA-6gcq-wc29-5xf2".into();
    low.severity = crate::api::browse::Severity::Low;
    low.summary = "Deep JSON bodies can crash the process".into();
    let mut critical = advisory();
    critical.ghsa = "GHSA-3c6w-j9xm-8h2h".into();
    critical.severity = crate::api::browse::Severity::Critical;
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
        commit: Some("4b7d0c2e9a1f3d5b6c8e0a2b4d6f8a0c2e4b6d8f".into()),
        file: Some("Home.md".into()),
    }
}
/// main's merge queue: one merging, one waiting for its checks.
pub fn merge_queue() -> crate::api::queue::MergeQueue {
    use crate::api::model::QueueState;
    use crate::api::queue::QueueEntry;
    let entry = |position, state, number, title: &str| QueueEntry {
        position,
        state,
        enqueued_at: "2026-10-01T12:00:00Z".into(),
        by: Some("octocat".into()),
        eta: Some(600),
        solo: false,
        jump: false,
        number: Some(number),
        title: title.into(),
        author: Some("octocat".into()),
    };
    crate::api::queue::MergeQueue {
        url: "https://github.com/gold-silver-copper/ghtui/queue/main".into(),
        method: Some("SQUASH".into()),
        entries: Capped::new(
            vec![
                entry(1, QueueState::Locked, 7, "Add thing"),
                entry(2, QueueState::AwaitingChecks, 8, "Fix other thing"),
            ],
            2,
        ),
    }
}
/// A wiki's change to its Home and FAQ.
fn wiki_revision() -> crate::api::browse::WikiRevision {
    crate::api::browse::WikiRevision {
        oid: "4b7d0c2e9a1f3d5b6c8e0a2b4d6f8a0c2e4b6d8f".into(),
        author: "The Octocat".into(),
        at: "2026-10-01T12:00:00Z".into(),
        message: "Updated Home (markdown)".into(),
        pages: vec!["Home".into(), "FAQ".into()],
    }
}
/// A wiki's history: one change.
pub fn wiki_history() -> crate::api::browse::WikiHistory {
    crate::api::browse::WikiHistory {
        page: None,
        revisions: Capped::new(vec![wiki_revision()], 3),
    }
}
/// What a wiki's change did to its Home.
pub fn wiki_diff() -> crate::api::browse::WikiDiff {
    let lines = ["@@ -1 +1 @@", "-Welcome.", "+Welcome to the wiki."];
    crate::api::browse::WikiDiff {
        to: wiki_revision(),
        from: None,
        files: vec![crate::api::browse::WikiFileDiff {
            title: "Home".into(),
            lines: lines.map(str::to_owned).to_vec(),
        }],
        cut: false,
    }
}
/// octocat's commit `oid`.
fn commit_info(oid: impl Into<String>, headline: &str, date: &str) -> CommitInfo {
    CommitInfo {
        oid: oid.into(),
        headline: headline.into(),
        author: Person::User(Login::unchecked("octocat")),
        date: date.into(),
    }
}

fn label(name: &str, color: &str) -> Label {
    Label {
        name: name.into(),
        color: color.into(),
    }
}

pub fn issue_summary(number: u64, state: Status) -> IssueSummary {
    IssueSummary {
        kind: match state {
            Status::Issue(state) => IssueKind::Issue(state),
            Status::Pr(state) => IssueKind::Pr {
                id: NodeId::new(format!("PR_{number}")),
                state,
                review: None,
                approvals: 0,
                checks: None,
            },
        },
        repo: RepoId::new("gold-silver-copper", "ghtui"),
        number,
        title: format!("Issue {number}: pages wrap at the terminal width"),
        author: "octocat".into(),
        updated_at: "2026-10-03T09:00:00Z".into(),
        comments: number % 3,
        labels: if number.is_multiple_of(2) {
            vec![label("bug", "d73a4a")].into()
        } else {
            Vec::new().into()
        },
        created_at: "2026-09-29T09:00:00Z".into(),
    }
}

pub fn issue_results(next: Option<&str>) -> SearchResults {
    SearchResults::Issues(page(
        7,
        vec![
            issue_summary(14, Status::Issue(IssueState::Open)),
            issue_summary(11, Status::Issue(IssueState::Open)),
            issue_summary(8, Status::Issue(IssueState::Open)),
        ],
        next,
    ))
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
        milestone: Some(crate::api::model::MilestoneRef {
            number: 3,
            title: "Links".into(),
        }),
        timeline: vec![
            event("octocat", "2026-10-01T12:05:00Z", labeled("bug", "d73a4a")),
            event("octocat", "2026-10-01T12:06:00Z", labeled("ui", "1d76db")),
            Item::Comment(Comment {
                id: Some(1_000_001),
                author: "hubot".into(),
                body: "Agreed. 120 columns, centered?".into(),
                created_at: "2026-10-02T12:00:00Z".into(),
                editable: Some(editable("IC_1")),
                hiding: crate::api::manage::Hiding {
                    can_hide: true,
                    can_unhide: true,
                    ..Default::default()
                },
            }),
            event(
                "monalisa",
                "2026-10-02T13:00:00Z",
                What::CrossReferenced {
                    from: linked_pr(12, "Cap the reading width"),
                    will_close: true,
                },
            ),
        ]
        .into(),
        comment_count: 1,
        closed_by: vec![linked_pr(12, "Cap the reading width")].into(),
        issue_type: Some("Bug".into()),
        id: NodeId::new("I_14"),
        can_close: true,
        can_reopen: true,
        can_write: true,
        editable: editable("I_14"),
        sidebar: crate::api::edit::SidebarPermits {
            labels: true,
            assignees: true,
            milestone: true,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// An event on a timeline.
pub(crate) fn event(actor: &str, at: &str, what: What) -> Item {
    Item::Event(Event {
        actor: actor.into(),
        at: at.into(),
        what,
    })
}

fn labeled(name: &str, color: &str) -> What {
    What::Labeled {
        label: label(name, color),
        added: true,
    }
}

/// An open pull request in ghtui's repository, as a timeline names it.
pub(crate) fn linked_pr(number: u64, title: &str) -> Linked {
    Linked {
        repo: RepoId::new("gold-silver-copper", "ghtui"),
        number,
        title: title.into(),
        state: LinkedState::Pr(PrState::Open),
    }
}

/// A review on the fixture pull request.
pub(crate) fn review(id: u64, author: &str, state: ReviewState, body: &str, at: &str) -> Review {
    Review {
        id: Some(id),
        author: author.into(),
        state,
        body: body.into(),
        at: at.into(),
        commit: Some("1b2c3d4e5f60718293a4b5c6d7e8f9012345678a".into()),
        comments: 0,
        editable: editable(&format!("PRR_{id}")),
    }
}

/// The projects the fixture issue is in.
pub fn in_projects() -> Capped<crate::api::project::InProject> {
    use crate::api::model::NodeId;
    use crate::api::project::{FieldKind, FieldValue, ProjectField, SelectOption};
    let option = |id: &str, name: &str| SelectOption {
        id: id.into(),
        name: name.into(),
        color: "GREEN".into(),
        description: String::new(),
    };
    let fields = vec![
        ProjectField {
            id: NodeId::new("F_status"),
            name: "Status".into(),
            kind: FieldKind::SingleSelect(vec![
                option("todo", "Todo"),
                option("doing", "In progress"),
                option("done", "Done"),
            ]),
        },
        ProjectField {
            id: NodeId::new("F_estimate"),
            name: "Estimate".into(),
            kind: FieldKind::Number,
        },
        ProjectField {
            id: NodeId::new("F_title"),
            name: "Title".into(),
            kind: FieldKind::Title,
        },
    ];
    vec![crate::api::project::InProject {
        title: "Roadmap".into(),
        url: "https://github.com/users/octocat/projects/1".into(),
        id: NodeId::new("PVT_1"),
        item: NodeId::new("PVTI_14"),
        can_update: true,
        fields: fields.into(),
        values: vec![
            (
                NodeId::new("F_status"),
                FieldValue::Option {
                    id: "doing".into(),
                    name: "In progress".into(),
                },
            ),
            (NodeId::new("F_estimate"), FieldValue::Number("3".into())),
        ],
    }]
    .into()
}

/// Every review of the fixture pull request, oldest first.
pub fn reviews() -> Capped<Review> {
    activity().reviews().cloned().collect::<Vec<_>>().into()
}

/// The two commits of the fixture pull request.
fn pr_commits() -> [CommitInfo; 2] {
    [
        commit_info(
            "0a1b2c3d4e5f60718293a4b5c6d7e8f901234567",
            "Theme: derive syntax colors from the seed",
            "2026-09-30T12:00:00Z",
        ),
        commit_info(
            "1b2c3d4e5f60718293a4b5c6d7e8f9012345678a",
            "Check every syntax role against diff backgrounds",
            "2026-10-01T12:00:00Z",
        ),
    ]
}

/// gold-silver-copper/ghtui#12, open, approved, failing checks and in
/// conflict.
pub(crate) fn pr_detail() -> PrDetail {
    use crate::api::change::MergeMethod;
    use crate::api::model::{
        ChecksState, MergeState, Mergeable, MilestoneRef, PrPermits, ReviewDecision,
    };
    PrDetail {
        id: NodeId::new("PR_12"),
        merge_state: MergeState::Dirty,
        behind_by: Some(2),
        may: PrPermits {
            merge: true,
            close: true,
            reopen: true,
            edit: true,
            push_head: true,
            ..Default::default()
        },
        merge_methods: vec![MergeMethod::Squash, MergeMethod::Merge],
        auto_merge_allowed: true,
        title: "Theme: generate syntax palette from seed".into(),
        author: "octocat".into(),
        state: PrState::Open,
        updated_at: "2026-10-03T09:00:00Z".into(),
        additions: 120,
        deletions: 34,
        review: Some(ReviewDecision::Approved),
        checks: Some(ChecksState::Failing),
        body: "Derives keyword, string and type colors from the seed's HCT palettes.\n\n\
               Every syntax role is now checked against every diff background, in both \
               schemes and at both color depths."
            .into(),
        created_at: "2026-09-30T12:00:00Z".into(),
        base_ref: "main".into(),
        head_ref: "syntax-palette".into(),
        base_oid: "0123456789abcdef0123456789abcdef01234567".into(),
        head_oid: "fedcba9876543210fedcba9876543210fedcba98".into(),
        head_repo: Some("octocat/ghtui".into()),
        changed_files: 7,
        mergeable: Mergeable::Conflicting,
        milestone: Some(MilestoneRef {
            number: 3,
            title: "Links".into(),
        }),
        labels: vec![label("enhancement", "a2eeef"), label("theme", "7057ff")].into(),
        assignees: vec!["octocat".into()].into(),
        editable: editable("PR_12"),
        sidebar: crate::api::edit::SidebarPermits {
            labels: true,
            assignees: true,
            milestone: true,
            reviewers: true,
        },
        ..Default::default()
    }
}

pub fn activity() -> PrActivity {
    let committed = |c: CommitInfo| event("octocat", &c.date.clone(), What::Committed(c));
    let [first, second] = pr_commits().map(committed);
    // hubot's review started a thread on the theme, which octocat answered.
    let mut thread = thread("PRRT_1", Some(42), false, false);
    thread.path = "src/theme.rs".into();
    thread.original_line = Some(42);
    thread.diff_hunk = "@@ -40,3 +40,4 @@ impl Theme {\n     fn syntax(&self) -> Syntax {\n-        Syntax::default()\n+        let seed = self.seed;\n+        Syntax::from_seed(seed)".into();
    let asked = ReviewComment {
        body: "Is the seed's tone checked against the diff backgrounds?".into(),
        author: "hubot".into(),
        review: Some(2_000_002),
        url: "https://github.com/gold-silver-copper/ghtui/pull/12#discussion_r3000001".into(),
        ..thread.comments.items[0].clone()
    };
    let answered = ReviewComment {
        id: NodeId::new("PRRC_2"),
        body: "Yes, in `contrast_tests`.".into(),
        author: "octocat".into(),
        created_at: "2026-10-01T11:00:00Z".into(),
        review: Some(2_000_003),
        url: String::new(),
        ..asked.clone()
    };
    thread.comments = vec![asked, answered].into();
    let mut commented = review(
        2_000_002,
        "hubot",
        ReviewState::Commented,
        "Yes: every role is checked at both depths.",
        "2026-10-01T10:00:00Z",
    );
    commented.comments = 1;
    let latest = |author: &str, state, id, at: &str, commit: &str, teams: &[&str]| Latest {
        author: author.into(),
        state,
        id: Some(id),
        at: at.into(),
        commit: Some(commit.into()),
        for_teams: teams.iter().map(|t| (*t).to_owned()).collect(),
    };
    PrActivity {
        id: NodeId::new("PR_12"),
        timeline: vec![
            first,
            Item::Comment(Comment {
                id: Some(1_000_001),
                author: "hubot".into(),
                body: "Does this cover the 256-color fallback?".into(),
                created_at: "2026-10-01T09:00:00Z".into(),
                editable: Some(editable("IC_2")),
                ..Default::default()
            }),
            Item::Review(commented),
            second,
            event(
                "octocat",
                "2026-10-01T12:30:00Z",
                What::ReviewRequested {
                    who: Some(Reviewer::User("defunkt".into())),
                    added: true,
                },
            ),
            Item::Review(review(
                2_000_001,
                "monalisa",
                ReviewState::Approved,
                "",
                "2026-10-02T09:00:00Z",
            )),
        ]
        .into(),
        comment_count: 1,
        commits: pr_commits().to_vec().into(),
        requested: vec![
            Requested {
                who: Reviewer::User("defunkt".into()),
                code_owner: true,
            },
            Requested {
                who: Reviewer::Team("octo-org/core".into()),
                code_owner: false,
            },
        ]
        .into(),
        latest: vec![
            latest(
                "hubot",
                ReviewState::Commented,
                2_000_002,
                "2026-10-01T10:00:00Z",
                "0a1b2c3d4e5f60718293a4b5c6d7e8f901234567",
                &[],
            ),
            latest(
                "monalisa",
                ReviewState::Approved,
                2_000_001,
                "2026-10-02T09:00:00Z",
                "1b2c3d4e5f60718293a4b5c6d7e8f9012345678a",
                &["octo-org/design"],
            ),
        ]
        .into(),
        closing: vec![Linked {
            repo: RepoId::new("gold-silver-copper", "ghtui"),
            number: 14,
            title: "Pages wrap at the terminal width".into(),
            state: LinkedState::Issue(IssueState::Open),
        }]
        .into(),
        merge_commit: None,
        threads: vec![thread],
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
        pinned: vec![repo_summary("octocat/Hello-World", 3100)],
        repos: Capped::new(
            vec![
                repo_summary("octocat/Spoon-Knife", 13_000),
                repo_summary("octocat/linguist", 640),
            ],
            8,
        ),
        star_count: 120,
        readme: Some("### Hi there 👋\n\nI'm the Octocat. I like **terminals**.\n".into()),
        status: Some("🐙 Reviewing pull requests".into()),
        pronouns: Some("they/them".into()),
        socials: vec![(
            "@octocat@hachyderm.io".into(),
            "https://hachyderm.io/@octocat".into(),
        )],
        orgs: vec!["github".into(), "ratatui".into()].into(),
        contributions: Some(contributions()),
        ..Default::default()
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
                reviews: vec![item("ratatui/ratatui", 1900, "Add a scrollbar widget")],
                ..Default::default()
            },
            MonthActivity {
                month: "2026-09".into(),
                commits: vec![("gold-silver-copper/ghtui".into(), 17)],
                issues: vec![item(
                    "ratatui/ratatui",
                    1888,
                    "Unicode width of emoji in tables",
                )],
                ..Default::default()
            },
        ],
        // More reviews than were fetched, from October back.
        short: crate::api::browse::Short {
            reviews: Some("2026-10".into()),
            ..Default::default()
        },
    }
}

/// The first page of a profile's repositories.
pub fn profile_repos() -> Results<RepoSummary> {
    page(8, profile().repos.items, Some("p1"))
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
        website: Some("https://ratatui.rs".into()),
        is_org: true,
        repos: Capped::new(repos, 24),
        readme: Some(
            "Welcome to **Ratatui**. Start with the [tutorial](https://ratatui.rs).\n".into(),
        ),
        verified: true,
        people: Capped::new(vec!["joshka".into(), "orhun".into(), "kdheepak".into()], 14),
        ..Default::default()
    }
}

pub fn repo_results() -> SearchResults {
    SearchResults::Repos(page(
        1243,
        vec![
            repo_summary("sxyazi/yazi", 42_600),
            repo_summary("jarun/nnn", 22_000),
        ],
        Some("c2"),
    ))
}

/// A page of people, with more to load.
pub fn people() -> Results<UserSummary> {
    let person = |login: &str, name: Option<&str>, bio: Option<&str>| UserSummary {
        login: login.into(),
        name: name.map(str::to_owned),
        bio: bio.map(str::to_owned),
        ..Default::default()
    };
    page(
        1234,
        vec![
            person("octocat", Some("The Octocat"), Some("GitHub's mascot.")),
            person("hubot", None, None),
            person("monalisa", Some("Mona Lisa Octocat"), None),
        ],
        Some("u1"),
    )
}

/// A page of a repository's forks.
pub fn forks() -> Results<RepoSummary> {
    page(
        87,
        vec![
            repo_summary("octocat/ghtui", 12),
            repo_summary("hubot/ghtui", 3),
        ],
        Some("f1"),
    )
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
        id: Some(272_100_001),
        name: "ghtui 0.2.0".into(),
        tag: "v0.2.0".into(),
        published_at: Some("2026-09-30T12:00:00Z".into()),
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
        ..Default::default()
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
    page(9, vec![release(false), pre], Some("r1"))
}

pub fn tags() -> Results<TagInfo> {
    let tag = |name: &str, oid: &str, date: &str| TagInfo {
        name: name.into(),
        oid: Some(oid.into()),
        date: Some(date.into()),
    };
    page(
        2,
        vec![
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
        None,
    )
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
        ]
        .into(),
        artifacts: vec![crate::api::workflows::Artifact {
            id: 31,
            name: "coverage-report".into(),
            size: 1_572_864,
            expires_at: Some("2026-12-31T12:00:00Z".into()),
            expired: false,
        }]
        .into(),
    }
}

/// A failing test's annotation, on a line of a file.
pub fn check_annotation() -> crate::api::workflows::Annotation {
    crate::api::workflows::Annotation {
        path: Some("src/theme.rs".into()),
        start_line: 3,
        end_line: 3,
        level: crate::api::workflows::Level::Failure,
        title: "heat_levels_stay_apart".into(),
        message: "assertion failed: contrast(a, b) > 1.5".into(),
        url: Some(
            "https://github.com/gold-silver-copper/ghtui/blob/0a1b2c3d4e5f60718293a4b5c6d7e8f901234567/src/theme.rs#L3".into(),
        ),
    }
}

/// A job whose test step failed, and its log.
pub fn job() -> (Job, crate::api::browse::JobLog) {
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
        attempt: 1,
        run: crate::api::browse::JobRun {
            name: "CI".into(),
            number: 412,
            outcome: CheckOutcome::Failure,
            attempt: 1,
            rerun: None,
        },
        annotations: vec![check_annotation()].into(),
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
    let log = crate::api::browse::JobLog {
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
        workflow: "CI".into(),
        number,
        event: "push".into(),
        branch: Some("main".into()),
        outcome,
        actor: Some("octocat".into()),
        created_at: Some("2026-10-03T12:00:00Z".into()),
        started_at: Some("2026-10-03T12:00:00Z".into()),
        updated_at: Some("2026-10-03T12:04:31Z".into()),
    };
    let workflow = Workflow {
        name: "CI".into(),
        path: ".github/workflows/ci.yml".into(),
        disabled: None,
    };
    let runs = page(
        412,
        vec![
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
        Some("2"),
    );
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
        author: Some(Person::User(Login::unchecked("octocat"))),
        date: Some("2026-10-02T12:00:00Z".into()),
        pr,
    };
    page(
        14,
        vec![
            branch(
                "links",
                false,
                "Discussions are pages",
                Some((6, PrState::Open)),
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
                Some((5, PrState::Merged)),
            ),
        ],
        Some("b1"),
    )
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
        results: page(
            3,
            vec![
                milestone_info(3, "Links", false),
                milestone_info(5, "Pages", false),
                later,
            ],
            None,
        ),
    }
}

/// A milestone with an issue and a pull request.
pub fn milestone() -> MilestoneDetail {
    MilestoneDetail {
        info: milestone_info(3, "Links", false),
        items: page(
            10,
            vec![
                issue_summary(42, Status::Issue(IssueState::Open)),
                issue_summary(6, Status::Pr(PrState::Merged)),
            ],
            Some("m1"),
        ),
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
        results: page(
            3,
            vec![
                deployment(
                    "production",
                    CheckOutcome::Success,
                    "active",
                    Some("https://ghtui.example.com"),
                ),
                deployment("staging", CheckOutcome::Failure, "failure", None),
                deployment("staging", CheckOutcome::Neutral, "inactive", None),
            ],
            None,
        ),
    }
}
/// Two tags compared: the first of many commits.
pub fn comparison() -> Comparison {
    let commit =
        |oid: char, headline, date| commit_info(oid.to_string().repeat(40), headline, date);
    Comparison {
        ahead: 120,
        commits: Capped::new(
            vec![
                commit('a', "Branches are a page", "2026-10-01T10:00:00Z"),
                commit('b', "Milestones are pages", "2026-10-01T12:00:00Z"),
                commit('c', "Deployments are a page", "2026-10-02T12:00:00Z"),
            ],
            120,
        ),
        from: "1".repeat(40),
        to: "c".repeat(40),
        files: 14,
        additions: 812,
        deletions: 40,
        ..Default::default()
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
        results: page(
            48,
            vec![
                summary(14603, "How do I review a PR offline?", "Q&A", true),
                summary(14590, "Show the contribution graph", "Ideas", false),
            ],
            Some("d1"),
        ),
    }
}

/// A question with an answer and a reply.
pub fn discussion() -> DiscussionDetail {
    use crate::api::edit::{Editable, Reaction, ReactionContent};
    let comment = |id: u64, author: &str, body: &str, hour: u32| Comment {
        id: Some(id),
        author: author.into(),
        body: body.into(),
        created_at: format!("2026-10-03T{hour:02}:00:00Z"),
        editable: Some(Editable {
            id: NodeId::new(format!("DC_{id}")),
            can_update: author == "octocat",
            can_delete: author == "octocat",
            can_react: true,
            reactions: Vec::new(),
        }),
        ..Default::default()
    };
    let mut answer = DiscussionComment {
        comment: comment(
            18765024,
            "hubot",
            "Fetch the PR first; the diff then works offline.",
            10,
        ),
        upvotes: 5,
        answer: true,
        can_unmark: true,
        // More than came: the page says how many more.
        replies: Capped::new(
            vec![comment(18765030, "octocat", "That works, thanks!", 11)],
            3,
        ),
        ..Default::default()
    };
    if let Some(e) = &mut answer.comment.editable {
        e.reactions = vec![Reaction {
            content: ReactionContent::Hooray,
            count: 3,
            mine: true,
        }];
    }
    DiscussionDetail {
        id: NodeId::new("D_14603"),
        repo: RepoId::new("cli", "cli"),
        number: 14603,
        title: "How do I review a PR offline?".into(),
        body: "I'm on a plane a lot. Can I review without a connection?".into(),
        author: "octocat".into(),
        created_at: "2026-10-02T12:00:00Z".into(),
        category: "Q&A".into(),
        answerable: true,
        answered: true,
        can_close: true,
        can_reopen: true,
        can_lock: true,
        upvotes: 12,
        editable: Editable {
            id: NodeId::new("D_14603"),
            can_update: true,
            can_react: true,
            ..Default::default()
        },
        answer: Some(Box::new(DiscussionComment { ..answer.clone() })),
        answer_by: Some("octocat".into()),
        comments: vec![
            answer,
            DiscussionComment {
                comment: comment(18765100, "monalisa", "Drafts are saved locally too.", 12),
                upvotes: 1,
                can_mark: true,
                ..Default::default()
            },
        ]
        .into(),
        ..Default::default()
    }
}
/// A discussion search's result.
pub fn discussion_results() -> SearchResults {
    let summary = discussions().results.items.remove(0);
    SearchResults::Discussions(page(
        6065,
        vec![DiscussionHit {
            repo: RepoId::new("cli", "cli"),
            summary,
        }],
        Some("2"),
    ))
}

/// A commit search's results.
pub fn commit_results() -> SearchResults {
    let hit = |oid: char, headline: &str| CommitHit {
        repo: RepoId::new("ratatui", "ratatui"),
        commit: commit_info(oid.to_string().repeat(40), headline, "2026-10-01T12:00:00Z"),
    };
    SearchResults::Commits(page(
        60,
        vec![
            hit('a', "perf(terminal): skip a redundant clear"),
            hit('b', "feat: a tui widget"),
        ],
        Some("2"),
    ))
}

/// A code search's results.
pub fn code_results() -> SearchResults {
    let hit = |path: &str| CodeHit {
        repo: RepoId::new("ratatui", "ratatui"),
        path: path.into(),
        sha: "f60d17e29bb58b7e699e9befbb97298ec22aac14".into(),
        lines: Vec::new(),
        rev: None,
    };
    SearchResults::Code(page(
        158,
        vec![hit("ratatui-termina/src/lib.rs"), hit("src/terminal.rs")],
        Some("2"),
    ))
}
pub fn user_results() -> SearchResults {
    SearchResults::Users(page(
        1,
        vec![UserSummary {
            login: "octocat".into(),
            name: Some("The Octocat".into()),
            ..Default::default()
        }],
        None,
    ))
}

/// An organization's projects.
pub fn projects() -> Results<crate::api::project::ProjectSummary> {
    let project = |number: u64, title: &str, closed: bool| crate::api::project::ProjectSummary {
        number,
        title: title.into(),
        description: if closed {
            String::new()
        } else {
            "What's next for ghtui".into()
        },
        url: format!("https://github.com/orgs/gold-silver-copper/projects/{number}"),
        closed,
        public: true,
        updated_at: "2026-10-02T09:00:00Z".into(),
        items: 5,
    };
    page(
        2,
        vec![project(1, "Roadmap", false), project(2, "Launch", true)],
        None,
    )
}

/// gold-silver-copper's Roadmap project: a board by Status and a table,
/// with issues, a pull request and drafts.
pub fn project() -> crate::api::project::Project {
    use crate::api::project::{
        Draft, FieldKind, FieldValue, ItemContent, Iteration, Layout, Linked, Project,
        ProjectField, ProjectItem, ProjectView, SelectOption,
    };
    let field = |id: &str, name: &str, kind| ProjectField {
        id: NodeId::new(id),
        name: name.into(),
        kind,
    };
    let option = |id: &str, name: &str, color: &str, description: &str| SelectOption {
        id: id.into(),
        name: name.into(),
        color: color.into(),
        description: description.into(),
    };
    let sprint = |id: &str, title: &str, start: &str, completed| Iteration {
        id: id.into(),
        title: title.into(),
        start: start.into(),
        days: 14,
        completed,
    };
    let fields = vec![
        field("F_title", "Title", FieldKind::Title),
        field("F_assignees", "Assignees", FieldKind::Assignees),
        field(
            "F_status",
            "Status",
            FieldKind::SingleSelect(vec![
                option("O_todo", "Todo", "GRAY", "Not started"),
                option("O_doing", "In Progress", "YELLOW", ""),
                option("O_done", "Done", "GREEN", "Shipped"),
            ]),
        ),
        field("F_labels", "Labels", FieldKind::Labels),
        field("F_estimate", "Estimate", FieldKind::Number),
        field(
            "F_sprint",
            "Sprint",
            FieldKind::Iteration(vec![
                sprint("I_2", "Sprint 2", "2026-10-01", false),
                sprint("I_1", "Sprint 1", "2026-09-17", true),
            ]),
        ),
        field("F_due", "Due", FieldKind::Date),
    ];
    let ids = |ids: &[&str]| ids.iter().map(|id| NodeId::new(*id)).collect::<Vec<_>>();
    let views = vec![
        ProjectView {
            number: 1,
            name: "Board".into(),
            fields: ids(&[
                "F_title",
                "F_status",
                "F_assignees",
                "F_labels",
                "F_estimate",
            ]),
            column: Some(NodeId::new("F_status")),
            ..Default::default()
        },
        ProjectView {
            number: 2,
            name: "Plan".into(),
            layout: Layout::Table,
            filter: "-status:Done".into(),
            fields: ids(&[
                "F_title",
                "F_status",
                "F_assignees",
                "F_estimate",
                "F_sprint",
                "F_due",
            ]),
            sort: vec![(NodeId::new("F_estimate"), true)],
            ..Default::default()
        },
    ];
    let repo = RepoId::new("gold-silver-copper", "ghtui");
    let linked = |number: u64, title: &str, pr: bool, who: &[&str], labels: Vec<Label>| Linked {
        repo: repo.clone(),
        number,
        title: title.into(),
        url: if pr {
            format!("https://github.com/gold-silver-copper/ghtui/pull/{number}")
        } else {
            format!("https://github.com/gold-silver-copper/ghtui/issues/{number}")
        },
        assignees: Capped::new(
            who.iter().map(|w| (*w).to_owned()).collect(),
            who.len() as u64,
        ),
        labels: Capped::new(labels.clone(), labels.len() as u64),
        milestone: None,
    };
    let status = |id: &str, name: &str| {
        (
            NodeId::new("F_status"),
            FieldValue::Option {
                id: id.into(),
                name: name.into(),
            },
        )
    };
    let enhancement = label("enhancement", "a2eeef");
    let item = |n: u64, content, values| ProjectItem {
        id: NodeId::new(format!("PVTI_{n}")),
        number: 1000 + n,
        archived: false,
        content,
        values,
    };
    let items = vec![
        item(
            1,
            ItemContent::Issue(
                linked(
                    14,
                    "Projects aren't there yet",
                    false,
                    &["alice"],
                    vec![enhancement],
                ),
                IssueState::Open,
            ),
            vec![
                status("O_todo", "Todo"),
                (NodeId::new("F_estimate"), FieldValue::Number("3".into())),
                (NodeId::new("F_due"), FieldValue::Date("2026-10-31".into())),
            ],
        ),
        item(
            2,
            ItemContent::Pr(
                linked(12, "Show a project's views", true, &["bob"], Vec::new()),
                PrState::Open,
            ),
            vec![
                status("O_doing", "In Progress"),
                (NodeId::new("F_estimate"), FieldValue::Number("5".into())),
                (
                    NodeId::new("F_sprint"),
                    FieldValue::Iteration {
                        id: "I_2".into(),
                        title: "Sprint 2".into(),
                    },
                ),
            ],
        ),
        item(
            3,
            ItemContent::Draft(Draft {
                id: NodeId::new("DI_3"),
                title: "Write the board's docs".into(),
                body: "Explain **h** and **l**, and moving cards.".into(),
                ..Default::default()
            }),
            vec![status("O_todo", "Todo")],
        ),
        item(
            4,
            ItemContent::Issue(
                linked(9, "Boards scroll sideways", false, &[], Vec::new()),
                IssueState::Closed,
            ),
            vec![status("O_done", "Done")],
        ),
        item(
            5,
            ItemContent::Draft(Draft {
                id: NodeId::new("DI_5"),
                title: "Someday: roadmaps".into(),
                ..Default::default()
            }),
            Vec::new(),
        ),
    ];
    Project {
        id: NodeId::new("PVT_1"),
        title: "Roadmap".into(),
        description: "What's next for ghtui".into(),
        url: "https://github.com/orgs/gold-silver-copper/projects/1".into(),
        closed: false,
        public: true,
        can_update: true,
        views: Capped::new(views, 2),
        fields: Capped::new(fields, 7),
        view: 1,
        items: page(5, items, None),
        archived: page(
            1,
            vec![ProjectItem {
                archived: true,
                ..item(
                    6,
                    ItemContent::Draft(Draft {
                        id: NodeId::new("DI_6"),
                        title: "Classic projects".into(),
                        ..Default::default()
                    }),
                    vec![status("O_done", "Done")],
                )
            }],
            None,
        ),
    }
}

/// Your subscription to a thread: notified of everything.
pub fn subscription() -> crate::api::watch::Subscription {
    crate::api::watch::Subscription {
        id: NodeId::new("I_sub"),
        state: Some(crate::api::watch::SubscriptionState::Subscribed),
        can: true,
    }
}

/// A page of `total` items, `next` after it.
pub(crate) fn page<T>(total: u64, items: Vec<T>, next: Option<&str>) -> Results<T> {
    let next = next.map(str::to_owned);
    Results { total, items, next }
}

/// Answers every fetch `cmds` start, and every fetch those answers
/// start, with its [`sample`].
pub(crate) fn served(state: &mut State, cmds: &[Cmd]) {
    let mut asked = asked(cmds);
    let mut answered = std::collections::HashSet::new();
    while let Some(key) = asked.pop() {
        if answered.insert(key.clone()) {
            let data = sample(&key);
            asked.extend(self::asked(&fetched(state, key, data)));
        }
    }
}

/// What GitHub might answer for `key`.
pub(crate) fn sample(key: &DataKey) -> Data {
    use crate::housekeeping::fixtures as housekeeping;
    use crate::site::fixtures as site;
    use DataKey as K;
    let search = |kind: &SearchKind| match kind {
        SearchKind::Repos => repo_results(),
        SearchKind::Issues | SearchKind::Pulls => issue_results(Some("c1")),
        SearchKind::Users => user_results(),
        SearchKind::Discussions => discussion_results(),
        SearchKind::Commits => commit_results(),
        SearchKind::Code => code_results(),
    };
    match key {
        K::Repo(_) => Data::from(overview()),
        K::Readme(_) => Data::Readme(Some(Box::new(readme()))),
        K::Tree(..) => Data::Tree(tree()),
        K::Blob(..) => Data::from(blob()),
        K::Search(kind, _) => Data::from(search(kind)),
        K::StateCounts(_) => {
            Data::StateCounts(crate::api::browse::StateCounts { open: 2, closed: 5 })
        }
        K::Issue(..) => Data::Issue(Some(Box::new(issue()))),
        K::Pr(_) => Data::from(pr_detail()),
        K::PrActivity(_) => Data::from(activity()),
        K::Reviews(_) => Data::from(reviews()),
        K::CommitComments(..) => Data::from(Capped::from(vec![commit_comment()])),
        K::Lines(..) => Data::Lines(Arc::default()),
        K::InProjects(..) => Data::from(in_projects()),
        K::Profile(login) if login == "ratatui" => Data::from(org_profile()),
        K::Profile(_) => Data::from(profile()),
        K::Files(..) => Data::Files(Arc::new(vec!["README.md".into()]), false),
        K::Refs(_) => Data::Refs(Box::default()),
        K::NewForm(_) => Data::from(new_form()),
        K::RefIn(..) => Data::RefIn(None),
        K::LastCommits(..) => Data::LastCommits(Arc::new(last_commits())),
        K::Commit(..) => Data::from(commit()),
        K::History(..) => Data::from(history(Some("h1"))),
        K::PrChecks(_) | K::CommitChecks(..) => Data::from(checks()),
        K::Users(_) => Data::from(people()),
        K::Forks(_) => Data::from(forks()),
        K::OwnerRepos(..) | K::Stars(_) => Data::from(profile_repos()),
        K::Releases(_) => Data::from(releases()),
        K::Release(..) => Data::from(release(true)),
        K::Tags(_) => Data::from(tags()),
        K::Branches(_) => Data::from(branches()),
        K::Wiki(..) => Data::from(wiki()),
        K::WikiHistory(..) => Data::from(wiki_history()),
        K::WikiDiff(..) => Data::from(wiki_diff()),
        K::Advisories(_) => {
            let items = advisories();
            let total = items.len() as u64;
            Data::from(page(total, items, None))
        }
        K::Advisory(..) => Data::from(advisory()),
        K::Sidebar(..) => Data::from(sidebar()),
        K::Teams(_) => Data::from(teams()),
        K::Team(..) => Data::from(team()),
        K::Gist(_) => Data::from(gist()),
        K::Gists(_) => Data::from(gists()),
        K::Blame(..) => Data::from(blame()),
        K::Compare(..) => Data::from(comparison()),
        K::Deployments(..) => Data::from(deployments()),
        K::Milestones(..) => Data::from(milestones()),
        K::Milestone(..) => Data::from(milestone()),
        K::Discussions(..) => Data::from(discussions()),
        K::Discussion(..) => Data::from(discussion()),
        K::Run(..) => Data::from(workflow_run()),
        K::Job(..) => Data::from(job().0),
        K::JobLog(..) => Data::Log(Arc::new(job().1)),
        K::Workflow(..) => Data::from(workflow_runs().0),
        K::Runs(..) => Data::from(workflow_runs().1),
        K::Notifications(_) => Data::from(notifications()),
        K::Projects(_) => Data::from(projects()),
        K::Project(..) => Data::from(project()),
        K::Alerts(_, kind, _) => Data::from(crate::security::fixtures::alerts(*kind)),
        K::Alert(a) => Data::from(crate::security::fixtures::alert(a.kind, a.number)),
        K::Subscription(_) => Data::from(subscription()),
        K::Activity(_) => Data::from(site::activity()),
        K::Traffic(_) => Data::from(site::traffic()),
        K::Dependencies(_) => Data::from(site::dependencies()),
        K::Community(_) => Data::from(site::community()),
        K::MergeQueue(..) => Data::from(merge_queue()),
        K::Featured => Data::from(site::featured()),
        K::Topic(_) => Data::from(site::rust()),
        K::StarLists(_) => Data::from(site::star_lists()),
        K::StarList(..) => Data::from(site::list_page()),
        K::MyLists(_) => Data::from(site::my_lists()),
        K::Packages(_) => Data::from(site::packages()),
        K::Package(_) => Data::from(site::package_detail()),
        K::SavedReplies => Data::from(site::saved_replies()),
        K::Labels(_) => Data::from(housekeeping::labels()),
        K::Owners => Data::from(housekeeping::owners()),
        K::Templates => Data::from(housekeeping::templates()),
        K::Settings(_, page) => Data::from(crate::settings::fixtures::page(*page)),
        K::OpenIssues(_) => Data::from(Capped::from(vec![(7, "Wrapping".into())])),
        K::MoveTargets(_) => Data::MoveTargets(Box::default()),
        K::Categories(_) => Data::from(crate::api::manage::Categories {
            repo: NodeId::new("R_1"),
            categories: Capped::default(),
        }),
        K::Clone(_) => Data::Clone(None),
        K::Head { .. } => Data::from(crate::create::HeadState {
            local: None,
            commits: Capped::default(),
        }),
        K::TagLook(..) => Data::from(crate::api::release::TagLook {
            exists: true,
            release: None,
        }),
        K::BranchRules(..) => Data::from(crate::api::housekeeping::BranchRules {
            protected: false,
            undeletable: false,
        }),
        K::FileLook(..) => Data::from(file_look()),
        K::Mentionable(_) => Data::Mentionable(Box::default()),
        K::Suggest(_) => Data::Suggestions(Vec::new()),
        K::Unread => Data::Unread(crate::api::notifications::Unread {
            count: 3,
            more: false,
            poll_secs: 60,
        }),
    }
}

/// Where a file's form starts: the README on main.
pub fn file_look() -> crate::api::housekeeping::FileLook {
    let oid = "8d1a76430406c877b35d0b627e7f796dcf0dfeca";
    crate::api::housekeeping::FileLook {
        repo_id: NodeId::new("R_1"),
        branch_head: Some(oid.into()),
        commit: oid.into(),
        file: Some(crate::api::housekeeping::FileAt {
            oid: "b10b".into(),
            text: blob().text,
        }),
    }
}

/// Git's answer to whether ghtui runs in a clone of `repo`.
pub(crate) fn cloned(
    state: &mut State,
    repo: &crate::api::model::RepoId,
    clone: Option<crate::git::local::Clone>,
) -> Cmds {
    let key = DataKey::Clone(repo.clone());
    fetched(state, key, Data::Clone(clone.map(Box::new)))
}
