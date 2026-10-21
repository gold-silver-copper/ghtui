//! Page builders: GitHub's pages (repository, file, lists, issue, pull
//! request, profile, search, home) as [`Page`]s, laid out the way GitHub
//! lays them out: boxes for file lists, READMEs, lists and comments, and a
//! sidebar when there's room. Every link is the GitHub URL of what it
//! points at; `ghtui:` links are actions (star, load more, filter...).

use std::collections::HashMap;

use crate::api::browse::{
    Advisory, Blame, Blob, BranchInfo, CheckItem, CheckOutcome, Checks, Comment, CommitDetail,
    CommitInfo, Comparison, Contributions, DeploymentList, DiscussionDetail, DiscussionList,
    EntryKind, Gist, GistSummary, IssueDetail, IssueKind, IssueState, IssueSummary, Job, JobLog,
    MilestoneDetail, MilestoneInfo, MilestoneList, Person, PrActivity, PrState, Profile, Readme,
    Release, RepoOverview, RepoSort, RepoSummary, Results, Reviewer, RunFilter, RunSummary,
    SearchKind, SearchResults, Severity, Short, Status, TagInfo, TeamDetail, TeamSummary,
    TreeEntry, UserSummary, WikiPage, Workflow, WorkflowRun,
};
use crate::api::model::{
    Capped, ChecksState, Label, Mergeable, MilestoneRef, PrDetail, PrRef, Readiness, RepoId,
    ReviewDecision, ReviewState, ReviewThread,
};
use crate::api::notifications::{Inbox, InboxFilter, SubjectKind, reason_label};
use crate::store::{DraftComment, DraftSide};
use crate::theme::{Bg, Syntax};

use crate::ui::markdown::{self, LinkBase};
use crate::ui::page::{Frame, Link, Page, PageLine, Role, Seg, Tone};
use crate::ui::text::{noun, short_sha};
use crate::ui::{Fetched, Icons, cols, time};

mod commits;
mod conversation;
mod create;
mod files;
mod home;
mod housekeeping;
mod inbox;
mod lists;
mod profile;
mod project;
mod projects;
mod pull;
mod repository;
mod review;
mod runs;
mod security;
mod settings;
mod timeline;
mod writes;

// Insights, Explore, starred lists and packages (`crate::site`).
mod charts;
mod explore;
mod insights;
mod packages;
mod starred;

pub use commits::*;
pub use conversation::*;
pub use create::*;
pub use files::*;
pub use home::*;
pub use housekeeping::*;
pub use inbox::*;
pub use lists::*;
pub use profile::*;
pub use project::*;
pub use projects::*;
pub use pull::*;
pub use repository::*;
pub use review::*;
pub use runs::*;
pub use security::*;
pub use settings::*;
pub use timeline::{FOLD_AFTER, LinesAt, Marks, commit_comments, pinned_issues};
pub use writes::*;

pub use explore::*;
pub use insights::*;
pub use packages::*;
pub use starred::*;

// ---- URLs ----------------------------------------------------------------------

pub mod url;

// ---- small helpers ---------------------------------------------------------------

/// 1234 → "1.2k".
pub fn compact(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..999_950 => trim_zero(&format!("{:.1}k", n as f64 / 1_000.0)),
        _ => trim_zero(&format!("{:.1}m", n as f64 / 1_000_000.0)),
    }
}

fn trim_zero(s: &str) -> String {
    s.replace(".0k", "k").replace(".0m", "m")
}

/// `1 file`, `2 files`, `1.2k repositories`.
fn plural(n: u64, word: &str) -> String {
    format!("{} {}", compact(n), noun(n, word))
}

fn chip(text: impl Into<String>, bg: Bg) -> Seg {
    Seg::new(format!(" {} ", text.into()), Role::Chip(bg))
}

fn space() -> Seg {
    Seg::new(" ", Role::Body)
}

fn labels(segs: &mut Vec<Seg>, labels: &Capped<Label>) {
    for l in labels {
        segs.push(space());
        segs.push(Seg::new(
            format!(" {} ", l.name),
            Role::Label(l.color.clone()),
        ));
    }
    more_chips(segs, labels);
}

/// ` +3` after chips, for the ones GitHub has that weren't fetched.
fn more_chips<T>(segs: &mut Vec<Seg>, list: &Capped<T>) {
    if list.left_out() > 0 {
        segs.push(Seg::new(format!(" +{}", list.left_out()), Role::Meta));
    }
}

fn link_seg(page: &mut Page, text: impl Into<String>, url: impl Into<Link>, role: Role) -> Seg {
    let link = page.link(url);
    Seg::linked(text, role, link)
}

/// A login, linked to its profile.
fn login_seg(page: &mut Page, login: &str, role: Role) -> Seg {
    link_seg(page, login.to_owned(), url::user(login), role)
}

/// A commit's author or committer: linked to their profile only when
/// GitHub linked an account; a git name is just text.
fn person_seg(page: &mut Page, who: &Person, role: Role) -> Seg {
    match who.login() {
        Some(login) => login_seg(page, login, role),
        None => Seg::new(who.name().to_owned(), role),
    }
}

/// A button: ` label ` on a raised tone.
fn button(page: &mut Page, text: impl Into<String>, url: impl Into<Link>) -> Seg {
    link_seg(
        page,
        format!(" {} ", text.into()),
        url,
        Role::Chip(Bg::ContainerHigh),
    )
}

/// GitHub's "Go to file" field, with its key.
fn go_to_file(page: &mut Page, keys: Keys<'_>) -> Vec<Seg> {
    let link = page.link(Link::FindFile);
    vec![
        Seg::linked(" ⌕ Go to file ", Role::Chip(Bg::ContainerHigh), link),
        Seg::linked(
            format!(" {} ", keys.find_file),
            Role::Chip(Bg::ContainerHighest),
            link,
        ),
    ]
}

/// `+12 −3`.
fn changes(additions: u64, deletions: u64) -> [Seg; 3] {
    [
        Seg::new(format!("+{additions}"), Role::Added),
        space(),
        Seg::new(format!("−{deletions}"), Role::Removed),
    ]
}

/// Gray bars where rows are still loading, as GitHub draws them.
fn skeleton(page: &mut Page) {
    let room = usize::from(page.room(Frame::Body, 0));
    for (i, share) in [60, 40, 52, 34].into_iter().enumerate() {
        if i > 0 {
            page.box_gap();
        }
        let width = (room * share / 100).max(4);
        body(
            page,
            vec![Seg::new(" ".repeat(width), Role::Chip(Bg::ContainerHigh))],
        );
    }
}

/// A box of rows still loading.
pub fn loading_box(page: &mut Page, title: Vec<Seg>) {
    page.box_top(title, Vec::new());
    skeleton(page);
    page.box_bottom();
}

/// A flash banner across the page (errors), like GitHub's.
pub fn flash(page: &mut Page, text: &str) {
    let width = usize::from(page.room(Frame::None, 0));
    for line in
        crate::ui::page::wrap_segs(vec![Seg::new(text, Role::Body)], width.saturating_sub(4))
    {
        let text: String = line.iter().map(Seg::text).collect();
        let pad = width.saturating_sub(crate::ui::text::width(&text) + 2);
        page.line(vec![Seg::new(
            format!("  {text}{}", " ".repeat(pad)),
            Role::Chip(Bg::ErrorContainer),
        )]);
    }
}

/// A line inside a box, with nothing on its right.
fn body(page: &mut Page, segs: Vec<Seg>) {
    page.box_line(segs, Vec::new(), 0);
}

/// A box's empty state.
fn empty_row(page: &mut Page, text: &str) {
    body(page, vec![Seg::new(text, Role::Meta)]);
}

/// A box holding only its empty state.
fn empty_box(page: &mut Page, title: Seg, text: &str) {
    page.box_top(vec![title], Vec::new());
    empty_row(page, text);
    page.box_bottom();
}

/// The lines `build` adds, as one item opening `target`.
fn item(page: &mut Page, target: impl Into<Link>, build: impl FnOnce(&mut Page, u32)) {
    let start = page.lines.len();
    let link = page.link(target);
    build(page, link);
    page.item(start, link);
}

/// A file shown as Markdown, read as prose rather than as code.
pub fn is_markdown(path: &str) -> bool {
    path.to_ascii_lowercase().ends_with(".md")
}

/// Each name once, in order.
fn unique<'a>(names: impl IntoIterator<Item = &'a String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for name in names {
        if !out.contains(name) {
            out.push(name.clone());
        }
    }
    out
}

/// What a list in a box leaves out, said after its rows.
#[derive(Clone)]
enum Rest {
    All,
    /// This many are only on GitHub.
    OnGitHub(u64),
    /// This many are only on GitHub, listed at this URL.
    At(u64, String),
    /// Another page loads; of `total`, if GitHub counted them.
    NextPage {
        total: Option<u64>,
    },
    /// GitHub has more, uncounted, that ghtui doesn't read.
    Uncounted,
}

/// A list drawn in a box: its rows and what it left out. A bare slice
/// isn't one, so a cut-short list can't be drawn as if whole; [`Whole`]
/// says a list is all there is.
trait Rows<T> {
    fn rows(&self) -> &[T];
    fn rest(&self) -> Rest;
}

/// A list that is all there is.
struct Whole<'a, T>(&'a [T]);

impl<T> Rows<T> for Whole<'_, T> {
    fn rows(&self) -> &[T] {
        self.0
    }
    fn rest(&self) -> Rest {
        Rest::All
    }
}

impl<T> Rows<T> for Capped<T> {
    fn rows(&self) -> &[T] {
        self
    }
    fn rest(&self) -> Rest {
        match self.left_out() {
            0 => Rest::All,
            n => Rest::OnGitHub(n),
        }
    }
}

/// A capped list whose rest GitHub lists at a page of its own (if known).
struct At<'a, T>(&'a Capped<T>, Option<String>);

impl<T> Rows<T> for At<'_, T> {
    fn rows(&self) -> &[T] {
        self.0
    }
    fn rest(&self) -> Rest {
        match (self.0.rest(), &self.1) {
            (Rest::OnGitHub(n), Some(url)) => Rest::At(n, url.clone()),
            (rest, _) => rest,
        }
    }
}

impl<T> Rows<T> for Results<T> {
    fn rows(&self) -> &[T] {
        &self.items
    }
    fn rest(&self) -> Rest {
        // A total no more than what's here means GitHub didn't count; more
        // with no next page, that it stopped short (search stops at 1000).
        let len = self.items.len() as u64;
        let counted = self.total > len;
        match self.next {
            Some(_) => Rest::NextPage {
                total: counted.then_some(self.total),
            },
            None if counted => Rest::OnGitHub(self.total - len),
            None => Rest::All,
        }
    }
}

/// One row per item, ruled apart, then what the list left out: "Load
/// more" while there's another page, or how many are only on GitHub.
fn box_rows<T>(page: &mut Page, list: &impl Rows<T>, mut row: impl FnMut(&mut Page, &T)) {
    let items = list.rows();
    for (n, item) in items.iter().enumerate() {
        if n > 0 && !page.compact {
            page.box_rule();
        }
        row(page, item);
    }
    rest_row(page, items.len(), list.rest());
}

/// After a list's `shown` rows, what it left out.
fn rest_row(page: &mut Page, shown: usize, rest: Rest) {
    let (target, text) = match rest {
        Rest::All => return,
        Rest::OnGitHub(n) => {
            page.box_rule();
            return empty_row(page, &left_out_text(n, "more", "more"));
        }
        Rest::Uncounted => {
            page.box_rule();
            let text = format!("… GitHub has more: the first {shown} are shown");
            return empty_row(page, &text);
        }
        Rest::At(n, url) => (Link::from(url), format!("… {n} more")),
        Rest::NextPage { total } => {
            let of = total.map_or_else(
                || format!("{shown} so far"),
                |total| format!("{shown} of {}", compact(total)),
            );
            (Link::More, format!("Load more  ({of})"))
        }
    };
    page.box_rule();
    item(page, target, |page, link| {
        body(page, vec![Seg::linked(text, Role::Link, link)]);
    });
}

/// A list's count: `45`, `20 of 45` when the rest are only on GitHub, or
/// GitHub's total (`20+` uncounted) while more pages load.
fn count_of<T>(list: &impl Rows<T>) -> String {
    let len = list.rows().len();
    match list.rest() {
        Rest::All => len.to_string(),
        Rest::OnGitHub(n) | Rest::At(n, _) => format!("{len} of {}", len as u64 + n),
        Rest::NextPage { total: Some(total) } => compact(total),
        Rest::NextPage { total: None } | Rest::Uncounted => format!("{len}+"),
    }
}

/// `… 12 more replies on GitHub`: what a list left out.
pub(crate) fn left_out_text(n: u64, one: &str, many: &str) -> String {
    format!("… {n} {} on GitHub", if n == 1 { one } else { many })
}

/// A box of a list, a row each, saying `empty` when there are none and
/// what it left out.
fn list_box<T>(
    page: &mut Page,
    title: Vec<Seg>,
    right: Vec<Seg>,
    list: &impl Rows<T>,
    empty: &str,
    row: impl FnMut(&mut Page, &T),
) {
    page.box_top(title, right);
    if list.rows().is_empty() {
        empty_row(page, empty);
    }
    box_rows(page, list, row);
    page.box_bottom();
}

/// A list, once it's loaded, in a box titled `name` and its total.
fn paged_list<T>(
    page: &mut Page,
    name: &str,
    list: Fetched<'_, Results<T>>,
    empty: &str,
    row: impl FnMut(&mut Page, &T),
) {
    let Some(l) = list.show(page, &name.to_lowercase()) else {
        return;
    };
    let title = format!("{name}  {}", count_of(l));
    list_box(
        page,
        vec![Seg::new(title, Role::Strong)],
        Vec::new(),
        l,
        empty,
        row,
    );
}

/// `prefix` then `segs` wrapped inside a box, continuation lines aligned
/// after the prefix.
fn hanging(page: &mut Page, prefix: Seg, segs: Vec<Seg>, frame: Frame) {
    let width = cols(crate::ui::text::width(prefix.text()));
    let start = page.lines.len();
    page.wrapped(segs, width, frame);
    if let Some(first) = page.lines.get_mut(start) {
        first.indent = 0;
        first.segs.insert(0, prefix);
    }
}

fn issue_icon(icons: Icons, state: Status) -> (String, Role) {
    let nerd = icons.nerd_font;
    let pr = |s| icons.pr_state(s).into();
    match state {
        Status::Issue(IssueState::Open) => {
            (if nerd { "\u{f41b}" } else { "◉" }.into(), Role::Success)
        }
        Status::Issue(IssueState::Closed) => {
            (if nerd { "\u{f41d}" } else { "✓" }.into(), Role::Accent)
        }
        Status::Issue(IssueState::NotPlanned) => {
            (if nerd { "\u{f468}" } else { "⊘" }.into(), Role::Meta)
        }
        Status::Pr(s @ PrState::Open) => (pr(s), Role::Success),
        Status::Pr(s @ PrState::Draft) => (pr(s), Role::Meta),
        Status::Pr(s @ PrState::Merged) => (pr(s), Role::Accent),
        Status::Pr(s @ PrState::Closed) => (pr(s), Role::Error),
    }
}

/// An issue's or pull request's state, as its chip says it.
pub fn state_label(state: Status) -> (&'static str, Bg) {
    match state {
        Status::Issue(IssueState::Open) | Status::Pr(PrState::Open) => {
            ("Open", Bg::SuccessContainer)
        }
        Status::Pr(PrState::Draft) => ("Draft", Bg::SecondaryContainer),
        Status::Pr(PrState::Merged) => ("Merged", Bg::TertiaryContainer),
        Status::Issue(IssueState::NotPlanned) => ("Closed as not planned", Bg::SecondaryContainer),
        Status::Pr(PrState::Closed) => ("Closed", Bg::ErrorContainer),
        Status::Issue(IssueState::Closed) => ("Closed", Bg::TertiaryContainer),
    }
}

fn state_chip(state: Status, icons: Icons) -> Seg {
    let (icon, _) = issue_icon(icons, state);
    let (text, bg) = state_label(state);
    chip(format!("{icon} {text}"), bg)
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `2026-10-01T…` → "Oct 1, 2026".
fn day(iso: &str) -> String {
    let parts: Vec<&str> = iso.get(..10).unwrap_or(iso).split('-').collect();
    match parts.as_slice() {
        [y, m, d] => {
            let month = m
                .parse::<usize>()
                .ok()
                .and_then(|m| MONTHS.get(m.wrapping_sub(1)))
                .copied()
                .unwrap_or("?");
            format!("{month} {}, {y}", d.trim_start_matches('0'))
        }
        _ => iso.to_owned(),
    }
}

/// Keys shown on the page where GitHub shows buttons.
#[derive(Debug, Clone, Copy, Default)]
pub struct Keys<'a> {
    pub comment: &'a str,
    pub find_file: &'a str,
    pub archive: &'a str,
    pub filter: &'a str,
}

/// What every page builder needs besides its data.
#[derive(Debug, Clone, Copy, Default)]
pub struct PageCtx<'a> {
    pub icons: Icons,
    pub keys: Keys<'a>,
    /// Unix seconds, for relative times.
    pub now: u64,
}

/// A directory's files, at a revision, with each one's latest commit.
#[derive(Debug, Clone, Copy)]
pub struct Listing<'a> {
    pub repo: &'a RepoId,
    pub rev: &'a str,
    pub path: &'a str,
    pub entries: Fetched<'a, [TreeEntry]>,
    pub commits: Option<&'a HashMap<String, CommitInfo>>,
}

fn aside_heading(page: &mut Page, text: &str) {
    page.blank();
    page.line(vec![Seg::new(text, Role::Strong)]);
}

/// What a conversation leaves out: `… 12 more replies on GitHub (o)`.
fn more_here<T>(page: &mut Page, list: &Capped<T>, one: &str, many: &str) {
    let (one, many) = (format!("more {one}"), format!("more {many}"));
    left_out(page, list.left_out(), &one, &many);
}

/// The older part of a conversation that isn't shown (only its newest
/// comments are fetched): `… 120 earlier comments on GitHub (o)`.
fn earlier_here<T>(page: &mut Page, list: &Capped<T>, one: &str, many: &str) {
    let (one, many) = (format!("earlier {one}"), format!("earlier {many}"));
    left_out(page, list.left_out(), &one, &many);
}

fn left_out(page: &mut Page, n: u64, one: &str, many: &str) {
    if n > 0 {
        let text = left_out_text(n, one, many);
        page.wrapped(vec![Seg::new(text, Role::Meta)], 2, Frame::None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_numbers() {
        assert_eq!(compact(999), "999");
        assert_eq!(compact(1_000), "1k");
        assert_eq!(compact(1_234), "1.2k");
        assert_eq!(compact(2_500_000), "2.5m");
    }

    #[test]
    fn urls() {
        let repo = RepoId::new("o", "r");
        assert_eq!(
            url::tree(&repo, "main", ""),
            "https://github.com/o/r/tree/main"
        );
        assert_eq!(
            url::blob(&repo, "main", "a/b.rs"),
            "https://github.com/o/r/blob/main/a/b.rs"
        );
        assert_eq!(
            url::search(SearchKind::Issues, "is:open ratatui"),
            "https://github.com/search?q=is:open+ratatui&type=issues"
        );
    }

    /// Links to a profile: a URL one name below GitHub's.
    fn profile_links(page: &Page) -> Vec<String> {
        page.links
            .iter()
            .filter_map(|l| l.url())
            .filter(|u| {
                let path = u.strip_prefix(&format!("{}/", url::base()));
                path.is_some_and(|p| !p.is_empty() && !p.contains('/'))
            })
            .map(str::to_owned)
            .collect()
    }

    /// A commit by `author`, committed by `committer`.
    fn commit_by(author: Person, committer: Option<Person>) -> CommitDetail {
        CommitDetail {
            oid: "0a1b2c3d4e5f60718293a4b5c6d7e8f901234567".into(),
            headline: "Fix the thing".into(),
            body: String::new(),
            author,
            authored_at: "2026-01-14T10:00:00Z".into(),
            committer,
            committed_at: "2026-01-15T09:30:00Z".into(),
            parents: Vec::new().into(),
            additions: 1,
            deletions: 1,
            changed_files: Some(1),
            verified: None,
        }
    }

    /// A commit by an email without an account is by its git name, which
    /// isn't a login: neither its author nor its committer links to a
    /// profile, as GitHub shows them.
    #[test]
    fn a_commit_by_git_names_doesnt_link_them_as_profiles() {
        let repo = RepoId::new("o", "r");
        let d = commit_by(
            Person::Git("Jane Doe".into()),
            Some(Person::Git("bob".into())),
        );
        let mut page = Page::new(100);
        commit(&mut page, &repo, &d, "", 0);
        let text: Vec<String> = page.lines.iter().map(PageLine::text).collect();
        assert!(text.iter().any(|l| l.contains("Jane Doe")), "{text:?}");
        assert_eq!(profile_links(&page), Vec::<String>::new());
    }

    /// A commit with no author at all reads "unknown", which isn't the
    /// GitHub user of that name.
    #[test]
    fn a_commit_by_nobody_doesnt_link_unknown_as_a_profile() {
        let d = commit_by(Person::Unknown, None);
        let mut page = Page::new(100);
        commit(&mut page, &RepoId::new("o", "r"), &d, "", 0);
        let text: Vec<String> = page.lines.iter().map(PageLine::text).collect();
        assert!(
            text.iter().any(|l| l.contains("unknown authored")),
            "{text:?}"
        );
        let urls: Vec<&str> = page.links.iter().filter_map(|l| l.url()).collect();
        assert!(!urls.iter().any(|u| u.ends_with("/unknown")), "{urls:?}");
    }

    /// The repo's last commit, by an email without an account, shows its
    /// author's git name without linking it as a profile.
    #[test]
    fn a_last_commit_by_a_git_name_doesnt_link_it_as_a_profile() {
        let repo = RepoId::new("o", "r");
        let overview = RepoOverview {
            summary: RepoSummary {
                repo: repo.clone(),
                ..Default::default()
            },
            default_branch: Some("main".into()),
            last_commit: Some(CommitInfo {
                oid: "0a1b2c3d4e5f60718293a4b5c6d7e8f901234567".into(),
                headline: "Fix the thing".into(),
                author: Person::Git("Jane Doe".into()),
                date: "2026-01-14T10:00:00Z".into(),
            }),
            commits: 1,
            id: crate::api::model::NodeId::new("R_1"),
            has_issues: true,
            branches: 1,
            entries: vec![TreeEntry {
                name: "a.rs".into(),
                path: "a.rs".into(),
                kind: EntryKind::File,
                size: Some(1),
            }],
            ..Default::default()
        };
        let mut page = Page::new(100);
        let readme = None;
        let readme = Fetched::ready(&readme);
        repo_code(
            &mut page,
            &repo,
            &overview,
            None,
            readme,
            PageCtx::default(),
        );
        let text: Vec<String> = page.lines.iter().map(PageLine::text).collect();
        assert!(text.iter().any(|l| l.contains("Jane Doe")), "{text:?}");
        assert_eq!(profile_links(&page), Vec::<String>::new());
    }
}
